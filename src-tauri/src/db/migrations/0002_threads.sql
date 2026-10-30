-- Phase 2: Konversationen. Threads gelten pro Konto und werden nach jedem
-- Abruf aus Message-ID, In-Reply-To, References und Betreff neu berechnet.
-- message_count, unread_count und last_date sind dabei mitgeführte Kennzahlen.
-- (Index auf messages.thread_id gibt es seit 0001.)

ALTER TABLE threads ADD COLUMN account_id INTEGER REFERENCES accounts(id);
ALTER TABLE threads ADD COLUMN message_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN unread_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_threads_account_date ON threads(account_id, last_date);
