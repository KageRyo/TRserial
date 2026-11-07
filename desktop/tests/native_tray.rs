//! Private D-Bus fixture: the application still uses its real native tray backend.
#[cfg(target_os = "linux")]
mod linux {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    struct Bus(std::process::Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct Watcher {
        host: bool,
        items: Arc<Mutex<Vec<String>>>,
    }
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: String) {
            let item = if service.contains('/') {
                service
            } else {
                format!("{service}/StatusNotifierItem")
            };
            let mut items = self.items.lock().unwrap();
            if !items.contains(&item) {
                items.push(item);
            }
        }
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            self.host
        }
        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.items.lock().unwrap().clone()
        }
    }
    #[test]
    #[ignore = "requires a real X11/Wayland session and Vulkan renderer"]
    fn actual_sni_activation_restores_the_window_and_no_host_keeps_it_reachable() {
        for host in [true, false] {
            check_lifecycle(host);
        }
    }
    fn check_lifecycle(host: bool) {
        let mut bus = Bus(Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap());
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let items = Arc::new(Mutex::new(Vec::new()));
        let _watcher = zbus::block_on(
            zbus::connection::Builder::address(address.trim())
                .name("org.kde.StatusNotifierWatcher")
                .serve_at(
                    "/StatusNotifierWatcher",
                    Watcher {
                        host,
                        items: items.clone(),
                    },
                )
                .build(),
        )
        .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_trserial-desktop"));
        command.env_remove("TRSERIAL_REQUIRE_TRAY");
        if host {
            command.env("TRSERIAL_REQUIRE_TRAY", "1");
        }
        let mut child = command
            .args(["--data-dir", dir.path().to_str().unwrap(), "--smoke-test"])
            .env("DBUS_SESSION_BUS_ADDRESS", address.trim())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("native tray smoke timed out");
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{}\n{stderr}", output.status);
        assert!(
            stderr.contains(if host {
                "TRserial native tray hide/restore passed"
            } else {
                "TRserial tray fallback kept the window visible"
            }),
            "{stderr}"
        );
        assert!(!items.lock().unwrap().is_empty());
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.path().join("desktop.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        assert!(
            trserial::store::Store::open(&dir.path().join("history.sqlite"))
                .unwrap()
                .attempts()
                .unwrap()
                .is_empty()
        );
    }
}
