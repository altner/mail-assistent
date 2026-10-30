-- Migration 0001: komplettes Grundschema.

CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  imap_host TEXT NOT NULL,
  imap_port INTEGER NOT NULL DEFAULT 993,
  username TEXT NOT NULL,           -- Passwort liegt im Schlüsselbund
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE imap_folders (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  path TEXT NOT NULL,
  uidvalidity INTEGER,
  last_uid INTEGER DEFAULT 0,
  UNIQUE (account_id, path)
);

CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  description TEXT,                 -- Hilfetext für die KI
  parent_id INTEGER REFERENCES categories(id)
);

CREATE TABLE tags (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE threads (
  id INTEGER PRIMARY KEY,
  subject_norm TEXT,                -- Betreff ohne Re:/AW:/Fwd:
  last_date TEXT
);

CREATE TABLE messages (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  folder_id INTEGER NOT NULL REFERENCES imap_folders(id),
  uid INTEGER NOT NULL,
  message_id TEXT,
  fingerprint TEXT NOT NULL,
  in_reply_to TEXT,
  refs TEXT,
  thread_id INTEGER REFERENCES threads(id),
  from_addr TEXT,
  from_name TEXT,
  to_addrs TEXT,
  subject TEXT,
  date TEXT,
  body_text TEXT,
  raw_path TEXT NOT NULL,
  is_read INTEGER DEFAULT 0,
  fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE (account_id, folder_id, uid),
  UNIQUE (account_id, fingerprint)
);
CREATE INDEX idx_msg_message_id ON messages(message_id);
CREATE INDEX idx_msg_thread ON messages(thread_id);
-- Zusatz: Mail-Liste sortiert nach Datum, optional je Konto.
CREATE INDEX idx_msg_account_date ON messages(account_id, date);
CREATE INDEX idx_msg_date ON messages(date);

CREATE TABLE classifications (
  id INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES messages(id),
  category_id INTEGER REFERENCES categories(id),
  source TEXT NOT NULL,             -- 'ai' oder 'user'
  confidence REAL,
  reason TEXT,
  status TEXT NOT NULL DEFAULT 'pending',  -- pending/accepted/changed/rejected
  model TEXT,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  decided_at TEXT
);
CREATE INDEX idx_class_message ON classifications(message_id);
-- Eine Mail hat höchstens eine bestätigte Kategorie.
CREATE UNIQUE INDEX idx_class_one_confirmed
  ON classifications(message_id) WHERE status IN ('accepted', 'changed');

CREATE TABLE message_tags (
  message_id INTEGER NOT NULL REFERENCES messages(id),
  tag_id INTEGER NOT NULL REFERENCES tags(id),
  source TEXT NOT NULL,             -- 'ai' oder 'user'
  status TEXT NOT NULL DEFAULT 'pending',
  PRIMARY KEY (message_id, tag_id)
);

CREATE TABLE attachments (
  id INTEGER PRIMARY KEY,
  message_id INTEGER NOT NULL REFERENCES messages(id),
  filename TEXT,
  mime_type TEXT,
  size INTEGER,
  stored_path TEXT
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_name, from_addr, body_text,
  content='messages', content_rowid='id'
);

-- FTS synchron halten (External-Content-Tabelle: Löschen über 'delete'-Befehl).
CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body_text)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body_text);
END;

CREATE TRIGGER messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body_text)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body_text);
END;

-- Nur bei Änderungen an indexierten Spalten (nicht bei uid/is_read).
CREATE TRIGGER messages_fts_au AFTER UPDATE OF subject, from_name, from_addr, body_text ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body_text)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body_text);
  INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body_text)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body_text);
END;
