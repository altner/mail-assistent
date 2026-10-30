pub mod db;
pub mod error;
pub mod mail;
pub mod paths;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("Tauri-App konnte nicht gestartet werden");
}
