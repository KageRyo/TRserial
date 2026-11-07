use crate::{
    backend::Command,
    ui::{self, Controller, Row, Rows},
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::InputState,
    table::{DataTable, TableEvent, TableState},
    *,
};
use gpui_kit::*;
use zeroize::Zeroizing;

pub(crate) struct AccountsPage {
    controller: Entity<Controller>,
    table: Entity<TableState<Rows>>,
    selected: Option<String>,
    alias: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    subscriptions: Vec<Subscription>,
}
impl AccountsPage {
    pub(crate) fn new(
        controller: Entity<Controller>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| {
            TableState::new(
                Rows::new(vec![
                    ("alias", "別名", 9.),
                    ("username", "遊戲帳號", 12.),
                    ("auto", "自動處理", 7.),
                    ("credential", "密碼", 11.),
                ]),
                window,
                cx,
            )
        });
        let alias = ui::input("帳號別名", window, cx);
        let username = ui::input("遊戲帳號", window, cx);
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("遊戲密碼")
        });
        let mut this = Self {
            controller: controller.clone(),
            table: table.clone(),
            selected: None,
            alias,
            username,
            password,
            subscriptions: Vec::new(),
        };
        this.subscriptions
            .push(cx.observe_in(&controller, window, |this, _, _, cx| {
                this.refresh(cx);
                cx.notify();
            }));
        this.subscriptions.push(cx.subscribe_in(
            &table,
            window,
            |this, table, event, window, cx| {
                if let TableEvent::SelectRow(ix) = event {
                    let id = table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*ix)
                        .map(|r| r.id.clone());
                    if let Some(id) = id {
                        this.selected = Some(id.clone());
                        let data = this.controller.read(cx).presentation.data.clone();
                        if let Some(a) = data
                            .as_ref()
                            .and_then(|d| d.accounts.iter().find(|a| a.account.id == id))
                        {
                            this.alias.update(cx, |s, cx| {
                                s.set_value(a.account.alias.clone(), window, cx)
                            });
                            this.username.update(cx, |s, cx| {
                                s.set_value(a.account.username.clone(), window, cx)
                            });
                            this.password
                                .update(cx, |s, cx| s.set_value("", window, cx));
                        }
                        cx.notify();
                    }
                }
            },
        ));
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(data) = self.controller.read(cx).presentation.data.clone() {
            let rows = data
                .accounts
                .iter()
                .map(|a| Row {
                    id: a.account.id.clone(),
                    cells: vec![
                        a.account.alias.clone(),
                        a.account.username.clone(),
                        if a.automatic { "啟用" } else { "停用" }.into(),
                        if a.account.test_only {
                            "測試帳號"
                        } else if a.account.session_only {
                            "需要更新密碼"
                        } else {
                            "系統憑證庫"
                        }
                        .into(),
                    ],
                })
                .collect();
            ui::replace_rows(&self.table, rows, cx);
            if self
                .selected
                .as_ref()
                .is_some_and(|id| !data.accounts.iter().any(|a| &a.account.id == id))
            {
                self.selected = None;
            }
        }
    }
    fn new_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        for field in [&self.alias, &self.username, &self.password] {
            field.update(cx, |s, cx| s.set_value("", window, cx));
        }
        cx.notify();
    }
}
impl Render for AccountsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let presentation = self.controller.read(cx).presentation.clone();
        let disabled = presentation.busy || presentation.state.is_busy();
        let selected = self.selected.clone();
        let record = presentation
            .data
            .as_ref()
            .and_then(|d| {
                d.accounts
                    .iter()
                    .find(|a| Some(&a.account.id) == selected.as_ref())
            })
            .cloned();
        let mut form = v_flex()
            .gap_3()
            .w_72()
            .flex_shrink_0()
            .p_4()
            .child(div().font_semibold().child(if selected.is_some() {
                "編輯帳號"
            } else {
                "新增帳號"
            }))
            .child(ui::field("account-alias", "別名", &self.alias));
        if selected.is_none() {
            form = form
                .child(ui::field("account-username", "遊戲帳號", &self.username))
                .child(ui::secret_field("account-secret", "密碼", &self.password))
                .child(
                    Button::new("account-add")
                        .primary()
                        .label("新增帳號")
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let command = Command::AddAccount {
                                alias: this.alias.read(cx).value().to_string(),
                                username: this.username.read(cx).value().to_string(),
                                password: Zeroizing::new(
                                    this.password.read(cx).value().to_string(),
                                ),
                            };
                            ui::send(&this.controller, command, cx);
                            this.password
                                .update(cx, |s, cx| s.set_value("", window, cx));
                        })),
                );
        } else if let Some(record) = record {
            form = form
                .child(
                    Button::new("account-rename")
                        .label("更新別名")
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.selected.clone() {
                                ui::send(
                                    &this.controller,
                                    Command::Rename {
                                        id,
                                        alias: this.alias.read(cx).value().to_string(),
                                    },
                                    cx,
                                );
                            }
                        })),
                )
                .child(ui::muted(
                    format!("遊戲帳號：{}", record.account.username),
                    cx,
                ))
                .child(ui::secret_field("account-secret", "新密碼", &self.password))
                .child(
                    Button::new("account-password")
                        .label("更新密碼")
                        .disabled(disabled || record.account.test_only)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(id) = this.selected.clone() {
                                ui::send(
                                    &this.controller,
                                    Command::Password {
                                        id,
                                        password: Zeroizing::new(
                                            this.password.read(cx).value().to_string(),
                                        ),
                                    },
                                    cx,
                                );
                                this.password
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                            }
                        })),
                );
            let id = record.account.id.clone();
            let controller = self.controller.clone();
            form = form.child(
                Checkbox::new("account-auto")
                    .label("自動處理新序號")
                    .checked(record.automatic)
                    .disabled(disabled || record.account.test_only)
                    .on_change(move |enabled, _, cx| {
                        ui::send(
                            &controller,
                            Command::Automatic {
                                id: id.clone(),
                                enabled: *enabled,
                            },
                            cx,
                        )
                    }),
            );
            let controller = self.controller.clone();
            let alias = record.account.alias.clone();
            let id = record.account.id.clone();
            form = form.child(
                Button::new("account-remove")
                    .danger()
                    .label("移除帳號…")
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        let controller = controller.clone();
                        let id = id.clone();
                        ui::confirm(
                            window,
                            cx,
                            format!("移除「{alias}」？"),
                            "會刪除帳號設定與憑證庫密碼，兌換紀錄保留。",
                            "移除帳號",
                            move |cx| ui::send(&controller, Command::Remove(id.clone()), cx),
                        );
                    }),
            );
        }
        v_flex().size_full().gap_3().child(h_flex().justify_between().child(ui::title("帳號")).child(ui::toolbar_button("account-new","新增帳號").disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.new_account(window,cx)))))
            .child(ui::muted("密碼存入 Windows Credential Manager／Linux Secret Service。自動處理只使用已儲存的密碼。",cx))
            .child(h_flex().items_stretch().flex_1().min_h_0().gap_4()
                .child(div().flex_1().min_w_0().child(DataTable::new(&self.table)))
                .child(form.id("account-form").overflow_y_scrollbar()))
    }
}

