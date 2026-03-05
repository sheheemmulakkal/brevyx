//! GTK4 application bootstrap for ZenGuard.
//!
//! # Responsibilities (Phase 2)
//! - Initialise the [`gtk4::Application`] with application ID
//!   `com.zenguard.ZenGuard`.
//! - On `activate`: wire up the [`crate::scheduler`], [`crate::overlay`],
//!   and [`crate::tray`] subsystems.
//! - Subscribe to the hot-reload [`tokio::sync::watch`] receiver from
//!   [`crate::config::watch_config`] and propagate live config changes.
//!
//! # Status
//! Stub — implemented in Phase 2.
