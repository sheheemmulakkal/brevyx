//! Daemon lifecycle management for ZenGuard.
//!
//! # Responsibilities (Phase 5)
//! - Expose `DaemonState` (Running / Paused / Stopping) to the tray and app.
//! - Coordinate pause / resume across the scheduler and overlay.
//! - Provide a clean shutdown path that drains in-flight GTK4 events before
//!   calling `app.quit()`.
//!
//! # Status
//! Stub — implemented in Phase 5.
