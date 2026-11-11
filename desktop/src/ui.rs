use crate::backend::{Backend, Command, Presentation};
use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt, WindowExt,
    button::Button,
    dialog::DialogButtonProps,
    input::{Input, InputContentType, InputState},
    table::{Column, TableDelegate, TableState},
    v_flex,
};
use gpui_kit::{
    App, AppContext, Context, Div, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, Stateful, Styled, Window, actions, div,
};

pub(crate) struct Controller {
    pub(crate) backend: Backend,
    pub(crate) presentation: Presentation,
    pub(crate) page: Page,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Dashboard,
    Codes,
    Accounts,
    History,
    Settings,
}
actions!(
    trserial_desktop,
    [
        ShowDashboard,
        ShowCodes,
        ShowAccounts,
        ShowHistory,
        ShowSettings,
        ToggleProcessing,
        StopProcessing,
        ShowWindow,
        HideWindow,
        QuitApp
    ]
);
impl Controller {
    pub(crate) fn new(backend: Backend) -> Self {
        let presentation = backend.snapshot();
        Self {
            backend,
            presentation,
            page: Page::Dashboard,
        }
    }
    pub(crate) fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        self.backend.send(command);
        self.presentation.busy = true;
        cx.notify();
    }
}
pub(crate) fn send(controller: &Entity<Controller>, command: Command, cx: &mut App) {
    controller.update(cx, |c, cx| c.send(command, cx));
}
pub(crate) fn input(label: &'static str, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(label))
}
pub(crate) fn field(id: &'static str, label: &'static str, state: &Entity<InputState>) -> Div {
    v_flex()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(Input::new(state).id(id).aria_label(label))
}
pub(crate) fn secret_field(
    id: &'static str,
    label: &'static str,
    state: &Entity<InputState>,
) -> Div {
    v_flex().gap_1().child(div().text_sm().child(label)).child(
        Input::new(state)
            .id(id)
            .aria_label(label)
            .content_type(InputContentType::Password),
    )
}
pub(crate) fn title(text: &'static str) -> Div {
    div().text_lg().font_semibold().child(text)
}
pub(crate) fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
pub(crate) fn status(value: &str) -> &str {
    match value {
        "pending" => "待處理",
        "processing" => "處理中",
        "waiting_verification" => "等待人工驗證",
        "success" => "兌換成功",
        "already_redeemed" => "已兌換過",
        "expired" => "已過期",
        "failed" => "失敗",
        "unknown" => "結果不明",
        _ => value,
    }
}
pub(crate) fn time(value: i64) -> String {
    chrono::DateTime::from_timestamp(value, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| value.to_string())
}
pub(crate) fn confirm(
    window: &mut Window,
    cx: &mut App,
    heading: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: &'static str,
    commit: impl Fn(&mut App) + 'static,
) {
    let heading = heading.into();
    let body = body.into();
    let commit = std::rc::Rc::new(commit);
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let commit = commit.clone();
        dialog
            .title(heading.clone())
            .child(body.clone())
            .button_props(
                DialogButtonProps::default()
                    .ok_text(action)
                    .cancel_text("取消")
                    .show_cancel(true),
            )
            .on_ok(move |_, _, cx| {
                commit(cx);
                true
            })
    });
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) cells: Vec<String>,
}
pub(crate) struct Rows {
    pub(crate) rows: Vec<Row>,
    pub(crate) columns: Vec<(&'static str, &'static str, f32)>,
}
impl Rows {
    pub(crate) fn new(columns: Vec<(&'static str, &'static str, f32)>) -> Self {
        Self {
            rows: Vec::new(),
            columns,
        }
    }
}
impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, ix: usize, cx: &App) -> Column {
        let (key, label, width) = self.columns[ix];
        // DataTable accepts resolved pixels; the source widths are relative to the UI font.
        Column::new(key, label)
            .width(cx.theme().font_size * width)
            .movable(false)
    }
    fn render_tr(
        &mut self,
        ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div().id(SharedString::from(self.rows[ix].id.clone()))
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div().truncate().child(self.rows[row].cells[col].clone())
    }
    fn cell_label(&self, row: usize, col: usize, _: &App) -> Option<SharedString> {
        Some(self.rows[row].cells[col].clone().into())
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        muted("目前沒有資料", cx)
    }
}
pub(crate) fn toolbar_button(id: &'static str, label: &'static str) -> Button {
    Button::new(id).label(label).small()
}
pub(crate) fn replace_rows(table: &Entity<TableState<Rows>>, rows: Vec<Row>, cx: &mut App) {
    table.update(cx, |table, cx| {
        if table.delegate().rows == rows {
            return;
        }
        let selected = table
            .selected_row()
            .and_then(|ix| table.delegate().rows.get(ix))
            .map(|r| r.id.clone());
        table.delegate_mut().rows = rows;
        table.refresh(cx);
        if let Some(ix) = selected
            .as_ref()
            .and_then(|id| table.delegate().rows.iter().position(|r| &r.id == id))
        {
            table.set_selected_row(ix, cx);
        } else {
            table.clear_selection(cx);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Row, Rows, TableState, replace_rows};
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn inserting_a_database_row_preserves_selection_by_identity(cx: &mut TestAppContext) {
        let fixture = crate::testing::Fixture::new();
        let (window, _, view) = fixture.open(cx, |controller, window, cx| {
            cx.new(|cx| crate::accounts::AccountsPage::new(controller, window, cx))
        });
        let table = cx
            .update_window(window, |_, window, cx| {
                cx.new(|cx| TableState::new(Rows::new(vec![("id", "ID", 8.)]), window, cx))
            })
            .unwrap();
        let row = |id: &str| Row {
            id: id.into(),
            cells: vec![id.into()],
        };
        cx.update(|cx| {
            replace_rows(&table, vec![row("old"), row("chosen")], cx);
            table.update(cx, |s, cx| s.set_selected_row(1, cx));
            replace_rows(&table, vec![row("new"), row("old"), row("chosen")], cx);
            assert_eq!(table.read(cx).selected_row(), Some(2));
            replace_rows(&table, vec![row("new")], cx);
            assert_eq!(table.read(cx).selected_row(), None);
        });
        drop(view);
    }
}
