//! CSS animation loader and switcher for the overlay eye graphic.
//!
//! Loads the appropriate GTK4 [`gtk4::CssProvider`] for the configured
//! [`crate::config::AnimationStyle`]:
//! - `BlinkEye` → `assets/animations/blink.css` (compiled in via
//!   `include_str!`, overridable from `$XDG_DATA_HOME/zenguard/animations/`)
//! - `Breathe`  → `assets/animations/breathe.css`
//! - `Custom`   → arbitrary path on disk
//!
//! # Status
//! Stub — implemented in Phase 4.
