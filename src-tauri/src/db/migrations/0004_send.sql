-- Phase 5: Antworten und Senden.

-- SMTP je Konto; Zugangsdaten sind die des IMAP-Kontos (Schlüsselbund).
ALTER TABLE accounts ADD COLUMN smtp_host TEXT;
ALTER TABLE accounts ADD COLUMN smtp_port INTEGER NOT NULL DEFAULT 465;

-- Absenderadressen je Konto, mit Signatur. Genau eine ist Standard.
CREATE TABLE identities (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  name TEXT NOT NULL,
  email TEXT NOT NULL,
  signature_text TEXT,
  signature_html TEXT,
  is_default INTEGER NOT NULL DEFAULT 0,
  UNIQUE (account_id, email)
);
CREATE UNIQUE INDEX idx_identity_default ON identities(account_id) WHERE is_default = 1;

-- Bestehende Konten: eine Identität aus dem Benutzernamen, sofern er wie eine Adresse aussieht.
INSERT INTO identities (account_id, name, email, is_default)
  SELECT id, name, lower(username), 1 FROM accounts WHERE username LIKE '%@%';

-- Entwürfe. Nichts davon verlässt den Rechner, bis es im Dialog bestätigt wurde.
CREATE TABLE drafts (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id),
  identity_id INTEGER REFERENCES identities(id),
  in_reply_to_message INTEGER REFERENCES messages(id),  -- Originalmail im Archiv
  kind TEXT NOT NULL,                                     -- new/reply/reply_all/forward
  to_addrs TEXT NOT NULL DEFAULT '[]',                    -- JSON [{name,address}]
  cc_addrs TEXT NOT NULL DEFAULT '[]',
  bcc_addrs TEXT NOT NULL DEFAULT '[]',
  subject TEXT NOT NULL DEFAULT '',
  body_html TEXT NOT NULL DEFAULT '',
  attachments TEXT NOT NULL DEFAULT '[]',                 -- JSON [{name,mime,source}]
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_drafts_account ON drafts(account_id, updated_at);

-- Adressbuch: aus dem Archiv gesammelt oder von Hand angelegt.
CREATE TABLE contacts (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,                             -- kleingeschrieben
  name TEXT,
  source TEXT NOT NULL DEFAULT 'archive',                 -- archive/manual
  seen_count INTEGER NOT NULL DEFAULT 0,
  last_seen TEXT
);
CREATE INDEX idx_contacts_seen ON contacts(seen_count);

-- Gesendete Mails liegen lokal in einem Pseudo-Ordner je Konto; is_local = 1
-- heißt: nicht vom Server, wird beim Abruf übersprungen.
ALTER TABLE imap_folders ADD COLUMN is_local INTEGER NOT NULL DEFAULT 0;
