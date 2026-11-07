use crate::config::Config;
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    net::TcpStream,
    process::{Child, Command},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Busy,
    Missing,
    Invalid,
}
/// Holds only a child launched by this instance. Reused external servers are never stopped.
pub struct DriverManager {
    config: Config,
    lock_root: PathBuf,
    child: Option<Child>,
    lock: Option<File>,
    client: reqwest::Client,
}
impl DriverManager {
    pub fn new(config: Config, lock_root: PathBuf) -> Result<Self> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()?;
        Ok(Self {
            config,
            lock_root,
            child: None,
            lock: None,
            client,
        })
    }
    pub fn owned_pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }
    pub async fn readiness(&self) -> Readiness {
        // Windows connection refusal can outlast the HTTP client's response timeout.
        // Probe TCP separately: an accepted but unresponsive service is Invalid, not Missing.
        let Ok(url) = url::Url::parse(&self.config.webdriver_url) else {
            return Readiness::Invalid;
        };
        let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
        match tokio::time::timeout(
            Duration::from_secs(5),
            TcpStream::connect((host, url.port_or_known_default().unwrap_or(80))),
        )
        .await
        {
            Ok(Ok(stream)) => drop(stream),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Readiness::Missing;
            }
            _ => return Readiness::Invalid,
        }
        let endpoint = format!("{}/status", self.config.webdriver_url.trim_end_matches('/'));
        match self.client.get(endpoint).send().await {
            Ok(response) if response.status().is_success() => {
                // Bound the status response, even if a different service occupies the port.
                if response.content_length().is_some_and(|n| n > 65536) {
                    return Readiness::Invalid;
                }
                let mut response = response;
                let mut bytes = Vec::new();
                loop {
                    match response.chunk().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= 65536 => {
                            bytes.extend_from_slice(&chunk)
                        }
                        Ok(None) => break,
                        _ => return Readiness::Invalid,
                    }
                }
                match serde_json::from_slice::<serde_json::Value>(&bytes)
                    .ok()
                    .and_then(|v| v["value"]["ready"].as_bool())
                {
                    Some(true) => Readiness::Ready,
                    Some(false) => Readiness::Busy,
                    None => Readiness::Invalid,
                }
            }
            Ok(_) => Readiness::Invalid,
            Err(e) if e.is_connect() => Readiness::Missing,
            Err(_) => Readiness::Invalid,
        }
    }
    pub async fn ensure_ready(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut()
            && child.try_wait()?.is_some()
        {
            self.child = None;
            self.lock = None;
        }
        let was_owned = self.child.is_some();
        match self.readiness().await {
            Readiness::Ready => return Ok(()),
            Readiness::Busy => bail!("WebDriver 忙碌中，請等目前工作結束"),
            Readiness::Invalid if was_owned => self.shutdown().await,
            Readiness::Invalid => {
                bail!("WebDriver 連接埠的 /status 回覆無效；請檢查位址與占用程序")
            }

            Readiness::Missing => {}
        }
        ensure!(
            self.config.driver.managed,
            "WebDriver 尚未啟動；請啟動 ChromeDriver 或啟用 driver.managed"
        );
        // A live but unresponsive owned child must be reaped before any replacement starts.
        if self.child.is_some() {
            self.shutdown().await;
        }
        crate::config::prepare_dir(&self.lock_root)?;
        let key = format!(
            "{:x}",
            Sha256::digest(
                url::Url::parse(&self.config.webdriver_url)?
                    .as_str()
                    .as_bytes()
            )
        );
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_root.join(format!("{key}.lock")))?;
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(self.config.driver.startup_timeout_seconds);
        loop {
            match lock.try_lock_exclusive() {
                Ok(()) => break,
                Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                    if self.readiness().await == Readiness::Ready {
                        return Ok(());
                    }
                    ensure!(
                        tokio::time::Instant::now() < deadline,
                        "另一個 TRserial 正在管理此 WebDriver，等待逾時"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        self.lock = Some(lock);
        // Recheck under the endpoint lock: another instance may just have finished starting it.
        match self.readiness().await {
            Readiness::Ready => {
                self.lock = None;
                return Ok(());
            }
            Readiness::Missing => {}
            _ => {
                self.lock = None;
                bail!("WebDriver 連接埠已被占用，未啟動其他程序");
            }
        }
        let url = url::Url::parse(&self.config.webdriver_url)?;
        let mut command = Command::new(&self.config.driver.binary);
        command
            .arg(format!(
                "--port={}",
                url.port_or_known_default().context("缺少連接埠")?
            ))
            // ChromeDriver's default is loopback binding. Even a loopback-only
            // --allowed-ips list enables an all-interface listener.
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW; no extra console at desktop login.
        match command.spawn() {
            Ok(child) => self.child = Some(child),
            Err(_) => {
                self.lock = None;
                bail!("無法啟動設定的 ChromeDriver；請檢查 driver.binary 路徑與執行權限");
            }
        }
        let result: Result<()> = async {
            loop {
                if self
                    .child
                    .as_mut()
                    .context("ChromeDriver 程序遺失")?
                    .try_wait()?
                    .is_some()
                {
                    bail!("ChromeDriver 提前結束；請檢查版本、連接埠與執行權限");
                }
                match self.readiness().await {
                    Readiness::Ready => return Ok(()),
                    Readiness::Invalid => bail!("ChromeDriver 啟動後的狀態回覆無效"),
                    _ => {}
                }
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "ChromeDriver 啟動逾時"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        .await;
        if result.is_err() {
            self.shutdown().await;
        }
        result
    }
    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.lock = None;
    }
}
impl Drop for DriverManager {
    fn drop(&mut self) {
        // kill_on_drop handles the child if cancellation skips explicit shutdown.
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}
