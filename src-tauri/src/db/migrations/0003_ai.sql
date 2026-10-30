-- Phase 3: KI-Vorschläge.

-- Einfache Einstellungen (KI-Server, Modell). Geheimnisse liegen nie hier,
-- sondern im Schlüsselbund.
CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Was die KI ursprünglich vorgeschlagen hat. category_id ist die aktuelle
-- bzw. bestätigte Kategorie; bei status = 'changed' unterscheiden sich beide.
ALTER TABLE classifications ADD COLUMN suggested_category_id INTEGER REFERENCES categories(id);
CREATE INDEX idx_class_status ON classifications(status);

-- Startvorschlag; im UI änder- und löschbar. Die Beschreibung ist der
-- Hilfetext, den die KI zu jeder Kategorie bekommt.
INSERT INTO categories (name, description) VALUES
  ('Rechnungen', 'Rechnungen, Zahlungsaufforderungen, Quittungen und Belege'),
  ('Bestellungen & Versand', 'Bestellbestätigungen, Versand- und Lieferbenachrichtigungen, Retouren'),
  ('Reisen', 'Buchungen und Bestätigungen für Flüge, Bahn, Hotels, Mietwagen, Reiseunterlagen'),
  ('Finanzen', 'Bank, Kreditkarte, Depot, Steuer, Versicherungen und Kontoauszüge'),
  ('Verträge & Behörden', 'Verträge, Kündigungen, Ämter, Behörden, offizielle Schreiben'),
  ('Arbeit', 'Berufliche Korrespondenz mit Kollegen, Kunden und Geschäftspartnern'),
  ('Persönlich', 'Persönliche Nachrichten von Familie, Freunden und Bekannten'),
  ('Newsletter', 'Abonnierte Newsletter, Blogs und redaktionelle Inhalte'),
  ('Benachrichtigungen', 'Automatische Hinweise von Diensten: Konten, Sicherheit, Passwörter, Systemmeldungen'),
  ('Werbung', 'Angebote, Rabatte und Marketing ohne eigenen Informationswert');
