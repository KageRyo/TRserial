use crate::{
    accounts::AccountsPage,
    backend::{Backend, Command, RunState},
    codes::CodesPage,
    dashboard::Dashboard,
    history::HistoryPage,
    settings::SettingsPage,
    ui::{
        self, Controller, HideWindow, Page, QuitApp, ShowAccounts, ShowCodes, ShowDashboard,
        ShowHistory, ShowSettings, ShowWindow, StopProcessing, ToggleProcessing,
    },
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    h_flex,
    sidebar::{Sidebar, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, AsyncApp, Context, Entity, Global, IntoElement, KeyBinding,
    MenuItem, NoAction, ParentElement, QuitMode, Render, Styled, Subscription, Task,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_tray::{Icon, Tray};
use std::time::Duration;

/// Fits a 1366×768 laptop above the taskbar; a small utility should not
/// take over a 1080p screen.
fn window_size() -> gpui_kit::Size<gpui_kit::Pixels> {
    size(px(960.), px(640.))
}
/// Smallest size where the account and code side panels still sit beside
/// their tables.
fn min_window_size() -> gpui_kit::Size<gpui_kit::Pixels> {
    size(px(800.), px(540.))
}
/// GPUI Kit derives text and spacing from the rem size. 15px instead of the
/// 16px default gives a compact utility layout while secondary text stays at
/// about 13px.
fn compact_theme(theme: &mut Theme) {
    theme.font_size = px(15.);
    #[cfg(target_os = "linux")]
    {
        theme.font_family = "Noto Sans CJK TC".into();
    }
    #[cfg(windows)]
    {
        theme.font_family = "Segoe UI".into();
    }
}

struct Shell {
    controller: Entity<Controller>,
    dashboard: Entity<Dashboard>,
    codes: Entity<CodesPage>,
    accounts: Entity<AccountsPage>,
    history: Entity<HistoryPage>,
    settings: Entity<SettingsPage>,
    _subscription: Subscription,
}
impl Shell {
    fn new(controller: Entity<Controller>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dashboard = cx.new(|cx| Dashboard::new(controller.clone(), cx));
        let codes = cx.new(|cx| CodesPage::new(controller.clone(), window, cx));
        let accounts = cx.new(|cx| AccountsPage::new(controller.clone(), window, cx));
        let history = cx.new(|cx| HistoryPage::new(controller.clone(), window, cx));
        let settings = cx.new(|cx| SettingsPage::new(controller.clone(), window, cx));
        let subscription = cx.observe(&controller, |_, _, cx| cx.notify());
        Self {
            controller,
            dashboard,
            codes,
            accounts,
            history,
            settings,
            _subscription: subscription,
        }
    }
}
impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controller = self.controller.read(cx);
        let p = controller.presentation.clone();
        let page = controller.page;
        let content = match page {
            Page::Dashboard => self.dashboard.clone().into_any_element(),
            Page::Codes => self.codes.clone().into_any_element(),
            Page::Accounts => self.accounts.clone().into_any_element(),
            Page::History => self.history.clone().into_any_element(),
            Page::Settings => self.settings.clone().into_any_element(),
        };
        let sidebar = Sidebar::new("main-sidebar")
            .w_48()
            .header(SidebarHeader::new().child(div().font_semibold().text_lg().child("TRserial")))
            .child(
                SidebarMenu::new()
                    .child(
                        SidebarMenuItem::new("總覽")
                            .active(page == Page::Dashboard)
                            .on_click(|_, _, cx| cx.dispatch_action(&ShowDashboard)),
                    )
                    .child(
                        SidebarMenuItem::new("序號")
                            .active(page == Page::Codes)
                            .on_click(|_, _, cx| cx.dispatch_action(&ShowCodes)),
                    )
                    .child(
                        SidebarMenuItem::new("帳號")
                            .active(page == Page::Accounts)
                            .on_click(|_, _, cx| cx.dispatch_action(&ShowAccounts)),
                    )
                    .child(
                        SidebarMenuItem::new("兌換紀錄")
                            .active(page == Page::History)
                            .on_click(|_, _, cx| cx.dispatch_action(&ShowHistory)),
                    )
                    .child(
                        SidebarMenuItem::new("設定")
                            .active(page == Page::Settings)
                            .on_click(|_, _, cx| cx.dispatch_action(&ShowSettings)),
                    ),
            )
            .footer(SidebarFooter::new().child(ui::muted("跑Online\nTalesRunner", cx)));
        let active = matches!(p.state, RunState::Running | RunState::Manual);
        let mut workspace = v_flex().flex_1().min_w_0().min_h_0().gap_2().p_3().child(
            h_flex()
                .gap_2()
                .child(div().flex_1().text_sm().child(p.state.label()))
                .child(
                    Button::new("processing-toggle")
                        .small()
                        .label(if active {
                            "暫停"
                        } else {
                            "開始自動處理"
                        })
                        .disabled(p.busy || p.data.is_none() || (p.state.is_busy() && !active))
                        .on_click(|_, _, cx| cx.dispatch_action(&ToggleProcessing)),
                )
                .child(
                    Button::new("processing-stop")
                        .small()
                        .label("停止")
                        .disabled(p.busy || !p.state.is_busy())
                        .on_click(|_, _, cx| cx.dispatch_action(&StopProcessing)),
                )
                .child(
                    Button::new("app-quit")
                        .small()
                        .ghost()
                        .label("結束")
                        .on_click(|_, _, cx| cx.dispatch_action(&QuitApp)),
                ),
        );
        if p.offline {
            workspace = workspace.child(ui::muted(
                "離線測試模式：只使用測試資料，不連線到兌換網站",
                cx,
            ));
        }
        if !p.tray_host
            || cx
                .try_global::<DesktopApp>()
                .is_none_or(|app| app.tray.is_none())
        {
            workspace = workspace.child(ui::muted("系統匣不可用；關閉視窗會結束程式。", cx));
        }
        if let Some(message) = p.error.clone().or(p.notice.clone()) {
            let controller = self.controller.clone();
            let color = if p.error.is_some() {
                cx.theme().danger
            } else {
                cx.theme().muted_foreground
            };
            workspace = workspace.child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().text_sm().text_color(color).child(message))
                    .child(
                        Button::new("notice-dismiss")
                            .ghost()
                            .small()
                            .label("關閉訊息")
                            .on_click(move |_, _, cx| ui::send(&controller, Command::Dismiss, cx)),
                    ),
            );
        }
        workspace = workspace.child(div().flex_1().min_h_0().min_w_0().child(content));
        h_flex()
            .items_stretch()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sidebar)
            .child(workspace)
    }
}

