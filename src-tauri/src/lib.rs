pub mod ai;
mod commands;
pub mod db;
pub mod error;
pub mod imap;
pub mod mail;
pub mod paths;
pub mod secrets;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("mail_assistent_lib=info"),
    )
    .init();

    tauri::Builder::default()
        .setup(|app| {
            let paths = paths::DataPaths::init(app.path().app_data_dir()?)?;
            log::info!("Datenverzeichnis: {}", paths.root().display());
            let db = db::Db::open(&paths.db_file())?;
            // Mails ohne Konversation (z. B. nach dem Update auf Schema 2) einordnen.
            db.with(|c| {
                for account_id in db::threads::accounts_needing_rebuild(c)? {
                    let stats = db::threads::rebuild(c, account_id)?;
                    log::info!("Konto {account_id}: {} Konversationen", stats.threads);
                }
                Ok(())
            })?;
            app.manage(AppState::new(db, paths));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_accounts,
            commands::create_account,
            commands::sync_account,
            commands::list_messages,
            commands::get_message,
            commands::list_threads,
            commands::get_thread,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri-App konnte nicht gestartet werden");
}
