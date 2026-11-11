use crate::{
    backend::{Command, Preferences, UpdateState},
    ui::{self, Controller},
    update,
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::InputState,
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, rems,
};

pub(crate) struct SettingsPage {
    controller: Entity<Controller>,
    chrome: Entity<InputState>,
    driver: Entity<InputState>,
    endpoint: Entity<InputState>,
    delay: Entity<InputState>,
    verification: Entity<InputState>,
    channels: Entity<InputState>,
    discord_token: Entity<InputState>,
    discord_enabled: bool,
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
            channels: ui::input("例如 123456789012345678, 234567890123456789", window, cx),
            discord_token: ui::input("貼上 Bot Token", window, cx),
            discord_enabled: false,
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
                (
                    &self.channels,
                    data.config
                        .discord
                        .channels
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ] {
                field.update(cx, |s, cx| s.set_value(value, window, cx));
            }
            self.managed = data.config.driver.managed;
            self.discord_enabled = data.config.discord.enabled;
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
            config.discord.enabled = self.discord_enabled;
            config.discord.channels = parse_channels(&self.channels.read(cx).value())?;
            anyhow::ensure!(
                !config.discord.enabled || !config.discord.channels.is_empty(),
                "啟用 Discord 前請填入至少一個頻道 ID"
            );
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
    fn save_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let token = zeroize::Zeroizing::new(self.discord_token.read(cx).value().trim().to_owned());
        if token.is_empty() {
            self.error = Some("請先貼上 Discord Bot Token".into());
            cx.notify();
            return;
        }
        // Clear the field at once; the token only travels to the OS credential store.
        self.discord_token
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.error = None;
        ui::send(&self.controller, Command::SaveDiscordToken(token), cx);
    }
}

/// Channel IDs separated by commas, Chinese commas or whitespace, without duplicates.
fn parse_channels(text: &str) -> anyhow::Result<Vec<u64>> {
    let mut channels = Vec::new();
    for part in text
        .split(|c: char| c == ',' || c == '，' || c == '、' || c.is_whitespace())
        .filter(|part| !part.is_empty())
    {
        let id = part
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| anyhow::anyhow!("頻道 ID 必須是數字：{part}"))?;
        if !channels.contains(&id) {
            channels.push(id);
        }
    }
    Ok(channels)
}
impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.controller.read(cx).presentation.clone();
        let disabled = p.busy || p.state.is_busy() || !self.initialized;
        let browser_controller = self.controller.clone();
        let update_controller = self.controller.clone();
        let token_controller = self.controller.clone();
        let update_running = matches!(
            p.update,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
        );
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
            .child(div().font_semibold().child("更新"))
            .child(
                Checkbox::new("settings-updates")
                    .label("開啟時檢查新版本（連線到 GitHub）")
                    .checked(self.preferences.check_updates)
                    .disabled(disabled || p.offline)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.preferences.check_updates = *value;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(ui::muted(format!("目前版本 {}", update::current()), cx))
                    .child(
                        Button::new("settings-update-check")
                            .small()
                            .label("立即檢查更新")
                            .disabled(p.offline || update_running)
                            .on_click(move |_, _, cx| {
                                ui::send(
                                    &update_controller,
                                    Command::CheckUpdate { interactive: true },
                                    cx,
                                )
                            }),
                    ),
            )
            .child(div().font_semibold().child("Discord（選用）"))
            .child(ui::muted(
                "在 Discord Developer Portal 建立 Bot、開啟 Message Content Intent，邀請到伺服器並給予讀取頻道與訊息紀錄的權限。在 Discord 開啟開發者模式後，對頻道按右鍵即可複製頻道 ID。",
                cx,
            ))
            .child(
                Checkbox::new("settings-discord")
                    .label("啟用 Discord 監聽，自動匯入頻道中的序號")
                    .checked(self.discord_enabled)
                    .disabled(disabled || p.offline)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.discord_enabled = *value;
                        cx.notify();
                    })),
            )
            .child(ui::field(
                "settings-discord-channels",
                "頻道 ID（以逗號或空白分隔）",
                &self.channels,
            ))
            .child(ui::secret_field(
                "settings-discord-token",
                "Bot Token",
                &self.discord_token,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("settings-discord-token-save")
                            .small()
                            .label("儲存 Token")
                            .disabled(p.busy || p.offline)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_token(window, cx)
                            })),
                    )
                    .child(
                        Button::new("settings-discord-token-clear")
                            .small()
                            .label("移除 Token")
                            .disabled(p.busy || p.offline || !p.discord_token_saved)
                            .on_click(move |_, window, cx| {
                                let controller = token_controller.clone();
                                ui::confirm(
                                    window,
                                    cx,
                                    "移除 Discord Bot Token？",
                                    "會從系統憑證庫刪除 Token 並停止 Discord 監聽，已匯入的序號不受影響。",
                                    "移除 Token",
                                    move |cx| ui::send(&controller, Command::ClearDiscordToken, cx),
                                );
                            }),
                    )
                    .child(ui::muted(
                        if p.discord_token_saved {
                            "Token 已存在系統憑證庫，不會寫入設定檔"
                        } else {
                            "尚未儲存 Token"
                        },
                        cx,
                    )),
            )
            .child(div().text_sm().child(p.discord.label()));
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

#[cfg(test)]
mod tests {
    #[test]
    fn channel_ids_accept_common_separators_and_reject_non_numbers() {
        assert_eq!(
            super::parse_channels("123, 456 789，123、42").unwrap(),
            [123, 456, 789, 42]
        );
        assert!(super::parse_channels("  ").unwrap().is_empty());
        assert!(super::parse_channels("123 abc").is_err());
        assert!(super::parse_channels("0").is_err());
    }
}
