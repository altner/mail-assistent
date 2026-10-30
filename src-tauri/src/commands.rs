//! Tauri-Commands für das Frontend. Dünne Schicht: validieren, an die
//! Module delegieren, Fehler als Text zurückgeben.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::client::{self as ai_client, MessagesClient};
use crate::ai::run::{self as ai_run, RunProgress};
use crate::ai::{AiSettings, DraftAiSettings};
use crate::db::Db;
use crate::db::accounts::{self, Account, NewAccount};
use crate::db::categories::{self, Category, CategoryInput, Tag};
use crate::db::classifications::{self, DecisionInput, ReviewItem};
use crate::db::contacts::{self, Contact, ContactInput};
use crate::db::drafts::{self, Draft, DraftAttachment, DraftKind, DraftUpdate, NewDraft};
use crate::db::identities::{self, Identity, IdentityInput};
use crate::db::messages::{self, MessageDetail, MessageSummary};
use crate::db::threads::{self, ThreadDetail, ThreadSummary};
use crate::error::{Error, Result};
use crate::imap::sync::{self, SyncProgress, SyncSummary};
use crate::mail::body::{self, Body};
use crate::mail::compose::{self, Outgoing, OutgoingAttachment, ReplyHeaders};
use crate::mail::html;
use crate::mail::outbox::{self, SendPhase, SendStatus};
use crate::paths::DataPaths;
use crate::secrets;
use crate::smtp::{self, SmtpConfig};

pub const SYNC_PROGRESS_EVENT: &str = "sync://progress";
pub const AI_PROGRESS_EVENT: &str = "ai://progress";
pub const SEND_STATUS_EVENT: &str = "send://status";

pub struct AppState {
    pub db: Db,
    pub paths: DataPaths,
    running_syncs: Arc<Mutex<HashSet<i64>>>,
    /// Abbruch-Schalter des laufenden Vorschlagslaufs (höchstens einer)
    ai_run: Mutex<Option<Arc<AtomicBool>>>,
    /// Wartende Sendeaufträge je Entwurf („Rückgängig“)
    pending_sends: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl AppState {
    pub fn new(db: Db, paths: DataPaths) -> Self {
        Self {
            db,
            paths,
            running_syncs: Arc::default(),
            ai_run: Mutex::default(),
            pending_sends: Mutex::default(),
        }
    }

    fn ai_slot(&self) -> Result<std::sync::MutexGuard<'_, Option<Arc<AtomicBool>>>> {
        self.ai_run
            .lock()
            .map_err(|_| Error::Other("Sperre beschädigt".into()))
    }

    fn begin_sync(&self, account_id: i64) -> Result<SyncGuard> {
        let mut running = self
            .running_syncs
            .lock()
            .map_err(|_| Error::Other("Sperre beschädigt".into()))?;
        if !running.insert(account_id) {
            return Err(Error::Invalid(
                "Für dieses Konto läuft bereits ein Abruf".into(),
            ));
        }
        Ok(SyncGuard {
            account_id,
            running: self.running_syncs.clone(),
        })
    }
}

/// Gibt das Konto wieder frei, auch wenn der Abruf mit Fehler endet.
struct SyncGuard {
    account_id: i64,
    running: Arc<Mutex<HashSet<i64>>>,
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(&self.account_id);
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("Hintergrundaufgabe abgebrochen: {e}")))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    data_dir: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        data_dir: state.paths.root().display().to_string(),
    }
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    state.db.with(|c| accounts::list(c))
}

/// Legt das Konto an; das Passwort geht direkt in den Schlüsselbund. Klappt
/// das nicht, wird der DB-Eintrag zurückgenommen.
#[tauri::command]
pub async fn create_account(
    state: State<'_, AppState>,
    account: NewAccount,
    password: String,
) -> Result<Account> {
    let account = account.validated()?;
    if password.is_empty() {
        return Err(Error::Invalid("Passwort fehlt".into()));
    }
    let db = state.db.clone();
    blocking(move || {
        let id = db.with(|c| accounts::insert(c, &account))?;
        if let Err(e) = secrets::set_password(id, &password) {
            db.with(|c| Ok(c.execute("DELETE FROM accounts WHERE id = ?1", [id])?))?;
            return Err(e);
        }
        db.with(|c| accounts::get(c, id))
    })
    .await
}

