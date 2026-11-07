use crate::{
    backend::{Command, Preferences},
    ui::{self, Controller},
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::InputState,
    *,
};
use gpui_kit::*;

pub(crate) struct SettingsPage {
    controller: Entity<Controller>,
    chrome: Entity<InputState>,
    driver: Entity<InputState>,
    endpoint: Entity<InputState>,
    delay: Entity<InputState>,
    verification: Entity<InputState>,
    managed: bool,
    notifications: bool,
    preferences: Preferences,
    initialized: bool,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
}
impl SettingsPage {
    pub(crate) fn new(
        controller: Entity<Controller>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            controller: controller.clone(),
            chrome: ui::input("Chrome／Chromium 執行檔，可留白", window, cx),
            driver: ui::input("ChromeDriver 執行檔", window, cx),
            endpoint: ui::input("WebDriver 位址", window, cx),
            delay: ui::input("兌換間隔（秒）", window, cx),
            verification: ui::input("驗證等待上限（秒）", window, cx),
            managed: false,
            notifications: true,
            preferences: Preferences::default(),
            initialized: false,
            error: None,
            subscriptions: Vec::new(),
        };
        this.subscriptions
            .push(cx.observe_in(&controller, window, |this, _, window, cx| {
                if !this.initialized {
                    this.load(window, cx);
                }
                cx.notify();
            }));
        this.load(window, cx);
        this
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.controller.read(cx).presentation.clone();
        if let Some(data) = p.data {
            for (field, value) in [
                (
                    &self.chrome,
                    data.config.browser_binary.clone().unwrap_or_default(),
                ),
                (
                    &self.driver,
                    data.config.driver.binary.to_string_lossy().into_owned(),
                ),
                (&self.endpoint, data.config.webdriver_url.clone()),
                (&self.delay, data.config.delay_seconds.to_string()),
                (
                    &self.verification,
                    data.config.verification_timeout_seconds.to_string(),
                ),
            ] {
                field.update(cx, |s, cx| s.set_value(value, window, cx));
            }
            self.managed = data.config.driver.managed;
            self.notifications = data.config.notifications;
            self.preferences = p.preferences;
            self.initialized = true;
            self.error = None;
        }
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.controller.read(cx).presentation.data.clone() else {
            return;
        };
        let mut config = data.config.clone();
        let result = (|| -> anyhow::Result<()> {
            let chrome = self.chrome.read(cx).value().to_string();
            config.browser_binary = (!chrome.trim().is_empty()).then(|| chrome.trim().to_owned());
            config.driver.binary = self.driver.read(cx).value().to_string().trim().into();
            config.webdriver_url = self.endpoint.read(cx).value().to_string().trim().into();
            config.delay_seconds = self
                .delay
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("兌換間隔請輸入整數秒數"))?;
            config.verification_timeout_seconds = self
                .verification
                .read(cx)
                .value()
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("驗證等待上限請輸入整數秒數"))?;
            config.driver.managed = self.managed;
            config.notifications = self.notifications;
            config.validate()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.error = None;
                ui::send(
                    &self.controller,
                    Command::Save {
                        config,
                        preferences: self.preferences.clone(),
                    },
                    cx,
                );
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
}
impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.controller.read(cx).presentation.clone();
        let disabled = p.busy || p.state.is_busy() || !self.initialized;
        let browser_controller = self.controller.clone();
        let mut body = v_flex()
            .gap_4()
            .p_4()
            .max_w(rems(48.))
            .child(ui::field(
                "settings-chrome",
                "Chrome／Chromium 執行檔",
                &self.chrome,
            ))
            .child(ui::field(
                "settings-driver",
                "ChromeDriver 執行檔",
                &self.driver,
            ))
            .child(
                Checkbox::new("settings-managed")
                    .label("由 TRserial 管理 ChromeDriver")
                    .checked(self.managed)
                    .disabled(disabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.managed = *value;
                        cx.notify();
                    })),
            )
            .child(ui::field(
                "settings-endpoint",
                "本機 WebDriver 位址",
                &self.endpoint,
            ))
            .child(
                h_flex()
                    .items_start()
                    .gap_4()
                    .child(div().flex_1().child(ui::field(
                        "settings-delay",
                        "兌換間隔（秒）",
                        &self.delay,
                    )))
                    .child(div().flex_1().child(ui::field(
                        "settings-verification",
                        "人工驗證等待上限（秒）",
                        &self.verification,
                    ))),
            )
            .child(
                Checkbox::new("settings-notifications")
                    .label("桌面通知")
                    .checked(self.notifications)
                    .disabled(disabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.notifications = *value;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("settings-background")
                    .label("開啟桌面版時自動處理新序號")
                    .checked(self.preferences.process_on_launch)
                    .disabled(disabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.preferences.process_on_launch = *value;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("settings-login")
                    .label("登入時啟動 TRserial")
                    .checked(self.preferences.start_on_login)
                    .disabled(disabled || p.offline)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.preferences.start_on_login = *value;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("settings-dark")
                    .label("深色外觀")
                    .checked(self.preferences.dark)
                    .disabled(disabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.preferences.dark = *value;
                        cx.notify();
                    })),
            )
            .child(ui::muted(
                "有系統匣時，關閉視窗會保留背景工作；暫停或停止請用主畫面或系統匣選單。",
                cx,
            ))
            .child(div().font_semibold().child("Discord（選用）"))
            .child(ui::muted(
                "桌面監聽未設定。此版本尚未提供安全的 Token 設定介面，使用 GUI 不需要 Discord。",
                cx,
            ));
        if let Some(data) = p.data.as_ref() {
            body = body.child(ui::muted(
                format!(
                    "既有頻道 ID：{}",
                    if data.config.discord.channels.is_empty() {
                        "未設定".into()
                    } else {
                        data.config
                            .discord
                            .channels
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join("、")
                    }
                ),
                cx,
            ));
        }
        body=body.child(div().font_semibold().child("瀏覽器檢查"))
            .child(ui::muted("會開啟官方表單檢查定位，不填入帳密或送出序號。桌面兌換會顯示 Chrome，供你完成人工驗證。",cx))
            .child(Button::new("settings-browser-check").label("檢查 Chrome 與官方表單").disabled(disabled||p.offline).on_click(move|_,_,cx|ui::send(&browser_controller,Command::CheckBrowser,cx)));
        if let Some(report) = p.browser_report {
            body = body.child(div().text_sm().child(report));
        }
        if let Some(error) = self.error.clone() {
            body = body.child(div().text_color(cx.theme().danger).child(error));
        }
        v_flex()
            .size_full()
            .gap_3()
            .child(ui::title("設定"))
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .overflow_y_scrollbar(),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("settings-save")
                            .primary()
                            .label("儲存設定")
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        Button::new("settings-reset")
                            .label("還原未儲存的變更")
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load(window, cx);
                                cx.notify();
                            })),
                    ),
            )
    }
}
