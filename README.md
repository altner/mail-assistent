# Mail-Assistent

Lokales Archiv für private IMAP-Postfächer mit (später) KI-Kategorisierung.
Desktop-App mit Tauri v2 (Rust) und Svelte 5 + TypeScript. Die Server-Postfächer
bleiben unangetastet und dienen nur als Backup.

Stand: **Phase 5**. Konten anlegen, Mails nur lesend abrufen und lokal archivieren,
Mail-Liste als einzelne Mails oder Konversationen, Volltextsuche, HTML-Anzeige im
abgeschotteten iframe mit Anhängen, KI-Vorschläge für Kategorien und Tags mit Review,
Antworten und Weiterleiten mit Rich-Text-Editor, Adressbuch, Signaturen, KI-Entwurf und
Versand nur nach Bestätigung.

## Harte Regeln

1. **Server nur lesen.** Ordner werden mit `EXAMINE` geöffnet (nie `SELECT`), Mails mit
   `BODY.PEEK[]` geholt. Keine Flags setzen, nichts verschieben oder löschen.
   Durchgesetzt über `ReadOnlySession` in `src-tauri/src/imap/session.rs`, die keine
   schreibenden Befehle anbietet. Tests sichern das ab.
2. **Der Gelesen-Status wird nur eingelesen** (`\Seen` → `is_read`) und angezeigt, nie zurückgeschrieben.
3. **Kategorien und Tags gibt es nur lokal**, mit einem gemeinsamen Schema für alle Postfächer.
4. **Die KI schlägt nur vor.** Jeder Vorschlag bleibt `pending`, bis er im UI bestätigt oder geändert wird.
5. **Gesendet wird nur, was du im Editor verfasst und im Dialog bestätigt hast.** Die KI
   schreibt höchstens Entwürfe. Es gibt keinen Versand ohne den Dialog, keine Warteschlange,
   die ohne dich sendet, und am IMAP-Postfach wird auch dabei nichts geschrieben. Siehe „Senden“.
6. **HTML-Mails** werden nur in einem abgeschotteten iframe angezeigt: ohne JavaScript,
   externe Bilder standardmäßig blockiert. Siehe „HTML-Anzeige“.
7. **Alle Daten liegen im App-Datenverzeichnis des Betriebssystems** (Tauri-Pfad-API),
   macOS: `~/Library/Application Support/de.adrianaltner.mailassistent/`
   - `archive.sqlite3`: Datenbank (SQLite mit FTS5)
   - `mail/<konto>/<fp[0..2]>/<fingerprint>.eml`: Rohmails
8. **Passwörter liegen nur im Schlüsselbund** (macOS Keychain, Windows Credential Manager,
   Linux Secret Service), Service `de.adrianaltner.mailassistent`. Sie stehen nie in der DB oder in Dateien.
9. **TLS nur mit rustls** (ring, Mozilla-Wurzelzertifikate), kein OpenSSL, auch für HTTP zum
   KI-Server (`src-tauri/src/tls.rs`). IMAP nur mit implizitem TLS (Port 993).

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

## Konversationen

Nach jedem Abruf (und beim Start für noch nicht zugeordnete Mails) werden die Konversationen
des Kontos komplett neu berechnet (`mail/thread.rs`, `db/threads.rs`). Das dauert bei einigen
tausend Mails Sekundenbruchteile und ordnet auch ältere Mails nachträglich richtig ein.

1. **JWZ-Verknüpfung:** Jede Mail wird mit allen Message-IDs aus `References` und `In-Reply-To`
   verbunden. Fehlt eine Mail im Archiv, hält ein Platzhalter die Antworten darauf zusammen.
   Da die Ansicht chronologisch ist, reichen die Zusammenhangskomponenten (Union-Find); ein
   Antwortbaum wird nicht gespeichert. Zirkelbezüge sind dadurch unproblematisch.
2. **Betreff als Ersatz, bewusst eng:** Eine Konversation wird nur an eine frühere mit gleichem
   Betreff (ohne Re/AW/Fwd/WG …) angehängt, wenn ihre erste Mail ein Antwort-Präfix trägt und
   höchstens 60 Tage nach deren letzter Mail liegt. Mails wie „Ihre Rechnung“ bleiben getrennt.
3. **Stabile IDs:** Eine Konversation behält ihre ID, solange die meisten ihrer Mails
   zusammenbleiben. Verschmelzen zwei, bleibt die ID der größeren (bei Gleichstand die kleinere ID).

Konversationen gelten pro Konto, wie die Duplikaterkennung.

