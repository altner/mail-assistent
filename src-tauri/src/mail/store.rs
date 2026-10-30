use std::io::Write;
use std::path::Path;

use crate::error::Result;
use crate::paths::DataPaths;

/// Relativer Ablagepfad: `mail/<konto>/<fp[0..2]>/<fingerprint>.eml`.
pub fn eml_relative_path(account_id: i64, fingerprint: &str) -> String {
    let shard = fingerprint.get(..2).unwrap_or("00");
    format!("mail/{account_id}/{shard}/{fingerprint}.eml")
}

/// Schreibt die Rohmail atomar (temporäre Datei + Umbenennen). Existiert die
/// Datei schon, bleibt sie unverändert. Liefert den relativen Pfad.
pub fn store_eml(paths: &DataPaths, account_id: i64, fingerprint: &str, raw: &[u8]) -> Result<String> {
    let relative = eml_relative_path(account_id, fingerprint);
    let target = paths.resolve(&relative);
    if target.exists() {
        return Ok(relative);
    }
    let dir = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile_in(dir)?;
    tmp.1.write_all(raw)?;
    tmp.1.sync_all()?;
    std::fs::rename(&tmp.0, &target)?;
    Ok(relative)
}

fn tempfile_in(dir: &Path) -> Result<(std::path::PathBuf, std::fs::File)> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(format!(".tmp-{}-{nanos}", std::process::id()));
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    Ok((path, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_once_and_keeps_existing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::init(dir.path().to_path_buf()).unwrap();
        let fp = "ab".repeat(32);
        let rel = store_eml(&paths, 7, &fp, b"erste").unwrap();
        assert_eq!(rel, format!("mail/7/ab/{fp}.eml"));
        store_eml(&paths, 7, &fp, b"zweite").unwrap();
        assert_eq!(std::fs::read(paths.resolve(&rel)).unwrap(), b"erste");
        // keine Temp-Dateien übrig
        let leftovers = std::fs::read_dir(paths.resolve("mail/7/ab")).unwrap().count();
        assert_eq!(leftovers, 1);
    }
}