struct DesktopApp {
    backend: Backend,
    controller: Entity<Controller>,
    window: Option<AnyWindowHandle>,
    tray: Option<Tray>,
    quitting: bool,
    hidden: bool,
    _quit: Subscription,
    _poll: Task<()>,
}
impl Global for DesktopApp {}
fn navigate(page: Page, cx: &mut App) {
    show(cx);
    let controller = cx.global::<DesktopApp>().controller.clone();
    controller.update(cx, |c, cx| {
        c.page = page;
        cx.notify();
    });
}
fn show(cx: &mut App) {
    let app = cx.global::<DesktopApp>();
    let existing = app.window;
    let controller = app.controller.clone();
    if let Some(handle) = existing {
        // A command dispatched by this window already holds its update lease.
        // Failure to re-enter that lease must not create a second window.
        let _ = handle.update(cx, |_, window, cx| {
            #[cfg(windows)]
            if let Err(error) = crate::platform::set_window_visible(window, true) {
                cx.global::<DesktopApp>()
                    .backend
                    .smoke_error(error.to_string());
                return;
            }
            window.activate_window();
            cx.global_mut::<DesktopApp>().hidden = false;
        });
        return;
    }
    controller.update(cx, |c, _| c.presentation = c.backend.snapshot());
    let options = WindowOptions {
        // Native window geometry is intentionally independent of control spacing.
        window_bounds: Some(WindowBounds::centered(window_size(), cx)),
        window_min_size: Some(min_window_size()),
        titlebar: Some(TitlebarOptions {
            title: Some("TRserial".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(controller.clone(), window, cx))
    })
    .expect("無法開啟 TRserial 視窗");
    handle
        .update(cx, |_, window, cx| {
            window.on_window_should_close(cx, |window, cx| {
                if cx.global::<DesktopApp>().quitting {
                    return true;
                }
                if tray_available(cx) {
                    hide_window(window, cx);
                } else {
                    quit(cx);
                }
                false
            });
        })
        .ok();
    cx.global_mut::<DesktopApp>().window = Some(handle);
    cx.global_mut::<DesktopApp>().hidden = false;
}
fn tray_available(cx: &App) -> bool {
    let app = cx.global::<DesktopApp>();
    app.tray.is_some() && app.controller.read(cx).presentation.tray_host
}
fn hide(cx: &mut App) {
    if !tray_available(cx) {
        show(cx);
        return;
    }
    if let Some(handle) = cx.global::<DesktopApp>().window {
        let _ = handle.update(cx, |_, window, cx| hide_window(window, cx));
    }
}
fn hide_window(window: &mut Window, cx: &mut App) {
    #[cfg(windows)]
    if let Err(error) = crate::platform::set_window_visible(window, false) {
        cx.global::<DesktopApp>()
            .backend
            .smoke_error(error.to_string());
        return;
    }
    #[cfg(not(windows))]
    {
        cx.global_mut::<DesktopApp>().window = None;
        window.remove_window();
    }
    cx.global_mut::<DesktopApp>().hidden = true;
}

fn quit(cx: &mut App) {
    let app = cx.global_mut::<DesktopApp>();
    app.quitting = true;
    app.backend.request_quit();
}
fn toggle(cx: &mut App) {
    let controller = cx.global::<DesktopApp>().controller.clone();
    let state = controller.read(cx).presentation.state;
    let command = if matches!(state, RunState::Running | RunState::Manual) {
        Command::Pause
    } else {
        Command::Start {
            backfill: false,
            confirmed: false,
        }
    };
    ui::send(&controller, command, cx);
}
fn menu(cx: &mut App) -> Vec<MenuItem> {
    let p = cx
        .global::<DesktopApp>()
        .controller
        .read(cx)
        .presentation
        .clone();
    vec![
        MenuItem::action(format!("狀態：{}", p.state.label()), NoAction).disabled(true),
        MenuItem::separator(),
        MenuItem::action("顯示視窗", ShowWindow),
        MenuItem::action("隱藏視窗", HideWindow).disabled(!tray_available(cx)),
        MenuItem::action(
            if p.state == RunState::Running {
                "暫停自動處理"
            } else {
                "開始自動處理"
            },
            ToggleProcessing,
        )
        .disabled(p.state.is_busy() && p.state != RunState::Running),
        MenuItem::action("設定…", ShowSettings),
        MenuItem::separator(),
        MenuItem::action("結束 TRserial", QuitApp),
    ]
}
fn icon() -> gpui_tray::Result<Icon> {
    // Original bitmap mark for native tray APIs; no game assets are bundled.
    let mut rgba = vec![0; 32 * 32 * 4];
    for y in 2..30 {
        for x in 2..30 {
            let i = (y * 32 + x) * 4;
            let t = (7..25).contains(&x) && (8..13).contains(&y)
                || (13..19).contains(&x) && (8..25).contains(&y);
            rgba[i..i + 4].copy_from_slice(if t {
                &[255, 255, 255, 255]
            } else {
                &[37, 99, 235, 255]
            });
        }
    }
    Icon::from_rgba(rgba, 32, 32).build()
}

fn synchronize(snapshot: &crate::backend::Presentation, cx: &mut App) -> bool {
    let controller = cx.global::<DesktopApp>().controller.clone();
    let revision = controller.read(cx).presentation.revision;
    if snapshot.revision != revision {
        let old_dark = controller.read(cx).presentation.preferences.dark;
        controller.update(cx, |c, cx| {
            c.presentation = snapshot.clone();
            cx.notify();
        });
        if old_dark != snapshot.preferences.dark {
            Theme::change(
                if snapshot.preferences.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
        }
        if let Some(tray) = cx.global::<DesktopApp>().tray.clone() {
            let _ = tray.refresh_menu(cx);
        }
    }
    if cx.global::<DesktopApp>().hidden
        && !tray_available(cx)
        && !cx.global::<DesktopApp>().quitting
    {
        show(cx);
    }
    if snapshot.closed && cx.global::<DesktopApp>().quitting {
        if let Some(tray) = cx.global::<DesktopApp>().tray.clone() {
            let _ = tray.close(cx);
        }
        cx.quit();
        true
    } else {
        false
    }
}

pub(crate) fn run(backend: Backend, smoke: bool) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::update(cx, compact_theme);
            Theme::change(
                if backend.snapshot().preferences.dark {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                },
                None,
                cx,
            );
            cx.bind_keys([
                KeyBinding::new("ctrl-q", QuitApp, None),
                KeyBinding::new("ctrl-,", ShowSettings, None),
                KeyBinding::new("ctrl-1", ShowDashboard, None),
                KeyBinding::new("ctrl-2", ShowCodes, None),
                KeyBinding::new("ctrl-3", ShowAccounts, None),
                KeyBinding::new("ctrl-4", ShowHistory, None),
            ]);
            cx.on_action(|_: &ShowDashboard, cx| navigate(Page::Dashboard, cx))
                .on_action(|_: &ShowCodes, cx| navigate(Page::Codes, cx))
                .on_action(|_: &ShowAccounts, cx| navigate(Page::Accounts, cx))
                .on_action(|_: &ShowHistory, cx| navigate(Page::History, cx))
                .on_action(|_: &ShowSettings, cx| navigate(Page::Settings, cx))
                .on_action(|_: &ShowWindow, cx| show(cx))
                .on_action(|_: &HideWindow, cx| hide(cx))
                .on_action(|_: &ToggleProcessing, cx| toggle(cx))
                .on_action(|_: &StopProcessing, cx| {
                    let c = cx.global::<DesktopApp>().controller.clone();
                    ui::send(&c, Command::Stop, cx);
                })
                .on_action(|_: &QuitApp, cx| quit(cx));
            let controller = cx.new(|_| Controller::new(backend.clone()));
            let quit_backend = backend.clone();
            let quit_subscription = cx.on_app_quit(move |_| {
                let backend = quit_backend.clone();
                async move {
                    backend.request_quit();
                    backend.wait_closed().await;
                }
            });
            let polling_backend = backend.clone();
            let poll = cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(160))
                        .await;
                    let snapshot = polling_backend.snapshot();
                    let exit = cx.update(|cx| synchronize(&snapshot, cx));
                    if exit {
                        break;
                    }
                }
            });
            cx.set_global(DesktopApp {
                backend: backend.clone(),
                controller,
                window: None,
                tray: None,
                quitting: false,
                hidden: false,
                _quit: quit_subscription,
                _poll: poll,
            });
            show(cx);
            let tray = icon().and_then(|icon| {
                Tray::builder()
                    .icon(icon)
                    .tooltip("TRserial")
                    .title("TRserial")
                    .menu(menu)
                    .on_activate_with(|_, cx| show(cx))
                    .build(cx)
            });
            if let Ok(tray) = tray {
                cx.global_mut::<DesktopApp>().tray = Some(tray);
            }
            if smoke {
                let smoke_backend = backend.clone();
                cx.spawn(async move |cx| {
                    if let Err(error) = native_smoke(&smoke_backend, cx).await {
                        smoke_backend.smoke_error(error.to_string());
                    }
                    cx.update(quit);
                })
                .detach();
            }
        });
}

