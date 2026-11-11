use crate::{platform, update};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use trserial::{
    accounts::Accounts,
    browser::{BrowserRedeemer, Redeemer},
    config::Config,
    service::{RedemptionPlan, RedemptionSession, RunScope, Service, Snapshot},
    store::{AttemptRecord, Store},
    worker::{StoredCredentials, WorkerLock},
};
use zeroize::Zeroizing;

fn new_confirmed<'a>(
    before: Option<&Snapshot>,
    after: &'a Snapshot,
) -> Vec<&'a trserial::store::CodeRecord> {
    let Some(before) = before else {
        return Vec::new();
    };
    after
        .codes
        .iter()
        .filter(|code| {
            !code.needs_review
                && !code.test_only
                && before
                    .codes
                    .iter()
                    .find(|old| old.code == code.code)
                    .is_none_or(|old| old.needs_review)
        })
        .collect()
}
fn publish_records(handle: &Backend, data: Snapshot) {
    let previous = handle.snapshot().data;
    for code in new_confirmed(previous.as_deref(), &data) {
        trserial::notifications::desktop(data.config.notifications).send(
            "TRserial — 新序號",
            &format!("{}\n{}", code.code, code.rewards.join("、")),
        );
    }
    handle.publish(|p| p.data = Some(Arc::new(data)));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RunState {
    #[default]
    Stopped,
    Running,
    Manual,
    Checking,
    Pausing,
    Paused,
    Stopping,
    Quitting,
}
impl RunState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Stopped => "已停止",
            Self::Running => "自動處理中",
            Self::Manual => "兌換中",
            Self::Checking => "檢查瀏覽器中",
            Self::Pausing => "正在暫停",
            Self::Paused => "已暫停",
            Self::Stopping => "正在停止",
            Self::Quitting => "正在結束",
        }
    }
    pub(crate) fn is_busy(self) -> bool {
        !matches!(self, Self::Stopped | Self::Paused)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Preferences {
    pub(crate) process_on_launch: bool,
    pub(crate) start_on_login: bool,
    pub(crate) dark: bool,
    /// Ask GitHub for a newer release when the app starts.
    pub(crate) check_updates: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            process_on_launch: false,
            start_on_login: false,
            dark: false,
            check_updates: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available(update::Release),
    Downloading {
        release: update::Release,
        written: u64,
        total: u64,
    },
    /// The new files are in place (Linux) or the installer is running
    /// (Windows); the app exits and, on Linux, starts the new version.
    Ready {
        relaunch: bool,
    },
}

#[derive(Clone, Default)]
pub(crate) struct Presentation {
    pub(crate) revision: u64,
    pub(crate) data: Option<Arc<Snapshot>>,
    pub(crate) state: RunState,
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<String>,
    pub(crate) plan: Option<(u64, RedemptionPlan)>,
    pub(crate) attempts: Option<(u64, i64, Vec<AttemptRecord>)>,
    pub(crate) browser_report: Option<String>,
    pub(crate) preferences: Preferences,
    pub(crate) tray_host: bool,
    pub(crate) closed: bool,
    pub(crate) offline: bool,
    pub(crate) smoke_complete: bool,
    pub(crate) update: UpdateState,
    pub(crate) update_method: update::Method,
}
impl Presentation {
    pub(crate) fn browser_status(&self) -> String {
        if let Some(job) = self.data.as_ref().and_then(|d| {
            d.jobs
                .iter()
                .find(|j| matches!(j.status.as_str(), "processing" | "waiting_verification"))
        }) {
            let phase = if job.status == "waiting_verification" {
                "請在 Chrome 完成人工驗證"
            } else if job.submitted {
                "等待網站回覆"
            } else {
                "準備瀏覽器與表單"
            };
            format!("{}／{}：{phase}", job.code, job.alias)
        } else if self.state == RunState::Checking {
            "正在檢查 Chrome 與官方表單".into()
        } else {
            "瀏覽器待命".into()
        }
    }
}

// Intentionally no Debug: password-bearing commands must never appear in logs.
pub(crate) enum Command {
    Ingest(String),
    Approve(String),
    AddAccount {
        alias: String,
        username: String,
        password: Zeroizing<String>,
    },
    Rename {
        id: String,
        alias: String,
    },
    Password {
        id: String,
        password: Zeroizing<String>,
    },
    Remove(String),
    Automatic {
        id: String,
        enabled: bool,
    },
    Plan {
        code: String,
        accounts: Vec<String>,
    },
    Redeem {
        plan: RedemptionPlan,
        confirmed: bool,
    },
    Retry {
        job: i64,
        confirmed: bool,
    },
    Attempts(i64),
    Start {
        backfill: bool,
        confirmed: bool,
    },
    Pause,
    Stop,
    CheckBrowser,
    Save {
        config: Config,
        preferences: Preferences,
    },
    Dismiss,
    CheckUpdate {
        interactive: bool,
    },
    InstallUpdate,
}

#[derive(Clone)]
pub(crate) struct Backend {
    sender: mpsc::UnboundedSender<Command>,
    presentation: Arc<Mutex<Presentation>>,
    quit: CancellationToken,
    closed: CancellationToken,
}
impl Backend {
    pub(crate) fn start(dir: PathBuf, config_path: PathBuf, offline: bool) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let backend = Self {
            sender,
            presentation: Arc::new(Mutex::new(Presentation {
                offline,
                ..Presentation::default()
            })),
            quit: CancellationToken::new(),
            closed: CancellationToken::new(),
        };
        let handle = backend.clone();
        std::thread::Builder::new()
            .name("trserial-desktop-io".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(run(dir, config_path, offline, receiver, &handle))
                })();
                handle.publish(|p| {
                    if let Err(error) = result {
                        p.error = Some(error.to_string());
                    }
                    p.closed = true;
                    p.busy = false;
                    p.state = RunState::Stopped;
                });
                handle.closed.cancel();
            })
            .expect("無法啟動背景工作執行緒");
        backend
    }
    pub(crate) fn send(&self, command: Command) {
        if self.sender.send(command).is_err() {
            self.publish(|p| p.error = Some("背景服務已停止，請重新開啟 TRserial".into()));
        }
    }
    pub(crate) fn snapshot(&self) -> Presentation {
        self.presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn publish(&self, update: impl FnOnce(&mut Presentation)) {
        let mut state = self
            .presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(&mut state);
        state.revision = state.revision.wrapping_add(1);
    }
    pub(crate) fn consume_plan(&self, serial: u64) -> Option<RedemptionPlan> {
        let mut state = self
            .presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.plan.as_ref().is_some_and(|(id, _)| *id == serial) {
            state.revision = state.revision.wrapping_add(1);
            state.plan.take().map(|(_, plan)| plan)
        } else {
            None
        }
    }
    pub(crate) fn smoke_complete(&self) {
        self.publish(|p| p.smoke_complete = true);
    }
    pub(crate) fn request_quit(&self) {
        self.quit.cancel();
    }
    pub(crate) fn smoke_error(&self, error: String) {
        self.publish(|p| p.error = Some(error));
    }
    pub(crate) async fn wait_closed(&self) {
        self.closed.cancelled().await;
    }
}