#[tauri::command]
pub async fn sync_account(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: i64,
) -> Result<SyncSummary> {
    let _guard = state.begin_sync(account_id)?;
    let account = state.db.with(|c| accounts::get(c, account_id))?;
    // Zugriff auf den Schlüsselbund kann blockieren (Freigabe-Dialog unter macOS).
    let password = blocking(move || secrets::get_password(account_id)).await?;

    let report = move |p: SyncProgress| {
        if let Err(e) = app.emit(SYNC_PROGRESS_EVENT, p) {
            log::warn!("Fortschritt konnte nicht gesendet werden: {e}");
        }
    };
    let result = sync::sync_account(&state.db, &state.paths, &account, &password, &report).await;

    // Auch nach einem Abbruch: die bis dahin übernommenen Mails einordnen.
    let db = state.db.clone();
    let threading = blocking(move || db.with(|c| threads::rebuild(c, account_id))).await;
    match (result, threading) {
        (Ok(summary), Ok(_)) => Ok(summary),
        (Ok(_), Err(e)) => Err(Error::Other(format!(
            "Abruf fertig, Konversationen konnten nicht berechnet werden: {e}"
        ))),
        (Err(e), threading) => {
            if let Err(t) = threading {
                log::warn!("Konversationen nach Abbruch nicht berechnet: {t}");
            }
            Err(e)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    items: Vec<MessageSummary>,
    total: i64,
}

/// Mit `query`: Volltextsuche (FTS5) in Betreff, Absender und Text.
#[tauri::command]
pub fn list_messages(
    state: State<'_, AppState>,
    account_id: Option<i64>,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<MessagePage> {
    let limit = limit.unwrap_or(200).min(1000);
    let offset = offset.unwrap_or(0);
    let query = query.unwrap_or_default();
    state.db.with(|c| {
        Ok(MessagePage {
            items: messages::search(c, &query, account_id, limit, offset)?,
            total: messages::search_count(c, &query, account_id)?,
        })
    })
}

#[tauri::command]
pub fn get_message(state: State<'_, AppState>, id: i64) -> Result<MessageDetail> {
    state.db.with(|c| messages::get(c, id))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    items: Vec<ThreadSummary>,
    total: i64,
}

#[tauri::command]
pub fn list_threads(
    state: State<'_, AppState>,
    account_id: Option<i64>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<ThreadPage> {
    let limit = limit.unwrap_or(200).min(1000);
    state.db.with(|c| {
        Ok(ThreadPage {
            items: threads::list(c, account_id, limit, offset.unwrap_or(0))?,
            total: threads::count(c, account_id)?,
        })
    })
}

#[tauri::command]
pub fn get_thread(state: State<'_, AppState>, id: i64) -> Result<ThreadDetail> {
    state.db.with(|c| threads::get(c, id))
}

// --- KI-Einstellungen -------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsView {
    #[serde(flatten)]
    settings: AiSettings,
    has_api_key: bool,
    /// Server läuft auf diesem Rechner (sonst verlassen Mail-Inhalte das Gerät)
    is_local: bool,
    draft: DraftAiSettings,
    draft_has_api_key: bool,
    /// Gilt für den Server, der tatsächlich Entwürfe schreibt (eigener oder Hauptprofil)
    draft_is_local: bool,
}

fn is_local(url: &str) -> bool {
    ai_client::parse_base_url(url).is_ok_and(|u| ai_client::is_local(&u))
}

#[tauri::command]
pub async fn get_ai_settings(state: State<'_, AppState>) -> Result<AiSettingsView> {
    let (settings, draft, target) = state.db.with(|c| {
        Ok((
            AiSettings::load(c)?,
            DraftAiSettings::load(c)?,
            crate::ai::draft_target(c)?,
        ))
    })?;
    let (has_api_key, draft_has_api_key) = blocking(|| {
        Ok((
            secrets::get_ai_key()?.is_some(),
            secrets::get_ai_draft_key()?.is_some(),
        ))
    })
    .await?;
    Ok(AiSettingsView {
        is_local: is_local(&settings.url),
        settings,
        has_api_key,
        draft,
        draft_has_api_key,
        draft_is_local: is_local(&target.url),
    })
}

/// `api_key`: `None` lässt den Schlüssel unverändert, `""` löscht ihn.
#[tauri::command]
pub async fn save_ai_settings(
    state: State<'_, AppState>,
    settings: AiSettings,
    api_key: Option<String>,
) -> Result<AiSettingsView> {
    state.db.with(|c| settings.save(c))?;
    if let Some(key) = api_key {
        blocking(move || secrets::set_ai_key(key.trim())).await?;
    }
    get_ai_settings(state).await
}

/// Eigenes Profil für Entwürfe; `api_key` wie bei `save_ai_settings`.
#[tauri::command]
pub async fn save_draft_ai_settings(
    state: State<'_, AppState>,
    settings: DraftAiSettings,
    api_key: Option<String>,
) -> Result<AiSettingsView> {
    state.db.with(|c| settings.save(c))?;
    if let Some(key) = api_key {
        blocking(move || secrets::set_ai_draft_key(key.trim())).await?;
    }
    get_ai_settings(state).await
}

/// Verbindungstest: liefert die Modelle des Servers. Ohne `api_key` wird der
/// gespeicherte Schlüssel verwendet (`for_drafts`: der des Entwurfsprofils).
#[tauri::command]
pub async fn test_ai(
    url: String,
    api_key: Option<String>,
    for_drafts: Option<bool>,
) -> Result<Vec<String>> {
    let key = match api_key.filter(|k| !k.trim().is_empty()) {
        Some(k) => Some(k.trim().to_string()),
        None if for_drafts.unwrap_or(false) => blocking(secrets::get_ai_draft_key).await?,
        None => blocking(secrets::get_ai_key).await?,
    };
    MessagesClient::new(&url, key)?.models().await
}

// --- Kategorien und Tags -----------------------------------------------------

#[tauri::command]
pub fn list_categories(state: State<'_, AppState>) -> Result<Vec<Category>> {
    state.db.with(|c| categories::list(c))
}

#[tauri::command]
pub fn create_category(state: State<'_, AppState>, input: CategoryInput) -> Result<i64> {
    state.db.with(|c| categories::create(c, &input))
}

#[tauri::command]
pub fn update_category(state: State<'_, AppState>, id: i64, input: CategoryInput) -> Result<()> {
    state.db.with(|c| categories::update(c, id, &input))
}

#[tauri::command]
pub fn delete_category(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| categories::delete(c, id))
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>) -> Result<Vec<Tag>> {
    state.db.with(|c| categories::list_tags(c))
}

#[tauri::command]
pub fn create_tag(state: State<'_, AppState>, name: String) -> Result<i64> {
    state.db.with(|c| categories::create_tag(c, &name))
}

#[tauri::command]
pub fn rename_tag(state: State<'_, AppState>, id: i64, name: String) -> Result<()> {
    state.db.with(|c| categories::rename_tag(c, id, &name))
}

#[tauri::command]
pub fn delete_tag(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| categories::delete_tag(c, id))
}

// --- Vorschläge und Review ---------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOverview {
    /// offene Vorschläge
    pending: i64,
    /// Mails ohne jeden Vorschlag
    candidates: i64,
    running: bool,
}

