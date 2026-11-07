use crate::{
    backend::{Backend, Presentation},
    ui::Controller,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, Render, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub(crate) struct Fixture {
    pub(crate) backend: Backend,
    pub(crate) dir: TempDir,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let backend = Backend::start(dir.path().into(), dir.path().join("config.toml"), true);
        wait(&backend, |p| p.data.is_some());
        Self { backend, dir }
    }
    pub(crate) fn open<V: Render>(
        &self,
        cx: &mut TestAppContext,
        build: impl FnOnce(Entity<Controller>, &mut Window, &mut App) -> Entity<V>,
    ) -> (AnyWindowHandle, Entity<Controller>, Entity<V>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let controller = cx.new(|_| Controller::new(self.backend.clone()));
            let (window, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1100.), px(760.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| build(controller.clone(), window, cx),
            )
            .unwrap();
            (window, controller, view)
        })
    }
    pub(crate) fn sync(&self, controller: &Entity<Controller>, cx: &mut TestAppContext) {
        cx.update(|cx| {
            controller.update(cx, |c, cx| {
                c.presentation = self.backend.snapshot();
                cx.notify();
            })
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.backend.request_quit();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.backend.snapshot().closed && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
pub(crate) fn wait(backend: &Backend, ready: impl Fn(&Presentation) -> bool) -> Presentation {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let p = backend.snapshot();
        if ready(&p) {
            return p;
        }
        assert!(p.error.is_none(), "{}", p.error.unwrap_or_default());
        assert!(
            Instant::now() < deadline,
            "GUI action did not settle: state={:?}, loaded={}, busy={}, closed={}, revision={}",
            p.state,
            p.data.is_some(),
            p.busy,
            p.closed,
            p.revision
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
