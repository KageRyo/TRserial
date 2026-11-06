//! Local-only real Chrome/WebDriver contract test. Never navigates to the official site.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use trserial::{
    accounts::Credentials,
    browser::{BrowserRedeemer, Preparation, Redeemer},
    config::Config,
    store::Status,
};
struct Driver {
    child: Child,
    logs: tempfile::TempDir,
}
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            for name in ["chromedriver.log", "chrome.log"] {
                if let Ok(log) = std::fs::read_to_string(self.logs.path().join(name)) {
                    eprintln!("{name} (local fixture with test credentials):\n{log}");
                }
            }
        }
    }
}
struct Fixture {
    url: String,
    posts: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let posts = Arc::new(AtomicUsize::new(0));
        let posted = posts.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..pos]);
                        let len = header
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= pos + 4 + len {
                            break;
                        }
                    }
                }
                if request.is_empty() {
                    continue;
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let body = if request.starts_with("POST ") {
                    posted.fetch_add(1, Ordering::SeqCst);
                    assert!(request.contains("account=fixture-user"));
                    assert!(request.contains("passward=fixture-password"));
                    assert!(request.contains("code=AbC123"));
                    let response = match path {
                        "/handler/success" => "兌換成功",
                        "/handler/already" => "此序號已兌換",
                        "/handler/expired" => "序號已過期",
                        "/handler/invalid" => "序號無效",
                        _ => "未知回應，fixture-password",
                    };
                    format!(
                        "<!doctype html><meta charset='utf-8'><body><script>alert({});</script></body>",
                        serde_json::to_string(response).unwrap()
                    )
                } else {
                    let case = path.trim_start_matches('/');
                    include_str!("fixtures/serial.html")
                        .replace(
                            "{{ACTION}}",
                            &if case == "foreign-action" {
                                "http://127.0.0.1:1/steal".to_owned()
                            } else {
                                format!("/handler/{case}")
                            },
                        )
                        .replace(
                            "{{TOKEN}}",
                            if case == "waiting" {
                                ""
                            } else {
                                "LOCAL-FIXTURE-TOKEN"
                            },
                        )
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            posts,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
#[tokio::test]
#[ignore = "requires local Chrome and ChromeDriver; set CHROME_BIN and CHROMEDRIVER"]
async fn real_browser_fills_verifies_submits_and_reads_confirmed_outcomes() {
    let fixture = Fixture::start();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let driver_path = std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into());
    let logs = tempfile::TempDir::new().unwrap();
    let _driver = Driver {
        child: Command::new(driver_path)
            .arg(format!("--port={port}"))
            .arg("--allowed-ips=127.0.0.1")
            .arg("--verbose")
            .arg(format!(
                "--log-path={}",
                logs.path().join("chromedriver.log").display()
            ))
            .env("CHROME_LOG_FILE", logs.path().join("chrome.log"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start chromedriver"),
        logs,
    };
    let mut available = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            available = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(available, "chromedriver did not start");
    let config = Config {
        webdriver_url: format!("http://127.0.0.1:{port}"),
        headless: true,
        browser_binary: std::env::var("CHROME_BIN").ok(),
        timeout_seconds: 5,
        notifications: false,
        ..Default::default()
    };
    let credentials = Credentials {
        username: "fixture-user".into(),
        password: zeroize::Zeroizing::new("fixture-password".into()),
    };
    let mut browser =
        BrowserRedeemer::local_fixture(config.clone(), &format!("{}/waiting", fixture.url))
            .unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(30),
            browser.prepare(&credentials, "AbC123")
        )
        .await
        .expect("browser preparation timed out")
        .unwrap(),
        Preparation::WaitingVerification
    );
    assert!(!browser.verification_ready().await.unwrap());
    assert_eq!(fixture.posts.load(Ordering::SeqCst), 0);
    browser.close().await;
    // Refuse a form that sends credentials outside the inspected origin.
    let mut browser =
        BrowserRedeemer::local_fixture(config.clone(), &format!("{}/foreign-action", fixture.url))
            .unwrap();
    assert!(browser.prepare(&credentials, "AbC123").await.is_err());
    browser.close().await;
    assert_eq!(fixture.posts.load(Ordering::SeqCst), 0);
    // A changed/malformed field must not silently truncate user input or submit anything.
    let mut browser =
        BrowserRedeemer::local_fixture(config.clone(), &format!("{}/success", fixture.url))
            .unwrap();
    assert!(
        browser
            .prepare(&credentials, &"A".repeat(31))
            .await
            .is_err()
    );
    browser.close().await;
    assert_eq!(fixture.posts.load(Ordering::SeqCst), 0);
    for (i, (case, status)) in [
        ("success", Status::Success),
        ("already", Status::AlreadyRedeemed),
        ("expired", Status::Expired),
        ("invalid", Status::Failed),
        ("unknown", Status::Unknown),
    ]
    .into_iter()
    .enumerate()
    {
        let mut browser =
            BrowserRedeemer::local_fixture(config.clone(), &format!("{}/{case}", fixture.url))
                .unwrap();
        assert_eq!(
            browser.prepare(&credentials, "AbC123").await.unwrap(),
            Preparation::Ready
        );
        let outcome = browser.submit().await.unwrap();
        assert_eq!(outcome.status, status);
        assert!(!outcome.detail.contains("fixture-password"));
        browser.close().await;
        assert_eq!(fixture.posts.load(Ordering::SeqCst), i + 1);
    }
}