struct Active {
    cancel: CancellationToken,
    task: JoinHandle<Result<Option<String>>>,
    finished: RunState,
}
impl Active {
    fn cancel(&mut self, finished: RunState) {
        self.finished = finished;
        self.cancel.cancel();
    }
}

struct Actor<'a> {
    _instance: std::fs::File,
    dir: PathBuf,
    config_path: PathBuf,
    pref_path: PathBuf,
    offline: bool,
    service: Service,
    preferences: Preferences,
    active: Option<Active>,
    serial: u64,
    handle: &'a Backend,
    /// Resolved at startup: on Linux an update replaces this file, after which
    /// `current_exe` would name the deleted original.
    executable: Option<PathBuf>,
    update_method: update::Method,
    update_task: Option<JoinHandle<()>>,
}
impl<'a> Actor<'a> {
    fn new(dir: PathBuf, config_path: PathBuf, offline: bool, handle: &'a Backend) -> Result<Self> {
        trserial::config::prepare_dir(&dir)?;
        let instance = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("desktop.lock"))?;
        instance
            .try_lock_exclusive()
            .context("這個資料目錄已有 TRserial 桌面版執行中，請從系統匣開啟")?;
        let mut service = Service::open(&dir, &config_path)?;
        let pref_path = dir.join("desktop.toml");
        let mut preferences: Preferences = if pref_path.exists() {
            toml::from_str(&std::fs::read_to_string(&pref_path)?)
                .map_err(|_| anyhow::anyhow!("desktop.toml 格式錯誤"))?
        } else {
            Preferences::default()
        };
        if offline {
            let data = service.snapshot()?;
            ensure!(
                data.accounts.is_empty() && data.codes.is_empty() && data.jobs.is_empty(),
                "離線 GUI 測試需要全新的資料目錄"
            );
            let mut config = data.config.clone();
            config.notifications = false;
            config.discord.enabled = false;
            service.save_config(config)?;
            preferences.process_on_launch = false;
        }
        preferences.start_on_login = platform::login_enabled();
        let executable = std::env::current_exe().and_then(std::fs::canonicalize).ok();
        let update_method = match executable.as_deref() {
            Some(path) if !offline => update::method(path),
            _ => update::Method::Manual,
        };
        let data = service.snapshot()?;
        handle.publish(|p| {
            p.data = Some(Arc::new(data));
            p.preferences = preferences.clone();
            p.update_method = update_method;
        });
        Ok(Self {
            _instance: instance,
            dir,
            config_path,
            pref_path,
            offline,
            service,
            preferences,
            active: None,
            serial: 0,
            handle,
            executable,
            update_method,
            update_task: None,
        })
    }
    fn installing_update(&self) -> bool {
        matches!(
            self.handle.snapshot().update,
            UpdateState::Downloading { .. } | UpdateState::Ready { .. }
        )
    }
    fn updating(&self) -> bool {
        self.update_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }
    /// Background check; `interactive` reports "up to date" and failures, while
    /// the startup check stays silent when GitHub is unreachable.
    fn check_update(&mut self, interactive: bool) -> Result<()> {
        ensure!(!self.offline, "離線測試不檢查更新");
        if self.updating() {
            return Ok(());
        }
        let handle = self.handle.clone();
        handle.publish(|p| p.update = UpdateState::Checking);
        self.update_task = Some(tokio::spawn(async move {
            let result = match update::client() {
                Ok(client) => update::check(&client).await,
                Err(error) => Err(error),
            };
            handle.publish(|p| {
                p.update = UpdateState::Idle;
                match result {
                    Ok(Some(release)) => p.update = UpdateState::Available(release),
                    Ok(None) if interactive => {
                        p.notice = Some(format!("已是最新版本 {}", update::current()));
                    }
                    Err(error) if interactive => {
                        p.error = Some(format!("檢查更新失敗：{error:#}"));
                    }
                    _ => {}
                }
            });
        }));
        Ok(())
    }
    fn install_update(&mut self) -> Result<()> {
        ensure!(!self.offline, "離線測試不安裝更新");
        ensure!(!self.updating(), "更新正在進行中");
        ensure!(
            self.active.is_none(),
            "請先停止自動處理或目前的兌換，再安裝更新"
        );
        update::ensure_supported(self.update_method)?;
        let UpdateState::Available(release) = self.handle.snapshot().update else {
            anyhow::bail!("目前沒有可安裝的更新");
        };
        let executable = self.executable.clone().context("無法取得程式位置")?;
        let method = self.update_method;
        let handle = self.handle.clone();
        handle.publish(|p| {
            p.update = UpdateState::Downloading {
                release: release.clone(),
                written: 0,
                total: release.asset.as_ref().map_or(0, |asset| asset.size),
            }
        });
        self.update_task = Some(tokio::spawn(async move {
            let result = async {
                let client = update::client()?;
                let path = update::download(&client, &release, |written, total| {
                    handle.publish(|p| {
                        if let UpdateState::Downloading {
                            written: w,
                            total: t,
                            ..
                        } = &mut p.update
                        {
                            (*w, *t) = (written, total);
                        }
                    });
                })
                .await?;
                apply(method, &path, &executable)
            }
            .await;
            handle.publish(|p| match result {
                Ok(relaunch) => p.update = UpdateState::Ready { relaunch },
                Err(error) => {
                    p.update = UpdateState::Available(release);
                    p.error = Some(format!("更新失敗：{error:#}"));
                }
            });
        }));
        Ok(())
    }
    fn process(&mut self, command: Command) {
        self.serial = self.serial.wrapping_add(1);
        self.handle.publish(|p| {
            p.busy = true;
            p.error = None;
        });
        let result = self.command(command);
        match self.service.snapshot() {
            Ok(data) => publish_records(self.handle, data),
            Err(error) => self.handle.publish(|p| p.error = Some(error.to_string())),
        }
        self.handle.publish(|p| {
            p.busy = false;
            match result {
                Ok(Some(notice)) => p.notice = Some(notice),
                Ok(None) => {}
                Err(error) => p.error = Some(error.to_string()),
            }
        });
    }
    fn command(&mut self, command: Command) -> Result<Option<String>> {
        match command {
            Command::Ingest(text) => {
                let n = if self.offline {
                    self.service.ingest_mock(text)?
                } else {
                    self.service.ingest(text)?
                };
                Ok(Some(format!("已匯入 {n} 組新序號")))
            }
            Command::Approve(code) => {
                self.service.approve(&code)?;
                Ok(Some(format!("已確認 {code}")))
            }
            Command::AddAccount {
                alias,
                username,
                password,
            } => {
                if self.offline {
                    self.service.add_test_account(&alias)?;
                } else {
                    self.service.add_account(&alias, &username, &password)?;
                }
                Ok(Some("帳號已新增".into()))
            }
            Command::Rename { id, alias } => {
                self.service.rename_account(&id, &alias)?;
                Ok(Some("別名已更新".into()))
            }
            Command::Password { id, password } => {
                self.service.update_password(&id, &password)?;
                Ok(Some("密碼已存入系統憑證庫".into()))
            }
            Command::Remove(id) => {
                self.service.remove_account(&id)?;
                Ok(Some("帳號已移除，兌換紀錄保留".into()))
            }
            Command::Automatic { id, enabled } => {
                self.service.set_automatic(&id, enabled)?;
                Ok(Some(
                    if enabled {
                        "已啟用；從之後首次發現的序號開始自動處理"
                    } else {
                        "已停用此帳號的自動處理"
                    }
                    .into(),
                ))
            }
            Command::Plan { code, accounts } => {
                let plan = self.service.plan(&code, &accounts)?;
                self.handle.publish(|p| p.plan = Some((self.serial, plan)));
                Ok(None)
            }
            Command::Redeem { plan, confirmed } => {
                ensure!(!self.offline, "離線測試不執行正式兌換");
                ensure!(self.active.is_none(), "請先暫停目前工作，再手動兌換");
                ensure!(!self.installing_update(), "正在安裝更新，請等更新完成");
                self.service.enqueue(&plan, confirmed)?;
                self.active = Some(start_worker(
                    self.dir.clone(),
                    self.config_path.clone(),
                    Some(plan),
                    false,
                )?);
                self.handle.publish(|p| p.state = RunState::Manual);
                Ok(Some("已開始處理選取的帳號".into()))
            }
            Command::Retry { job, confirmed } => {
                ensure!(!self.offline, "離線測試不執行正式兌換");
                ensure!(self.active.is_none(), "請先暫停目前工作，再重試");
                let data = self.service.snapshot()?;
                let job_record = data
                    .jobs
                    .iter()
                    .find(|j| j.id == job)
                    .context("找不到工作")?;
                let plan = self.service.plan(
                    &job_record.code,
                    std::slice::from_ref(&job_record.account_id),
                )?;
                self.service.retry(job, confirmed)?;
                self.active = Some(start_worker(
                    self.dir.clone(),
                    self.config_path.clone(),
                    Some(plan),
                    false,
                )?);
                self.handle.publish(|p| p.state = RunState::Manual);
                Ok(Some("已排入人工確認的重試".into()))
            }
            Command::Attempts(id) => {
                let attempts = self.service.attempts(id)?;
                self.handle
                    .publish(|p| p.attempts = Some((self.serial, id, attempts)));
                Ok(None)
            }
            Command::Start {
                backfill,
                confirmed,
            } => {
                ensure!(self.active.is_none(), "已有工作執行中");
                ensure!(!self.installing_update(), "正在安裝更新，請等更新完成");
                ensure!(!backfill || confirmed, "歷史補抓需要明確確認");
                ensure!(!self.offline || !backfill, "離線測試不補抓正式歷史序號");
                self.active = Some(start_worker(
                    self.dir.clone(),
                    self.config_path.clone(),
                    None,
                    backfill,
                )?);
                self.handle.publish(|p| p.state = RunState::Running);
                Ok(Some(
                    if backfill {
                        "已開始歷史補抓"
                    } else {
                        "已開始自動處理，新序號會依帳號基準排入"
                    }
                    .into(),
                ))
            }
            Command::Pause | Command::Stop => {
                let paused = matches!(command, Command::Pause);
                let target = if paused {
                    RunState::Paused
                } else {
                    RunState::Stopped
                };
                if let Some(active) = self.active.as_mut() {
                    active.cancel(target);
                    self.handle.publish(|p| {
                        p.state = if paused {
                            RunState::Pausing
                        } else {
                            RunState::Stopping
                        }
                    });
                } else {
                    self.handle.publish(|p| p.state = target);
                }
                Ok(None)
            }
            Command::CheckBrowser => {
                ensure!(!self.offline, "離線測試不連線到官方網站");
                ensure!(self.active.is_none(), "請先暫停目前工作，再檢查瀏覽器");
                ensure!(!self.installing_update(), "正在安裝更新，請等更新完成");
                self.active = Some(check_browser(
                    self.dir.clone(),
                    self.service.snapshot()?.config,
                ));
                self.handle.publish(|p| {
                    p.state = RunState::Checking;
                    p.browser_report = None;
                });
                Ok(None)
            }
            Command::Save {
                config,
                preferences: next,
            } => {
                self.save_settings(config, next)?;
                Ok(Some("設定已儲存".into()))
            }
            Command::Dismiss => {
                self.handle.publish(|p| {
                    p.error = None;
                    p.notice = None;
                });
                Ok(None)
            }
            Command::CheckUpdate { interactive } => {
                self.check_update(interactive)?;
                Ok(None)
            }
            Command::InstallUpdate => {
                self.install_update()?;
                Ok(Some("正在下載更新，核對檢查碼後會自動安裝".into()))
            }
        }
    }
    fn save_settings(&mut self, config: Config, next: Preferences) -> Result<()> {
        ensure!(self.active.is_none(), "請先暫停目前工作，再儲存設定");
        ensure!(
            !self.offline || next.start_on_login == self.preferences.start_on_login,
            "離線測試不修改登入啟動設定"
        );
        config.validate()?;
        let previous = self.service.snapshot()?.config;
        let temporary = self.pref_path.with_extension("toml.tmp");
        std::fs::write(&temporary, toml::to_string_pretty(&next)?)?;
        if let Err(error) = self.service.save_config(config) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        let login_changed = next.start_on_login != self.preferences.start_on_login;
        let result = (|| -> Result<()> {
            if login_changed {
                platform::set_login(next.start_on_login, &self.dir, &self.config_path)?;
            }
            std::fs::rename(&temporary, &self.pref_path)?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::remove_file(&temporary);
            self.service
                .save_config(previous)
                .context("設定儲存失敗，還原瀏覽器設定時也發生錯誤；請檢查 config.toml")?;
            if login_changed {
                platform::set_login(
                    self.preferences.start_on_login,
                    &self.dir,
                    &self.config_path,
                )
                .context("設定儲存失敗，請檢查登入啟動設定")?;
            }
            return Err(error);
        }
        self.preferences = next;
        self.handle.publish(|p| {
            p.preferences = self.preferences.clone();
            p.browser_report = None;
        });
        Ok(())
    }
    async fn refresh(&mut self) -> Result<()> {
        if self.active.as_ref().is_some_and(|a| a.task.is_finished()) {
            let finished = self.active.take().expect("finished worker");
            let result = finished.task.await.context("背景工作中斷")?;
            self.handle.publish(|p| {
                p.state = finished.finished;
                match result {
                    Ok(Some(report)) => {
                        p.browser_report = Some(report);
                        p.notice = Some("瀏覽器檢查已完成".into());
                    }
                    Ok(None) => {}
                    Err(error) => p.error = Some(error.to_string()),
                }
            });
        }
        let data = self.service.snapshot();
        let host = platform::tray_host_available().await;
        match data {
            Ok(data) => publish_records(self.handle, data),
            Err(error) => self.handle.publish(|p| p.error = Some(error.to_string())),
        }
        self.handle.publish(|p| p.tray_host = host);
        Ok(())
    }
}
async fn run(
    dir: PathBuf,
    config_path: PathBuf,
    offline: bool,
    mut receiver: mpsc::UnboundedReceiver<Command>,
    handle: &Backend,
) -> Result<()> {
    let mut actor = Actor::new(dir, config_path, offline, handle)?;
    if !offline {
        update::cleanup_stale_downloads();
        if actor.preferences.check_updates {
            actor.check_update(false)?;
        }
    }
    if actor.preferences.process_on_launch && !offline {
        actor.active = Some(start_worker(
            actor.dir.clone(),
            actor.config_path.clone(),
            None,
            false,
        )?);
        handle.publish(|p| p.state = RunState::Running);
    }
    let mut interval = tokio::time::interval(Duration::from_millis(600));
    let result = async {
        loop {
            tokio::select! {
                _ = handle.quit.cancelled() => break,
                _ = interval.tick() => actor.refresh().await?,
                command = receiver.recv() => match command {
                    Some(command) => actor.process(command),
                    None => break,
                },
            }
        }
        Ok(())
    }
    .await;
    handle.publish(|p| p.state = RunState::Quitting);
    if let Some(mut active) = actor.active.take() {
        active.cancel(RunState::Stopped);
        // Cancellation is a normal shutdown outcome for a read-only browser check.
        let _ = active.task.await.context("結束背景工作時中斷")?;
    }
    result
}

