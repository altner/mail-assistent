//! Tauri-Commands für das Frontend. Dünne Schicht: validieren, an die
//! Module delegieren, Fehler als Text zurückgeben.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::AiSettings;
use crate::ai::client::{self as ai_client, MessagesClient};
use crate::ai::run::{self as ai_run, RunProgress};
use crate::db::Db;
use crate::db::accounts::{self, Account, NewAccount};
use crate::db::categories::{self, Category, CategoryInput, Tag};
use crate::db::classifications::{self, DecisionInput, ReviewItem};
use crate::db::messages::{self, MessageDetail, MessageSummary};
use crate::db::threads::{self, ThreadDetail, ThreadSummary};
use crate::error::{Error, Result};
use crate::imap::sync::{self, SyncProgress, SyncSummary};
use crate::mail::body::{self, Body};
use crate::paths::DataPaths;
use crate::secrets;

pub const SYNC_PROGRESS_EVENT: &str = "sync://progress";
pub const AI_PROGRESS_EVENT: &str = "ai://progress";

pub struct AppState {
    pub db: Db,
    pub paths: DataPaths,
    running_syncs: Arc<Mutex<HashSet<i64>>>,
    /// Abbruch-Schalter des laufenden Vorschlagslaufs (höchstens einer)
    ai_run: Mutex<Option<Arc<AtomicBool>>>,
}

impl AppState {
    pub fn new(db: Db, paths: DataPaths) -> Self {
        Self {
            db,
            paths,
            running_syncs: Arc::default(),
            ai_run: Mutex::default(),
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

#[tauri::command]
pub fn list_messages(
    state: State<'_, AppState>,
    account_id: Option<i64>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<MessagePage> {
    let limit = limit.unwrap_or(200).min(1000);
    state.db.with(|c| {
        Ok(MessagePage {
            items: messages::list(c, account_id, limit, offset.unwrap_or(0))?,
            total: messages::count(c, account_id)?,
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
}

fn is_local(url: &str) -> bool {
    ai_client::parse_base_url(url).is_ok_and(|u| ai_client::is_local(&u))
}

#[tauri::command]
pub async fn get_ai_settings(state: State<'_, AppState>) -> Result<AiSettingsView> {
    let settings = state.db.with(|c| AiSettings::load(c))?;
    let has_api_key = blocking(secrets::get_ai_key).await?.is_some();
    Ok(AiSettingsView {
        is_local: is_local(&settings.url),
        settings,
        has_api_key,
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

/// Verbindungstest: liefert die Modelle des Servers. Ohne `api_key` wird der
/// gespeicherte Schlüssel verwendet.
#[tauri::command]
pub async fn test_ai(url: String, api_key: Option<String>) -> Result<Vec<String>> {
    let key = match api_key.filter(|k| !k.trim().is_empty()) {
        Some(k) => Some(k.trim().to_string()),
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
