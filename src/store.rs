use crate::{accounts::Account, parser::ParsedCode};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Processing,
    WaitingVerification,
    Success,
    AlreadyRedeemed,
    Expired,
    Failed,
    Unknown,
}
impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::WaitingVerification => "waiting_verification",
            Self::Success => "success",
            Self::AlreadyRedeemed => "already_redeemed",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub code: String,
    pub account_id: String,
    pub alias: String,
    pub status: String,
    pub attempts: u32,
    pub submitted: bool,
    pub detail: String,
    pub updated_at: i64,
}
pub struct Store {
    db: Connection,
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CodeRecord {
    pub code: String,
    pub needs_review: bool,
    pub test_only: bool,
    pub discovered_at: i64,
    pub rewards: Vec<String>,
    pub expiration: Option<String>,
}
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AttemptRecord {
    pub job_id: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub detail: String,
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 2, "資料庫版本比程式新，請更新 TRserial");
        if version < 2 {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            // Another process may have migrated while we waited for the write lock.
            let v: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if v == 0 {
                tx.execute_batch(include_str!("../migrations/001_initial.sql"))?;
            }
            if v < 2 {
                tx.execute_batch(include_str!("../migrations/002_desktop_safety.sql"))?;
            }
            tx.commit()?;
        }
        Ok(Self { db })
    }
    pub fn ingest(
        &mut self,
        source: &str,
        external: Option<&str>,
        text: &str,
        codes: &[ParsedCode],
    ) -> Result<usize> {
        if codes.is_empty() {
            return Ok(0);
        }
        let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
        let tx = self.db.transaction()?;
        tx.execute("INSERT OR IGNORE INTO messages(source,external_id,digest,original,created_at) VALUES(?1,?2,?3,?4,?5)", params![source,external,digest,text,now()])?;
        let mid: i64 = tx.query_row(
            "SELECT id FROM messages WHERE source=?1 AND digest=?2",
            params![source, digest],
            |r| r.get(0),
        )?;
        let mut added = 0;
        for c in codes {
            added += tx.execute(
                "INSERT OR IGNORE INTO codes(code,needs_review,created_at) VALUES(?1,?2,?3)",
                params![c.code, c.needs_review, now()],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO code_sequence(code) VALUES(?1)",
                [&c.code],
            )?;
            if source == "mock" || source.starts_with("mock:") || source == "discord:mock" {
                tx.execute("UPDATE codes SET test_only=1 WHERE code=?1", [&c.code])?;
            }
            // A later, explicit labelled discovery can confirm an ambiguous candidate.
            if !c.needs_review {
                tx.execute("UPDATE codes SET needs_review=0 WHERE code=?1", [&c.code])?;
            }
            tx.execute(
                "INSERT OR IGNORE INTO discoveries VALUES(?1,?2,?3,?4)",
                params![
                    mid,
                    c.code,
                    serde_json::to_string(&c.rewards)?,
                    c.expiration
                ],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }
    pub fn approve(&self, code: &str) -> Result<()> {
        ensure!(
            self.db
                .execute("UPDATE codes SET needs_review=0 WHERE code=?1", [code])?
                == 1,
            "找不到序號"
        );
        Ok(())
    }
    pub fn enqueue(&mut self, code: &str, accounts: &[Account]) -> Result<usize> {
        let review: bool = self
            .db
            .query_row(
                "SELECT needs_review FROM codes WHERE code=?1",
                [code],
                |r| r.get(0),
            )
            .optional()?
            .context("找不到序號；請先 ingest")?;
        ensure!(!review, "序號需要人工確認；請先執行 code approve");
        let tx = self.db.transaction()?;
        let mut added = 0;
        for a in accounts {
            added += tx.execute("INSERT OR IGNORE INTO jobs(code,account_id,account_alias,status,updated_at,manual) VALUES(?1,?2,?3,'pending',?4,1)", params![code,a.id,a.alias,now()])?;
            tx.execute(
                "UPDATE jobs SET manual=1 WHERE code=?1 AND account_id=?2",
                params![code, a.id],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }
    pub fn test_only(&self, code: &str) -> Result<bool> {
        Ok(self
            .db
            .query_row("SELECT test_only FROM codes WHERE code=?1", [code], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false))
    }
    /// Baselines are durable and account-specific. Run before any new source ingestion.
    pub fn initialize_daemon(&mut self, accounts: &[Account]) -> Result<()> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for a in accounts {
            tx.execute("INSERT OR IGNORE INTO daemon_accounts VALUES(?1,(SELECT COALESCE(MAX(id),0) FROM code_sequence))",[&a.id])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn schedule_daemon(&mut self, accounts: &[Account], backfill: bool) -> Result<usize> {
        self.initialize_daemon(accounts)?;
        let tx = self.db.transaction()?;
        let mut added = 0;
        for a in accounts.iter().filter(|a| !a.test_only) {
            added+=tx.execute("INSERT OR IGNORE INTO jobs(code,account_id,account_alias,status,updated_at) SELECT c.code,?1,?2,'pending',?3 FROM codes c JOIN code_sequence s ON s.code=c.code JOIN daemon_accounts d ON d.account_id=?1 WHERE c.needs_review=0 AND c.test_only=0 AND (s.id>d.baseline OR ?4)",params![a.id,a.alias,now(),backfill])?;
        }
        tx.commit()?;
        Ok(added)
    }
    pub fn reset_baseline(&self, account: &str) -> Result<()> {
        self.db.execute("INSERT INTO daemon_accounts VALUES(?1,(SELECT COALESCE(MAX(id),0) FROM code_sequence)) ON CONFLICT(account_id) DO UPDATE SET baseline=excluded.baseline", [account])?;
        Ok(())
    }
    pub fn historical_for(&self, code: &str, account: &str) -> Result<bool> {
        Ok(self.db.query_row("SELECT COALESCE(s.id<=d.baseline,1) FROM code_sequence s LEFT JOIN daemon_accounts d ON d.account_id=?2 WHERE s.code=?1", params![code,account], |r| r.get(0))?)
    }
    pub fn code_records(&self) -> Result<Vec<CodeRecord>> {
        let mut q = self.db.prepare("SELECT c.code,c.needs_review,c.test_only,c.created_at FROM codes c JOIN code_sequence s ON s.code=c.code ORDER BY s.id DESC")?;
        let mut records = q
            .query_map([], |r| {
                Ok(CodeRecord {
                    code: r.get(0)?,
                    needs_review: r.get(1)?,
                    test_only: r.get(2)?,
                    discovered_at: r.get(3)?,
                    rewards: Vec::new(),
                    expiration: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut discovery = self.db.prepare(
            "SELECT rewards,expiration FROM discoveries WHERE code=?1 ORDER BY message_id DESC",
        )?;
        for record in &mut records {
            let descriptions = discovery
                .query_map([&record.code], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (rewards, expiration) in descriptions {
                for reward in serde_json::from_str::<Vec<String>>(&rewards)? {
                    if !record.rewards.contains(&reward) {
                        record.rewards.push(reward);
                    }
                }
                if record.expiration.is_none() {
                    record.expiration = expiration;
                }
            }
        }
        Ok(records)
    }
    pub fn job_attempts(&self, job_id: i64) -> Result<Vec<AttemptRecord>> {
        let mut q=self.db.prepare("SELECT job_id,started_at,finished_at,status,detail FROM attempts WHERE job_id=?1 ORDER BY id")?;
        Ok(q.query_map([job_id], |r| {
            Ok(AttemptRecord {
                job_id: r.get(0)?,
                started_at: r.get(1)?,
                finished_at: r.get(2)?,
                status: r.get(3)?,
                detail: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn daemon_job_allowed(&self, id: i64, backfill: bool) -> Result<bool> {
        Ok(self.db.query_row("SELECT j.manual OR ?2 OR COALESCE(s.id>d.baseline,0) FROM jobs j JOIN code_sequence s ON s.code=j.code LEFT JOIN daemon_accounts d ON d.account_id=j.account_id WHERE j.id=?1",params![id,backfill],|r|r.get(0))?)
    }
    pub fn job_policy(&self, id: i64) -> Result<&'static str> {
        let (manual,new,test):(bool,bool,bool)=self.db.query_row("SELECT j.manual,COALESCE(s.id>d.baseline,0),c.test_only FROM jobs j JOIN codes c ON c.code=j.code JOIN code_sequence s ON s.code=j.code LEFT JOIN daemon_accounts d ON d.account_id=j.account_id WHERE j.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        Ok(if test {
            "test_only"
        } else if manual {
            "manual"
        } else if new {
            "new"
        } else {
            "historical"
        })
    }
    pub fn codes(&self) -> Result<Vec<(String, bool)>> {
        let mut q = self
            .db
            .prepare("SELECT code,needs_review FROM codes ORDER BY created_at,code")?;
        Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        let mut q = self.db.prepare("SELECT id,code,account_id,account_alias,status,attempts,submitted,detail,updated_at FROM jobs ORDER BY id")?;
        Ok(q.query_map([], |r| {
            Ok(Job {
                id: r.get(0)?,
                code: r.get(1)?,
                account_id: r.get(2)?,
                alias: r.get(3)?,
                status: r.get(4)?,
                attempts: r.get(5)?,
                submitted: r.get(6)?,
                detail: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    /// Caller MUST hold WorkerLock for the entire browser operation.
    pub fn recover(&mut self) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE jobs SET status=CASE submitted WHEN 1 THEN 'unknown' ELSE 'pending' END,detail='程式中斷後復原',updated_at=?1 WHERE status IN ('processing','waiting_verification')", [now()])?;
        tx.execute("UPDATE attempts SET finished_at=?1,status=(SELECT status FROM jobs WHERE jobs.id=attempts.job_id),detail='程式中斷後復原' WHERE finished_at IS NULL", [now()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn claim(&mut self, id: i64, max: u32) -> Result<bool> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE jobs SET status='failed',detail='已達嘗試上限，請人工確認後重試',updated_at=?1 WHERE id=?2 AND status='pending' AND attempts>=?3",params![now(),id,max])?;
        let n = tx.execute("UPDATE jobs SET status='processing',attempts=attempts+1,submitted=0,updated_at=?1 WHERE id=?2 AND status='pending' AND next_at<=?1 AND attempts<?3", params![now(),id,max])?;
        if n == 1 {
            tx.execute(
                "INSERT INTO attempts(job_id,started_at,status) VALUES(?1,?2,'processing')",
                params![id, now()],
            )?;
        }
        tx.commit()?;
        Ok(n == 1)
    }
    pub fn waiting(&self, id: i64) -> Result<()> {
        ensure!(self.db.execute("UPDATE jobs SET status='waiting_verification',updated_at=?1 WHERE id=?2 AND status='processing' AND submitted=0", params![now(),id])? == 1, "無法進入人工驗證狀態");
        Ok(())
    }
    pub fn mark_submitted(&self, id: i64) -> Result<()> {
        ensure!(self.db.execute("UPDATE jobs SET submitted=1,status='processing',updated_at=?1 WHERE id=?2 AND status IN ('processing','waiting_verification') AND submitted=0", params![now(),id])? == 1, "工作已送出或狀態不符");
        Ok(())
    }
    pub fn finish(&mut self, id: i64, status: Status, detail: &str, next_at: i64) -> Result<()> {
        ensure!(
            !matches!(status, Status::Processing | Status::WaitingVerification),
            "不可用完成操作設定執行中狀態"
        );
        let tx = self.db.transaction()?;
        ensure!(tx.execute(
            "UPDATE jobs SET status=?1,detail=?2,next_at=?3,updated_at=?4 WHERE id=?5 AND status IN ('pending','processing','waiting_verification') AND (?1!='pending' OR submitted=0)",
            params![status.as_str(), detail, next_at, now(), id],
        )? == 1, "無法改寫已完成的工作，或將已送出的工作排回佇列");
        tx.execute("UPDATE attempts SET finished_at=?1,status=?2,detail=?3 WHERE job_id=?4 AND finished_at IS NULL",params![now(),status.as_str(),detail,id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn retry_reviewed(&self, id: i64) -> Result<()> {
        ensure!(self.db.execute("UPDATE jobs SET status='pending',attempts=0,submitted=0,next_at=0,manual=1,detail='使用者確認後重試',updated_at=?1 WHERE id=?2 AND status IN ('unknown','failed')",params![now(),id])? == 1, "僅可重試 unknown 或 failed 工作");
        Ok(())
    }
    pub fn cursor(&self, source: &str) -> Result<Option<String>> {
        Ok(self
            .db
            .query_row(
                "SELECT last_id FROM cursors WHERE source=?1",
                [source],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn set_cursor(&self, source: &str, id: &str) -> Result<()> {
        self.db.execute("INSERT INTO cursors VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET last_id=excluded.last_id",params![source,id])?;
        Ok(())
    }
    pub fn source_messages(&self) -> Result<Vec<(String, String)>> {
        let mut q = self
            .db
            .prepare("SELECT source,original FROM messages ORDER BY id")?;
        Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn attempts(&self) -> Result<Vec<(i64, i64, String, String)>> {
        let mut q = self
            .db
            .prepare("SELECT job_id,started_at,status,detail FROM attempts ORDER BY id")?;
        Ok(
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?,
        )
    }
}
