use crate::{
    backend::Command,
    ui::{self, Controller, Row, Rows},
};
use gpui_kit::component::{
    Disableable, IndexPath, WindowExt,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    table::{DataTable, TableEvent, TableState},
    v_flex,
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div,
};

const STATUSES: [(&str, &str); 9] = [
    ("全部狀態", ""),
    ("待處理", "pending"),
    ("處理中", "processing"),
    ("等待人工驗證", "waiting_verification"),
    ("兌換成功", "success"),
    ("已兌換過", "already_redeemed"),
    ("已過期", "expired"),
    ("失敗", "failed"),
    ("結果不明", "unknown"),
];

pub(crate) struct HistoryPage {
    controller: Entity<Controller>,
    table: Entity<TableState<Rows>>,
    query: Entity<InputState>,
    account: Entity<InputState>,
    status: Entity<SelectState<Vec<&'static str>>>,
    selected: Option<i64>,
    pending_attempts: Option<(i64, u64)>,
    subscriptions: Vec<Subscription>,
}
impl HistoryPage {
    pub(crate) fn new(
        controller: Entity<Controller>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| {
            TableState::new(
                Rows::new(vec![
                    ("code", "序號", 12.),
                    ("account", "帳號", 9.),
                    ("status", "狀態", 10.),
                    ("attempts", "次數", 4.),
                    ("updated", "更新時間", 14.),
                    ("detail", "結果", 30.),
                ]),
                window,
                cx,
            )
        });
        let query = ui::input("搜尋序號", window, cx);
        let account = ui::input("篩選帳號別名或 ID", window, cx);
        let status = cx.new(|cx| {
            SelectState::new(
                STATUSES.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let mut this = Self {
            controller: controller.clone(),
            table: table.clone(),
            query: query.clone(),
            account: account.clone(),
            status: status.clone(),
            selected: None,
            pending_attempts: None,
            subscriptions: Vec::new(),
        };
        this.subscriptions
            .push(cx.observe_in(&controller, window, |this, _, window, cx| {
                this.refresh(cx);
                let attempts = this.controller.read(cx).presentation.attempts.clone();
                if let Some((serial, id, attempts)) = attempts
                    && this.pending_attempts.is_some_and(|(pending_id, previous)| {
                        pending_id == id && previous != serial
                    })
                {
                    this.pending_attempts = None;
                    let content = if attempts.is_empty() {
                        "沒有嘗試紀錄".into()
                    } else {
                        attempts
                            .iter()
                            .map(|a| {
                                format!(
                                    "{}　{}\n{}",
                                    ui::time(a.started_at),
                                    ui::status(&a.status),
                                    a.detail
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    };
                    window.open_dialog(cx, move |dialog, _, _| {
                        dialog
                            .title(format!("工作 {id} 的嘗試紀錄"))
                            .child(content.clone())
                    });
                }
                cx.notify();
            }));
        for field in [&query, &account] {
            this.subscriptions
                .push(cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.refresh(cx);
                        cx.notify();
                    }
                }));
        }
        this.subscriptions.push(cx.subscribe(
            &status,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| {
                this.refresh(cx);
                cx.notify();
            },
        ));
        this.subscriptions
            .push(cx.subscribe(&table, |this, table, event, cx| {
                if let TableEvent::SelectRow(ix) = event {
                    this.selected = table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*ix)
                        .and_then(|r| r.id.parse().ok());
                    cx.notify();
                }
            }));
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_lowercase();
        let account = self.account.read(cx).value().to_lowercase();
        let label = self
            .status
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or("全部狀態");
        let status = STATUSES
            .iter()
            .find(|(title, _)| *title == label)
            .map_or("", |(_, value)| *value);
        if let Some(data) = self.controller.read(cx).presentation.data.clone() {
            let rows = data
                .jobs
                .iter()
                .rev()
                .filter(|j| {
                    j.code.to_lowercase().contains(&query)
                        && (account.is_empty()
                            || j.alias.to_lowercase().contains(&account)
                            || j.account_id == account)
                        && (status.is_empty() || j.status == status)
                })
                .map(|j| Row {
                    id: j.id.to_string(),
                    cells: vec![
                        j.code.clone(),
                        j.alias.clone(),
                        ui::status(&j.status).into(),
                        j.attempts.to_string(),
                        ui::time(j.updated_at),
                        j.detail.clone(),
                    ],
                })
                .collect();
            ui::replace_rows(&self.table, rows, cx);
            if self.selected.is_some_and(|id| {
                !self
                    .table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .any(|r| r.id == id.to_string())
            }) {
                self.selected = None;
            }
        }
    }
}
impl Render for HistoryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.controller.read(cx).presentation.clone();
        let selected = p
            .data
            .as_ref()
            .and_then(|d| d.jobs.iter().find(|j| Some(j.id) == self.selected))
            .cloned();
        let mut footer = v_flex().gap_2();
        if let Some(job) = selected {
            footer = footer
                .child(
                    div()
                        .text_sm()
                        .child(format!("{}／{}：{}", job.code, job.alias, job.detail)),
                )
                .child(ui::muted(
                    if job.submitted {
                        "已進入送出階段"
                    } else {
                        "尚未進入送出階段"
                    },
                    cx,
                ));
            if job.status == "unknown" {
                footer = footer.child(ui::muted("請先核對網站或遊戲紀錄，再決定是否重試", cx));
            }
            let id = job.id;
            let attempts = Button::new("history-attempts")
                .label("查看嘗試紀錄…")
                .disabled(p.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let previous = this
                        .controller
                        .read(cx)
                        .presentation
                        .attempts
                        .as_ref()
                        .map_or(0, |(serial, _, _)| *serial);
                    this.pending_attempts = Some((id, previous));
                    ui::send(&this.controller, Command::Attempts(id), cx);
                }));
            let controller = self.controller.clone();
            let code = job.code.clone();
            let alias = job.alias.clone();
            let retry = Button::new("history-retry")
                .label("人工重試…")
                .disabled(
                    p.busy
                        || p.state.is_busy()
                        || !matches!(job.status.as_str(), "failed" | "unknown"),
                )
                .on_click(move |_, window, cx| {
                    let controller = controller.clone();
                    ui::confirm(
                        window,
                        cx,
                        format!("重試 {code}／{alias}？"),
                        "請先確認網站或遊戲紀錄。再次送出可能重複兌換。",
                        "已確認，重試",
                        move |cx| {
                            ui::send(
                                &controller,
                                Command::Retry {
                                    job: id,
                                    confirmed: true,
                                },
                                cx,
                            )
                        },
                    );
                });
            footer = footer.child(h_flex().gap_2().child(attempts).child(retry));
        } else {
            footer = footer.child(ui::muted("選取工作可查看結果與每次嘗試紀錄", cx));
        }
        v_flex()
            .size_full()
            .gap_3()
            .child(ui::title("兌換紀錄"))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.query).aria_label("搜尋序號")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.account).aria_label("篩選帳號")),
                    )
                    .child(
                        div()
                            .w_40()
                            .child(Select::new(&self.status).accessibility_label("篩選狀態")),
                    ),
            )
            .child(div().flex_1().min_h_0().child(DataTable::new(&self.table)))
            .child(footer)
    }
}