#[tauri::command]
pub fn ai_overview(state: State<'_, AppState>) -> Result<AiOverview> {
    let running = state.ai_slot()?.is_some();
    state.db.with(|c| {
        Ok(AiOverview {
            pending: classifications::count_pending(c)?,
            candidates: classifications::count_candidates(c)?,
            running,
        })
    })
}

/// Holt Vorschläge für die nächsten `limit` Mails (Standard 50). Fortschritt
/// kommt als Event `ai://progress`.
#[tauri::command]
pub async fn start_suggestions(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<RunProgress> {
    let settings = state.db.with(|c| AiSettings::load(c))?;
    let model = settings.model.clone().ok_or_else(|| {
        Error::Invalid("Bitte zuerst unter „Einstellungen“ ein Modell wählen".into())
    })?;
    let key = blocking(secrets::get_ai_key).await?;
    let client = MessagesClient::new(&settings.url, key)?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.ai_slot()?;
        if slot.is_some() {
            return Err(Error::Invalid("Es läuft bereits ein Vorschlagslauf".into()));
        }
        *slot = Some(cancel.clone());
    }
    let report = move |p: RunProgress| {
        if let Err(e) = app.emit(AI_PROGRESS_EVENT, p) {
            log::warn!("Fortschritt konnte nicht gesendet werden: {e}");
        }
    };
    let limit = limit.unwrap_or(50).clamp(1, 1000);
    let result = ai_run::suggest_batch(&state.db, &client, &model, limit, &cancel, &report).await;
    *state.ai_slot()? = None;
    result
}

