use std::time::Duration;
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use trserial::{
    config::Config,
    driver::{DriverManager, Readiness},
};
async fn server(body: Option<&'static str>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = [0; 4096];
                let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
                let Some(body) = body else {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    return;
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    (url, task)
}
#[tokio::test]
async fn validates_webdriver_protocol_without_spawning_over_an_occupied_port() {
    for (body, state) in [
        (Some(r#"{"value":{"ready":true}}"#), Readiness::Ready),
        (Some(r#"{"value":{"ready":false}}"#), Readiness::Busy),
        (Some("not a WebDriver"), Readiness::Invalid),
        (None, Readiness::Invalid),
    ] {
        let (url, task) = server(body).await;
        let dir = TempDir::new().unwrap();
        let mut c = Config {
            webdriver_url: url,
            ..Default::default()
        };
        c.driver.managed = true;
        c.driver.binary = "nonexistent-driver".into();
        let mut m = DriverManager::new(c, dir.path().into()).unwrap();
        assert_eq!(m.readiness().await, state);
        assert_eq!(m.ensure_ready().await.is_ok(), state == Readiness::Ready);
        assert!(m.owned_pid().is_none());
        m.shutdown().await;
        assert_eq!(m.readiness().await, state);
        task.abort();
    }
}
#[tokio::test]
async fn missing_binary_and_early_exit_fail_cleanly_and_release_the_endpoint_lock() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let root = TempDir::new().unwrap();
    for binary in [
        "trserial-nonexistent-driver".into(),
        std::env::current_exe().unwrap(),
    ] {
        let mut c = Config {
            webdriver_url: url.clone(),
            ..Default::default()
        };
        c.driver.managed = true;
        c.driver.binary = binary;
        c.driver.startup_timeout_seconds = 1;
        let mut m = DriverManager::new(c, root.path().into()).unwrap();
        assert!(m.ensure_ready().await.is_err());
        assert!(m.owned_pid().is_none());
        assert_eq!(m.readiness().await, Readiness::Missing);
    }
}
#[test]
fn rejects_remote_driver_and_invalid_managed_endpoints() {
    let root = TempDir::new().unwrap();
    let mut prefixed = Config {
        webdriver_url: "http://127.0.0.1:9515/wd/hub".into(),
        ..Default::default()
    };
    assert!(DriverManager::new(prefixed.clone(), root.path().into()).is_ok());
    prefixed.driver.managed = true;
    assert!(DriverManager::new(prefixed, root.path().into()).is_err());
    for url in [
        "http://example.com:9515",
        "http://user:secret@127.0.0.1:9515",
        "http://127.0.0.1:0",
    ] {
        assert!(
            DriverManager::new(
                Config {
                    webdriver_url: url.into(),
                    ..Default::default()
                },
                root.path().into()
            )
            .is_err()
        );
    }
}