## HTML-Anzeige

Drei Schichten, jede für sich schon ausreichend (`src-tauri/src/mail/html.rs`, `src/components/MailBody.svelte`):

1. **Bereinigung im Rust-Kern** mit `lol_html`: `script`, `iframe`, `object`, `embed`, `form`
   und Eingabefelder, `<base>` und `<meta http-equiv>` fliegen samt Inhalt raus, ebenso alle
   `on*`-Attribute und `javascript:`-Links. `url(...)` in Styles bleibt nur für `data:`.
2. **`<iframe sandbox="allow-scripts">`** ohne `allow-same-origin`: Das Mail-Dokument hat eine
   fremde Herkunft und kann das App-Fenster nicht erreichen (kein DOM-Zugriff, keine IPC),
   dazu keine Formulare, keine Navigation des Fensters, keine Popups.
3. **Eigene CSP im Mail-Dokument:** `default-src 'none'`, `script-src` erlaubt per SHA-256-Hash
   genau ein Skript, Bilder nur `data:` (mit Freigabe zusätzlich `https:`), dazu
   `referrer: no-referrer`.

Das eine erlaubte Skript ist das **Brücken-Skript der App** (`BRIDGE_JS` in `mail/html.rs`),
das Rust in jedes bereinigte Dokument einsetzt. Es meldet Klicks auf Links, den Link unter dem
Zeiger und die Inhaltshöhe per `postMessage`; das Frontend nimmt nur Nachrichten aus dem eigenen
iframe an und öffnet nur `http(s)`. Skripte aus der Mail haben einen anderen Hash und laufen
nicht. Der Umweg ist nötig, weil WebKit in einem iframe ohne `allow-scripts` keine
Event-Listener ausführt, auch nicht die vom Elternfenster angehängten.

**Bilder:** Eingebettete Bilder (`cid:`) werden aus der Mail selbst als `data:`-URL eingesetzt.
Externe Bilder (auch in Styles) werden entfernt und gezählt; „Bilder laden“ gibt sie nur für diese
Anzeige frei (nur `https:`), gespeichert wird die Freigabe nicht. So laden Tracking-Pixel nie
unbemerkt. Die Fenster-CSP enthält dafür `img-src https:` und `frame-src about:`.

**Links:** Ein Klick öffnet `http(s)`-Links im Standardbrowser (Plugin `opener`, nur diese
Schemata, aus Rust heraus). Beim Überfahren steht das echte Ziel unter der Mail, damit
Phishing-Links auffallen. `mailto:` wird nur angezeigt, nichts wird gesendet. In der Textansicht
(Umschalter HTML/Text, Mails ohne HTML) werden URLs ebenfalls klickbar.

**Anhänge** werden bei Bedarf aus der `.eml` gelesen und unter der Mail aufgelistet; „Speichern“
öffnet den System-Dialog (Plugin `dialog`), Dateinamen werden bereinigt. Die Tabelle
`attachments` aus dem Schema bleibt dafür ungenutzt, bis Anhänge einmal durchsuchbar sein sollen.

## Volltextsuche

Das Suchfeld in der Mail-Liste sucht per FTS5 in Betreff, Absender und Text. Jedes Wort wird
als Präfix gesucht, alle Wörter müssen vorkommen („rechn märz“ findet „Rechnung März“).
FTS-Sonderzeichen in der Eingabe sind unschädlich. Die Suche gilt für einzelne Mails; sie
schaltet die Liste automatisch um.

## Senden

**Antworten, Allen antworten, Weiterleiten** (Knöpfe in der Mail- und Konversationsansicht) legen
einen Entwurf an (`mail/compose.rs`): Empfänger aus `Reply-To` oder `From`, bei „Allen“ plus
`To`/`Cc` ohne die eigenen Adressen; Antwort auf eine eigene Mail geht an deren Empfänger. Betreff
ohne doppelte `Re:`/`AW:`/`Fwd:`. `In-Reply-To` und `References` werden gesetzt. Das Original
steht bereinigt im `blockquote`, bei Weiterleitungen mit Kopfzeilen und den Original-Anhängen.
Die **Identität** (Absenderadresse, Reiter „Konten“) wird automatisch gewählt: die angeschriebene,
sonst die Standard-Identität; ihre **Signatur** steht mit `-- ` über dem Zitat und wechselt mit der
Identität.