async fn wait_smoke(
    backend: &Backend,
    cx: &mut AsyncApp,
    ready: impl Fn(&crate::backend::Presentation) -> bool,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let p = backend.snapshot();
        anyhow::ensure!(p.error.is_none(), "{}", p.error.unwrap_or_default());
        if ready(&p) {
            return Ok(());
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "原生視窗測試等待狀態逾時"
        );
        cx.background_executor()
            .timer(Duration::from_millis(30))
            .await;
    }
}
async fn native_smoke(backend: &Backend, cx: &mut AsyncApp) -> anyhow::Result<()> {
    wait_smoke(backend, cx, |p| p.data.is_some()).await?;
    backend.send(Command::AddAccount {
        alias: "smoke".into(),
        username: "fixture-user".into(),
        password: zeroize::Zeroizing::new("fixture-password".into()),
    });
    wait_smoke(backend, cx, |p| {
        p.data.as_ref().is_some_and(|d| !d.accounts.is_empty())
    })
    .await?;
    backend.send(Command::Ingest(
        "序號: DesktopSmoke1\n獎勵: 測試資料".into(),
    ));
    wait_smoke(backend, cx, |p| {
        p.data.as_ref().is_some_and(|d| !d.codes.is_empty())
    })
    .await?;
    for page in [
        Page::Dashboard,
        Page::Accounts,
        Page::Codes,
        Page::History,
        Page::Settings,
    ] {
        cx.update(|cx| match page {
            Page::Dashboard => cx.dispatch_action(&ShowDashboard),
            Page::Codes => cx.dispatch_action(&ShowCodes),
            Page::Accounts => cx.dispatch_action(&ShowAccounts),
            Page::History => cx.dispatch_action(&ShowHistory),
            Page::Settings => cx.dispatch_action(&ShowSettings),
        });
        cx.background_executor()
            .timer(Duration::from_millis(100))
            .await;
        cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1, "navigation created another window");
            assert_eq!(cx.global::<DesktopApp>().controller.read(cx).page, page);
            cx.global::<DesktopApp>()
                .window
                .expect("main window")
                .update(cx, |_, window, cx| {
                    window.draw(cx).clear(cx);
                })
                .expect("render page");
        });
    }
    backend.send(Command::Start {
        backfill: false,
        confirmed: false,
    });
    wait_smoke(backend, cx, |p| p.state == RunState::Running).await?;
    if std::env::var_os("TRSERIAL_REQUIRE_TRAY").is_some() {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while !cx.update(|cx| tray_available(cx)) {
            anyhow::ensure!(
                std::time::Instant::now() < deadline,
                "系統匣未向測試 host 註冊"
            );
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
        }
    }
    let tray = cx.update(|cx| tray_available(cx));
    cx.update(hide);
    if tray {
        anyhow::ensure!(
            cx.update(|cx| cx.global::<DesktopApp>().hidden),
            "視窗未收起"
        );
        anyhow::ensure!(
            backend.snapshot().state == RunState::Running,
            "收起視窗中止了背景工作"
        );
        #[cfg(target_os = "linux")]
        crate::platform::activate_own_tray().await?;
        #[cfg(not(target_os = "linux"))]
        cx.update(|cx| cx.dispatch_action(&ShowWindow));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while cx.update(|cx| cx.global::<DesktopApp>().hidden) {
            anyhow::ensure!(std::time::Instant::now() < deadline, "系統匣未還原視窗");
            cx.background_executor()
                .timer(Duration::from_millis(30))
                .await;
        }
        eprintln!("TRserial native tray hide/restore passed");
    } else {
        anyhow::ensure!(
            cx.update(|cx| cx.global::<DesktopApp>().window.is_some()
                && !cx.global::<DesktopApp>().hidden),
            "無系統匣時遺失唯一視窗"
        );
        eprintln!("TRserial tray fallback kept the window visible");
    }
    backend.send(Command::Pause);
    wait_smoke(backend, cx, |p| p.state == RunState::Paused).await?;
    backend.send(Command::Stop);
    wait_smoke(backend, cx, |p| p.state == RunState::Stopped).await?;
    let p = backend.snapshot();
    let data = p.data.as_ref().expect("loaded snapshot");
    anyhow::ensure!(
        data.jobs.is_empty()
            && data.codes.iter().all(|c| c.test_only)
            && data.accounts.iter().all(|a| a.account.test_only),
        "離線測試資料隔離失效"
    );
    backend.smoke_complete();
    eprintln!("TRserial native pages, persisted state, worker start/pause/stop passed");
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod native_render_tests {
    use super::Shell;
    use crate::{
        backend::Command,
        testing::{Fixture, wait},
        ui::{Controller, Page},
    };
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Bounds, HeadlessAppContext, WindowBounds, WindowOptions};
    use std::{path::PathBuf, sync::Arc};

    #[test]
    #[ignore = "requires GPUI's real Vulkan renderer and installed CJK fonts"]
    fn capture_all_pages_with_native_renderer() {
        let fixture = Fixture::new();
        fixture.backend.send(Command::AddAccount {
            alias: "測試帳號".into(),
            username: "fixture-user".into(),
            password: zeroize::Zeroizing::new("fake".into()),
        });
        wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.accounts.is_empty())
        });
        fixture.backend.send(Command::Ingest(
            "序號: DesktopSmoke1\n獎勵: 測試資料\n到期: 2026-12-31".into(),
        ));
        let p = wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.codes.is_empty())
        });
        let account = p.data.unwrap().accounts[0].account.clone();
        let mut store =
            trserial::store::Store::open(&fixture.dir.path().join("history.sqlite")).unwrap();
        store.enqueue("DesktopSmoke1", &[account]).unwrap();
        let id = store.jobs().unwrap()[0].id;
        store.claim(id, 3).unwrap();
        store.mark_submitted(id).unwrap();
        store
            .finish(
                id,
                trserial::store::Status::Unknown,
                "本機測試：結果待核對",
                0,
            )
            .unwrap();
        wait(&fixture.backend, |p| {
            p.data.as_ref().is_some_and(|d| !d.jobs.is_empty())
        });
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(gpui_kit::assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(gpui_kit::init);
        cx.update(|cx| Theme::update(cx, super::compact_theme));
        let controller = cx.update(|cx| cx.new(|_| Controller::new(fixture.backend.clone())));
        let destination = std::env::var_os("TRSERIAL_SCREENSHOTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/desktop-screenshots"));
        std::fs::create_dir_all(&destination).unwrap();
        for (size_name, window_size) in [
            ("default", super::window_size()),
            ("min", super::min_window_size()),
        ] {
            let window = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: window_size,
                        })),
                        show: false,
                        focus: false,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| Shell::new(controller.clone(), window, cx)),
                )
                .unwrap()
                .0
            });
            for (theme_name, theme) in [("light", ThemeMode::Light), ("dark", ThemeMode::Dark)] {
                cx.update(|cx| Theme::change(theme, None, cx));
                for (name, page) in [
                    ("dashboard", Page::Dashboard),
                    ("accounts", Page::Accounts),
                    ("codes", Page::Codes),
                    ("history", Page::History),
                    ("settings", Page::Settings),
                ] {
                    cx.update(|cx| {
                        controller.update(cx, |c, cx| {
                            c.page = page;
                            cx.notify();
                        })
                    });
                    cx.update_window(window, |_, window, cx| window.render_frame(cx))
                        .unwrap();
                    cx.capture_screenshot(window)
                        .unwrap()
                        .save(destination.join(format!("{name}-{theme_name}-{size_name}.png")))
                        .unwrap();
                }
            }
        }
    }
}
