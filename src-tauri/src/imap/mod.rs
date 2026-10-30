//! IMAP-Zugriff – ausschließlich lesend.
//!
//! Nach außen gibt es nur [`session::ReadOnlySession`]: Ordner werden mit
//! `EXAMINE` geöffnet, Mails mit `BODY.PEEK[]` geholt. Befehle, die den
//! Server verändern (SELECT, STORE, COPY, MOVE, EXPUNGE, APPEND, …), sind
//! über diese Schnittstelle nicht erreichbar.

pub mod connect;
pub mod mutf7;
pub mod session;
pub mod sync;
