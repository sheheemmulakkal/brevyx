//! Unified error types for ZenGuard.
//!
//! All fallible public functions return [`anyhow::Result`] at the call-site,
//! which provides automatic context chaining and coloured error reports.
//! This module additionally provides [`ZenGuardError`] — a
//! [`thiserror`]-derived enum — for cases where callers need to pattern-match
//! on the error kind (e.g. distinguishing a parse failure from an I/O error).

use thiserror::Error;

/// Domain-specific error variants for ZenGuard.
///
/// Use [`anyhow::Context`] to attach context before returning these from
/// library functions; surface them to users via the [`anyhow`] chain.
#[derive(Debug, Error)]
pub enum ZenGuardError {
    /// A problem reading, parsing, or writing the TOML configuration.
    #[error("configuration error: {0}")]
    Config(String),

    /// An underlying OS / file-system I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// TOML deserialization failure.
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// TOML serialization failure.
    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    /// A GTK4 / GLib error (e.g. CSS provider failure).
    #[error("GTK error: {0}")]
    Gtk(String),

    /// The reminder scheduler encountered an inconsistency.
    #[error("scheduler error: {0}")]
    Scheduler(String),

    /// An animation asset (SVG, CSS) could not be loaded.
    #[error("animation error: {0}")]
    Animation(String),

    /// The system-tray subsystem failed (libayatana-appindicator3 not
    /// available, or SNI watcher offline).
    #[error("tray error: {0}")]
    Tray(String),

    /// A daemon lifecycle operation failed.
    #[error("daemon error: {0}")]
    Daemon(String),
}