/// Hands the verified download to the platform installer and reports whether
/// the app should start the new version itself after it exits.
fn apply(method: update::Method, path: &Path, executable: &Path) -> Result<bool> {
    match method {
        #[cfg(windows)]
        update::Method::Installer => {
            let _ = executable;
            update::launch_installer(path)?;
            Ok(false)
        }
        #[cfg(target_os = "linux")]
        update::Method::ReplaceFiles => {
            let replaced = update::replace_files(path, executable);
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
            replaced.map(|()| true)
        }
        _ => {
            let _ = (path, executable);
            anyhow::bail!("這個版本請從 GitHub 下載安裝")
        }
    }
}

fn start_worker(
    dir: PathBuf,
    config_path: PathBuf,
    manual: Option<RedemptionPlan>,
    backfill: bool,
) -> Result<Active> {
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let task = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let _lock=WorkerLock::acquire(&dir)?;
            let mut config=Config::load(&config_path)?;
            config.headless=false;
            let accounts=Accounts::open(&dir)?;
            let mut store=Store::open(&dir.join("history.sqlite"))?;
            store.recover()?;
            store.initialize_daemon(&accounts.list()?)?;
            let mut session=RedemptionSession::new(config);
            let result:Result<()>=async {
                loop {
                    let selected=if let Some(plan)=manual.as_ref() {accounts.list()?.into_iter().filter(|a|plan.accounts.iter().any(|(id,_)|id==&a.id)).collect()} else {accounts.automatic_accounts()?};
                    if worker_cancel.is_cancelled() {break;}
                    if let Some(plan)=manual.as_ref() {
                        session.sweep(&mut store,&selected,RunScope::manual(Some(&plan.code),None),worker_cancel.clone(),&StoredCredentials).await?;
                        break;
                    }
                    store.schedule_daemon(&selected,backfill)?;
                    session.sweep(&mut store,&selected,RunScope::automatic(backfill),worker_cancel.clone(),&StoredCredentials).await?;
                    tokio::select! {_=worker_cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(2))=>{}}
                }
                Ok(())
            }.await;
            session.shutdown().await;
            result?;
            Ok(None)
        })
    });
    Ok(Active {
        cancel,
        task,
        finished: RunState::Stopped,
    })
}

