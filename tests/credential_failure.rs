//! Separate test process: replace the credential builder, never contact the real OS store.
use keyring::credential::{Credential, CredentialBuilderApi};
use tempfile::TempDir;
use trserial::accounts::Accounts;
struct Unavailable;
impl CredentialBuilderApi for Unavailable {
    fn build(&self, _: Option<&str>, _: &str, _: &str) -> keyring::Result<Box<Credential>> {
        Err(keyring::Error::NoEntry)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
#[test]
fn unavailable_os_store_does_not_save_account_or_fall_back_to_plaintext() {
    keyring::set_default_credential_builder(Box::new(Unavailable));
    let dir = TempDir::new().unwrap();
    let accounts = Accounts::open(dir.path()).unwrap();
    assert!(
        accounts
            .add("alias", "username", "DO-NOT-SAVE-THIS-PASSWORD", false)
            .is_err()
    );
    assert!(accounts.list().unwrap().is_empty());
    let account = accounts
        .add(
            "session",
            "fixture-session",
            "DO-NOT-SAVE-THIS-PASSWORD",
            true,
        )
        .unwrap();
    assert!(
        accounts
            .update_password(&account.id, "DO-NOT-SAVE-THIS-PASSWORD")
            .is_err()
    );
    assert!(accounts.list().unwrap()[0].session_only);
    assert!(trserial::accounts::stored_credentials(&account).is_err());
    assert!(
        accounts
            .set_discord_token("DO-NOT-SAVE-THIS-PASSWORD")
            .is_err()
    );
    assert!(accounts.discord_token().is_err());
    let bytes = std::fs::read(dir.path().join("accounts.sqlite")).unwrap();
    assert!(
        !bytes
            .windows(b"DO-NOT-SAVE-THIS-PASSWORD".len())
            .any(|w| w == b"DO-NOT-SAVE-THIS-PASSWORD")
    );
}
