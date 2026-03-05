//! GTK4 full-screen overlay window builder.
//!
//! Constructs a borderless, full-screen [`gtk4::Window`] containing:
//! - The animated eye SVG (`assets/eye_blink.svg`) via [`gtk4::Picture`].
//! - The reminder icon, title ([`crate::config::ReminderConfig::label`]),
//!   and message.
//! - A live countdown label updated every second.
//! - An optional "Skip" button that appears after
//!   [`crate::config::OverlayConfig::skip_after_seconds`].
//!
//! # Status
//! Stub — implemented in Phase 4.
