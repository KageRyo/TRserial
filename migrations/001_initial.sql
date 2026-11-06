CREATE TABLE messages (
 id INTEGER PRIMARY KEY, source TEXT NOT NULL, external_id TEXT,
 digest TEXT NOT NULL, original TEXT NOT NULL, created_at INTEGER NOT NULL,
 UNIQUE(source, digest)
);
CREATE TABLE codes (
 code TEXT PRIMARY KEY COLLATE BINARY, needs_review INTEGER NOT NULL,
 created_at INTEGER NOT NULL
);
CREATE TABLE discoveries (
 message_id INTEGER NOT NULL REFERENCES messages(id), code TEXT NOT NULL REFERENCES codes(code),
 rewards TEXT NOT NULL, expiration TEXT, PRIMARY KEY(message_id, code)
);
CREATE TABLE jobs (
 id INTEGER PRIMARY KEY, code TEXT NOT NULL REFERENCES codes(code), account_id TEXT NOT NULL,
 account_alias TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('pending','processing','waiting_verification','success','already_redeemed','expired','failed','unknown')),
 attempts INTEGER NOT NULL DEFAULT 0, submitted INTEGER NOT NULL DEFAULT 0,
 next_at INTEGER NOT NULL DEFAULT 0, detail TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL,
 UNIQUE(code, account_id)
);
CREATE TABLE attempts (
 id INTEGER PRIMARY KEY, job_id INTEGER NOT NULL REFERENCES jobs(id), started_at INTEGER NOT NULL,
 finished_at INTEGER, status TEXT NOT NULL, detail TEXT NOT NULL DEFAULT ''
);
CREATE TABLE cursors (source TEXT PRIMARY KEY, last_id TEXT NOT NULL);
CREATE INDEX ready_jobs ON jobs(status,next_at);
PRAGMA user_version = 1;
