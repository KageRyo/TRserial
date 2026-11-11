use crate::{
    backend::{Command, RunState},
    ui::{self, Controller, ShowCodes, ShowHistory, ShowSettings},
};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable, StyledExt, button::Button, h_flex, v_flex};
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div,
};

pub(crate) struct Dashboard {
    controller: Entity<Controller>,
    _subscription: Subscription,
}
impl Dashboard {
    pub(crate) fn new(controller: Entity<Controller>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&controller, |_, _, cx| cx.notify());
        Self {
            controller,
            _subscription: subscription,
        }
    }
}
impl Render for Dashboard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.controller.read(cx).presentation.clone();
        let mut body = v_flex()
            .gap_3()
            .p_1()
            .child(ui::title("總覽"))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().font_semibold().child(p.state.label()))
                    .child(ui::muted(
                        if p.data.as_ref().is_some_and(|d| d.config.discord.enabled) {
                            "Discord：桌面監聽未設定"
                        } else {
                            "Discord：停用"
                        },
                        cx,
                    )),
            )
            .child(div().text_sm().child(p.browser_status()));
        if let Some(data) = p.data.as_ref() {
            let counters = [
                ("帳號", data.accounts.len()),
                (
                    "待處理",
                    data.jobs.iter().filter(|j| j.status == "pending").count(),
                ),
                (
                    "成功",
                    data.jobs.iter().filter(|j| j.status == "success").count(),
                ),
                (
                    "失敗",
                    data.jobs.iter().filter(|j| j.status == "failed").count(),
                ),
                (
                    "結果不明",
                    data.jobs.iter().filter(|j| j.status == "unknown").count(),
                ),
            ];
            let mut summary = h_flex().gap_6().py_1();
            for (label, count) in counters {
                summary = summary.child(
                    v_flex()
                        .child(ui::muted(label, cx))
                        .child(div().text_xl().font_semibold().child(count.to_string())),
                );
            }
            body = body
                .child(summary)
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("dashboard-codes")
                                .small()
                                .label("序號與匯入")
                                .on_click(|_, _, cx| cx.dispatch_action(&ShowCodes)),
                        )
                        .child(
                            Button::new("dashboard-history")
                                .small()
                                .label("兌換紀錄")
                                .on_click(|_, _, cx| cx.dispatch_action(&ShowHistory)),
                        )
                        .child(
                            Button::new("dashboard-settings")
                                .small()
                                .label("設定")
                                .on_click(|_, _, cx| cx.dispatch_action(&ShowSettings)),
                        ),
                )
                .child(div().font_semibold().child("最近序號"));
            if data.codes.is_empty() {
                body = body.child(ui::muted(
                    "還沒有序號。到序號頁輸入一組序號，或貼上活動訊息。",
                    cx,
                ));
            }
            for code in data.codes.iter().take(5) {
                body = body.child(
                    h_flex()
                        .gap_3()
                        .child(div().font_medium().child(code.code.clone()))
                        .child(ui::muted(
                            if code.test_only {
                                "測試資料"
                            } else if code.needs_review {
                                "待確認"
                            } else {
                                "已確認"
                            },
                            cx,
                        ))
                        .child(ui::muted(code.rewards.join("、"), cx)),
                );
            }
            body = body.child(div().font_semibold().mt_1().child("最近活動"));
            if data.jobs.is_empty() {
                body = body.child(ui::muted("目前沒有兌換工作", cx));
            }
            for job in data.jobs.iter().rev().take(6) {
                body = body.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(format!(
                            "{}／{}　{}",
                            job.code,
                            job.alias,
                            ui::status(&job.status)
                        )))
                        .child(ui::muted(
                            format!("{}　{}", ui::time(job.updated_at), job.detail),
                            cx,
                        )),
                );
            }
            let controller = self.controller.clone();
            body = body.child(
                Button::new("dashboard-backfill")
                    .small()
                    .label("補抓歷史序號…")
                    .disabled(
                        p.busy
                            || p.state.is_busy()
                            || !data
                                .accounts
                                .iter()
                                .any(|a| a.automatic && !a.account.test_only),
                    )
                    .on_click(move |_, window, cx| {
                        let controller = controller.clone();
                        ui::confirm(
                            window,
                            cx,
                            "為所有已啟用帳號補抓歷史？",
                            "會排入已確認的歷史序號；成功、已兌換及過期的組合仍會略過。",
                            "開始歷史補抓",
                            move |cx| {
                                ui::send(
                                    &controller,
                                    Command::Start {
                                        backfill: true,
                                        confirmed: true,
                                    },
                                    cx,
                                )
                            },
                        );
                    }),
            );
        } else {
            body = body.child(ui::muted("正在讀取本機資料…", cx));
        }
        if p.state == RunState::Paused {
            body = body.child(ui::muted(
                "工作已暫停；已進入送出階段的結果請到兌換紀錄核對",
                cx,
            ));
        }
        div()
            .id("dashboard-scroll")
            .size_full()
            .child(body)
            .overflow_y_scrollbar()
    }
}
