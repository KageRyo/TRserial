//! Local-only real Chrome/WebDriver contract test. Never navigates to the official site.
use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
    typed: Arc<AtomicUsize>,
    invalid_posts: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let posted = posts.clone();
        let typed = Arc::new(AtomicUsize::new(0));
        let typing = typed.clone();
        let invalid_posts = Arc::new(AtomicUsize::new(0));
        let invalid = invalid_posts.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    incoming=listener.accept()=>{
                        let (stream,_)=incoming.unwrap();
                        let posted=posted.clone();let typing=typing.clone();let invalid=invalid.clone();
                        connections.spawn(async move {
                            let service=hyper::service::service_fn(move |request| Self::respond(request,posted.clone(),typing.clone(),invalid.clone()));
                            let _=hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(stream),service).await;
                        });
                    }
                    _=connections.join_next(), if !connections.is_empty()=>{},
                }
            }
        });
        Self {
            url,
            posts,
            typed,
            invalid_posts,
            task,
        }
    }
    async fn respond(
        request: hyper::Request<hyper::body::Incoming>,
        posts: Arc<AtomicUsize>,
        typed: Arc<AtomicUsize>,
        invalid: Arc<AtomicUsize>,
    ) -> Result<hyper::Response<http_body_util::Full<hyper::body::Bytes>>, std::convert::Infallible>
    {
        use http_body_util::BodyExt;
        let path = request.uri().path().to_owned();
        let status = if path == "/redirect" {
            hyper::StatusCode::FOUND
        } else {
            hyper::StatusCode::OK
        };
        let body = if path == "/typed" {
            typed.fetch_add(1, Ordering::SeqCst);
            String::new()
        } else if request.method() == hyper::Method::POST {
            let bytes = request.into_body().collect().await.unwrap().to_bytes();
            let fields: std::collections::HashMap<_, _> =
                url::form_urlencoded::parse(&bytes).collect();
            if fields.get("account").map(|s| s.as_ref()) != Some("fixture-user")
                || fields.get("passward").map(|s| s.as_ref()) != Some("fixture-password")
                || fields.get("code").map(|s| s.as_ref()) != Some("AbC123")
            {
                invalid.fetch_add(1, Ordering::SeqCst);
            }
            posts.fetch_add(1, Ordering::SeqCst);
            let response = match path.as_str() {
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
            let page = include_str!("fixtures/serial.html");
            let page = match case {
                "changed" => page.replace("name=\"code\"", "name=\"changed-code\""),
                "changed-type" => page.replace("type=\"password\"", "type=\"text\""),
                "readonly" => format!(
                    "{page}<script>document.querySelectorAll('input').forEach(e=>e.addEventListener('input',()=>fetch('/typed')));</script>"
                ),
                _ => page.to_owned(),
            };
            page.replace(
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
        let mut response = hyper::Response::builder()
            .status(status)
            .header(hyper::header::CONTENT_TYPE, "text/html; charset=utf-8");
        if status == hyper::StatusCode::FOUND {
            response = response.header(hyper::header::LOCATION, "/waiting");
        }
        Ok(response
            .body(http_body_util::Full::new(hyper::body::Bytes::from(body)))
            .unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[tokio::test]
#[ignore = "requires local Chrome and ChromeDriver; set CHROME_BIN and CHROMEDRIVER"]
async fn real_browser_fills_verifies_submits_and_reads_confirmed_outcomes() {
    let fixture = Fixture::start().await;
    // Chrome's speculative idle connections must not block active page requests.
    let address = fixture.url.trim_start_matches("http://");
    let _idle_connections = [
        tokio::net::TcpStream::connect(address).await.unwrap(),
        tokio::net::TcpStream::connect(address).await.unwrap(),
    ];
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let driver_path = std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".into());
    let logs = tempfile::TempDir::new().unwrap();
    let _driver = Driver {
        child: Command::new(driver_path)
            .arg(format!("--port={port}"))
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
    for (case, expected_missing) in [
        ("readonly", false),
        ("changed", true),
        ("changed-type", false),
    ] {
        let mut browser =
            BrowserRedeemer::local_fixture(config.clone(), &format!("{}/{case}", fixture.url))
                .unwrap();
        let report = browser.inspect().await.unwrap();
        assert_eq!(!report.missing.is_empty(), expected_missing);
        assert!(report.verification_present);
        assert_eq!(!report.changed_types.is_empty(), case == "changed-type");
        browser.shutdown().await;
        assert_eq!(fixture.posts.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.typed.load(Ordering::SeqCst), 0);
    }
    let mut redirected =
        BrowserRedeemer::local_fixture(config.clone(), &format!("{}/redirect", fixture.url))
            .unwrap();
    assert!(
        redirected
            .inspect()
            .await
            .unwrap_err()
            .detail
            .contains("轉址")
    );
    redirected.shutdown().await;
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
        assert_eq!(fixture.invalid_posts.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
#[ignore = "requires ChromeDriver; set CHROMEDRIVER"]
async fn managed_driver_owns_only_its_child_prevents_duplicates_and_recovers_after_exit() {
    use trserial::driver::{DriverManager, Readiness};
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let root = tempfile::TempDir::new().unwrap();
    let mut config = Config {
        webdriver_url: format!("http://127.0.0.1:{port}"),
        ..Default::default()
    };
    config.driver.managed = true;
    config.driver.binary = std::env::var("CHROMEDRIVER")
        .unwrap_or_else(|_| "chromedriver".into())
        .into();
    let mut first = DriverManager::new(config.clone(), root.path().into()).unwrap();
    // Trailing slash must share the same endpoint lock.
    let mut other_config = config.clone();
    other_config.webdriver_url.push('/');
    let mut second = DriverManager::new(other_config, root.path().into()).unwrap();
    let (a, b) = tokio::join!(first.ensure_ready(), second.ensure_ready());
    a.unwrap();
    b.unwrap();
    assert_ne!(first.owned_pid().is_some(), second.owned_pid().is_some());
    let (owner, borrower) = if first.owned_pid().is_some() {
        (&mut first, &mut second)
    } else {
        (&mut second, &mut first)
    };
    let old_pid = owner.owned_pid().unwrap();
    let mut external_config = config.clone();
    external_config.driver.managed = false;
    let mut external = DriverManager::new(external_config, root.path().into()).unwrap();
    external.ensure_ready().await.unwrap();
    external.shutdown().await;
    assert_eq!(owner.readiness().await, Readiness::Ready);
    borrower.shutdown().await;
    assert_eq!(owner.readiness().await, Readiness::Ready);
    #[cfg(unix)]
    let status = tokio::process::Command::new("kill")
        .args(["-9", &old_pid.to_string()])
        .status()
        .await
        .unwrap();
    #[cfg(windows)]
    let status = tokio::process::Command::new("taskkill")
        .args(["/PID", &old_pid.to_string(), "/F"])
        .status()
        .await
        .unwrap();
    assert!(status.success());
    owner.ensure_ready().await.unwrap();
    assert_ne!(owner.owned_pid(), Some(old_pid));
    owner.shutdown().await;
    assert_eq!(owner.readiness().await, Readiness::Missing);
    assert!(external.ensure_ready().await.is_err());
    assert!(external.owned_pid().is_none());
}