#[tauri::command]
pub fn cancel_suggestions(state: State<'_, AppState>) -> Result<()> {
    if let Some(cancel) = state.ai_slot()?.as_ref() {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPage {
    items: Vec<ReviewItem>,
    total: i64,
}

#[tauri::command]
pub fn list_review(
    state: State<'_, AppState>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<ReviewPage> {
    let limit = limit.unwrap_or(50).min(500);
    state.db.with(|c| {
        Ok(ReviewPage {
            items: classifications::pending(c, limit, offset.unwrap_or(0))?,
            total: classifications::count_pending(c)?,
        })
    })
}

#[tauri::command]
pub fn decide_review(state: State<'_, AppState>, id: i64, input: DecisionInput) -> Result<()> {
    state.db.with(|c| classifications::decide(c, id, &input))
}

// --- Mail-Inhalt, Links, Anhänge ---------------------------------------------

fn raw_mail(state: &AppState, id: i64) -> Result<Vec<u8>> {
    let detail = state.db.with(|c| messages::get(c, id))?;
    let path = state.paths.resolve(&detail.raw_path);
    std::fs::read(&path)
        .map_err(|e| Error::Other(format!("Mail-Datei {} nicht lesbar: {e}", detail.raw_path)))
}

/// Bereinigtes HTML der Mail für den abgeschotteten iframe. Externe Bilder
/// nur mit `allow_remote` (gilt nur für diesen Aufruf, wird nicht gespeichert).
#[tauri::command]
pub async fn get_message_body(
    state: State<'_, AppState>,
    id: i64,
    allow_remote: Option<bool>,
) -> Result<Body> {
    let raw = raw_mail(&state, id)?;
    blocking(move || body::body(&raw, allow_remote.unwrap_or(false))).await
}

/// Öffnet einen Link aus einer Mail im Standardbrowser. Nur http/https.
#[tauri::command]
pub fn open_link(app: AppHandle, url: String) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = ai_client::parse_base_url(&url)?;
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| Error::Other(format!("Link konnte nicht geöffnet werden: {e}")))
}

/// Speichert einen Anhang über den System-Dialog. `Ok(None)`: abgebrochen.
#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    part: usize,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    let raw = raw_mail(&state, id)?;
    let (name, _mime, bytes) = blocking(move || body::attachment_bytes(&raw, part)).await?;
    let suggested = body::safe_filename(&name);

    let chosen = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Anhang speichern")
            .set_file_name(&suggested)
            .blocking_save_file()
    })
    .await
    .map_err(|e| Error::Other(format!("Dialog abgebrochen: {e}")))?;

    let Some(target) = chosen else {
        return Ok(None);
    };
    let path = target
        .into_path()
        .map_err(|e| Error::Other(format!("Ungültiger Speicherort: {e}")))?;
    blocking(move || {
        std::fs::write(&path, &bytes)?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

// --- SMTP und Identitäten ----------------------------------------------------

#[tauri::command]
pub fn update_account_smtp(
    state: State<'_, AppState>,
    account_id: i64,
    smtp_host: String,
    smtp_port: u16,
) -> Result<Account> {
    let host = smtp_host.trim().to_ascii_lowercase();
    if host.contains(|c: char| c.is_whitespace() || c == '/' || c == ':') {
        return Err(Error::Invalid(
            "Server bitte ohne Protokoll/Port angeben".into(),
        ));
    }
    if smtp_port == 0 {
        return Err(Error::Invalid("Port fehlt".into()));
    }
    state.db.with(|c| {
        c.execute(
            "UPDATE accounts SET smtp_host = ?2, smtp_port = ?3 WHERE id = ?1",
            rusqlite::params![account_id, (!host.is_empty()).then_some(host), smtp_port],
        )?;
        accounts::get(c, account_id)
    })
}

/// Anmeldung am SMTP-Server prüfen, ohne etwas zu senden.
#[tauri::command]
pub async fn test_smtp(
    state: State<'_, AppState>,
    account_id: i64,
    smtp_host: String,
    smtp_port: u16,
) -> Result<()> {
    let account = state.db.with(|c| accounts::get(c, account_id))?;
    let password = blocking(move || secrets::get_password(account_id)).await?;
    smtp::test_connection(&SmtpConfig {
        host: smtp_host.trim(),
        port: smtp_port,
        username: &account.username,
        password: &password,
    })
    .await
}

#[tauri::command]
pub fn list_identities(
    state: State<'_, AppState>,
    account_id: Option<i64>,
) -> Result<Vec<Identity>> {
    state.db.with(|c| identities::list(c, account_id))
}

#[tauri::command]
pub fn create_identity(
    state: State<'_, AppState>,
    account_id: i64,
    input: IdentityInput,
) -> Result<i64> {
    state.db.with(|c| identities::create(c, account_id, &input))
}

#[tauri::command]
pub fn update_identity(state: State<'_, AppState>, id: i64, input: IdentityInput) -> Result<()> {
    state.db.with(|c| identities::update(c, id, &input))
}

#[tauri::command]
pub fn set_default_identity(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| identities::set_default(c, id))
}

#[tauri::command]
pub fn delete_identity(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| identities::delete(c, id))
}

