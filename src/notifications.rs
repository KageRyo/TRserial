use std::sync::Arc;
pub trait Notifier: Send + Sync {
    fn send(&self, title: &str, body: &str);
}
pub struct DesktopNotifier {
    pub enabled: bool,
}
impl Notifier for DesktopNotifier {
    fn send(&self, title: &str, body: &str) {
        if !self.enabled {
            return;
        }
        // Escaping avoids notification-server markup interpreting account aliases.
        fn escape(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }
        let title = escape(title);
        let body = escape(body);
        if notify_rust::Notification::new()
            .appname("TRserial")
            .summary(&title)
            .body(&body)
            .show()
            .is_err()
        {
            tracing::warn!("桌面通知無法送達；結果仍已寫入資料庫");
        }
    }
}
pub fn desktop(enabled: bool) -> Arc<dyn Notifier> {
    Arc::new(DesktopNotifier { enabled })
}
