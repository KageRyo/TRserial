use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use trserial::{
    accounts::{Accounts, prompt},
    browser::{BrowserRedeemer, Redeemer},
    config::{self, Config},
    notifications::{self, Notifier},
    parser,
    source::{self, ManualSource, MockSource},
    store::{Status, Store},
    worker::{SystemCredentials, Worker, WorkerLock},
};

#[derive(Parser)]
#[command(version, about = "TRserial：多帳號序號兌換與本機工作佇列")]
struct Cli {
    #[arg(long, global = true, help = "應用程式資料目錄")]
    data_dir: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "設定檔路徑（預設為資料目錄內 config.toml）"
    )]
    config: Option<PathBuf>,
    #[arg(long, global = true, help = "本次執行自動管理 ChromeDriver")]
    managed_driver: bool,
    #[arg(
        long,
        global = true,
        help = "ChromeDriver 執行檔，同時啟用 managed 模式"
    )]
    driver_binary: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Init,
    Doctor {
        #[arg(long)]
        test_notification: bool,
    },
    /// Open and inspect the official form, with no account access or submission.
    BrowserCheck,
    /// Offline first-run workflow in a fresh data directory.
    SmokeTest,
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    Ingest {
        #[arg(long,conflicts_with_all=["file","mock"])]
        text: Option<String>,
        #[arg(long, conflicts_with = "mock")]
        file: Option<PathBuf>,
        #[arg(long, help = "JSON 訊息陣列，走共用匯入流程")]
        mock: Option<PathBuf>,
    },
    Code {
        #[command(subcommand)]
        command: CodeCommand,
    },
    Redeem {
        code: String,
        #[arg(long, conflicts_with = "account", required_unless_present = "account")]
        all: bool,
        #[arg(long, required_unless_present = "all")]
        account: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Retry {
        job: i64,
        #[arg(long, required = true, help = "已人工確認網站結果，理解可能重複兌換")]
        acknowledge: bool,
    },
    History {
        #[arg(long)]
        attempts: bool,
        #[arg(long)]
        messages: bool,
    },
    Status,
    Daemon {
        #[arg(long)]
        once: bool,
        #[arg(long,conflicts_with_all=["backfill","clipboard"])]
        mock: Option<PathBuf>,
        #[arg(long)]
        clipboard: bool,
        #[arg(long, help = "為所有現有帳號排入已確認的歷史序號")]
        backfill: bool,
    },
}
#[derive(Subcommand)]
enum AccountCommand {
    /// Add metadata for dry runs; no password or OS credential store access.
    AddTest {
        alias: String,
    },
    Add {
        #[arg(long)]
        session_only: bool,
    },
    List,
    Remove {
        alias: Option<String>,
    },
}
#[derive(Subcommand)]
enum CodeCommand {
    List,
    Approve { code: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Dependencies' verbose HTTP logs can contain form payloads. Only our own events are enabled.
    tracing_subscriber::fmt()
        .with_env_filter("trserial=info")
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let dir = cli.data_dir.unwrap_or(config::default_data_dir()?);
    config::prepare_dir(&dir)?;
    let config_path = cli.config.unwrap_or_else(|| dir.join("config.toml"));
    if matches!(cli.command, Command::Init) && !config_path.exists() {
        std::fs::write(&config_path, toml::to_string_pretty(&Config::default())?)?;
    }
    let mut config = Config::load(&config_path)?;
    if cli.managed_driver || cli.driver_binary.is_some() {
        config.driver.managed = true;
    }
    if let Some(binary) = cli.driver_binary {
        config.driver.binary = binary;
    }
    config.validate()?;
    let accounts = Accounts::open(&dir)?;
    let mut store = Store::open(&dir.join("history.sqlite"))?;
    match cli.command {
        Command::Init => println!(
            "初始化完成：{}\n設定檔：{}",
            dir.display(),
            config_path.display()
        ),
        Command::Doctor { test_notification } => doctor(&config, &dir, test_notification).await?,
        Command::BrowserCheck => {
            let _lock = WorkerLock::acquire(&dir)?;
            let cancel = cancellation();
            let mut browser = BrowserRedeemer::new(config.clone());
            let result = tokio::select! {
                _=cancel.cancelled()=>Err(anyhow::anyhow!("瀏覽器檢查已取消")),
                result=tokio::time::timeout(Duration::from_secs(config.timeout_seconds+config.driver.startup_timeout_seconds+5),browser.inspect())=>match result {Ok(report)=>report.map_err(Into::into),Err(_)=>Err(anyhow::anyhow!("瀏覽器檢查逾時"))},
            };
            browser.shutdown().await;
            let report = result?;
            println!(
                "官方頁面：{}\n驗證控制項：{}",
                config::OFFICIAL_URL,
                if report.verification_present {
                    "已找到；兌換時由使用者完成驗證"
                } else {
                    "未顯示"
                }
            );
            ensure!(
                report.missing.is_empty() && report.changed_types.is_empty(),
                "表單異常：缺少定位 [{}]；欄位類型變更 [{}]。請檢查表單改版或驗證中介頁",
                report.missing.join(", "),
                report.changed_types.join(", ")
            );
            println!("表單與欄位檢查通過；未填入帳密或序號，未送出表單。");
        }
        Command::SmokeTest => {
            let _lock = WorkerLock::acquire(&dir)?;
            ensure!(
                accounts.list()?.is_empty()
                    && store.codes()?.is_empty()
                    && store.jobs()?.is_empty(),
                "smoke-test 需使用全新的 --data-dir"
            );
            accounts.add_test("smoke")?;
            source::ingest_source(
                &mut store,
                &mut MockSource(vec![source::Message {
                    source: "smoke".into(),
                    id: None,
                    text: include_str!("../examples/message.txt").into(),
                }]),
            )?;
            let selected = accounts.list()?;
            for (code, _) in store.codes()? {
                show_dry_run(&store, &code, &selected)?;
            }
            println!(
                "離線 smoke test：測試帳號 smoke；已解析範例訊息；佇列與嘗試紀錄皆為零。\n下一步：trserial --data-dir {} redeem malojam1106 --account smoke --dry-run\n瀏覽器檢查：trserial --data-dir {} browser-check",
                dir.display(),
                dir.display()
            );
        }
        Command::Account { command } => match command {
            AccountCommand::AddTest { alias } => {
                let _lock = WorkerLock::acquire(&dir)?;
                accounts.add_test(&alias)?;
                println!("已新增測試帳號 {alias}，可供 dry run 使用。");
            }
            AccountCommand::Add { session_only } => {
                let _lock = WorkerLock::acquire(&dir)?;
                let alias = prompt("帳號別名")?;
                let username = prompt("遊戲帳號")?;
                let password = zeroize::Zeroizing::new(rpassword::prompt_password("遊戲密碼：")?);
                let a = accounts.add(&alias, &username, &password, session_only)?;
                println!(
                    "已新增 {}。{}",
                    a.alias,
                    if session_only {
                        "執行時會重新詢問密碼。"
                    } else {
                        "密碼已存入系統憑證庫。"
                    }
                );
            }
            AccountCommand::List => {
                for a in accounts.list()? {
                    println!(
                        "{}\t{}\t{}",
                        a.alias,
                        a.username,
                        if a.test_only {
                            "測試帳號（dry run）"
                        } else if a.session_only {
                            "每次詢問密碼"
                        } else {
                            "系統憑證庫"
                        }
                    );
                }
            }
            AccountCommand::Remove { alias } => {
                let _lock = WorkerLock::acquire(&dir)?;
                accounts.remove(&alias.map_or_else(|| prompt("要移除的別名"), Ok)?)?;
                println!("已移除帳號與憑證；歷史紀錄保留。");
            }
        },
        Command::Ingest { text, file, mock } => {
            let count = if let Some(path) = mock {
                ingest_mock(&mut store, &path)?
            } else {
                let text = match (text, file) {
                    (Some(t), _) => t,
                    (_, Some(p)) => read_text(&p)?,
                    _ => prompt("貼上序號或訊息（多行訊息請用 --file）")?,
                };
                source::ingest_source(&mut store, &mut ManualSource(text))?
            };
            println!("新增 {count} 組序號。可用 code list 檢查待確認項目。");
        }
        Command::Code { command } => match command {
            CodeCommand::List => {
                for (code, review) in store.codes()? {
                    println!(
                        "{code}\t{}",
                        if store.test_only(&code)? {
                            "test_only"
                        } else if review {
                            "needs_review"
                        } else {
                            "confirmed"
                        }
                    );
                }
            }
            CodeCommand::Approve { code } => {
                store.approve(&code)?;
                println!("已確認 {code}");
            }
        },
        Command::Redeem {
            code,
            all: _,
            account,
            dry_run,
        } => {
            ensure!(
                parser::valid_code(&code),
                "序號僅能包含英數字、連字號與底線"
            );
            let selected = select_accounts(&accounts, account.as_deref())?;
            if dry_run {
                show_dry_run(&store, &code, &selected)?;
            } else {
                ensure!(
                    selected.iter().all(|a| !a.test_only) && !store.test_only(&code)?,
                    "測試帳號與 mock 序號僅供 dry run 或本機測試使用"
                );
                let _lock = WorkerLock::acquire(&dir)?;
                store.recover()?;
                if !store.codes()?.iter().any(|(c, _)| c == &code) {
                    source::ingest_source(&mut store, &mut ManualSource(format!("序號：{code}")))?;
                }
                store.enqueue(&code, &selected)?;
                let filter = selected
                    .first()
                    .filter(|_| account.is_some())
                    .map(|a| a.id.as_str());
                let n = run_worker(
                    &mut store,
                    &config,
                    &selected,
                    Some(&code),
                    filter,
                    cancellation(),
                )
                .await?;
                println!(
                    "本次處理 {n} 筆；可用 history 檢查狀態。pending 工作會由 daemon 依重試時間接續。"
                );
            }
        }
        Command::Retry {
            job,
            acknowledge: _,
        } => {
            let _lock = WorkerLock::acquire(&dir)?;
            store.retry_reviewed(job)?;
            println!("工作 {job} 已排回佇列；執行 daemon 接續處理。");
        }
        Command::History { attempts, messages } => {
            if attempts {
                for (job, time, status, detail) in store.attempts()? {
                    println!("job={job}\t{time}\t{status}\t{detail}");
                }
            } else if messages {
                for (source, text) in store.source_messages()? {
                    println!("[{source}]\n{text}\n");
                }
            } else {
                for j in store.jobs()? {
                    println!(
                        "{}\t{}\t{}\t{}\tattempts={}\tupdated={}\t{}",
                        j.id,
                        j.code,
                        j.alias,
                        j.status,
                        j.attempts,
                        j.updated_at,
                        format_args!("{} / {}", store.job_policy(j.id)?, j.detail)
                    );
                }
            }
        }
        Command::Status => {
            let jobs = store.jobs()?;
            println!(
                "序號：{}；帳號：{}；工作：{}",
                store.codes()?.len(),
                accounts.list()?.len(),
                jobs.len()
            );
            for status in [
                Status::Pending,
                Status::Processing,
                Status::WaitingVerification,
                Status::Success,
                Status::AlreadyRedeemed,
                Status::Expired,
                Status::Failed,
                Status::Unknown,
            ] {
                println!(
                    "{}：{}",
                    status.as_str(),
                    jobs.iter().filter(|j| j.status == status.as_str()).count()
                );
            }
        }
        Command::Daemon {
            once,
            mock,
            clipboard,
            backfill,
        } => {
            // Mock mode is ingestion-only, even when this directory already contains live pending jobs.
            if let Some(path) = mock {
                println!(
                    "匯入 {} 組測試序號；mock 模式只匯入，不執行佇列。",
                    ingest_mock(&mut store, &path)?
                );
                return Ok(());
            }
            #[cfg(not(feature = "clipboard"))]
            ensure!(
                !clipboard,
                "剪貼簿功能未編譯；請使用 cargo build --features clipboard"
            );
            #[cfg(not(feature = "discord"))]
            ensure!(
                !config.discord.enabled,
                "Discord 功能未編譯；請使用 cargo build --features discord"
            );
            let _lock = WorkerLock::acquire(&dir)?;
            store.recover()?;
            store.initialize_daemon(&accounts.list()?)?;
            #[cfg(feature = "clipboard")]
            let mut clipboard_source = if clipboard {
                Some(source::ClipboardSource::new()?)
            } else {
                None
            };
            let cancel = cancellation();
            #[cfg(feature = "discord")]
            let discord = trserial::discord::start(&config.discord, &dir, cancel.clone()).await?;
            let mut browser = BrowserRedeemer::new(config.clone());
            println!("背景處理已啟動；Ctrl+C 結束。歷史補抓：{}", backfill);

            let result:Result<()> = async {
                loop {
                    #[cfg(feature="clipboard")]
                    if let Some(source)=&mut clipboard_source {source::ingest_source(&mut store,source)?;}
                    let current=accounts.list()?;
                    store.schedule_daemon(&current,backfill)?;
                    Worker{store:&mut store,config:&config,redeemer:&mut browser,credentials:&SystemCredentials,notifier:notifications::desktop(config.notifications),cancel:cancel.clone()}.run_scoped(&current,None,None,Some(backfill)).await?;
                    if once || cancel.is_cancelled() {break;}
                    tokio::select! {_=cancel.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(2))=>{}}
                }
                Ok(())
            }.await;
            cancel.cancel();
            browser.shutdown().await;
            #[cfg(feature = "discord")]
            if let Some(task) = discord {
                let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
            }
            result?;
        }
    }
    Ok(())
}
fn show_dry_run(store: &Store, code: &str, accounts: &[trserial::accounts::Account]) -> Result<()> {
    ensure!(
        !store
            .codes()?
            .iter()
            .any(|(c, review)| c == code && *review),
        "序號需要人工確認；請先執行 code approve"
    );
    let jobs = store.jobs()?;
    for a in accounts {
        let state = jobs
            .iter()
            .find(|j| j.code == code && j.account_id == a.id)
            .map_or("新工作", |j| j.status.as_str());
        println!(
            "DRY RUN：{code} / {} / {state}{}",
            a.alias,
            if a.test_only || store.test_only(code)? {
                "（測試資料）"
            } else {
                ""
            }
        );
    }
    println!("未開啟瀏覽器、未讀取密碼、未建立兌換工作、未送出網站。");
    Ok(())
}
fn read_text(path: &Path) -> Result<String> {
    ensure!(
        std::fs::metadata(path)?.len() <= 1024 * 1024,
        "輸入檔案超過 1 MiB"
    );
    std::fs::read_to_string(path).context("無法讀取 UTF-8 訊息檔案")
}
fn ingest_mock(store: &mut Store, path: &Path) -> Result<usize> {
    let mut source =
        MockSource(serde_json::from_str(&read_text(path)?).context("假訊息必須是 JSON 陣列")?);
    source::ingest_source(store, &mut source)
}
fn select_accounts(
    accounts: &Accounts,
    alias: Option<&str>,
) -> Result<Vec<trserial::accounts::Account>> {
    let selected: Vec<_> = accounts
        .list()?
        .into_iter()
        .filter(|a| alias.is_none_or(|alias| alias == a.alias))
        .collect();
    ensure!(
        !selected.is_empty(),
        "找不到帳號；請先 account add 或檢查別名"
    );
    Ok(selected)
}
fn cancellation() -> CancellationToken {
    let token = CancellationToken::new();
    let child = token.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut term) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        child.cancel();
    });
    token
}
async fn run_worker(
    store: &mut Store,
    config: &Config,
    accounts: &[trserial::accounts::Account],
    code: Option<&str>,
    account: Option<&str>,
    cancel: CancellationToken,
) -> Result<usize> {
    let mut browser = BrowserRedeemer::new(config.clone());
    let mut worker = Worker {
        store,
        config,
        redeemer: &mut browser,
        credentials: &SystemCredentials,
        notifier: notifications::desktop(config.notifications),
        cancel,
    };
    let result = worker.run(accounts, code, account).await;
    browser.shutdown().await;
    result
}
async fn doctor(config: &Config, dir: &Path, test_notification: bool) -> Result<()> {
    println!(
        "資料目錄可用：{}\n目標：{}\nDiscord：{}\n通知：{}",
        dir.display(),
        config::OFFICIAL_URL,
        config.discord.enabled,
        config.notifications
    );
    let driver = trserial::driver::DriverManager::new(
        config.clone(),
        config::default_data_dir()?.join("driver-locks"),
    )?;
    println!(
        "WebDriver /status：{:?}；管理模式：{}；執行檔：{}",
        driver.readiness().await,
        config.driver.managed,
        config.driver.binary.display()
    );
    match keyring::Entry::new("TRserial-doctor", "probe").and_then(|e| e.get_password()) {
        Ok(_) | Err(keyring::Error::NoEntry) => println!("憑證庫可讀取；未寫入測試密碼"),
        Err(_) => {
            println!("憑證庫無法讀取；請解鎖 Windows Credential Manager／Linux Secret Service")
        }
    }
    if test_notification {
        notifications::DesktopNotifier { enabled: true }
            .send("TRserial — 通知測試", "這是本機測試通知，沒有送出序號。");
    }
    println!("瀏覽器與表單檢查：trserial browser-check");
    Ok(())
}