// --- Entwürfe ------------------------------------------------------------------

fn own_addresses(c: &rusqlite::Connection, account_id: i64) -> Result<Vec<String>> {
    Ok(identities::list(c, Some(account_id))?
        .into_iter()
        .map(|i| i.email)
        .collect())
}

/// Legt einen Entwurf als Antwort/Weiterleitung auf eine archivierte Mail an.
#[tauri::command]
pub async fn prepare_draft(
    state: State<'_, AppState>,
    message_id: i64,
    kind: DraftKind,
) -> Result<Draft> {
    let detail = state.db.with(|c| messages::get(c, message_id))?;
    let account_id = detail.summary.account_id;
    let raw = raw_mail(&state, message_id)?;
    let (own, all_ids) = state.db.with(|c| {
        Ok((
            own_addresses(c, account_id)?,
            identities::list(c, Some(account_id))?,
        ))
    })?;
    // Identität: die angeschriebene, sonst Standard.
    let first_pass = blocking({
        let raw = raw.clone();
        let own = own.clone();
        move || compose::prepare(&raw, message_id, kind, &own, None)
    })
    .await?;
    let identity = first_pass
        .addressed_identity
        .as_ref()
        .and_then(|a| {
            all_ids
                .iter()
                .find(|i| i.email.eq_ignore_ascii_case(a))
                .cloned()
        })
        .or_else(|| all_ids.iter().find(|i| i.is_default).cloned())
        .or_else(|| all_ids.first().cloned());
    let prepared = blocking({
        let identity = identity.clone();
        move || compose::prepare(&raw, message_id, kind, &own, identity.as_ref())
    })
    .await?;

    state.db.with(|c| {
        let id = drafts::create(
            c,
            &NewDraft {
                account_id,
                identity_id: identity.as_ref().map(|i| i.id),
                in_reply_to_message: Some(message_id),
                kind,
                to: &prepared.to,
                cc: &prepared.cc,
                subject: &prepared.subject,
                body_html: &prepared.body_html,
                attachments: &prepared.attachments,
            },
        )?;
        drafts::get(c, id)
    })
}

/// Leerer neuer Entwurf für ein Konto (Signatur der Standard-Identität).
#[tauri::command]
pub fn new_draft(state: State<'_, AppState>, account_id: i64) -> Result<Draft> {
    state.db.with(|c| {
        let identity = identities::default_for(c, account_id)?;
        let body = format!(
            "<div><br></div>{}",
            compose::signature_block(identity.as_ref())
        );
        let id = drafts::create(
            c,
            &NewDraft {
                account_id,
                identity_id: identity.map(|i| i.id),
                in_reply_to_message: None,
                kind: DraftKind::New,
                to: &[],
                cc: &[],
                subject: "",
                body_html: &body,
                attachments: &[],
            },
        )?;
        drafts::get(c, id)
    })
}

#[tauri::command]
pub fn list_drafts(state: State<'_, AppState>) -> Result<Vec<Draft>> {
    state.db.with(|c| drafts::list(c))
}

#[tauri::command]
pub fn get_draft(state: State<'_, AppState>, id: i64) -> Result<Draft> {
    state.db.with(|c| drafts::get(c, id))
}

#[tauri::command]
pub fn update_draft(state: State<'_, AppState>, id: i64, update: DraftUpdate) -> Result<()> {
    state.db.with(|c| drafts::update(c, id, &update))
}

