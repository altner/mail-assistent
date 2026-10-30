# Mail-Assistent

Lokales Archiv für private IMAP-Postfächer mit (später) KI-Kategorisierung.
Desktop-App mit Tauri v2 (Rust) und Svelte 5 + TypeScript. Die Server-Postfächer
bleiben unangetastet und dienen nur als Backup.

Stand: **Scaffold (Phase 1)**. Konten anlegen, Mails nur lesend abrufen und
lokal archivieren, Mail-Liste mit Konto-Filter.

## Harte Regeln

1. **Server nur lesen.** Ordner werden mit `EXAMINE` geöffnet (nie `SELECT`), Mails mit
   `BODY.PEEK[]` geholt. Keine Flags setzen, nichts verschieben oder löschen.
   Durchgesetzt über `ReadOnlySession` in `src-tauri/src/imap/session.rs`, die keine
   schreibenden Befehle anbietet. Tests sichern das ab.
2. **Der Gelesen-Status wird nur eingelesen** (`\Seen` → `is_read`) und angezeigt, nie zurückgeschrieben.
3. **Kategorien und Tags gibt es nur lokal**, mit einem gemeinsamen Schema für alle Postfächer.
4. **Die KI schlägt nur vor.** Jeder Vorschlag bleibt `pending`, bis er im UI bestätigt oder geändert wird.
5. **Nichts wird gesendet.** SMTP und Antwortentwürfe kommen erst in einer späteren Phase.
6. **HTML-Mails** werden später nur in einem abgeschotteten iframe angezeigt: ohne JavaScript,
   externe Bilder standardmäßig blockiert. Bis dahin gibt es nur Klartext.
7. **Alle Daten liegen im App-Datenverzeichnis des Betriebssystems** (Tauri-Pfad-API),
   macOS: `~/Library/Application Support/de.adrianaltner.mailassistent/`
   - `archive.sqlite3`: Datenbank (SQLite mit FTS5)
   - `mail/<konto>/<fp[0..2]>/<fingerprint>.eml`: Rohmails
8. **Passwörter liegen nur im Schlüsselbund** (macOS Keychain, Windows Credential Manager,
   Linux Secret Service), Service `de.adrianaltner.mailassistent`. Sie stehen nie in der DB oder in Dateien.
9. **TLS nur mit rustls** (ring, Mozilla-Wurzelzertifikate), kein OpenSSL. Nur implizites TLS (Port 993).

## Voraussetzungen

- Rust (stable) und Xcode Command Line Tools
- Node.js ≥ 20 und npm

## Starten

```bash
npm install
npm run tauri dev
```

Alternativ mit der Cargo-CLI: `cargo install tauri-cli --version "^2" --locked`, danach `cargo tauri dev`.

Weitere Befehle:

```bash
cd src-tauri && cargo test     # Rust-Tests (Migration, Parsing, Fingerprint, Ingest, IMAP-Hilfen)
npm run check                  # svelte-check / TypeScript
npm run tauri build            # App-Bundle, ad-hoc signiert (keine Notarisierung)
```

Log-Ausgabe: `RUST_LOG=mail_assistent_lib=debug npm run tauri dev`

## Abruf: so funktioniert er

1. Anmeldung per TLS, `LIST "" "*"`. Papierkorb und Spam werden übersprungen (erkannt über
   SPECIAL-USE `\Trash`/`\Junk` oder übliche Namen). INBOX kommt zuerst, „Alle Nachrichten“ zuletzt.
2. Je Ordner `EXAMINE`. Hat sich `UIDVALIDITY` geändert, werden die bisherigen UIDs des Ordners
   als veraltet markiert (`uid = -id`) und der Ordner neu abgerufen. Bekannte Mails bekommen
   per Fingerprint ihre neue UID, lokal bleibt alles erhalten.
3. `UID SEARCH UID <last_uid+1>:*`, dann `UID FETCH (UID FLAGS INTERNALDATE BODY.PEEK[])` in Blöcken à 50.
4. Pro Mail: parsen, Fingerprint bilden, Duplikate überspringen, `.eml` atomar schreiben, Zeile in `messages`.
   `last_uid` wird in derselben Transaktion fortgeschrieben, ein Abbruch setzt also genau dort wieder an.
5. Fortschritt geht als Event `sync://progress` ans UI.

**Fingerprint:** SHA-256 aus Message-ID und Absender (kleingeschrieben). Fehlt die Message-ID:
SHA-256 aus Absender, Datum (UTC), Betreff und den ersten 500 Zeichen Text (Leerraum normalisiert).

## Struktur

```
src/                         Svelte 5 + TS
  lib/          api.ts (invoke-Wrapper), types.ts, state.svelte.ts, format.ts
  components/   AccountManager, MailList, MailDetail, ReviewView (Platzhalter)
src-tauri/src/
  lib.rs        Setup, AppState, Command-Registrierung
  commands.rs   Tauri-Commands
  db/           Verbindung, Migrationen (migrations/*.sql), Queries
  imap/         connect (TLS), session (nur lesen), sync, mutf7 (Ordnernamen)
  mail/         parse, fingerprint, store (.eml), ingest (Übernahme ins Archiv)
  secrets/      Schlüsselbund-Wrapper
  ai/           Classifier-Trait + Platzhalter (Phase 3: Ollama, Standard http://localhost:11434)
```

Neue Migrationen kommen als `src-tauri/src/db/migrations/000N_*.sql` dazu und werden in
`migrations.rs` angehängt. Bestehende Migrationen werden nie geändert.

Zusätzlich zum vorgegebenen Schema gibt es:
- einen partiellen Unique-Index: höchstens eine bestätigte Kategorie (`accepted`/`changed`) pro Mail
- FTS-Trigger für Insert, Update und Delete
- Indizes auf `messages(account_id, date)`, `messages(date)` und `classifications(message_id)`

## Nächste Phasen

- **2** Threading (JWZ über In-Reply-To/References, Fallback über normalisierten Betreff), Thread-Ansicht
- **3** Ollama-Anbindung, strukturierte Vorschläge mit Begründung, Review-UI mit Tastenkürzeln
- **4** Volltextsuche im UI, HTML im abgeschotteten iframe, Anhänge
- **5** Antwortentwürfe und SMTP (nur mit manueller Freigabe)
- **6** mehrere Postfächer parallel, Windows/Linux, Signierung

Noch offen im Scaffold: Konto bearbeiten oder löschen, Passwort ändern, Gelesen-Status
bereits archivierter Mails aktualisieren (bisher nur beim ersten Abruf), STARTTLS.
