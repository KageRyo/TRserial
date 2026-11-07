use tempfile::TempDir;
use trserial::{
    accounts::{Account, Accounts},
    parser,
    source::{self, Message, MockSource},
    store::{Status, Store},
};
fn a(id: &str) -> Account {
    Account {
        id: id.into(),
        alias: id.into(),
        username: id.into(),
        session_only: true,
        test_only: false,
    }
}
fn ingest(s: &mut Store, code: &str) {
    let text = format!("序號: {code}");
    s.ingest("manual", None, &text, &parser::parse(&text))
        .unwrap();
}
fn db(dir: &TempDir) -> Store {
    Store::open(&dir.path().join("history.sqlite")).unwrap()
}
#[test]
fn initial_history_is_held_and_new_codes_survive_restarts_without_replaying_success() {
    let dir = TempDir::new().unwrap();
    let mut s = db(&dir);
    let accounts = [a("A")];
    ingest(&mut s, "old1");
    s.initialize_daemon(&accounts).unwrap();
    assert_eq!(s.schedule_daemon(&accounts, false).unwrap(), 0);
    // Reposting the same historical code is not a new discovery, even within the same timestamp.
    ingest(&mut s, "old1");
    assert_eq!(s.schedule_daemon(&accounts, false).unwrap(), 0);
    ingest(&mut s, "new1");
    assert_eq!(s.schedule_daemon(&accounts, false).unwrap(), 1);
    let id = s.jobs().unwrap()[0].id;
    assert!(s.daemon_job_allowed(id, false).unwrap());
    s.claim(id, 3).unwrap();
    s.mark_submitted(id).unwrap();
    s.finish(id, Status::Success, "confirmed", 0).unwrap();
    drop(s);
    let mut s = db(&dir);
    s.recover().unwrap();
    s.initialize_daemon(&accounts).unwrap();
    assert_eq!(s.schedule_daemon(&accounts, false).unwrap(), 0);
    assert_eq!(s.jobs().unwrap().len(), 1);
    assert!(!s.claim(id, 3).unwrap());
    // An explicit backfill adds the old pair, but still does not duplicate success.
    assert_eq!(s.schedule_daemon(&accounts, true).unwrap(), 1);
    assert_eq!(s.schedule_daemon(&accounts, true).unwrap(), 0);
}
#[test]
fn newly_configured_accounts_get_their_own_baseline_and_old_approvals_stay_historical() {
    let dir = TempDir::new().unwrap();
    let mut s = db(&dir);
    let first = [a("A")];
    let text = "ambiguous1";
    s.ingest("manual", None, text, &parser::parse(text))
        .unwrap();
    s.initialize_daemon(&first).unwrap();
    ingest(&mut s, "new1");
    s.schedule_daemon(&first, false).unwrap();
    s.approve("ambiguous1").unwrap();
    let both = [a("A"), a("B")];
    s.initialize_daemon(&both).unwrap();
    assert_eq!(s.schedule_daemon(&both, false).unwrap(), 0);
    assert_eq!(
        s.jobs()
            .unwrap()
            .iter()
            .map(|j| (&*j.account_id, &*j.code))
            .collect::<Vec<_>>(),
        [("A", "new1")]
    );
    ingest(&mut s, "new2");
    assert_eq!(s.schedule_daemon(&both, false).unwrap(), 2);
}
#[test]
fn old_database_queue_is_preserved_but_requires_explicit_authorization() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("history.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/001_initial.sql"))
        .unwrap();
    old.execute("INSERT INTO codes VALUES('old1',0,0)", [])
        .unwrap();
    old.execute("INSERT INTO jobs(code,account_id,account_alias,status,updated_at) VALUES('old1','A','A','pending',0)",[]).unwrap();
    drop(old);
    let mut s = db(&dir);
    s.initialize_daemon(&[a("A")]).unwrap();
    let id = s.jobs().unwrap()[0].id;
    assert!(!s.daemon_job_allowed(id, false).unwrap());
    assert!(s.daemon_job_allowed(id, true).unwrap());
    s.enqueue("old1", &[a("A")]).unwrap();
    assert!(s.daemon_job_allowed(id, false).unwrap());
    assert_eq!(s.jobs().unwrap()[0].status, "pending");
}
#[test]
fn mocks_stay_test_only_after_real_reposts_and_backfill_and_test_accounts_are_excluded() {
    let dir = TempDir::new().unwrap();
    let mut s = db(&dir);
    let mut test = a("test");
    test.test_only = true;
    s.initialize_daemon(&[a("A"), test.clone()]).unwrap();
    // Caller-controlled JSON source labels cannot escape the MockSource boundary.
    source::ingest_source(
        &mut s,
        &mut MockSource(vec![Message {
            source: "manual".into(),
            id: None,
            text: "序號: mock1".into(),
        }]),
    )
    .unwrap();
    ingest(&mut s, "mock1");
    assert!(s.test_only("mock1").unwrap());
    assert_eq!(s.schedule_daemon(&[a("A"), test.clone()], true).unwrap(), 0);
    ingest(&mut s, "new1");
    assert_eq!(s.schedule_daemon(&[a("A"), test], false).unwrap(), 1);
    drop(s);
    assert!(db(&dir).test_only("mock1").unwrap());
    let accounts = Accounts::open(dir.path()).unwrap();
    accounts.add_test("dry-run").unwrap();
    assert!(accounts.list().unwrap()[0].test_only);
}

#[test]
fn account_schema_upgrade_preserves_credential_identity() {
    let dir = TempDir::new().unwrap();
    let old = rusqlite::Connection::open(dir.path().join("accounts.sqlite")).unwrap();
    old.execute_batch("CREATE TABLE accounts(id TEXT PRIMARY KEY,alias TEXT NOT NULL UNIQUE,username TEXT NOT NULL,session_only INTEGER NOT NULL); CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('identity_seed','existing-seed'); INSERT INTO accounts VALUES('existing-credential-id','main','existing-user',0);").unwrap();
    drop(old);
    let accounts = Accounts::open(dir.path()).unwrap();
    let a = &accounts.list().unwrap()[0];
    assert_eq!(a.id, "existing-credential-id");
    assert_eq!(a.username, "existing-user");
    assert!(!a.test_only);
    assert!(!a.session_only);
}
