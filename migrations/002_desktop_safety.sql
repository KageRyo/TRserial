ALTER TABLE codes ADD COLUMN test_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN manual INTEGER NOT NULL DEFAULT 0;
CREATE TABLE code_sequence (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 code TEXT NOT NULL UNIQUE REFERENCES codes(code)
);
INSERT INTO code_sequence(code) SELECT code FROM codes ORDER BY created_at, rowid;
CREATE TABLE daemon_accounts (
 account_id TEXT PRIMARY KEY,
 baseline INTEGER NOT NULL
);
UPDATE codes SET test_only=1 WHERE code IN (
 SELECT d.code FROM discoveries d JOIN messages m ON m.id=d.message_id
 WHERE m.source='mock' OR m.source LIKE 'mock:%' OR m.source='discord:mock'
);
PRAGMA user_version = 2;
