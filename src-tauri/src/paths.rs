use std::path::{Path, PathBuf};

use crate::error::Result;

/// Alle Ablageorte der App, relativ zum App-Datenverzeichnis des
/// Betriebssystems (kommt aus der Tauri-Pfad-API, nie hartkodiert).
#[derive(Debug, Clone)]
pub struct DataPaths {
    root: PathBuf,
}

impl DataPaths {
    /// Legt das Verzeichnis samt Unterordnern an, falls nötig.
    pub fn init(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(root.join("mail"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db_file(&self) -> PathBuf {
        self.root.join("archive.sqlite3")
    }

    /// Wandelt einen in der DB gespeicherten relativen Pfad in einen absoluten um.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}
