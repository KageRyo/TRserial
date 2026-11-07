use anyhow::{Context, Result, ensure};
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

pub(crate) fn login_command(executable: &Path, dir: &Path) -> Result<String> {
    fn quote(value: &Path) -> Result<String> {
        let value = value.to_str().context("啟動路徑必須是有效文字")?;
        ensure!(
            !value.contains(['\n', '\r', '\0']),
            "啟動路徑不可包含換行字元"
        );
        #[cfg(windows)]
        {
            // Windows file names cannot contain quotes. Double trailing slashes
            // before the closing quote so CommandLineToArgvW preserves the path.
            ensure!(!value.contains('"'), "啟動路徑不可包含引號");
            let trailing = value.chars().rev().take_while(|c| *c == '\\').count();
            Ok(format!("\"{value}{}\"", "\\".repeat(trailing)))
        }
        #[cfg(not(windows))]
        {
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            Ok(format!("\"{escaped}\""))
        }
    }
    Ok(format!("{} --data-dir {}", quote(executable)?, quote(dir)?))
}

#[cfg(target_os = "linux")]
fn login_file() -> Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("無法取得使用者設定目錄")?
        .config_dir()
        .join("autostart/trserial.desktop"))
}
pub(crate) fn login_enabled() -> bool {
    #[cfg(target_os = "linux")]
    {
        login_file().is_ok_and(|path| path.is_file())
    }
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
            .and_then(|key| key.get_value::<String, _>("TRserial"))
            .is_ok()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}
pub(crate) fn set_login(enabled: bool, dir: &Path) -> Result<()> {
    let command = login_command(&std::env::current_exe()?, dir)?;
    #[cfg(target_os = "linux")]
    {
        let path = login_file()?;
        if enabled {
            std::fs::create_dir_all(path.parent().context("無法取得自動啟動目錄")?)?;
            std::fs::write(
                path,
                format!(
                    "[Desktop Entry]\nType=Application\nName=TRserial\nComment=跑Online序號兌換\nExec={command}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
                ),
            )?;
        } else if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_CURRENT_USER};
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
        if enabled {
            key.set_value("TRserial", &command)?;
        } else {
            match key.delete_value("TRserial") {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (enabled, command);
        anyhow::bail!("此平台尚未支援登入時啟動")
    }
}

/// Keep the GPUI window alive while the Windows shell hides its HWND.
#[cfg(windows)]
pub(crate) fn set_window_visible(window: &gpui_kit::Window, visible: bool) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, SW_HIDE, SW_SHOW, ShowWindow,
    };
    let handle = HasWindowHandle::window_handle(window)?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        anyhow::bail!("無法取得 TRserial 的 Windows 視窗");
    };
    // GPUI owns this live HWND; the call runs on its UI thread and keeps ownership intact.
    // ShowWindow returns the previous visibility, not a success/failure flag.
    let hwnd = handle.hwnd.get() as _;
    unsafe {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        ensure!(
            (IsWindowVisible(hwnd) != 0) == visible,
            "無法切換 TRserial 視窗顯示狀態"
        );
    }
    Ok(())
}

/// A watcher alone is insufficient: the shell must host our registered item.
#[cfg(target_os = "linux")]
pub(crate) async fn tray_host_available() -> bool {
    async fn probe() -> Result<bool> {
        let connection = zbus::Connection::session().await?;
        let watcher = zbus::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        )
        .await?;
        if !watcher
            .get_property::<bool>("IsStatusNotifierHostRegistered")
            .await?
        {
            return Ok(false);
        }
        let items = watcher
            .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
            .await?;
        let bus = zbus::Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        for item in items {
            let service = item.split('/').next().unwrap_or_default();
            if bus
                .call::<_, _, u32>("GetConnectionUnixProcessID", &(service,))
                .await
                .ok()
                == Some(std::process::id())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    tokio::time::timeout(std::time::Duration::from_millis(750), probe())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false)
}
#[cfg(target_os = "linux")]
pub(crate) async fn activate_own_tray() -> Result<()> {
    let connection = zbus::Connection::session().await?;
    let watcher = zbus::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .await?;
    let items = watcher
        .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
        .await?;
    let bus = zbus::Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;
    for item in items {
        let service = item.split('/').next().unwrap_or_default();
        if bus
            .call::<_, _, u32>("GetConnectionUnixProcessID", &(service,))
            .await
            .ok()
            == Some(std::process::id())
        {
            zbus::Proxy::new(
                &connection,
                service,
                "/StatusNotifierItem",
                "org.kde.StatusNotifierItem",
            )
            .await?
            .call::<_, _, ()>("Activate", &(0_i32, 0_i32))
            .await?;
            return Ok(());
        }
    }
    anyhow::bail!("找不到此程序的系統匣項目")
}
#[cfg(not(target_os = "linux"))]
pub(crate) async fn tray_host_available() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_arguments_are_quoted_and_reject_line_injection() {
        assert!(login_command(Path::new("app\ninjected"), Path::new("data")).is_err());
        let command = login_command(
            Path::new("/Program Files/TRserial/app"),
            Path::new("/user/data directory"),
        )
        .unwrap();
        assert!(
            command.contains("\"/Program Files/TRserial/app\" --data-dir \"/user/data directory\"")
        );
        #[cfg(not(windows))]
        assert!(
            login_command(Path::new("app%U"), Path::new("data"))
                .unwrap()
                .contains("app%%U")
        );
    }
}
