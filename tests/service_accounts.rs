//! Mock credential backend in a separate process; no OS keyring access.
use tempfile::TempDir;
use trserial::{accounts::Accounts, service::Service};

#[test]
fn readding_an_account_preserves_identity_but_holds_the_absent_period_backlog() {
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let dir = TempDir::new().unwrap();
    let mut service = Service::open(dir.path(), &dir.path().join("config.toml")).unwrap();
    service
        .add_account("first", "fixture-user", "fixture-password")
        .unwrap();
    let original = service.snapshot().unwrap().accounts[0].account.clone();
    service.remove_account(&original.id).unwrap();
    service.ingest("序號: absent1".into()).unwrap();
    service
        .add_account("returned", "fixture-user", "fixture-password")
        .unwrap();
    let current = service.snapshot().unwrap().accounts[0].account.clone();
    assert_eq!(original.id, current.id);
    let plan = service.plan("absent1", &[current.id]).unwrap();
    assert!(plan.historical);
    assert!(service.enqueue(&plan, false).is_err());
    assert!(service.snapshot().unwrap().jobs.is_empty());
    assert_eq!(
        Accounts::open(dir.path()).unwrap().list().unwrap()[0].alias,
        "returned"
    );
}