**Editor** (`Composer.svelte`): Rich-Text ohne Fremdbibliothek (fett, kursiv, Listen, Zitat, Links
nur `http(s)`), Einfügen nur als Text, automatisches Speichern. Dateien aus dem System-Dialog
werden nach `drafts/<id>/` kopiert. Empfängerfelder schlagen Kontakte vor (häufigste zuerst).
Gesendet wird `multipart/alternative` aus Klartext (`html_to_text`) und HTML; das HTML läuft vor
dem Senden durch dieselbe Bereinigung wie beim Anzeigen.

**KI-Entwurf**: Knopf mit Hinweisfeld (`ai/draft.rs`). Das Modell bekommt die Originalmail, den
bisherigen Verlauf der Konversation, den Hinweis und den Namen der Identität und liefert nur den
Text, der oben in den Editor kommt. Der Mailinhalt gilt wieder als fremdes Material.

**Versand**: „Senden …“ öffnet den Dialog mit Konto, Absender, Empfängern, Betreff und Anhängen.
Erst „Jetzt senden“ ruft `send_draft` auf; es gibt keinen anderen Aufrufer. Mit Wartezeit (Reiter
„Einstellungen“: aus, 10, 30 oder 60 s) läuft zuerst eine Leiste „Wird in N s gesendet ·
Rückgängig“; in der Zeit ist nichts beim Server. Beim Beenden der App geht nichts Wartendes raus,
der Entwurf bleibt. SMTP (`smtp/`) nutzt `lettre` mit rustls/ring, nur implizites TLS (Port 465)
und die IMAP-Zugangsdaten aus dem Schlüsselbund; „Anmeldung testen“ sendet nichts. Danach wird
die Mail als `.eml` im lokalen Pseudo-Ordner `[Lokal]/Gesendet` des Kontos abgelegt (sichtbar in
Liste und Konversation), die Empfänger landen im Adressbuch, der Entwurf verschwindet.

**Adressbuch** (`db/contacts.rs`, Reiter „Kontakte“): Absender und Empfänger des Archivs werden
beim Abruf gezählt, beim ersten Start nach dem Update einmal rückwirkend. Eigene Identitäten
werden ausgelassen. Bearbeitete oder manuell angelegte Einträge überschreibt das Archiv nicht mehr.

## KI-Vorschläge

Die KI läuft auf einem lokalen Server mit **Anthropic-Messages-Schnittstelle** (`POST /v1/messages`,
`GET /v1/models`). Getestet mit oMLX; Splash und Ollama bieten dieselbe Schnittstelle (Ollama erst
in aktuellen Versionen). Unter „Einstellungen“ wird der Server gewählt (Vorlage, URL, Modell aus
`/v1/models`, Verbindungstest). Ein API-Schlüssel (oMLX verlangt einen) liegt nur im Schlüsselbund.
Zeigt die URL nicht auf diesen Rechner, warnt die App, weil Mail-Inhalte dann das Gerät verlassen.

| Server | Standard-URL | API-Schlüssel |
|---|---|---|
| Ollama | `http://localhost:11434` | wird nicht geprüft |
| oMLX | `http://127.0.0.1:8000` | nötig |
| Splash | `http://127.0.0.1:8000` | nur mit `--api-key` |

**Ablauf** (`src-tauri/src/ai/`): „Vorschläge für 50 Mails holen“ nimmt die neuesten Mails ohne
Vorschlag. Das Modell bekommt Absender, Betreff, Datum, die ersten 1500 Zeichen Text sowie die
Kategorien mit Beschreibung und die Tags, und antwortet mit JSON (`category`, `confidence`,
`reason`, `tags`). Die Auswertung verträgt Denkphasen, Codeblöcke und Beiwerk; unbekannte
Kategorien oder Tags werden verworfen, die KI legt also nichts an. Der Mailtext gilt als fremder
Inhalt, Anweisungen darin werden laut Prompt ignoriert.

**Regel 4 in der Datenbank:** Jeder Vorschlag ist `classifications.status = 'pending'`
(Tags: `message_tags.status`). Erst das Review setzt `accepted`, `changed` (dann bleibt der
ursprüngliche Vorschlag in `suggested_category_id`) oder `rejected`. Nur `accepted`/`changed`
gelten als Kategorie der Mail. Bereits einmal vorgeschlagene Mails werden nicht erneut angefragt.

**Review:** ⏎ übernehmen, 1–9 andere Kategorie (Reihenfolge wie unter „Einstellungen“),
X ablehnen, J/K vor und zurück. Vorgeschlagene Tags sind vorausgewählt und abwählbar.

Zum Start gibt es zehn Kategorien (Rechnungen, Bestellungen & Versand, Reisen, Finanzen,
Verträge & Behörden, Arbeit, Persönlich, Newsletter, Benachrichtigungen, Werbung); alle änderbar.

