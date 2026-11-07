//! Shared application operations for command-line and desktop clients.
use crate::{
    accounts::{Account, Accounts},
    browser::{BrowserRedeemer, Redeemer},
    config::{self, Config},
    notifications, parser,
    source::{self, ManualSource},
    store::{CodeRecord, Job, Store},
    worker::{CredentialProvider, Worker, WorkerLock},
};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
#[non_exhaustive]
pub struct AccountRecord {
    pub account: Account,
    pub automatic: bool,
}

#[derive(Clone)]
#[non_exhaustive]
pub struct Snapshot {
    pub accounts: Vec<AccountRecord>,
    pub codes: Vec<CodeRecord>,
    pub jobs: Vec<Job>,
    pub config: Config,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RedemptionPlan {
    pub code: String,
    pub accounts: Vec<(String, String)>,
    pub historical: bool,
}
impl RedemptionPlan {
    pub fn needs_confirmation(&self) -> bool {
        self.historical || self.accounts.len() > 1
    }
}

/// Own on an I/O thread. UI components consume snapshots, never connections or SQL.
pub struct Service {
    dir: PathBuf,
    config_path: PathBuf,
    config: Config,
    accounts: Accounts,
    store: Store,
}
impl Service {
    pub fn open(dir: &Path, config_path: &Path) -> Result<Self> {
        config::prepare_dir(dir)?;
        let config = Config::load(config_path)?;
        let accounts = Accounts::open(dir)?;
        let mut store = Store::open(&dir.join("history.sqlite"))?;
        // Establish the safe first-run boundary before any GUI imports.
        store.initialize_daemon(&accounts.list()?)?;
        Ok(Self {
            dir: dir.into(),
            config_path: config_path.into(),
            config,
            accounts,
            store,
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let accounts = self
            .accounts
            .list()?
            .into_iter()
            .map(|account| {
                Ok(AccountRecord {
                    automatic: self.accounts.automatic(&account.id)?,
                    account,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Snapshot {
            accounts,
            codes: self.store.code_records()?,
            jobs: self.store.jobs()?,
            config: self.config.clone(),
        })
    }
    pub fn ingest(&mut self, text: String) -> Result<usize> {
        source::ingest_source(&mut self.store, &mut ManualSource(text))
    }
    pub fn ingest_mock(&mut self, text: String) -> Result<usize> {
        source::ingest(
            &mut self.store,
            &source::Message {
                source: "mock:desktop".into(),
                id: None,
                text,
            },
        )
    }
    pub fn approve(&self, code: &str) -> Result<()> {
        self.store.approve(code)
    }
    pub fn add_account(&mut self, alias: &str, username: &str, password: &str) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        let account = self.accounts.add(alias, username, password, false)?;
        self.store.reset_baseline(&account.id)
    }
    pub fn add_test_account(&mut self, alias: &str) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        self.accounts.add_test(alias)?;
        self.store.initialize_daemon(&self.accounts.list()?)
    }
    pub fn rename_account(&self, id: &str, alias: &str) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        self.accounts.rename(id, alias)
    }
    pub fn update_password(&self, id: &str, password: &str) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        self.accounts.update_password(id, password)
    }
    pub fn remove_account(&self, id: &str) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        let account = self
            .accounts
            .list()?
            .into_iter()
            .find(|a| a.id == id)
            .context("找不到帳號")?;
        self.accounts.remove(&account.alias)
    }
    pub fn set_automatic(&self, id: &str, enabled: bool) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        // Re-enabling never silently redeems the backlog accumulated while disabled.
        if enabled && !self.accounts.automatic(id)? {
            self.store.reset_baseline(id)?;
        }
        self.accounts.set_automatic(id, enabled)
    }
    pub fn plan(&self, code: &str, ids: &[String]) -> Result<RedemptionPlan> {
        ensure!(parser::valid_code(code), "序號格式不正確");
        let record = self
            .store
            .code_records()?
            .into_iter()
            .find(|c| c.code == code)
            .context("請先匯入序號")?;
        ensure!(!record.needs_review, "請先確認候選序號");
        ensure!(!record.test_only, "mock 序號僅供本機測試");
        let all = self.accounts.list()?;
        let mut accounts = Vec::new();
        let mut historical = false;
        for id in ids {
            if accounts.iter().any(|(existing, _)| existing == id) {
                continue;
            }
            let a = all
                .iter()
                .find(|a| a.id == *id)
                .context("帳號已移除，請重新選取")?;
            ensure!(!a.test_only, "測試帳號僅供離線測試");
            historical |= self.store.historical_for(code, &a.id)?;
            accounts.push((a.id.clone(), a.alias.clone()));
        }
        ensure!(!accounts.is_empty(), "請選取至少一個帳號");
        Ok(RedemptionPlan {
            code: code.into(),
            accounts,
            historical,
        })
    }
    pub fn enqueue(&mut self, plan: &RedemptionPlan, confirmed: bool) -> Result<usize> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        // Revalidate after the confirmation, because account/configuration state may have changed.
        let current = self.plan(
            &plan.code,
            &plan
                .accounts
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        )?;
        ensure!(
            confirmed || !current.needs_confirmation(),
            "多帳號或歷史序號兌換需要明確確認"
        );
        let accounts = self
            .accounts
            .list()?
            .into_iter()
            .filter(|a| current.accounts.iter().any(|(id, _)| id == &a.id))
            .collect::<Vec<_>>();
        self.store.enqueue(&plan.code, &accounts)
    }
    pub fn retry(&self, id: i64, confirmed: bool) -> Result<()> {
        ensure!(confirmed, "重試前需確認網站結果，避免重複兌換");
        let _lock = WorkerLock::acquire(&self.dir)?;
        self.store.retry_reviewed(id)
    }
    pub fn attempts(&self, id: i64) -> Result<Vec<crate::store::AttemptRecord>> {
        self.store.job_attempts(id)
    }
    pub fn save_config(&mut self, config: Config) -> Result<()> {
        let _lock = WorkerLock::acquire(&self.dir)?;
        config.validate()?;
        let text = toml::to_string_pretty(&config)?;
        let temporary = self
            .config_path
            .with_file_name(format!(".trserial-config-{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, text).context("無法寫入設定檔")?;
        std::fs::rename(&temporary, &self.config_path).context("無法更新設定檔")?;
        self.config = config;
        Ok(())
    }
}

/// One browser/driver lifecycle shared by CLI and GUI orchestration.
pub struct RedemptionSession {
    config: Config,
    browser: BrowserRedeemer,
}
#[derive(Default)]
pub struct RunScope<'a> {
    code: Option<&'a str>,
    account: Option<&'a str>,
    backfill: Option<bool>,
}
impl<'a> RunScope<'a> {
    pub fn manual(code: Option<&'a str>, account: Option<&'a str>) -> Self {
        Self {
            code,
            account,
            backfill: None,
        }
    }
    pub fn automatic(backfill: bool) -> Self {
        Self {
            backfill: Some(backfill),
            ..Self::default()
        }
    }
}
impl RedemptionSession {
    pub fn new(config: Config) -> Self {
        Self {
            browser: BrowserRedeemer::new(config.clone()),
            config,
        }
    }
    pub async fn sweep(
        &mut self,
        store: &mut Store,
        accounts: &[Account],
        scope: RunScope<'_>,
        cancel: CancellationToken,
        credentials: &dyn CredentialProvider,
    ) -> Result<usize> {
        Worker {
            store,
            config: &self.config,
            redeemer: &mut self.browser,
            credentials,
            notifier: notifications::desktop(self.config.notifications),
            cancel,
        }
        .run_scoped(accounts, scope.code, scope.account, scope.backfill)
        .await
    }
    pub async fn shutdown(&mut self) {
        self.browser.shutdown().await;
    }
}