#[cfg(test)]
mod tests {
    use super::AccountsPage;
    use crate::testing::{Fixture, wait};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Role, TestAppContext};
    #[gpui_kit::test]
    fn account_form_masks_password_and_keyboard_submission_updates_the_database(
        cx: &mut TestAppContext,
    ) {
        let fixture = Fixture::new();
        let (window, controller, page) = fixture.open(cx, |controller, window, cx| {
            cx.new(|cx| AccountsPage::new(controller, window, cx))
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("account-alias", cx);
            window.input("測試別名", cx);
            window.click("account-username", cx);
            window.input("not-a-real-user", cx);
            window.click("account-secret", cx);
            window.input("FAKE_NOT_SAVED", cx);
            let secret = window.find("account-secret");
            assert_eq!(secret.role(), Some(Role::PasswordInput));
            assert_eq!(secret.value(), None);
            window.press("tab", cx);
            assert_eq!(window.find("account-add").focused(), Some(true));
            window.press("enter", cx);
        })
        .unwrap();
        let p = wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| d.accounts.len() == 1)
        });
        assert_eq!(
            p.data.as_ref().unwrap().accounts[0].account.alias,
            "測試別名"
        );
        fixture.sync(&controller, cx);
        cx.update(|cx| {
            assert_eq!(
                page.read(cx).table.read(cx).delegate().rows[0].cells[0],
                "測試別名"
            );
            assert!(page.read(cx).password.read(cx).value().is_empty());
        });
        let bytes = std::fs::read(fixture.dir.path().join("accounts.sqlite")).unwrap();
        assert!(
            !bytes
                .windows(b"FAKE_NOT_SAVED".len())
                .any(|w| w == b"FAKE_NOT_SAVED")
        );
    }
}
