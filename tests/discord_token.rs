//! Separate test process: an in-memory credential store replaces the OS one.
use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;
use trserial::accounts::Accounts;

type Secrets = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;
struct Memory(Secrets);
struct Item(Secrets, (String, String));
impl CredentialApi for Item {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(self.1.clone(), secret.to_vec());
        Ok(())
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .get(&self.1)
            .cloned()
            .ok_or(keyring::Error::NoEntry)
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.0
            .lock()
            .unwrap()
            .remove(&self.1)
            .map(|_| ())
            .ok_or(keyring::Error::NoEntry)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl CredentialBuilderApi for Memory {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        Ok(Box::new(Item(
            self.0.clone(),
            (service.into(), user.into()),
        )))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn discord_token_lives_only_in_the_credential_store_of_its_data_directory() {
    keyring::set_default_credential_builder(Box::new(Memory(Secrets::default())));
    let (first, second) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let accounts = Accounts::open(first.path()).unwrap();
    let other = Accounts::open(second.path()).unwrap();
    assert!(accounts.discord_token().unwrap().is_none());
    assert!(accounts.set_discord_token("  ").is_err());
    accounts.set_discord_token(" FAKE.DISCORD.TOKEN ").unwrap();
    assert_eq!(
        accounts.discord_token().unwrap().unwrap().as_str(),
        "FAKE.DISCORD.TOKEN"
    );
    assert!(
        other.discord_token().unwrap().is_none(),
        "each data directory has its own entry"
    );
    let bytes = std::fs::read(first.path().join("accounts.sqlite")).unwrap();
    assert!(!bytes.windows(18).any(|w| w == b"FAKE.DISCORD.TOKEN"));
    accounts.delete_discord_token().unwrap();
    accounts.delete_discord_token().unwrap();
    assert!(accounts.discord_token().unwrap().is_none());
}
