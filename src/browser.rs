use crate::{
    accounts::Credentials,
    config::{Config, OFFICIAL_URL},
    store::Status,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::time::Duration;
use thirtyfour::prelude::*;

#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct RedeemError {
    pub transient: bool,
    pub detail: &'static str,
}
impl RedeemError {
    pub fn transient(detail: &'static str) -> Self {
        Self {
            transient: true,
            detail,
        }
    }
    pub fn permanent(detail: &'static str) -> Self {
        Self {
            transient: false,
            detail,
        }
    }
}
pub type BrowserResult<T> = Result<T, RedeemError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preparation {
    Ready,
    WaitingVerification,
}
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: Status,
    pub detail: &'static str,
}

#[async_trait]
pub trait Redeemer: Send {
    async fn prepare(
        &mut self,
        credentials: &Credentials,
        code: &str,
    ) -> BrowserResult<Preparation>;
    async fn verification_ready(&mut self) -> BrowserResult<bool>;
    async fn submit(&mut self) -> BrowserResult<Outcome>;
    async fn close(&mut self);
}

pub struct BrowserRedeemer {
    config: Config,
    driver: Option<WebDriver>,
    target: String,
    before_url: String,
    expected_fields: Vec<(&'static str, [u8; 32])>,
}
impl BrowserRedeemer {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            driver: None,
            target: OFFICIAL_URL.into(),
            before_url: String::new(),
            expected_fields: Vec::new(),
        }
    }
    /// Only loopback fixtures are allowed; the CLI never accepts a replacement redemption site.
    pub fn local_fixture(config: Config, target: &str) -> anyhow::Result<Self> {
        let url = url::Url::parse(target)?;
        anyhow::ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
            "fixture must be loopback HTTP"
        );
        Ok(Self {
            config,
            driver: None,
            target: target.into(),
            before_url: String::new(),
            expected_fields: Vec::new(),
        })
    }
    fn driver(&self) -> BrowserResult<&WebDriver> {
        self.driver
            .as_ref()
            .ok_or_else(|| RedeemError::permanent("瀏覽器工作階段不存在"))
    }
    async fn check_origin(&self) -> BrowserResult<()> {
        let current = self
            .driver()?
            .current_url()
            .await
            .map_err(|_| RedeemError::transient("無法確認頁面位址"))?;
        let target =
            url::Url::parse(&self.target).map_err(|_| RedeemError::permanent("兌換位址錯誤"))?;
        if current.origin() != target.origin() || current.path() != target.path() {
            return Err(RedeemError::permanent("頁面已轉址；為保護憑證停止操作"));
        }
        let action = self
            .driver()?
            .find(By::Css("form#serial"))
            .await
            .map_err(|_| RedeemError::permanent("找不到官方表單"))?
            .prop("action")
            .await
            .map_err(|_| RedeemError::permanent("無法確認表單目的地"))?
            .unwrap_or_default();
        let action =
            url::Url::parse(&action).map_err(|_| RedeemError::permanent("表單目的地錯誤"))?;
        if action.origin() != target.origin()
            || (self.target == OFFICIAL_URL
                && action.path() != "/serial/app/serial_code_handler.php")
        {
            return Err(RedeemError::permanent("表單目的地已變更；停止送出"));
        }
        Ok(())
    }
}
#[async_trait]
impl Redeemer for BrowserRedeemer {
    async fn prepare(
        &mut self,
        credentials: &Credentials,
        code: &str,
    ) -> BrowserResult<Preparation> {
        self.close().await;
        let mut caps = DesiredCapabilities::chrome();
        if self.config.headless {
            caps.set_headless()
                .map_err(|_| RedeemError::permanent("無法設定瀏覽器"))?;
        }
        if let Some(binary) = &self.config.browser_binary {
            caps.set_binary(binary)
                .map_err(|_| RedeemError::permanent("無法設定瀏覽器路徑"))?;
        }
        let driver = WebDriver::new(&self.config.webdriver_url, caps)
            .await
            .map_err(|_| RedeemError::transient("無法連線本機 WebDriver"))?;
        self.driver = Some(driver);
        let driver = self.driver()?.clone();
        driver
            .set_page_load_timeout(Duration::from_secs(self.config.timeout_seconds))
            .await
            .map_err(|_| RedeemError::transient("無法設定頁面逾時"))?;
        driver
            .goto(&self.target)
            .await
            .map_err(|_| RedeemError::transient("兌換頁面載入失敗或逾時"))?;
        self.check_origin().await?;
        driver.execute("document.querySelector('form#serial').addEventListener('submit', e => { if (!window.__trserialAllowSubmit) e.preventDefault(); }, true);",vec![]).await.map_err(|_|RedeemError::permanent("無法安裝送出保護"))?;
        for (selector, value) in [
            (
                "form#serial input[name='account']",
                credentials.username.as_str(),
            ),
            (
                "form#serial input[name='passward']",
                credentials.password.as_str(),
            ),
            ("form#serial input[name='code']", code),
        ] {
            self.expected_fields
                .push((selector, Sha256::digest(value.as_bytes()).into()));
            let field = driver
                .find(By::Css(selector))
                .await
                .map_err(|_| RedeemError::permanent("官方表單欄位已變更"))?;
            field
                .clear()
                .await
                .map_err(|_| RedeemError::permanent("無法清除欄位"))?;
            field
                .send_keys(value)
                .await
                .map_err(|_| RedeemError::permanent("無法填入欄位"))?;
            // Avoid silently redeeming a truncated or transformed code/password.
            if field
                .prop("value")
                .await
                .map_err(|_| RedeemError::permanent("無法核對欄位"))?
                .as_deref()
                != Some(value)
            {
                return Err(RedeemError::permanent("表單欄位截斷或改變輸入；停止送出"));
            }
        }
        self.before_url = driver
            .current_url()
            .await
            .map_err(|_| RedeemError::transient("無法確認頁面位址"))?
            .to_string();
        Ok(if self.verification_ready().await? {
            Preparation::Ready
        } else {
            Preparation::WaitingVerification
        })
    }
    async fn verification_ready(&mut self) -> BrowserResult<bool> {
        self.check_origin().await?;
        let result = self
            .driver()?
            .execute(
                r#"
const form=document.querySelector('form#serial');
const token=form.querySelector('[name="cf-turnstile-response"]');
if (form.querySelector('.cf-turnstile') || token || window.turnstileOn === true)
    return !!token && token.value.trim().length > 0;
const check=form.querySelector('[name="checkResult"]');
const text=form.querySelector('[name="checkTxt"]');
return !!check && !!text && check.value === '0' && text.value.trim().length > 0;
"#,
                vec![],
            )
            .await
            .map_err(|_| RedeemError::transient("無法讀取驗證狀態"))?;
        Ok(result.json().as_bool().unwrap_or(false))
    }
    async fn submit(&mut self) -> BrowserResult<Outcome> {
        self.check_origin().await?;
        if !self.verification_ready().await? {
            return Ok(Outcome {
                status: Status::Unknown,
                detail: "送出前驗證已失效，請人工檢查",
            });
        }
        let driver = self.driver()?;
        for (selector, expected) in &self.expected_fields {
            let value = driver
                .find(By::Css(*selector))
                .await
                .map_err(|_| RedeemError::permanent("送出前表單欄位消失"))?
                .prop("value")
                .await
                .map_err(|_| RedeemError::permanent("送出前無法核對欄位"))?
                .unwrap_or_default();
            let actual: [u8; 32] = Sha256::digest(value.as_bytes()).into();
            if &actual != expected {
                return Err(RedeemError::permanent("驗證期間欄位已變更；停止送出"));
            }
        }
        // Clear no page data; only enable the normal form's own submit event.
        driver
            .execute("window.__trserialAllowSubmit=true;", vec![])
            .await
            .map_err(|_| RedeemError::permanent("無法送出表單"))?;
        driver
            .find(By::Css("form#serial #btnSend"))
            .await
            .map_err(|_| RedeemError::permanent("找不到送出按鈕"))?
            .click()
            .await
            .map_err(|_| RedeemError::permanent("送出後連線中斷；結果不明"))?;
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(self.config.timeout_seconds);
        loop {
            if let Ok(text) = driver.get_alert_text().await {
                let status = classify_response(&text);
                let _ = driver.accept_alert().await;
                return Ok(Outcome {
                    status,
                    detail: response_detail(status),
                });
            }
            // Only inspect the handler response after navigation; never success text in form instructions.
            if let Ok(url) = driver.current_url().await
                && url.as_str() != self.before_url
            {
                let target = url::Url::parse(&self.target)
                    .map_err(|_| RedeemError::permanent("兌換位址錯誤"))?;
                if url.origin() != target.origin() {
                    return Ok(Outcome {
                        status: Status::Unknown,
                        detail: "送出後轉到其他網站",
                    });
                }
                if let Ok(body) = driver.find(By::Css("body")).await
                    && let Ok(text) = body.text().await
                {
                    let status = classify_response(&text);
                    if status != Status::Unknown {
                        return Ok(Outcome {
                            status,
                            detail: response_detail(status),
                        });
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(Outcome {
                    status: Status::Unknown,
                    detail: "網站未提供可辨識的確認結果",
                });
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    async fn close(&mut self) {
        self.expected_fields.clear();
        if let Some(driver) = self.driver.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), driver.quit()).await;
        }
    }
}
/// Conservative exact allowlist. These phrases are not claimed to cover all live handler responses.
pub fn classify_response(text: &str) -> Status {
    let text = text.trim().trim_end_matches(['。', '！', '!', '.']).trim();
    match text {
        "兌換成功" | "序號兌換成功" | "領取成功" | "Redemption successful" => {
            Status::Success
        }
        "此序號已兌換" | "此序號已使用" | "您已兌換過此序號" | "Already redeemed" => {
            Status::AlreadyRedeemed
        }
        "序號已過期" | "此序號已過期" | "Code expired" => Status::Expired,
        "序號無效" | "無效序號" | "序號不存在" | "Invalid code" | "帳號或密碼錯誤" => {
            Status::Failed
        }
        _ => Status::Unknown,
    }
}
fn response_detail(status: Status) -> &'static str {
    match status {
        Status::Success => "網站確認兌換成功",
        Status::AlreadyRedeemed => "網站表示此帳號已兌換",
        Status::Expired => "網站表示序號已過期",
        Status::Failed => "網站拒絕兌換；請檢查序號或帳號",
        _ => "網站回應無法辨識，請人工檢查",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_infers_success_from_instructions_or_negative_text() {
        for text in [
            "未能兌換成功",
            "如果兌換成功，獎勵將送出",
            "Error: not Redemption successful",
            "兌換成功嗎？",
            "",
            "<script>alert('兌換成功')</script>",
        ] {
            assert_eq!(classify_response(text), Status::Unknown);
        }
        assert_eq!(classify_response("兌換成功。"), Status::Success);
        assert_eq!(classify_response("此序號已兌換"), Status::AlreadyRedeemed);
        assert_eq!(classify_response("序號已過期"), Status::Expired);
        assert_eq!(classify_response("序號無效"), Status::Failed);
    }
}
