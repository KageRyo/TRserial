use tempfile::TempDir;
use trserial::{
    accounts::Accounts,
    config::Config,
    service::Service,
    store::{Status, Store},
};

fn open(dir: &TempDir) -> Service {
    let path = dir.path().join("config.toml");
    if !path.exists() {
        let config = Config {
            notifications: false,
            ..Default::default()
        };
        std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    }
    Service::open(dir.path(), &path).unwrap()
}

#[test]
fn manual_scope_requires_confirmation_and_never_resurrects_completed_pairs() {
    let dir = TempDir::new().unwrap();
    let accounts = Accounts::open(dir.path()).unwrap();
    let a = accounts.add("A", "fixture-a", "fake", true).unwrap();
    let b = accounts.add("B", "fixture-b", "fake", true).unwrap();
    let mut service = open(&dir);
    service.ingest("序號: new1".into()).unwrap();
    let one = service.plan("new1", std::slice::from_ref(&a.id)).unwrap();
    assert!(!one.needs_confirmation());
    let both = service.plan("new1", &[a.id.clone(), b.id.clone()]).unwrap();
    assert!(both.needs_confirmation());
    assert!(service.enqueue(&both, false).is_err());
    assert!(service.snapshot().unwrap().jobs.is_empty());
    assert_eq!(service.enqueue(&both, true).unwrap(), 2);
    let mut store = Store::open(&dir.path().join("history.sqlite")).unwrap();
    let id = store.jobs().unwrap()[0].id;
    store.claim(id, 3).unwrap();
    store.mark_submitted(id).unwrap();
    store
        .finish(id, Status::Success, "confirmed fixture", 0)
        .unwrap();
    drop(service);
    let mut restarted = open(&dir);
    assert_eq!(restarted.enqueue(&both, true).unwrap(), 0);
    assert_eq!(restarted.snapshot().unwrap().jobs[0].status, "success");
    assert_eq!(restarted.attempts(id).unwrap()[0].status, "success");
    assert!(restarted.retry(id, true).is_err());
}

#[test]
fn reenable_holds_accumulated_codes_and_account_edits_keep_history_identity() {
    let dir = TempDir::new().unwrap();
    let accounts = Accounts::open(dir.path()).unwrap();
    let a = accounts.add("A", "fixture-a", "fake", true).unwrap();
    let mut service = open(&dir);
    service.set_automatic(&a.id, false).unwrap();
    service
        .ingest("序號: old1\n獎勵: 測試禮物\n到期: 2026-12-31".into())
        .unwrap();
    assert!(accounts.automatic_accounts().unwrap().is_empty());
    service.set_automatic(&a.id, true).unwrap();
    let plan = service.plan("old1", std::slice::from_ref(&a.id)).unwrap();
    assert!(plan.historical);
    assert!(service.enqueue(&plan, false).is_err());
    service.enqueue(&plan, true).unwrap();
    service.rename_account(&a.id, "新別名").unwrap();
    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.accounts[0].account.id, a.id);
    assert_eq!(snapshot.accounts[0].account.alias, "新別名");
    assert!(!snapshot.codes[0].rewards.is_empty());
    service.remove_account(&a.id).unwrap();
    assert!(service.snapshot().unwrap().accounts.is_empty());
    assert_eq!(service.snapshot().unwrap().jobs.len(), 1);
    let restored = accounts.add("重新加入", "fixture-a", "fake", true).unwrap();
    assert_eq!(restored.id, a.id);
    assert_eq!(service.enqueue(&plan, true).unwrap(), 0);
}

#[test]
fn mock_data_approval_and_unknown_retry_share_the_existing_safety_rules() {
    let dir = TempDir::new().unwrap();
    let a = Accounts::open(dir.path())
        .unwrap()
        .add("A", "fixture-a", "fake", true)
        .unwrap();
    let mut service = open(&dir);
    service.ingest("candidate1".into()).unwrap();
    assert!(
        service
            .plan("candidate1", std::slice::from_ref(&a.id))
            .is_err()
    );
    service.approve("candidate1").unwrap();
    let plan = service
        .plan("candidate1", std::slice::from_ref(&a.id))
        .unwrap();
    service.enqueue(&plan, true).unwrap();
    let mut store = Store::open(&dir.path().join("history.sqlite")).unwrap();
    let id = store.jobs().unwrap()[0].id;
    store.claim(id, 3).unwrap();
    store.mark_submitted(id).unwrap();
    store
        .finish(id, Status::Unknown, "fixture unknown", 0)
        .unwrap();
    assert!(service.retry(id, false).is_err());
    assert_eq!(service.snapshot().unwrap().jobs[0].status, "unknown");
    service.retry(id, true).unwrap();
    assert_eq!(service.snapshot().unwrap().jobs[0].status, "pending");
    service.ingest_mock("序號: mock1".into()).unwrap();
    service.ingest("序號: mock1".into()).unwrap();
    assert!(service.plan("mock1", &[a.id]).is_err());
    assert!(
        service
            .snapshot()
            .unwrap()
            .codes
            .iter()
            .find(|c| c.code == "mock1")
            .unwrap()
            .test_only
    );
}

#[test]
fn invalid_settings_preserve_the_existing_configuration() {
    let dir = TempDir::new().unwrap();
    let mut service = open(&dir);
    let path = dir.path().join("config.toml");
    let before = std::fs::read(&path).unwrap();
    let mut config = service.snapshot().unwrap().config;
    config.driver.startup_timeout_seconds = 0;
    assert!(service.save_config(config).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut config = service.snapshot().unwrap().config;
    config.delay_seconds = 12;
    service.save_config(config).unwrap();
    assert_eq!(Config::load(&path).unwrap().delay_seconds, 12);
}
