//! Brevyx binary entry point.
//!
//! Parses CLI arguments, initialises the tracing subscriber, loads the
//! configuration, and delegates to [`brevyx::app::build_and_run`].

use std::path::PathBuf;

use anyhow::Result;
use tracing_subscriber::{fmt, EnvFilter};

fn main() -> Result<()> {
    // ── Display backend (tray feature only) ───────────────────────────────────
    //
    // When the `tray` feature is enabled, GTK3 (appindicator3) and GTK4 are
    // both loaded in the same process.  On Wayland they race to claim the
    // compositor connection and GTK4's gtk_init() fails with "GTK was not
    // actually initialized".  Forcing X11 (via XWayland) for both lets them
    // share the display connection without conflict.
    //
    // This must be set BEFORE any GTK or GDK code runs.
    #[cfg(feature = "tray")]
    if std::env::var("GDK_BACKEND").is_err() {
        std::env::set_var("GDK_BACKEND", "x11");
    }

    // ── CLI argument parsing ──────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();
    let mut config_path_override: Option<PathBuf> = None;
    let mut log_level_override: Option<String> = None;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("brevyx {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--config" | "-c" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    config_path_override = Some(PathBuf::from(p));
                }
            }
            "--log-level" | "-l" => {
                i += 1;
                if let Some(l) = args.get(i) {
                    log_level_override = Some(l.clone());
                }
            }
            other => {
                eprintln!("brevyx: unknown argument '{other}' — try --help");
            }
        }
        i += 1;
    }

    // ── Tracing subscriber ────────────────────────────────────────────────────
    //
    // Priority: --log-level flag → RUST_LOG env var → "info" fallback.
    let filter = match log_level_override.as_deref() {
        Some(level) => EnvFilter::new(level),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();

    // ── Configuration ─────────────────────────────────────────────────────────
    let cfg = match &config_path_override {
        Some(path) => brevyx::config::load_from_path(path)?,
        None => brevyx::config::load_config()?,
    };

    let config_path = config_path_override.unwrap_or_else(brevyx::config::config_path);

    // ── Run ───────────────────────────────────────────────────────────────────
    brevyx::app::build_and_run(cfg, config_path)
}

fn print_help() {
    println!(
        "Brevyx {ver} — wellness reminders daemon\n\n\
         Usage: brevyx [OPTIONS]\n\n\
         Options:\n  \
           -c, --config <PATH>        Use a custom config file\n  \
           -l, --log-level <LEVEL>    Set log level (trace|debug|info|warn|error)\n  \
           -V, --version              Print version and exit\n  \
           -h, --help                 Print this help",
        ver = env!("CARGO_PKG_VERSION"),
    );
}
