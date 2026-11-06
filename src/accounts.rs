use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub alias: String,
    pub username: String,
    pub session_only: bool,
}
/// Intentionally has no Debug/Serialize implementation.
pub struct Credentials {
    pub username: String,
    pub password: Zeroizing<String>,
}
pub struct Accounts(Connection);
impl Accounts {
    pub fn open(dir: &Path) -> Result<Self> {
        let db = Connection::open(dir.join("accounts.sqlite"))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS accounts(id TEXT PRIMARY KEY, alias TEXT NOT NULL UNIQUE, username TEXT NOT NULL, session_only INTEGER NOT NULL);")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        db.execute(
            "INSERT OR IGNORE INTO settings VALUES('identity_seed',?1)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        Ok(Self(db))
    }
    pub fn list(&self) -> Result<Vec<Account>> {
        let mut q = self
            .0
            .prepare("SELECT id,alias,username,session_only FROM accounts ORDER BY alias")?;
        Ok(q.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                alias: r.get(1)?,
                username: r.get(2)?,
                session_only: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn add(
        &self,
        alias: &str,
        username: &str,
        password: &str,
        session_only: bool,
    ) -> Result<Account> {
        ensure!(
            !alias.trim().is_empty() && !username.trim().is_empty() && !password.is_empty(),
            "別名、帳號與密碼不可留白"
        );
        ensure!(
            !self
                .list()?
                .iter()
                .any(|a| a.alias == alias || a.username == username),
            "別名或帳號已存在"
        );
        // A removed and re-added username retains its history identity without retaining removed metadata.
        // Scope the identifier to this installation, so independent data directories cannot delete each other's keyring entry.
        let seed: String = self.0.query_row(
            "SELECT value FROM settings WHERE key='identity_seed'",
            [],
            |r| r.get(0),
        )?;
        let id = format!(
            "{:x}",
            Sha256::digest(format!("{seed}\0{username}").as_bytes())
        );
        let a = Account {
            id,
            alias: alias.into(),
            username: username.into(),
            session_only,
        };
        if !session_only {
            entry(&a)?.set_password(password).map_err(|_| {
                anyhow::anyhow!("無法安全儲存密碼；請解鎖作業系統憑證庫，或使用 --session-only")
            })?;
        }
        if let Err(e) = self.0.execute(
            "INSERT INTO accounts VALUES(?1,?2,?3,?4)",
            params![a.id, a.alias, a.username, a.session_only],
        ) {
            if !session_only {
                let _ = entry(&a)?.delete_credential();
            }
            return Err(e.into());
        }
        Ok(a)
    }
    pub fn remove(&self, alias: &str) -> Result<()> {
        let a = self
            .list()?
            .into_iter()
            .find(|a| a.alias == alias)
            .context("找不到帳號別名")?;
        if !a.session_only {
            match entry(&a)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => (),
                Err(_) => anyhow::bail!("憑證庫刪除失敗；帳號設定已保留"),
            }
        }
        self.0.execute("DELETE FROM accounts WHERE id=?1", [a.id])?;
        Ok(())
    }
}
fn entry(a: &Account) -> Result<keyring::Entry> {
    keyring::Entry::new("TRserial", &a.id).map_err(|_| anyhow::anyhow!("作業系統憑證庫無法使用"))
}
pub fn credentials(a: &Account) -> Result<Credentials> {
    if !a.session_only
        && let Ok(e) = entry(a)
        && let Ok(password) = e.get_password()
    {
        return Ok(Credentials {
            username: a.username.clone(),
            password: Zeroizing::new(password),
        });
    }
    ensure!(
        io::stdin().is_terminal(),
        "憑證庫無法提供密碼；請在互動終端執行兌換或修復憑證庫"
    );
    let password = Zeroizing::new(rpassword::prompt_password(format!(
        "{} 的密碼（僅供本次使用）：",
        a.alias
    ))?);
    ensure!(!password.is_empty(), "密碼不可留白");
    Ok(Credentials {
        username: a.username.clone(),
        password,
    })
}
pub fn prompt(label: &str) -> Result<String> {
    ensure!(io::stdin().is_terminal(), "此指令需要互動終端");
    print!("{label}：");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_owned())
}
