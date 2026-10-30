//! Mail-Verarbeitung unabhängig vom Transport: Parsen, Fingerprint,
//! Ablage als `.eml` und Übernahme ins Archiv.

pub mod body;
pub mod compose;
pub mod fingerprint;
pub mod html;
pub mod ingest;
pub mod outbox;
pub mod parse;
pub mod store;
pub mod thread;
