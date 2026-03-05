//! System-tray icon for ZenGuard.
//!
//! # Backend
//! Uses the `appindicator3` Rust crate which links against
//! `libayatana-appindicator3` — the standard Ubuntu/GNOME system-tray
//! library.  Build requirement: `sudo apt install libayatana-appindicator3-dev`.
//!
//! # Responsibilities (Phase 5)
//! - Register a tray icon with the desktop environment via the
//!   AppIndicator / SNI protocol.
//! - Provide a context menu: Pause/Resume, Skip current reminder, Quit.
//! - Display "next reminder in Xm" in the tooltip.
//! - Forward menu actions to [`crate::daemon`] via a channel.
//!
//! # Status
//! Stub — implemented in Phase 5.