fn check_browser(dir: PathBuf, mut config: Config) -> Active {
    config.headless = false;
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let task = tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let _lock=WorkerLock::acquire(&dir)?;
            let budget=config.timeout_seconds+config.driver.startup_timeout_seconds+5;
            let mut browser=BrowserRedeemer::new(config);
            let result=tokio::select! {
                _=worker_cancel.cancelled()=>Err(anyhow::anyhow!("瀏覽器檢查已取消")),
                report=tokio::time::timeout(Duration::from_secs(budget),browser.inspect())=>match report {Ok(report)=>report.map_err(Into::into),Err(_)=>Err(anyhow::anyhow!("瀏覽器檢查逾時"))},
            };
            browser.shutdown().await;
            let report=result?;
            ensure!(report.missing.is_empty()&&report.changed_types.is_empty(),"表單異常：缺少 [{}]；類型變更 [{}]",report.missing.join(", "),report.changed_types.join(", "));
            Ok(Some(format!("Chrome 與官方表單定位檢查通過。驗證控制項：{}。未填入帳密或送出表單。",if report.verification_present {"已找到"} else {"未顯示"})))
        })
    });
    Active {
        cancel,
        task,
        finished: RunState::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    async fn wait(backend: &Backend, ready: impl Fn(&Presentation) -> bool) -> Presentation {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let p = backend.snapshot();
                if ready(&p) {
                    return p;
                }
                assert!(p.error.is_none(), "{}", p.error.unwrap_or_default());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            let p = backend.snapshot();
            panic!("desktop state did not settle: state={:?}, loaded={}, busy={}, closed={}, revision={}, error={:?}", p.state, p.data.is_some(), p.busy, p.closed, p.revision, p.error);
        })
    }
    #[tokio::test]
    async fn consumed_confirmation_does_not_reopen_and_attempt_requests_are_fresh() {
        let dir = TempDir::new().unwrap();
        let backend = Backend::start(dir.path().into(), dir.path().join("config.toml"), true);
        wait(&backend, |p| p.data.is_some()).await;
        let account = Accounts::open(dir.path())
            .unwrap()
            .add("fixture", "fixture-user", "fake", true)
            .unwrap();
        let mut service = Service::open(dir.path(), &dir.path().join("config.toml")).unwrap();
        service.ingest("序號: fixture1".into()).unwrap();
        let plan = service.plan("fixture1", &[account.id]).unwrap();
        backend.publish(|p| p.plan = Some((7, plan)));
        assert!(backend.consume_plan(6).is_none());
        assert!(backend.consume_plan(7).is_some());
        assert!(backend.consume_plan(7).is_none());
        assert!(backend.snapshot().plan.is_none());
        backend.send(Command::Attempts(1));
        let first = wait(&backend, |p| p.attempts.is_some())
            .await
            .attempts
            .unwrap();
        backend.send(Command::Attempts(1));
        let next = wait(&backend, |p| {
            p.attempts
                .as_ref()
                .is_some_and(|(serial, _, _)| *serial != first.0)
        })
        .await
        .attempts
        .unwrap();
        assert_eq!(first.1, next.1);
        assert!(next.2.is_empty());
        backend.request_quit();
        backend.wait_closed().await;
    }
    #[tokio::test]
    async fn settings_failure_rolls_back_and_recovery_invalidates_browser_diagnostics() {
        let dir = TempDir::new().unwrap();
        let backend = Backend::start(dir.path().into(), dir.path().join("config.toml"), true);
        let before = wait(&backend, |p| p.data.is_some()).await;
        let mut config = before.data.as_ref().unwrap().config.clone();
        let old_delay = config.delay_seconds;
        config.delay_seconds = old_delay + 1;
        let mut preferences = before.preferences.clone();
        preferences.dark = !preferences.dark;
        std::fs::create_dir(dir.path().join("desktop.toml")).unwrap();
        backend.send(Command::Save {
            config,
            preferences,
        });
        let after = wait(&backend, |p| p.error.is_some()).await;
        assert_eq!(after.data.unwrap().config.delay_seconds, old_delay);
        assert_eq!(after.preferences.dark, before.preferences.dark);
        assert_eq!(
            Config::load(&dir.path().join("config.toml"))
                .unwrap()
                .delay_seconds,
            old_delay
        );
        assert!(!dir.path().join("desktop.toml.tmp").exists());
        std::fs::remove_dir(dir.path().join("desktop.toml")).unwrap();
        backend.publish(|p| {
            p.error = None;
            p.browser_report = Some("previous inspection fixture".into());
        });
        let mut config = Config::load(&dir.path().join("config.toml")).unwrap();
        config.delay_seconds = old_delay + 1;
        backend.send(Command::Save {
            config,
            preferences: before.preferences,
        });
        let saved = wait(&backend, |p| {
            p.data
                .as_ref()
                .is_some_and(|d| d.config.delay_seconds == old_delay + 1)
        })
        .await;
        assert!(saved.browser_report.is_none());
        backend.request_quit();
        backend.wait_closed().await;
    }
    #[tokio::test]
    async fn background_actions_publish_database_changes_pause_and_release_the_worker_lock() {
        let dir = TempDir::new().unwrap();
        let backend = Backend::start(dir.path().into(), dir.path().join("config.toml"), true);
        wait(&backend, |p| p.data.is_some()).await;
        backend.send(Command::AddAccount {
            alias: "fixture".into(),
            username: "not-a-real-user".into(),
            password: Zeroizing::new("fake-test-only".into()),
        });
        let p = wait(&backend, |p| {
            p.data.as_ref().is_some_and(|d| d.accounts.len() == 1)
        })
        .await;
        let id = p.data.unwrap().accounts[0].account.id.clone();
        backend.send(Command::Ingest(
            "序號: DesktopFixture1\n獎勵: 測試文字".into(),
        ));
        let p = wait(&backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.codes.is_empty())
        })
        .await;
        assert!(p.data.unwrap().codes[0].test_only);
        backend.send(Command::Start {
            backfill: false,
            confirmed: false,
        });
        wait(&backend, |p| p.state == RunState::Running).await;
        backend.send(Command::Pause);
        wait(&backend, |p| p.state == RunState::Paused).await;
        {
            let _lock = WorkerLock::acquire(dir.path()).unwrap();
        }
        backend.send(Command::Rename {
            id,
            alias: "renamed".into(),
        });
        let p = wait(&backend, |p| {
            p.data
                .as_ref()
                .is_some_and(|d| d.accounts[0].account.alias == "renamed")
        })
        .await;
        assert!(p.data.unwrap().jobs.is_empty());
        backend.send(Command::Stop);
        wait(&backend, |p| p.state == RunState::Stopped).await;
        backend.request_quit();
        tokio::time::timeout(Duration::from_secs(5), backend.wait_closed())
            .await
            .unwrap();
        assert!(backend.snapshot().closed);
        assert!(
            Store::open(&dir.path().join("history.sqlite"))
                .unwrap()
                .attempts()
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn a_second_instance_cannot_own_the_same_directory_and_shutdown_allows_restart() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let first = Backend::start(dir.path().into(), path.clone(), true);
        wait(&first, |p| p.data.is_some()).await;
        let second = Backend::start(dir.path().into(), path.clone(), false);
        second.wait_closed().await;
        assert!(second.snapshot().error.is_some());
        assert!(!first.snapshot().closed);
        first.send(Command::Start {
            backfill: false,
            confirmed: false,
        });
        wait(&first, |p| p.state == RunState::Running).await;
        first.request_quit();
        tokio::time::timeout(Duration::from_secs(5), first.wait_closed())
            .await
            .unwrap();
        let restarted = Backend::start(dir.path().into(), path, false);
        let p = wait(&restarted, |p| p.data.is_some()).await;
        assert_eq!(p.state, RunState::Stopped);
        restarted.request_quit();
        restarted.wait_closed().await;
    }
    #[tokio::test]
    async fn restart_keeps_unknown_outcomes_and_preexisting_codes_out_of_automatic_replay() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            toml::to_string(&Config {
                notifications: false,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let account = Accounts::open(dir.path())
            .unwrap()
            .add("fixture", "not-a-real-user", "fake", true)
            .unwrap();
        let mut store = Store::open(&dir.path().join("history.sqlite")).unwrap();
        for code in ["Historical1", "Unknown1"] {
            let text = format!("序號: {code}");
            store
                .ingest("manual", None, &text, &trserial::parser::parse(&text))
                .unwrap();
        }
        store.enqueue("Unknown1", &[account]).unwrap();
        let id = store.jobs().unwrap()[0].id;
        store.claim(id, 3).unwrap();
        store.mark_submitted(id).unwrap();
        store
            .finish(id, trserial::store::Status::Unknown, "fixture outcome", 0)
            .unwrap();
        let backend = Backend::start(dir.path().into(), config_path, false);
        wait(&backend, |p| p.data.is_some()).await;
        backend.send(Command::Start {
            backfill: false,
            confirmed: false,
        });
        wait(&backend, |p| p.state == RunState::Running).await;
        backend.send(Command::Pause);
        wait(&backend, |p| p.state == RunState::Paused).await;
        let jobs = store.jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, "unknown");
        assert_eq!(jobs[0].attempts, 1);
        backend.request_quit();
        backend.wait_closed().await;
    }
}
