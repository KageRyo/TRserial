use anyhow::Result;
use async_trait::async_trait;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use trserial::{
    accounts::{Account, Accounts, Credentials},
    browser::{BrowserResult, Outcome, Preparation, RedeemError, Redeemer},
    config::Config,
    notifications::Notifier,
    parser,
    source::{self, Message, MockSource},
    store::{Status, Store},
    worker::{CredentialProvider, Worker, WorkerLock, backoff},
};

fn account(id: &str) -> Account {
    Account {
        id: id.into(),
        alias: format!("Account {id}"),
        username: format!("user{id}"),
        session_only: true,
        test_only: false,
    }
}
fn store(dir: &TempDir) -> Store {
    Store::open(&dir.path().join("history.sqlite")).unwrap()
}
fn discover(s: &mut Store) {
    source::ingest(
        s,
        &Message {
            source: "manual".into(),
            id: None,
            text: "序號: abc123\n獎勵箱子 1個".into(),
        },
    )
    .unwrap();
}
struct FakeCredentials;
impl CredentialProvider for FakeCredentials {
    fn get(&self, a: &Account) -> Result<Credentials> {
        Ok(Credentials {
            username: a.username.clone(),
            password: zeroize::Zeroizing::new("FAKE-TEST-PASSWORD".into()),
        })
    }
}
#[derive(Default)]
struct Notices(Mutex<Vec<String>>);
impl Notifier for Notices {
    fn send(&self, title: &str, body: &str) {
        self.0.lock().unwrap().push(format!("{title}: {body}"));
    }
}
struct FakeRedeemer {
    prepare: VecDeque<BrowserResult<Preparation>>,
    outcomes: VecDeque<BrowserResult<Outcome>>,
    submitted: usize,
    ready: bool,
    cancel_on_submit: Option<CancellationToken>,
}
impl Default for FakeRedeemer {
    fn default() -> Self {
        Self {
            prepare: VecDeque::new(),
            outcomes: VecDeque::new(),
            submitted: 0,
            ready: true,
            cancel_on_submit: None,
        }
    }
}
#[async_trait]
impl Redeemer for FakeRedeemer {
    fn is_live(&self) -> bool {
        false
    }
    async fn prepare(&mut self, _: &Credentials, _: &str) -> BrowserResult<Preparation> {
        self.prepare.pop_front().unwrap_or(Ok(Preparation::Ready))
    }
    async fn verification_ready(&mut self) -> BrowserResult<bool> {
        Ok(self.ready)
    }
    async fn submit(&mut self) -> BrowserResult<Outcome> {
        self.submitted += 1;
        if let Some(c) = &self.cancel_on_submit {
            c.cancel();
            std::future::pending::<()>().await;
        }
        self.outcomes.pop_front().unwrap_or(Ok(Outcome {
            status: Status::Success,
            detail: "網站確認兌換成功",
        }))
    }
    async fn close(&mut self) {}
}
fn config() -> Config {
    Config {
        delay_seconds: 0,
        retry_base_seconds: 1,
        notifications: false,
        ..Default::default()
    }
}
async fn run(
    s: &mut Store,
    c: &Config,
    r: &mut FakeRedeemer,
    accounts: &[Account],
    notice: Arc<Notices>,
    cancel: CancellationToken,
) {
    Worker {
        store: s,
        config: c,
        redeemer: r,
        credentials: &FakeCredentials,
        notifier: notice,
        cancel,
    }
    .run(accounts, None, None)
    .await
    .unwrap();
}
#[tokio::test]
async fn multiple_accounts_dedup_restart_and_notification() {
    let dir = TempDir::new().unwrap();
    let _lock = WorkerLock::acquire(dir.path()).unwrap();
    let mut s = store(&dir);
    let a = [account("a"), account("b")];
    let message = Message {
        source: "discord:mock".into(),
        id: Some("fixture-1".into()),
        text: "序號: abc123\n獎勵箱子 1個".into(),
    };
    source::ingest_source(&mut s, &mut MockSource(vec![message.clone(), message])).unwrap();
    assert_eq!(s.codes().unwrap().len(), 1);
    assert_eq!(s.source_messages().unwrap().len(), 1);
    assert_eq!(s.enqueue("abc123", &a).unwrap(), 2);
    assert_eq!(s.enqueue("abc123", &a).unwrap(), 0);
    let mut r = FakeRedeemer {
        outcomes: VecDeque::from([
            Ok(Outcome {
                status: Status::Success,
                detail: "confirmed",
            }),
            Ok(Outcome {
                status: Status::AlreadyRedeemed,
                detail: "already",
            }),
        ]),
        ..Default::default()
    };
    let n = Arc::new(Notices::default());
    run(
        &mut s,
        &config(),
        &mut r,
        &a,
        n.clone(),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(r.submitted, 2);
    assert_eq!(s.jobs().unwrap()[0].status, "success");
    assert_eq!(s.jobs().unwrap()[1].status, "already_redeemed");
    drop(s);
    let mut s = store(&dir);
    s.recover().unwrap();
    s.enqueue("abc123", &a).unwrap();
    run(
        &mut s,
        &config(),
        &mut r,
        &a,
        n.clone(),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(r.submitted, 2);
    assert_eq!(s.attempts().unwrap().len(), 2);
    assert!(n.0.lock().unwrap().iter().any(|s| s.contains("兌換成功")));
    assert!(
        n.0.lock()
            .unwrap()
            .iter()
            .any(|s| s.contains("本次處理結果"))
    );
}
#[tokio::test]
async fn selecting_one_account_does_not_claim_or_fail_other_accounts_pending_jobs() {
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    let accounts = [account("a"), account("b")];
    s.enqueue("abc123", &accounts).unwrap();
    let mut redeemer = FakeRedeemer::default();
    run(
        &mut s,
        &config(),
        &mut redeemer,
        &accounts[..1],
        Arc::new(Notices::default()),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(redeemer.submitted, 1);
    let jobs = s.jobs().unwrap();
    let held = jobs.iter().find(|j| j.account_id == "b").unwrap();
    assert_eq!(held.status, "pending");
    assert_eq!(held.attempts, 0);
    assert!(s.job_attempts(held.id).unwrap().is_empty());
}

#[test]
fn crash_recovery_distinguishes_before_and_after_submission() {
    let dir = TempDir::new().unwrap();
    let _lock = WorkerLock::acquire(dir.path()).unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    s.enqueue("abc123", &[account("a"), account("b"), account("c")])
        .unwrap();
    let j = s.jobs().unwrap();
    for job in &j {
        assert!(s.claim(job.id, 3).unwrap());
    }
    s.waiting(j[0].id).unwrap();
    s.mark_submitted(j[1].id).unwrap();
    drop(s);
    let mut s = store(&dir);
    s.recover().unwrap();
    let j = s.jobs().unwrap();
    assert_eq!(
        j.iter().map(|j| j.status.as_str()).collect::<Vec<_>>(),
        ["pending", "unknown", "pending"]
    );
    assert!(!s.claim(j[1].id, 3).unwrap());
    assert!(s.mark_submitted(j[1].id).is_err());
    s.retry_reviewed(j[1].id).unwrap();
    assert!(s.claim(j[1].id, 3).unwrap());
    s.mark_submitted(j[1].id).unwrap();
    assert!(s.mark_submitted(j[1].id).is_err());
    assert!(WorkerLock::acquire(dir.path()).is_err());
}
#[tokio::test]
async fn only_pre_submit_transient_errors_retry_with_bound() {
    assert_eq!(
        [
            backoff(10, 1),
            backoff(10, 2),
            backoff(10, 3),
            backoff(10, 99)
        ],
        [10, 20, 40, 3600]
    );
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    let a = [account("a")];
    s.enqueue("abc123", &a).unwrap();
    let id = s.jobs().unwrap()[0].id;
    let mut r = FakeRedeemer {
        prepare: VecDeque::from([
            Err(RedeemError::transient("network")),
            Err(RedeemError::transient("network")),
            Err(RedeemError::transient("network")),
        ]),
        ..Default::default()
    };
    // Timestamps have one-second resolution, so a 1 s backoff becomes due
    // whenever the two sweeps straddle a second boundary. An hour keeps the
    // retry pending until the test advances it explicitly.
    let c = Config {
        max_attempts: 2,
        retry_base_seconds: 3600,
        ..config()
    };
    let n = Arc::new(Notices::default());
    run(&mut s, &c, &mut r, &a, n.clone(), CancellationToken::new()).await;
    assert_eq!(s.jobs().unwrap()[0].status, "pending");
    run(&mut s, &c, &mut r, &a, n.clone(), CancellationToken::new()).await;
    assert_eq!(s.jobs().unwrap()[0].attempts, 1); // next_at not reached
    // Deterministically advance the schedule, no wall-clock sleep in this test.
    s.finish(id, Status::Pending, "reschedule test", 0).unwrap();
    run(&mut s, &c, &mut r, &a, n.clone(), CancellationToken::new()).await;
    assert_eq!(s.jobs().unwrap()[0].status, "failed");
    assert_eq!(s.jobs().unwrap()[0].attempts, 2);
    assert_eq!(r.submitted, 0);
    assert!(!s.claim(id, 2).unwrap());
}
#[tokio::test]
async fn post_submit_errors_and_cancellation_require_review() {
    for cancel_submit in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut s = store(&dir);
        discover(&mut s);
        let a = [account("a")];
        s.enqueue("abc123", &a).unwrap();
        let cancel = CancellationToken::new();
        let mut r = FakeRedeemer {
            outcomes: VecDeque::from([Err(RedeemError::transient("disconnected"))]),
            cancel_on_submit: cancel_submit.then_some(cancel.clone()),
            ..Default::default()
        };
        run(
            &mut s,
            &config(),
            &mut r,
            &a,
            Arc::new(Notices::default()),
            cancel,
        )
        .await;
        assert_eq!(s.jobs().unwrap()[0].status, "unknown");
        s.recover().unwrap();
        run(
            &mut s,
            &config(),
            &mut r,
            &a,
            Arc::new(Notices::default()),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(r.submitted, 1);
    }
}
#[tokio::test(start_paused = true)]
async fn verification_wait_and_timeout_never_submit_early() {
    for ready in [true, false] {
        let dir = TempDir::new().unwrap();
        let mut s = store(&dir);
        discover(&mut s);
        let a = [account("a")];
        s.enqueue("abc123", &a).unwrap();
        let mut r = FakeRedeemer {
            prepare: VecDeque::from([Ok(Preparation::WaitingVerification)]),
            ready,
            ..Default::default()
        };
        let n = Arc::new(Notices::default());
        run(
            &mut s,
            &Config {
                verification_timeout_seconds: 2,
                ..config()
            },
            &mut r,
            &a,
            n.clone(),
            CancellationToken::new(),
        )
        .await;
        assert_eq!(r.submitted, usize::from(ready));
        assert_eq!(
            s.jobs().unwrap()[0].status,
            if ready { "success" } else { "failed" }
        );
        assert!(
            n.0.lock()
                .unwrap()
                .iter()
                .any(|s| s.contains("需要人工驗證"))
        );
    }
}
#[test]
fn review_gate_and_mock_message_source() {
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    let mut source = MockSource(vec![Message {
        source: "discord:mock".into(),
        id: Some("1".into()),
        text: "AbC123".into(),
    }]);
    assert_eq!(source::ingest_source(&mut s, &mut source).unwrap(), 1);
    assert!(s.enqueue("AbC123", &[account("a")]).is_err());
    s.approve("AbC123").unwrap();
    s.enqueue("AbC123", &[account("a")]).unwrap();
    assert_eq!(
        s.source_messages().unwrap()[0],
        ("mock:discord:mock".into(), "AbC123".into())
    );
    let c = parser::parse("序號: AbC123");
    s.ingest("manual", None, "序號: AbC123", &c).unwrap();
    assert_eq!(s.codes().unwrap().len(), 1);
    assert!(s.retry_reviewed(s.jobs().unwrap()[0].id).is_err());
}
#[test]
fn session_account_metadata_never_contains_password_and_remove_preserves_history() {
    let dir = TempDir::new().unwrap();
    let accounts = Accounts::open(dir.path()).unwrap();
    let a = accounts
        .add("main", "user", "UNIQUE_SECRET_DO_NOT_PERSIST", true)
        .unwrap();
    assert!(accounts.add("other", "user", "password", true).is_err());
    let mut s = store(&dir);
    discover(&mut s);
    s.enqueue("abc123", std::slice::from_ref(&a)).unwrap();
    accounts.remove("main").unwrap();
    assert!(accounts.list().unwrap().is_empty());
    assert_eq!(s.jobs().unwrap().len(), 1);
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(26)
                .any(|b| b == b"UNIQUE_SECRET_DO_NOT_PERSIST")
        );
    }
}

#[test]
fn recovered_pre_submit_crash_at_attempt_limit_does_not_stay_pending_forever() {
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    s.enqueue("abc123", &[account("a")]).unwrap();
    let id = s.jobs().unwrap()[0].id;
    assert!(s.claim(id, 1).unwrap());
    s.recover().unwrap();
    assert!(!s.claim(id, 1).unwrap());
    assert_eq!(s.jobs().unwrap()[0].status, "failed");
}

#[test]
fn readding_same_username_retains_redemption_identity_but_other_installations_do_not() {
    let dir = TempDir::new().unwrap();
    let accounts = Accounts::open(dir.path()).unwrap();
    let original = accounts.add("old", "username", "fake", true).unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    s.enqueue("abc123", std::slice::from_ref(&original))
        .unwrap();
    let id = s.jobs().unwrap()[0].id;
    s.claim(id, 3).unwrap();
    s.mark_submitted(id).unwrap();
    s.finish(id, Status::Success, "confirmed", 0).unwrap();
    assert!(
        s.finish(id, Status::Pending, "should not reset success", 0)
            .is_err()
    );
    accounts.remove("old").unwrap();
    let added = accounts.add("new", "username", "fake", true).unwrap();
    assert_eq!(original.id, added.id);
    assert_eq!(
        s.enqueue("abc123", std::slice::from_ref(&added)).unwrap(),
        0
    );
    let other = TempDir::new().unwrap();
    let independent = Accounts::open(other.path())
        .unwrap()
        .add("new", "username", "fake", true)
        .unwrap();
    assert_ne!(independent.id, added.id);
}

struct NoCredentials;
impl CredentialProvider for NoCredentials {
    fn get(&self, _: &Account) -> Result<Credentials> {
        panic!("test data reached credential access")
    }
}
struct LiveGuard;
#[async_trait]
impl Redeemer for LiveGuard {
    async fn prepare(&mut self, _: &Credentials, _: &str) -> BrowserResult<Preparation> {
        panic!("test data reached live browser")
    }
    async fn verification_ready(&mut self) -> BrowserResult<bool> {
        panic!("live verification")
    }
    async fn submit(&mut self) -> BrowserResult<Outcome> {
        panic!("live submission")
    }
    async fn close(&mut self) {}
}
#[tokio::test]
async fn mock_codes_and_test_accounts_are_blocked_before_credentials_or_live_browser() {
    for test_account in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut s = store(&dir);
        let mut a = account("A");
        a.test_only = test_account;
        if test_account {
            discover(&mut s);
        } else {
            source::ingest_source(
                &mut s,
                &mut MockSource(vec![Message {
                    source: "manual".into(),
                    id: None,
                    text: "序號: abc123".into(),
                }]),
            )
            .unwrap();
        }
        s.enqueue("abc123", std::slice::from_ref(&a)).unwrap();
        Worker {
            store: &mut s,
            config: &config(),
            redeemer: &mut LiveGuard,
            credentials: &NoCredentials,
            notifier: Arc::new(Notices::default()),
            cancel: CancellationToken::new(),
        }
        .run(&[a], None, None)
        .await
        .unwrap();
        assert_eq!(s.jobs().unwrap()[0].status, "failed");
        assert!(!s.jobs().unwrap()[0].submitted);
    }
}
#[tokio::test]
async fn daemon_worker_skips_legacy_jobs_and_restart_cannot_resubmit_success() {
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    let accounts = [account("A")];
    discover(&mut s);
    s.enqueue("abc123", &accounts).unwrap();
    rusqlite::Connection::open(dir.path().join("history.sqlite"))
        .unwrap()
        .execute("UPDATE jobs SET manual=0", [])
        .unwrap();
    s.initialize_daemon(&accounts).unwrap();
    let text = "序號: new123";
    s.ingest("manual", None, text, &parser::parse(text))
        .unwrap();
    s.schedule_daemon(&accounts, false).unwrap();
    let mut r = FakeRedeemer::default();
    for _ in 0..2 {
        Worker {
            store: &mut s,
            config: &config(),
            redeemer: &mut r,
            credentials: &FakeCredentials,
            notifier: Arc::new(Notices::default()),
            cancel: CancellationToken::new(),
        }
        .run_scoped(&accounts, None, None, Some(false))
        .await
        .unwrap();
        drop(s);
        s = store(&dir);
        s.recover().unwrap();
        s.schedule_daemon(&accounts, false).unwrap();
    }
    assert_eq!(r.submitted, 1);
    assert_eq!(s.jobs().unwrap()[0].status, "pending");
    assert_eq!(s.jobs().unwrap()[1].status, "success");
}

#[tokio::test]
async fn headless_verification_failure_explains_how_to_continue_without_submitting() {
    let dir = TempDir::new().unwrap();
    let mut s = store(&dir);
    discover(&mut s);
    let accounts = [account("A")];
    s.enqueue("abc123", &accounts).unwrap();
    let mut r = FakeRedeemer {
        prepare: VecDeque::from([Ok(Preparation::WaitingVerification)]),
        ready: false,
        ..Default::default()
    };
    run(
        &mut s,
        &Config {
            headless: true,
            ..config()
        },
        &mut r,
        &accounts,
        Arc::new(Notices::default()),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(r.submitted, 0);
    assert_eq!(s.jobs().unwrap()[0].status, "failed");
    assert!(s.jobs().unwrap()[0].detail.contains("headless=false"));
}
