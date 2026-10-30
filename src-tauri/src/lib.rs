pub mod ai;
mod commands;
pub mod db;
pub mod error;
pub mod imap;
pub mod mail;
pub mod paths;
pub mod secrets;
pub mod smtp;
pub mod tls;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("mail_assistent_lib=info"),
    )
    .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = paths::DataPaths::init(app.path().app_data_dir()?)?;
            log::info!("Datenverzeichnis: {}", paths.root().display());
            let db = db::Db::open(&paths.db_file())?;
            // Adressbuch einmalig aus dem Archiv füllen (Update auf Schema 4).
            db.with(|c| {
                if db::contacts::count(c)? == 0 && db::messages::count(c, None)? > 0 {
                    let n = db::contacts::rebuild_from_archive(c)?;
                    log::info!("Adressbuch aus dem Archiv aufgebaut: {n} Adressen gezählt");
                }
                Ok(())
            })?;
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
            commands::get_ai_settings,
            commands::save_ai_settings,
            commands::test_ai,
            commands::list_categories,
            commands::create_category,
            commands::update_category,
            commands::delete_category,
            commands::list_tags,
            commands::create_tag,
            commands::rename_tag,
            commands::delete_tag,
            commands::ai_overview,
            commands::start_suggestions,
            commands::cancel_suggestions,
            commands::list_review,
            commands::decide_review,
            commands::get_message_body,
            commands::open_link,
            commands::save_attachment,
            commands::update_account_smtp,
            commands::test_smtp,
            commands::list_identities,
            commands::create_identity,
            commands::update_identity,
            commands::set_default_identity,
            commands::delete_identity,
            commands::prepare_draft,
            commands::new_draft,
            commands::list_drafts,
            commands::get_draft,
            commands::update_draft,
            commands::delete_draft,
            commands::add_draft_file,
            commands::remove_draft_attachment,
            commands::suggest_contacts,
            commands::list_contacts,
            commands::create_contact,
            commands::update_contact,
            commands::delete_contact,
            commands::rebuild_contacts,
            commands::get_send_delay,
            commands::set_send_delay,
            commands::send_draft,
            commands::cancel_send,
            commands::ai_draft,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri-App konnte nicht gestartet werden");
}