#[tauri::command]
pub fn delete_draft(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| drafts::delete(c, id))?;
    outbox::remove_draft_files(&state.paths, id);
    Ok(())
}

/// Datei über den System-Dialog wählen und an den Entwurf hängen.
#[tauri::command]
pub async fn add_draft_file(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Option<Draft>> {
    use tauri_plugin_dialog::DialogExt;
    let draft = state.db.with(|c| drafts::get(c, id))?;
    let chosen = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Datei anhängen")
            .blocking_pick_file()
    })
    .await
    .map_err(|e| Error::Other(format!("Dialog abgebrochen: {e}")))?;
    let Some(source) = chosen else {
        return Ok(None);
    };
    let source = source
        .into_path()
        .map_err(|e| Error::Other(format!("Ungültige Datei: {e}")))?;
    let name = crate::mail::body::safe_filename(
        source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("anhang"),
    );
    let paths = state.paths.clone();
    let (rel, size) = blocking(move || {
        let dir = outbox::draft_dir(&paths, id);
        std::fs::create_dir_all(&dir)?;
        let target = dir.join(&name);
        std::fs::copy(&source, &target)?;
        let size = std::fs::metadata(&target)?.len() as usize;
        Ok((format!("drafts/{id}/{name}"), size))
    })
    .await?;
    let name = rel.rsplit('/').next().unwrap_or("anhang").to_string();
    let mime = mime_from_name(&name).to_string();
    let mut list = draft.attachments;
    list.retain(|a| !matches!(a, DraftAttachment::File { path, .. } if *path == rel));
    list.push(DraftAttachment::File {
        path: rel,
        name,
        mime,
        size,
    });
    state.db.with(|c| {
        drafts::set_attachments(c, id, &list)?;
        drafts::get(c, id).map(Some)
    })
}

fn mime_from_name(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("txt") => "text/plain",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        Some("ics") => "text/calendar",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

#[tauri::command]
pub fn remove_draft_attachment(state: State<'_, AppState>, id: i64, index: usize) -> Result<Draft> {
    let draft = state.db.with(|c| drafts::get(c, id))?;
    let mut list = draft.attachments;
    if index >= list.len() {
        return Err(Error::NotFound(format!("Anhang {index}")));
    }
    if let DraftAttachment::File { path, .. } = list.remove(index) {
        let _ = std::fs::remove_file(state.paths.resolve(&path));
    }
    state.db.with(|c| {
        drafts::set_attachments(c, id, &list)?;
        drafts::get(c, id)
    })
}

// --- Adressbuch ---------------------------------------------------------------

#[tauri::command]
pub fn suggest_contacts(state: State<'_, AppState>, query: String) -> Result<Vec<Contact>> {
    state.db.with(|c| contacts::suggest(c, &query, 8))
}

#[tauri::command]
pub fn list_contacts(state: State<'_, AppState>) -> Result<Vec<Contact>> {
    state.db.with(|c| contacts::list(c))
}

#[tauri::command]
pub fn create_contact(state: State<'_, AppState>, input: ContactInput) -> Result<i64> {
    state.db.with(|c| contacts::create(c, &input))
}

#[tauri::command]
pub fn update_contact(state: State<'_, AppState>, id: i64, input: ContactInput) -> Result<()> {
    state.db.with(|c| contacts::update(c, id, &input))
}

#[tauri::command]
pub fn delete_contact(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.db.with(|c| contacts::delete(c, id))
}

/// Adressbuch aus dem Archiv neu aufbauen (manuelle Einträge bleiben).
#[tauri::command]
pub async fn rebuild_contacts(state: State<'_, AppState>) -> Result<usize> {
    let db = state.db.clone();
    blocking(move || db.with(contacts::rebuild_from_archive)).await
}

// --- Senden -------------------------------------------------------------------

#[tauri::command]
pub fn get_send_delay(state: State<'_, AppState>) -> Result<u32> {
    state.db.with(|c| outbox::delay_seconds(c))
}

#[tauri::command]
pub fn set_send_delay(state: State<'_, AppState>, seconds: u32) -> Result<()> {
    state.db.with(|c| outbox::set_delay_seconds(c, seconds))
}

fn pending(state: &AppState) -> Result<std::sync::MutexGuard<'_, HashMap<i64, Arc<AtomicBool>>>> {
    state
        .pending_sends
        .lock()
        .map_err(|_| Error::Other("Sperre beschädigt".into()))
}

