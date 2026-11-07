use crate::{
    accounts::{self, Account, Credentials},
    browser::{Preparation, Redeemer},
    config::Config,
    notifications::Notifier,
    store::{Job, Status, Store, now},
};
use anyhow::{Context, Result};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct WorkerLock(File);
impl WorkerLock {
    pub fn acquire(dir: &Path) -> Result<Self> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("worker.lock"))?;
        f.try_lock_exclusive()
            .context("已有兌換程序執行中；請稍後再試")?;
        Ok(Self(f))
    }
}
impl Drop for WorkerLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub trait CredentialProvider {
    fn get(&self, account: &Account) -> Result<Credentials>;
}
pub struct SystemCredentials;
impl CredentialProvider for SystemCredentials {
    fn get(&self, account: &Account) -> Result<Credentials> {
        accounts::credentials(account)
    }
}
pub struct Worker<'a> {
    pub store: &'a mut Store,
    pub config: &'a Config,
    pub redeemer: &'a mut dyn Redeemer,
    pub credentials: &'a dyn CredentialProvider,
    pub notifier: Arc<dyn Notifier>,
    pub cancel: CancellationToken,
}
impl Worker<'_> {
    /// A single bounded sweep. Caller holds WorkerLock; daemon repeats sweeps to service next_at.
    pub async fn run(
        &mut self,
        accounts: &[Account],
        code_filter: Option<&str>,
        account_filter: Option<&str>,
    ) -> Result<usize> {
        self.run_scoped(accounts, code_filter, account_filter, None)
            .await
    }
    pub async fn run_scoped(
        &mut self,
        accounts: &[Account],
        code_filter: Option<&str>,
        account_filter: Option<&str>,
        daemon_backfill: Option<bool>,
    ) -> Result<usize> {
        let jobs = self.store.jobs()?;
        let mut processed = 0;
        let mut summary = Vec::new();
        for job in jobs {
            if self.cancel.is_cancelled() {
                break;
            }
            if code_filter.is_some_and(|code| code != job.code)
                || account_filter.is_some_and(|id| id != job.account_id)
            {
                continue;
            }
            if let Some(backfill) = daemon_backfill
                && !self.store.daemon_job_allowed(job.id, backfill)?
            {
                continue;
            }
            if !self.store.claim(job.id, self.config.max_attempts)? {
                continue;
            }
            let result = self.process(&job, accounts).await;
            self.redeemer.close().await;
            result?;
            let updated = self
                .store
                .jobs()?
                .into_iter()
                .find(|j| j.id == job.id)
                .context("工作遺失")?;
            println!(
                "{} / {}：{}（{}）",
                updated.code, updated.alias, updated.status, updated.detail
            );
            summary.push(format!(
                "{} / {}: {}",
                updated.code, updated.alias, updated.status
            ));
            if updated.status == "success" {
                self.notifier.send(
                    "TRserial — 兌換成功",
                    &format!(
                        "{} / {}\n網站已確認成功；訊息中的獎勵尚未核對。",
                        job.code, job.alias
                    ),
                );
            } else if ["failed", "unknown"].contains(&updated.status.as_str()) {
                self.notifier.send(
                    "TRserial — 請檢查兌換結果",
                    &format!("{} / {}: {}", job.code, job.alias, updated.status),
                );
            }
            processed += 1;
            tokio::select! {_=self.cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(self.config.delay_seconds))=>{}}
        }
        if !summary.is_empty() {
            self.notifier
                .send("TRserial — 本次處理結果", &summary.join("\n"));
        }
        Ok(processed)
    }
    async fn process(&mut self, job: &Job, accounts: &[Account]) -> Result<()> {
        let Some(account) = accounts.iter().find(|a| a.id == job.account_id) else {
            return self.store.finish(job.id, Status::Failed, "帳號已移除", 0);
        };
        if self.redeemer.is_live() && (account.test_only || self.store.test_only(&job.code)?) {
            return self.store.finish(
                job.id,
                Status::Failed,
                "測試帳號或 mock 序號僅供本機測試，未開啟瀏覽器",
                0,
            );
        }
        let credentials = match self.credentials.get(account) {
            Ok(c) => c,
            Err(_) => {
                return self.store.finish(
                    job.id,
                    Status::Failed,
                    "無法安全取得密碼；請檢查憑證庫或使用互動終端",
                    0,
                );
            }
        };
        let prepare = tokio::select! {
            _=self.cancel.cancelled()=>return self.store.finish(job.id,Status::Pending,"送出前取消",0),
            r=tokio::time::timeout(Duration::from_secs(self.config.timeout_seconds),self.redeemer.prepare(&credentials,&job.code))=>r,
        };
        drop(credentials);
        let prep = match prepare {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => return self.pre_submit_error(job, e.transient, &e.detail),
            Err(_) => return self.pre_submit_error(job, true, "瀏覽器準備逾時，尚未送出"),
        };
        if prep == Preparation::WaitingVerification {
            if self.config.headless {
                return self.store.finish(
                    job.id,
                    Status::Failed,
                    "需要人工驗證；請設定 headless=false 後重試，尚未送出",
                    0,
                );
            }
            self.store.waiting(job.id)?;
            self.notifier.send(
                "TRserial — 需要人工驗證",
                &format!(
                    "{} / {}：請在瀏覽器完成驗證，程式會在驗證通過後繼續。",
                    job.code, job.alias
                ),
            );
            println!("{}：請在瀏覽器完成驗證；Ctrl+C 可安全暫停。", job.alias);
            let deadline = tokio::time::Instant::now()
                + Duration::from_secs(self.config.verification_timeout_seconds);
            loop {
                tokio::select! {_=self.cancel.cancelled()=>return self.store.finish(job.id,Status::Pending,"驗證期間取消；尚未送出",0),_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
                if tokio::time::Instant::now() >= deadline {
                    return self.store.finish(
                        job.id,
                        Status::Failed,
                        "人工驗證逾時；請用有視窗模式完成 Cloudflare 驗證後重試",
                        0,
                    );
                }
                match tokio::time::timeout(
                    Duration::from_secs(self.config.timeout_seconds),
                    self.redeemer.verification_ready(),
                )
                .await
                {
                    Ok(Ok(true)) => break,
                    Ok(Ok(false)) => {}
                    _ => {
                        return self.store.finish(
                            job.id,
                            Status::Failed,
                            "無法確認人工驗證；尚未送出",
                            0,
                        );
                    }
                }
            }
        }
        if self.cancel.is_cancelled() {
            return self.store.finish(job.id, Status::Pending, "送出前取消", 0);
        }
        // Durable marker MUST precede the browser action. Recovery errs on the side of no resubmission.
        self.store.mark_submitted(job.id)?;
        let result = tokio::select! {
            _=self.cancel.cancelled()=>None,
            r=tokio::time::timeout(Duration::from_secs(self.config.timeout_seconds+5),self.redeemer.submit())=>Some(r),
        };
        match result {
            Some(Ok(Ok(outcome)))
                if matches!(
                    outcome.status,
                    Status::Success
                        | Status::AlreadyRedeemed
                        | Status::Expired
                        | Status::Failed
                        | Status::Unknown
                ) =>
            {
                self.store.finish(job.id, outcome.status, outcome.detail, 0)
            }
            _ => self.store.finish(
                job.id,
                Status::Unknown,
                "送出後中斷、逾時或取消；請人工確認後再決定是否重試",
                0,
            ),
        }
    }
    fn pre_submit_error(&mut self, job: &Job, transient: bool, detail: &str) -> Result<()> {
        let attempt = job.attempts + 1;
        if transient && attempt < self.config.max_attempts {
            self.store.finish(
                job.id,
                Status::Pending,
                detail,
                now() + backoff(self.config.retry_base_seconds, attempt) as i64,
            )
        } else {
            self.store.finish(job.id, Status::Failed, detail, 0)
        }
    }
}
pub fn backoff(base: u64, attempt: u32) -> u64 {
    base.saturating_mul(
        1u64.checked_shl(attempt.saturating_sub(1).min(20))
            .unwrap_or(u64::MAX),
    )
    .min(3600)
}
