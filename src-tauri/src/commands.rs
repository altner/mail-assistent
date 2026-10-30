//! Tauri-Commands für das Frontend. Dünne Schicht: validieren, an die
//! Module delegieren, Fehler als Text zurückgeben.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::AiConfig;
use crate::db::Db;
use crate::db::accounts::{self, Account, NewAccount};
use crate::db::messages::{self, MessageDetail, MessageSummary};
use crate::db::threads::{self, ThreadDetail, ThreadSummary};
use crate::error::{Error, Result};
use crate::imap::sync::{self, SyncProgress, SyncSummary};
use crate::paths::DataPaths;
use crate::secrets;

pub const SYNC_PROGRESS_EVENT: &str = "sync://progress";

pub struct AppState {
    pub db: Db,
    pub paths: DataPaths,
    running_syncs: Arc<Mutex<HashSet<i64>>>,
}

impl AppState {
    pub fn new(db: Db, paths: DataPaths) -> Self {
        Self {
            db,
            paths,
            running_syncs: Arc::default(),
        }
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
    ai: AiConfig,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        data_dir: state.paths.root().display().to_string(),
        ai: AiConfig::default(),
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