/// Sendet einen Entwurf. Wird nur nach dem Bestätigungsdialog aufgerufen.
/// Mit eingestellter Verzögerung erst nach Ablauf, sofern nicht `cancel_send`
/// dazwischenkommt. Danach lokale Ablage, Entwurf weg.
#[tauri::command]
pub async fn send_draft(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<SendStatus> {
    let draft = state.db.with(|c| drafts::get(c, id))?;
    let account = state.db.with(|c| accounts::get(c, draft.account_id))?;
    let (smtp_host, smtp_port): (Option<String>, u16) = state.db.with(|c| {
        Ok(c.query_row(
            "SELECT smtp_host, smtp_port FROM accounts WHERE id = ?1",
            [draft.account_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    let smtp_host = smtp_host.filter(|h| !h.is_empty()).ok_or_else(|| {
        Error::Invalid("Für dieses Konto ist kein SMTP-Server eingetragen (Reiter „Konten“)".into())
    })?;
    let identity = state.db.with(|c| match draft.identity_id {
        Some(i) => identities::get(c, i),
        None => identities::default_for(c, draft.account_id)?
            .ok_or_else(|| Error::Invalid("Keine Absenderadresse für dieses Konto".into())),
    })?;
    let reply = match draft.in_reply_to_message {
        Some(m) => {
            let orig = state.db.with(|c| messages::get(c, m))?;
            ReplyHeaders::from_original(
                orig.message_id.as_deref(),
                orig_refs(&state, m)?.as_deref(),
            )
        }
        None => ReplyHeaders::default(),
    };

    // Anhänge einsammeln.
    let mut attachments = Vec::new();
    for a in &draft.attachments {
        match a {
            DraftAttachment::Message {
                message_id, part, ..
            } => {
                let raw = raw_mail(&state, *message_id)?;
                let part = *part;
                let (name, mime, bytes) =
                    blocking(move || crate::mail::body::attachment_bytes(&raw, part)).await?;
                attachments.push(OutgoingAttachment { name, mime, bytes });
            }
            DraftAttachment::File {
                path, name, mime, ..
            } => {
                let bytes = std::fs::read(state.paths.resolve(path))
                    .map_err(|e| Error::Other(format!("Anhang {name} nicht lesbar: {e}")))?;
                attachments.push(OutgoingAttachment {
                    name: name.clone(),
                    mime: mime.clone(),
                    bytes,
                });
            }
        }
    }

    let (message, _mid) = compose::build(&Outgoing {
        identity: &identity,
        to: &draft.to,
        cc: &draft.cc,
        bcc: &draft.bcc,
        subject: &draft.subject,
        body_html: &draft.body_html,
        reply: &reply,
        attachments,
    })?;

    let report = {
        let app = app.clone();
        move |s: SendStatus| {
            if let Err(e) = app.emit(SEND_STATUS_EVENT, s) {
                log::warn!("Sendestatus konnte nicht gesendet werden: {e}");
            }
        }
    };
    let status = |phase: SendPhase, error: Option<String>, message_id: Option<i64>| SendStatus {
        draft_id: id,
        phase,
        seconds_left: 0,
        error,
        message_id,
    };

    // Wartezeit mit „Rückgängig“.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut p = pending(&state)?;
        if p.contains_key(&id) {
            return Err(Error::Invalid("Dieser Entwurf wird gerade gesendet".into()));
        }
        p.insert(id, cancel.clone());
    }
    let delay = state.db.with(|c| outbox::delay_seconds(c))?;
    let go = outbox::wait_with_undo(id, delay, &cancel, &report).await;
    if !go {
        pending(&state)?.remove(&id);
        let s = status(SendPhase::Cancelled, None, None);
        report(s.clone());
        return Ok(s);
    }
    report(status(SendPhase::Sending, None, None));

    let password = blocking(move || secrets::get_password(account.id)).await?;
    let result = smtp::send(
        &SmtpConfig {
            host: &smtp_host,
            port: smtp_port,
            username: &account.username,
            password: &password,
        },
        message.clone(),
    )
    .await;
    pending(&state)?.remove(&id);
    if let Err(e) = result {
        let s = status(SendPhase::Failed, Some(e.to_string()), None);
        report(s);
        return Err(e);
    }

    // Lokal ablegen, Konversation nachziehen, Entwurf entfernen.
    let raw = message.formatted();
    let db = state.db.clone();
    let paths = state.paths.clone();
    let account_id = draft.account_id;
    let archived = blocking(move || {
        let mid = db.with(|c| outbox::archive_sent(c, &paths, account_id, &raw))?;
        db.with(|c| threads::rebuild(c, account_id))?;
        db.with(|c| drafts::delete(c, id))?;
        Ok(mid)
    })
    .await;
    let message_id = match archived {
        Ok(m) => Some(m),
        Err(e) => {
            // Gesendet ist gesendet; die Ablage nur melden.
            log::warn!("Gesendete Mail nicht abgelegt: {e}");
            None
        }
    };
    outbox::remove_draft_files(&state.paths, id);
    let s = status(SendPhase::Sent, None, message_id);
    report(s.clone());
    Ok(s)
}

fn orig_refs(state: &AppState, message_id: i64) -> Result<Option<String>> {
    state.db.with(|c| {
        Ok(c.query_row(
            "SELECT refs FROM messages WHERE id = ?1",
            [message_id],
            |r| r.get(0),
        )?)
    })
}

#[tauri::command]
pub fn cancel_send(state: State<'_, AppState>, id: i64) -> Result<bool> {
    match pending(&state)?.get(&id) {
        Some(c) => {
            c.store(true, Ordering::Relaxed);
            Ok(true)
        }
        None => Ok(false),
    }
}

// --- KI-Entwurf ----------------------------------------------------------------

/// Schreibt mit der KI einen Textvorschlag für den Entwurf. Liefert HTML für
/// den Editor; eingefügt wird es dort, gesendet nur über den Dialog.
#[tauri::command]
pub async fn ai_draft(state: State<'_, AppState>, id: i64, hint: Option<String>) -> Result<String> {
    use crate::ai::draft::{self as ai_draft_mod, DraftInput};

    let draft = state.db.with(|c| drafts::get(c, id))?;
    let target = state.db.with(|c| crate::ai::draft_target(c))?;
    let model = target.model.clone().ok_or_else(|| {
        Error::Invalid("Bitte zuerst unter „Einstellungen“ ein Modell für Entwürfe wählen".into())
    })?;
    let key = if target.separate_key {
        blocking(secrets::get_ai_draft_key).await?
    } else {
        blocking(secrets::get_ai_key).await?
    };
    let client = MessagesClient::new(&target.url, key)?;
    let identity = state.db.with(|c| match draft.identity_id {
        Some(i) => identities::get(c, i).map(Some),
        None => identities::default_for(c, draft.account_id),
    })?;
    let sender_name = identity.map(|i| i.name).unwrap_or_default();

    let (original, thread) = match draft.in_reply_to_message {
        Some(m) => {
            let orig = state.db.with(|c| messages::get(c, m))?;
            let thread: Vec<(String, String)> = match orig.summary.thread_id {
                Some(t) => state
                    .db
                    .with(|c| messages::list_by_thread(c, t))?
                    .into_iter()
                    .filter(|x| x.summary.id != m)
                    .filter(|x| x.summary.date <= orig.summary.date)
                    .map(|x| {
                        (
                            x.summary
                                .from_name
                                .or(x.summary.from_addr)
                                .unwrap_or_default(),
                            x.body_text.unwrap_or_default(),
                        )
                    })
                    .collect(),
                None => Vec::new(),
            };
            (Some(orig), thread)
        }
        None => (None, Vec::new()),
    };
    let from = original
        .as_ref()
        .map(|o| match (&o.summary.from_name, &o.summary.from_addr) {
            (Some(n), Some(a)) => format!("{n} <{a}>"),
            (Some(n), None) => n.clone(),
            (None, Some(a)) => a.clone(),
            (None, None) => "(unbekannt)".into(),
        });
    let original_text = original
        .as_ref()
        .and_then(|o| o.body_text.clone())
        .unwrap_or_default();
    let subject = original.as_ref().and_then(|o| o.summary.subject.clone());
    let text = ai_draft_mod::draft(
        &client,
        &model,
        &DraftInput {
            original_from: from.as_deref(),
            original_subject: subject.as_deref(),
            original_text: &original_text,
            thread: &thread,
            sender_name: &sender_name,
            hint: hint.as_deref(),
            forward: draft.kind == DraftKind::Forward,
        },
    )
    .await?;
    Ok(html::text_to_fragment(&text))
}