Erfahrung mit oMLX und `Qwen3.8-27B-4bit` auf einem M-Series-Mac mit 24 GB: etwa 15 s pro Mail,
Vorschläge mit passender deutscher Begründung. Die MTP-Variante lädt in oMLX 0.7.2 nicht
(„Model type qwen3_5_mtp not supported“).

## Struktur

```
src/                         Svelte 5 + TS
  lib/          api.ts (invoke-Wrapper), types.ts, state.svelte.ts, format.ts
  components/   AccountManager, ListPane (Konto-Filter, Ansicht, Suche), MailList, ThreadList,
                MailDetail, ThreadView, MailBody (iframe, Links, Anhänge), ReplyButtons,
                Composer (Editor, Dialog, Rückgängig), RecipientField, DraftsView, ContactsView,
                AccountSending (SMTP, Identitäten), ReviewView, SettingsView
src-tauri/src/
  lib.rs        Setup, AppState, Command-Registrierung
  commands.rs   Tauri-Commands (Konten, Mails, Threads, KI, Kategorien, Review, Entwürfe,
                Kontakte, Identitäten, Senden)
  db/           Verbindung, Migrationen (migrations/*.sql), Queries
  imap/         connect (TLS), session (nur lesen), sync, mutf7 (Ordnernamen)
  mail/         parse, fingerprint, store (.eml), ingest (Übernahme ins Archiv), thread (JWZ),
                html (Bereinigung), body (Anzeige-Inhalt und Anhänge aus der .eml),
                compose (Antwort/Weiterleitung, MIME), outbox (Wartezeit, lokale Ablage)
  secrets/      Schlüsselbund-Wrapper (IMAP-Passwörter, API-Schlüssel des KI-Servers)
  smtp/         Versand über lettre (rustls, SMTPS)
  tls.rs        gemeinsame rustls-Konfiguration für IMAP und HTTP
  ai/           client (/v1/messages, /v1/models), prompt (Prompt + Auswertung), run (Vorschlagslauf),
                draft (Antwortentwurf)
```

Neue Migrationen kommen als `src-tauri/src/db/migrations/000N_*.sql` dazu und werden in
`migrations.rs` angehängt. Bestehende Migrationen werden nie geändert.

Zusätzlich zum vorgegebenen Schema gibt es:
- einen partiellen Unique-Index: höchstens eine bestätigte Kategorie (`accepted`/`changed`) pro Mail
- FTS-Trigger für Insert, Update und Delete
- Indizes auf `messages(account_id, date)`, `messages(date)` und `classifications(message_id)`
- Migration 0002: `threads.account_id`, `threads.message_count`, `threads.unread_count`,
  Index `threads(account_id, last_date)`
- Migration 0003: Tabelle `settings`, `classifications.suggested_category_id`,
  Index `classifications(status)`, Startkategorien
- Migration 0004: `accounts.smtp_host/smtp_port`, Tabellen `identities`, `drafts`, `contacts`,
  `imap_folders.is_local` (lokaler Gesendet-Ordner)

## Nächste Phasen

- ~~**2** Threading (JWZ über In-Reply-To/References, Fallback über normalisierten Betreff), Thread-Ansicht~~ erledigt
- ~~**3** KI-Anbindung, strukturierte Vorschläge mit Begründung, Review-UI mit Tastenkürzeln~~ erledigt
  (statt nur Ollama: jeder Server mit `/v1/messages`, also auch oMLX und Splash)
- ~~**4** Volltextsuche im UI, HTML im abgeschotteten iframe, Anhänge~~ erledigt
- ~~**5** Antwortentwürfe und SMTP (nur mit manueller Freigabe)~~ erledigt, dazu Rich-Text,
  Adressbuch, Signaturen, mehrere Absenderadressen und KI-Entwurf
- **6** mehrere Postfächer parallel, Windows/Linux, Signierung

Noch offen: Konto bearbeiten oder löschen, Passwort ändern, Gelesen-Status
bereits archivierter Mails aktualisieren (bisher nur beim ersten Abruf), STARTTLS (IMAP und
SMTP), abweichende SMTP-Zugangsdaten, HTML-Signaturen im UI (nur Text), Bilder im Editor,
Konversationen über mehrere Konten, Gmail-Labels (`X-GM-LABELS`) übernehmen, Kategorie einer
Mail nachträglich ändern (bisher nur im Review), Vorschläge für abgelehnte Mails erneut holen.
