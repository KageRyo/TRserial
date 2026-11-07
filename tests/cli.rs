use std::process::Command;
use tempfile::TempDir;
use trserial::{accounts::Accounts, store::Store};
fn run(dir: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_trserial"))
        .arg("--data-dir")
        .arg(dir.path())
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn usable_cli_ingestion_dry_run_and_offline_daemon() {
    let dir = TempDir::new().unwrap();
    assert!(run(&dir, &["init"]).status.success());
    let accounts = Accounts::open(dir.path()).unwrap();
    accounts
        .add("A", "user-a", "NEVER-READ-THIS", true)
        .unwrap();
    accounts
        .add("B", "user-b", "NEVER-READ-THIS", true)
        .unwrap();
    for _ in 0..2 {
        assert!(
            run(&dir, &["ingest", "--text", "序號: demo123"])
                .status
                .success()
        );
    }
    let dry = run(&dir, &["redeem", "demo123", "--all", "--dry-run"]);
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let s = Store::open(&dir.path().join("history.sqlite")).unwrap();
    assert!(s.jobs().unwrap().is_empty());
    assert!(s.attempts().unwrap().is_empty());
    assert_eq!(s.codes().unwrap().len(), 1);
    // Dry run must report the same review gate as real execution, without approving or queuing anything.
    assert!(run(&dir, &["ingest", "--text", "AbC9"]).status.success());
    assert!(
        !run(&dir, &["redeem", "AbC9", "--all", "--dry-run"])
            .status
            .success()
    );
    assert!(s.jobs().unwrap().is_empty());
    // Default disabled Discord never asks for credentials/token; no accounts means no browser work.
    let empty = TempDir::new().unwrap();
    let daemon = run(&empty, &["daemon", "--once"]);
    assert!(
        daemon.status.success(),
        "{}",
        String::from_utf8_lossy(&daemon.stderr)
    );
    // CLI enforces explicit account scope and review acknowledgement.
    assert!(!run(&dir, &["redeem", "demo123"]).status.success());
    assert!(!run(&dir, &["retry", "1"]).status.success());
}
#[test]
fn unrelated_and_malformed_messages_do_not_persist() {
    let dir = TempDir::new().unwrap();
    assert!(
        run(
            &dir,
            &[
                "ingest",
                "--text",
                "unrelated sensitive clipboard-like text"
            ]
        )
        .status
        .success()
    );
    assert!(
        Store::open(&dir.path().join("history.sqlite"))
            .unwrap()
            .source_messages()
            .unwrap()
            .is_empty()
    );
    let bad = dir.path().join("bad.toml");
    std::fs::write(&bad, "password = 'not-an-allowed-setting'").unwrap();
    let output = run(&dir, &["--config", bad.to_str().unwrap(), "status"]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not-an-allowed-setting"));
}

#[test]
fn smoke_workflow_is_offline_and_test_data_cannot_reach_live_redemption() {
    let dir = TempDir::new().unwrap();
    assert!(run(&dir, &["init"]).status.success());
    assert!(run(&dir, &["smoke-test"]).status.success());
    let out = run(
        &dir,
        &["redeem", "malojam1106", "--account", "smoke", "--dry-run"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !run(&dir, &["redeem", "malojam1106", "--account", "smoke"])
            .status
            .success()
    );
    assert!(
        run(&dir, &["daemon", "--once", "--backfill"])
            .status
            .success()
    );
    let s = Store::open(&dir.path().join("history.sqlite")).unwrap();
    assert!(s.jobs().unwrap().is_empty());
    assert!(s.attempts().unwrap().is_empty());
    let accounts = Accounts::open(dir.path()).unwrap();
    accounts
        .add("real-metadata", "fixture-not-real", "fake", true)
        .unwrap();
    assert!(
        !run(
            &dir,
            &["redeem", "malojam1106", "--account", "real-metadata"]
        )
        .status
        .success()
    );
}
#[test]
fn daemon_mock_ingestion_never_processes_existing_live_pending_jobs() {
    let dir = TempDir::new().unwrap();
    assert!(run(&dir, &["init"]).status.success());
    let a = Accounts::open(dir.path())
        .unwrap()
        .add("A", "fixture-user", "fake", true)
        .unwrap();
    let mut s = Store::open(&dir.path().join("history.sqlite")).unwrap();
    let text = "序號: real123";
    s.ingest("manual", None, text, &trserial::parser::parse(text))
        .unwrap();
    s.enqueue("real123", &[a]).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/messages.json");
    let out = run(
        &dir,
        &["daemon", "--mock", path.to_str().unwrap(), "--once"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(s.jobs().unwrap()[0].status, "pending");
    assert!(s.attempts().unwrap().is_empty());
    assert!(
        !run(
            &dir,
            &["daemon", "--mock", path.to_str().unwrap(), "--backfill"]
        )
        .status
        .success()
    );
}
