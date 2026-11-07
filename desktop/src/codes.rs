use crate::{
    backend::Command,
    ui::{self, Controller, Row, Rows},
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    table::{DataTable, TableEvent, TableState},
    *,
};
use gpui_kit::*;
use std::collections::HashSet;

pub(crate) struct CodesPage {
    controller: Entity<Controller>,
    table: Entity<TableState<Rows>>,
    entry: Entity<InputState>,
    selected: Option<String>,
    accounts: HashSet<String>,
    subscriptions: Vec<Subscription>,
}
impl CodesPage {
    pub(crate) fn new(
        controller: Entity<Controller>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| {
            TableState::new(
                Rows::new(vec![
                    ("code", "序號", 12.),
                    ("review", "確認狀態", 8.),
                    ("reward", "訊息獎勵", 24.),
                    ("expiry", "到期文字", 14.),
                ]),
                window,
                cx,
            )
        });
        let entry = ui::input("輸入一組序號", window, cx);
        let mut this = Self {
            controller: controller.clone(),
            table: table.clone(),
            entry,
            selected: None,
            accounts: HashSet::new(),
            subscriptions: Vec::new(),
        };
        this.subscriptions
            .push(cx.observe_in(&controller, window, |this, _, window, cx| {
                this.refresh(window, cx);
                cx.notify();
            }));
        this.subscriptions.push(
            cx.subscribe_in(&table, window, |this, table, event, _, cx| {
                if let TableEvent::SelectRow(ix) = event {
                    this.selected = table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*ix)
                        .map(|r| r.id.clone());
                    cx.notify();
                }
            }),
        );
        this.refresh(window, cx);
        this
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let presentation = self.controller.read(cx).presentation.clone();
        if let Some(data) = presentation.data.as_ref() {
            let rows = data
                .codes
                .iter()
                .map(|c| Row {
                    id: c.code.clone(),
                    cells: vec![
                        c.code.clone(),
                        if c.test_only {
                            "測試資料"
                        } else if c.needs_review {
                            "待確認"
                        } else {
                            "已確認"
                        }
                        .into(),
                        c.rewards.join("、"),
                        c.expiration.clone().unwrap_or_else(|| "未提供".into()),
                    ],
                })
                .collect();
            ui::replace_rows(&self.table, rows, cx);
            self.accounts.retain(|id| {
                data.accounts
                    .iter()
                    .any(|a| a.account.id == *id && !a.account.test_only)
            });
        }
        if let Some((serial, _)) = presentation.plan.as_ref()
            && let Some(plan) = self.controller.read(cx).backend.consume_plan(*serial)
        {
            let controller = self.controller.clone();
            let aliases = plan
                .accounts
                .iter()
                .map(|(_, alias)| alias.as_str())
                .collect::<Vec<_>>()
                .join("、");
            let body = format!(
                "帳號：{aliases}\n{}",
                if plan.historical {
                    "包含歷史序號。已完成的帳號／序號組合會略過。"
                } else {
                    "程式會開啟官方表單；需要驗證時，請在 Chrome 完成。"
                }
            );
            ui::confirm(
                window,
                cx,
                format!("兌換 {}？", plan.code),
                body,
                "開始兌換",
                move |cx| {
                    ui::send(
                        &controller,
                        Command::Redeem {
                            plan: plan.clone(),
                            confirmed: true,
                        },
                        cx,
                    )
                },
            );
        }
    }
    fn import(&self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("貼上序號訊息、獎勵或到期文字")
        });
        let controller = self.controller.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let controller = controller.clone();
            let text_for_button = text.clone();
            dialog.title("匯入訊息").child(Textarea::new(&text)).footer(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("import-cancel")
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("import-save").primary().label("匯入").on_click(
                        move |_, window, cx| {
                            ui::send(
                                &controller,
                                Command::Ingest(text_for_button.read(cx).value().to_string()),
                                cx,
                            );
                            window.close_dialog(cx);
                        },
                    )),
            )
        });
    }
}
impl Render for CodesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.controller.read(cx).presentation.clone();
        let busy = p.busy || p.state.is_busy();
        let record = p
            .data
            .as_ref()
            .and_then(|d| {
                d.codes
                    .iter()
                    .find(|c| Some(&c.code) == self.selected.as_ref())
            })
            .cloned();
        let mut detail = v_flex().gap_3().w_72().flex_shrink_0().p_4();
        if let Some(record) = record {
            detail = detail
                .child(div().text_lg().font_semibold().child(record.code.clone()))
                .child(ui::muted(
                    if record.test_only {
                        "測試序號，僅供離線測試"
                    } else if record.needs_review {
                        "待人工確認"
                    } else {
                        "已確認"
                    },
                    cx,
                ))
                .child(div().text_sm().child(format!(
                    "訊息獎勵：{}",
                    if record.rewards.is_empty() {
                        "未提供".into()
                    } else {
                        record.rewards.join("、")
                    }
                )))
                .child(ui::muted(
                    format!(
                        "到期文字：{}",
                        record.expiration.as_deref().unwrap_or("未提供")
                    ),
                    cx,
                ));
            if record.needs_review {
                let controller = self.controller.clone();
                let code = record.code.clone();
                detail = detail.child(
                    Button::new("code-approve")
                        .label("確認候選序號")
                        .disabled(p.busy)
                        .on_click(move |_, _, cx| {
                            ui::send(&controller, Command::Approve(code.clone()), cx)
                        }),
                );
            }
            detail = detail.child(div().font_medium().child("選取兌換帳號"));
            if let Some(data) = p.data.as_ref() {
                for a in &data.accounts {
                    let id = a.account.id.clone();
                    let id_for_callback = id.clone();
                    detail = detail.child(
                        Checkbox::new(SharedString::from(format!("redeem-account-{id}")))
                            .label(a.account.alias.clone())
                            .checked(self.accounts.contains(&id))
                            .disabled(busy || a.account.test_only)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                if *checked {
                                    this.accounts.insert(id_for_callback.clone());
                                } else {
                                    this.accounts.remove(&id_for_callback);
                                }
                                cx.notify();
                            })),
                    );
                }
                if data.accounts.is_empty() {
                    detail = detail.child(ui::muted("先到帳號頁新增遊戲帳號", cx));
                }
            }
            let code = record.code;
            detail = detail.child(
                Button::new("code-redeem")
                    .primary()
                    .label("兌換…")
                    .disabled(
                        busy || record.test_only || record.needs_review || self.accounts.is_empty(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        ui::send(
                            &this.controller,
                            Command::Plan {
                                code: code.clone(),
                                accounts: this.accounts.iter().cloned().collect(),
                            },
                            cx,
                        )
                    })),
            );
        } else {
            detail = detail.child(ui::muted("從列表選取一組序號，查看獎勵與兌換帳號", cx));
        }
        v_flex()
            .size_full()
            .gap_3()
            .child(
                h_flex().justify_between().child(ui::title("序號")).child(
                    ui::toolbar_button("import-open", "匯入訊息…")
                        .disabled(p.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.import(window, cx))),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.entry).aria_label("序號")),
                    )
                    .child(
                        Button::new("code-ingest")
                            .label("加入序號")
                            .disabled(p.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let value = this.entry.read(cx).value().to_string();
                                ui::send(
                                    &this.controller,
                                    Command::Ingest(format!("序號：{}", value.trim())),
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_stretch()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().child(DataTable::new(&self.table)))
                    .child(detail.id("code-detail").overflow_y_scrollbar()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{CodesPage, Command};
    use crate::testing::{Fixture, wait};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn approving_a_candidate_updates_the_table_but_keeps_mock_redemption_disabled(
        cx: &mut TestAppContext,
    ) {
        let fixture = Fixture::new();
        fixture
            .backend
            .send(Command::Ingest("CandidateGui1".into()));
        wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.codes.is_empty())
        });
        let (window, controller, page) = fixture.open(cx, |controller, window, cx| {
            cx.new(|cx| CodesPage::new(controller, window, cx))
        });
        cx.update(|cx| {
            page.update(cx, |page, cx| {
                page.selected = Some("CandidateGui1".into());
                cx.notify();
            })
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("code-approve", cx);
        })
        .unwrap();
        let p = wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.codes[0].needs_review)
        });
        assert!(p.data.unwrap().codes[0].test_only);
        fixture.sync(&controller, cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("code-redeem").disabled(), Some(true));
        })
        .unwrap();
        assert!(fixture.backend.snapshot().data.unwrap().jobs.is_empty());
    }
}
