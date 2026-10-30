//! Mail-Verarbeitung unabhängig vom Transport: Parsen, Fingerprint,
//! Ablage als `.eml` und Übernahme ins Archiv.

pub mod fingerprint;
pub mod ingest;
pub mod parse;
pub mod store;
