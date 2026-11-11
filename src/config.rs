use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const OFFICIAL_URL: &str = "https://trevent.funtown.com.hk/serial/app/serial_code.php";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub webdriver_url: String,
    pub browser_binary: Option<String>,
    pub headless: bool,
    pub delay_seconds: u64,
    pub timeout_seconds: u64,
    pub verification_timeout_seconds: u64,
    pub max_attempts: u32,
    pub retry_base_seconds: u64,
    pub notifications: bool,
    pub discord: DiscordConfig,
    pub driver: DriverConfig,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiscordConfig {
    pub enabled: bool,
    pub channels: Vec<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DriverConfig {
    pub managed: bool,
    pub binary: PathBuf,
    pub startup_timeout_seconds: u64,
}
impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            managed: false,
            binary: PathBuf::from("chromedriver"),
            startup_timeout_seconds: 15,
        }
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            webdriver_url: "http://127.0.0.1:9515".into(),
            browser_binary: None,
            headless: false,
            delay_seconds: 5,
            timeout_seconds: 30,
            verification_timeout_seconds: 300,
            max_attempts: 3,
            retry_base_seconds: 10,
            notifications: true,
            discord: DiscordConfig::default(),
            driver: DriverConfig::default(),
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let c: Self = if path.exists() {
            toml::from_str(&std::fs::read_to_string(path)?).map_err(|_| {
                anyhow::anyhow!(
                    "設定檔格式錯誤；請比對 config.example.toml（錯誤內容不回顯，以免洩露憑證）"
                )
            })?
        } else {
            Self::default()
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        let c = self;
        ensure!(
            (1..=120).contains(&c.driver.startup_timeout_seconds),
            "driver.startup_timeout_seconds 必須介於 1 與 120"
        );
        ensure!(
            (1..=10).contains(&c.max_attempts),
            "max_attempts 必須介於 1 與 10"
        );
        ensure!(
            (1..=300).contains(&c.timeout_seconds),
            "timeout_seconds 必須介於 1 與 300"
        );
        ensure!(
            (1..=3600).contains(&c.verification_timeout_seconds),
            "verification_timeout_seconds 必須介於 1 與 3600"
        );
        ensure!(
            (1..=3600).contains(&c.retry_base_seconds) && c.delay_seconds <= 3600,
            "延遲設定超出範圍"
        );
        let u = url::Url::parse(&c.webdriver_url)?;
        ensure!(
            matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                && u.scheme() == "http"
                && u.username().is_empty()
                && u.password().is_none(),
            "WebDriver 必須使用本機 HTTP 位址"
        );
        ensure!(
            u.query().is_none() && u.fragment().is_none() && u.port_or_known_default() != Some(0),
            "WebDriver 位址不可包含查詢參數或片段，且須使用有效連接埠"
        );
        if c.driver.managed {
            ensure!(
                u.host_str() == Some("127.0.0.1") && u.path() == "/",
                "managed 模式請使用 http://127.0.0.1:PORT 根路徑"
            );
        }
        Ok(())
    }
}
pub fn default_data_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from("", "", "trserial")
        .map(|d| d.data_local_dir().to_owned())
        .context("無法取得資料目錄；請指定 --data-dir")
}
pub fn prepare_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
