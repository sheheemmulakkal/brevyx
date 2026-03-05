//! ZenGuard — production-grade Ubuntu wellness daemon.
//!
//! # Phase status
//! - **Phase 1** ✓ Config system (`src/config/`)
//! - Phase 2   — GTK4 application bootstrap (`src/app.rs`)
//! - Phase 3   — Reminder scheduler (`src/scheduler/`)
//! - Phase 4   — Full-screen overlay (`src/overlay/`)
//! - Phase 5   — System tray + daemon lifecycle (`src/tray/`, `src/daemon/`)
//!
//! # Entry point
//! Initialises the tracing subscriber, loads the config (triggering a
//! first-run default write if needed), and will delegate to [`app`] in
//! Phase 2.

mod app;
mod config;
mod daemon;
mod error;
mod overlay;
mod scheduler;
mod tray;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

fn main() -> Result<()> {
    // ── Logging ───────────────────────────────────────────────────────────────
    // Priority: RUST_LOG env var → config file log_level → "info" fallback.
    // We need to bootstrap with a default filter before the config is loaded.
    let bootstrap_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(bootstrap_filter)
        .with_target(false)
        .compact()
        .init();

    // ── Config (Phase 1) ──────────────────────────────────────────────────────
    let cfg = config::load_config()?;

    info!(
        version    = env!("CARGO_PKG_VERSION"),
        log_level  = %cfg.general.log_level,
        reminders  = cfg.reminders.len(),
        "ZenGuard starting — config loaded"
    );

    // ── Phase 2: GTK4 application bootstrap (not yet implemented) ────────────
    // app::build_and_run(cfg)?;

    Ok(())
}
