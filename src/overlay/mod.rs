//! Full-screen overlay window controller for ZenGuard.
//!
//! # Responsibilities (Phase 4)
//! - Accept a resolved [`crate::scheduler::reminder::Reminder`] and present
//!   a full-screen GTK4 overlay window.
//! - Enforce the no-stack invariant: at most one overlay is visible at a time.
//! - Auto-dismiss after `overlay.duration_seconds` from
//!   [`crate::config::OverlayConfig`].
//! - Conditionally show a "Skip" button after `skip_after_seconds`.
//! - Drive the CSS animation (blink / breathe / custom) on the eye graphic.
//!
//! # Status
//! Stub — implemented in Phase 4.

pub mod animation;
pub mod window;
