//! Configuration loading, persistence, and hot-reload for ZenGuard.
//!
//! # Overview
//!
//! ```text
//! disk                   load_config()           watch_config()
//!  ┌─────────────────────┐         ┌──────────────────────────────────┐
//!  │ config.toml         │──parse──▶  ZenGuardConfig (initial value)  │
//!  │ (or bundled default)│         │                                  │
//!  └─────────────────────┘         │  tokio::sync::watch::Receiver    │
//!          ▲  file-change           │  updated on every save           │
//!          │  (notify watcher)      └──────────────────────────────────┘
//!          └────────────────────────── background thread
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use zenguard::config;
//!
//! // One-shot load (synchronous):
//! let cfg = config::load_config()?;
//!
//! // Live hot-reload (returns a receiver updated on every file save):
//! let rx = config::watch_config(config::config_path(), cfg.clone())?;
//! // In an async context:
//! // let latest = rx.borrow().clone();
//! ```
//!
//! # File resolution
//!
//! `load_config` resolves the config file in this order:
//!
//! 1. `$XDG_CONFIG_HOME/zenguard/config.toml`
//!    (typically `~/.config/zenguard/config.toml`)
//! 2. If the file does not exist, the bundled `config/default_config.toml`
//!    is written to that path and parsed, giving the user a starting point.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

pub mod schema;
// Re-export the full schema surface so downstream modules (Phase 2+) can
// write `use crate::config::ZenGuardConfig` instead of
// `use crate::config::schema::ZenGuardConfig`.
#[allow(unused_imports)]
pub use schema::{AnimationStyle, GeneralConfig, OverlayConfig, ReminderConfig,
                 TrayConfig, ZenGuardConfig};

// ── Bundled default ────────────────────────────────────────────────────────────

/// The config file shipped inside the binary as a compile-time fallback.
///
/// Resolved relative to `src/config/mod.rs` → `../../config/default_config.toml`.
const BUNDLED_DEFAULT_TOML: &str =
    include_str!("../../config/default_config.toml");

// ── Path helpers ───────────────────────────────────────────────────────────────

/// Returns the canonical user config path:
/// `$XDG_CONFIG_HOME/zenguard/config.toml` (usually
/// `~/.config/zenguard/config.toml`).
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            // Rare fallback for systems without $HOME set
            PathBuf::from("/tmp")
        })
        .join("zenguard")
        .join("config.toml")
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Loads [`ZenGuardConfig`] from `config_path()`.
///
/// If the config file does not yet exist the bundled default is written there
/// and parsed, so the user always has an editable starting point.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or contains
/// invalid TOML.  A missing file is **not** an error — it falls back to
/// the bundled default.
///
/// # Example
///
/// ```no_run
/// let config = zenguard::config::load_config()?;
/// println!("log level: {}", config.general.log_level);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn load_config() -> Result<ZenGuardConfig> {
    load_from_path(&config_path())
}

/// Spawns a background file-system watcher that updates `initial` in-place
/// whenever `path` changes on disk.
///
/// Returns a [`watch::Receiver`] that always holds the latest
/// [`ZenGuardConfig`].  Clone the receiver to share it across threads or
/// tasks; call [`watch::Receiver::borrow`] (sync) or
/// [`watch::Receiver::changed`] (async) to consume updates.
///
/// The watcher runs in a dedicated `std::thread` and lives for the duration
/// of the process.  Dropping the returned receiver does **not** stop the
/// watcher (it continues updating silently).
///
/// # Errors
///
/// Returns an error only if the underlying `notify` watcher cannot be
/// created (e.g. inotify limit hit).
///
/// # Example
///
/// ```no_run
/// use zenguard::config;
///
/// let initial = config::load_config()?;
/// let rx = config::watch_config(config::config_path(), initial)?;
///
/// // Sync read of the current value:
/// let live = rx.borrow().clone();
///
/// // Async wait for the next change:
/// // rx.changed().await?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn watch_config(
    path: PathBuf,
    initial: ZenGuardConfig,
) -> Result<watch::Receiver<ZenGuardConfig>> {
    let (tx, rx) = watch::channel(initial);

    // Clone for the callback closure (path is also needed inside the thread)
    let path_for_cb  = path.clone();
    let path_for_dir = path.clone();

    // The notify callback runs on notify's internal inotify thread.
    // `watch::Sender::send` is synchronous — safe to call from any thread.
    let mut watcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            match res {
                Ok(event) if event.kind.is_modify() || event.kind.is_create() => {
                    debug!("config file event: {:?}", event.kind);
                    match load_from_path(&path_for_cb) {
                        Ok(cfg) => {
                            if tx.send(cfg).is_err() {
                                // All receivers dropped — nothing to do
                                debug!("config watch: all receivers dropped");
                            } else {
                                info!("config hot-reloaded");
                            }
                        }
                        Err(e) => warn!("config hot-reload failed: {e:#}"),
                    }
                }
                Ok(_)   => {} // remove / access / other events — ignore
                Err(e)  => error!("config watcher error: {e}"),
            }
        })
        .context("creating notify config watcher")?;

    // Watch the parent directory rather than the file directly so that
    // editor atomic-rename writes (e.g. Vim's :w) are detected correctly.
    let watch_dir = path_for_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .with_context(|| {
            format!("watching config directory {}", watch_dir.display())
        })?;

    // Spawn a thread whose sole job is to keep the watcher alive.
    // The watcher itself delivers events via its internal inotify thread;
    // this thread just holds the watcher object so it isn't dropped.
    std::thread::Builder::new()
        .name("zenguard-config-watcher".into())
        .spawn(move || {
            // Block forever — the OS will clean up when the process exits.
            let (_sentinel_tx, sentinel_rx) = std::sync::mpsc::channel::<()>();
            // sentinel_tx is kept alive here so sentinel_rx.recv() blocks
            // until this thread is killed.  Meanwhile `watcher` stays in
            // scope (and therefore alive).
            let _ = sentinel_rx.recv(); // blocks until process exit
            drop(watcher);
        })
        .context("spawning config watcher thread")?;

    debug!(dir = %watch_dir.display(), "config watcher active");
    Ok(rx)
}

// ── Internal helpers ───────────────────────────────────────────────────────────

/// Loads config from `path`, writing the bundled default there first if the
/// file is absent.
///
/// This function is `pub(crate)` so unit tests can call it with an arbitrary
/// path without going through `config_path()`.
pub(crate) fn load_from_path(path: &Path) -> Result<ZenGuardConfig> {
    if !path.exists() {
        info!(
            path = %path.display(),
            "config file absent — writing bundled defaults"
        );
        write_bundled_default(path)?;
        // Parse the bundled TOML directly rather than re-reading the file
        // we just wrote, to keep the cold-start path offline-safe.
        return toml::from_str(BUNDLED_DEFAULT_TOML)
            .context("parsing bundled default config");
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading config file {}", path.display()))?;

    toml::from_str(&raw)
        .with_context(|| format!("parsing TOML in {}", path.display()))
}

/// Writes `BUNDLED_DEFAULT_TOML` to `path`, creating parent directories as
/// needed.
fn write_bundled_default(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("creating config directory {}", parent.display())
        })?;
    }
    std::fs::write(path, BUNDLED_DEFAULT_TOML).with_context(|| {
        format!("writing default config to {}", path.display())
    })?;
    info!(path = %path.display(), "default config written");
    Ok(())
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── Test 1: Valid config loads correctly ───────────────────────────────────

    /// A fully-specified config file round-trips without loss or mutation.
    #[test]
    fn valid_config_loads_correctly() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");

        let toml = r#"
[general]
log_level = "debug"
autostart = false

[tray]
show_tray        = true
pause_on_startup = true

[overlay]
animation_style   = "breathe"
dim_opacity       = 0.80
duration_seconds  = 30
allow_skip        = false
skip_after_seconds = 0

[[reminders]]
id               = "my_reminder"
label            = "Focus Block"
message          = "Time to focus for 25 minutes."
interval_minutes = 25
enabled          = true
icon             = "🎯"

[[reminders]]
id               = "stretch"
label            = "Stretch"
message          = "Stand up and stretch."
interval_minutes = 50
enabled          = false
# icon omitted → None
"#;

        std::fs::write(&path, toml).unwrap();
        let cfg = load_from_path(&path).expect("load should succeed");

        // General
        assert_eq!(cfg.general.log_level, "debug");
        assert!(!cfg.general.autostart);

        // Tray
        assert!(cfg.tray.show_tray);
        assert!(cfg.tray.pause_on_startup);

        // Overlay
        assert_eq!(cfg.overlay.animation_style, AnimationStyle::Breathe);
        assert!((cfg.overlay.dim_opacity - 0.80).abs() < f64::EPSILON);
        assert_eq!(cfg.overlay.duration_seconds, 30);
        assert!(!cfg.overlay.allow_skip);
        assert_eq!(cfg.overlay.skip_after_seconds, 0);

        // Reminders
        assert_eq!(cfg.reminders.len(), 2);

        let r0 = &cfg.reminders[0];
        assert_eq!(r0.id, "my_reminder");
        assert_eq!(r0.label, "Focus Block");
        assert_eq!(r0.interval_minutes, 25);
        assert!(r0.enabled);
        assert_eq!(r0.icon.as_deref(), Some("🎯"));

        let r1 = &cfg.reminders[1];
        assert_eq!(r1.id, "stretch");
        assert!(!r1.enabled);
        assert!(r1.icon.is_none(), "omitted icon should deserialise as None");
    }

    /// The `Custom` animation variant round-trips correctly.
    #[test]
    fn custom_animation_style_round_trips() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");

        let toml = r#"
[overlay]
animation_style = { custom = "/home/user/blink.css" }
"#;
        std::fs::write(&path, toml).unwrap();
        let cfg = load_from_path(&path).expect("load");

        assert_eq!(
            cfg.overlay.animation_style,
            AnimationStyle::Custom("/home/user/blink.css".into()),
        );
    }

    // ── Test 2: Missing fields fall back to defaults ────────────────────────────

    /// A config file that specifies only a subset of fields gets the rest from
    /// the compiled-in `Default` implementations.
    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");

        // Only override one field per section; everything else is absent.
        let sparse = r#"
[general]
log_level = "warn"

[tray]
pause_on_startup = true
"#;
        std::fs::write(&path, sparse).unwrap();
        let cfg = load_from_path(&path).expect("sparse config should load");
        let def = ZenGuardConfig::default();

        // Overridden fields
        assert_eq!(cfg.general.log_level, "warn");
        assert!(cfg.tray.pause_on_startup);

        // Fields absent from the file should match the Default implementation
        assert_eq!(cfg.general.autostart,          def.general.autostart,
            "autostart should fall back to default");
        assert_eq!(cfg.tray.show_tray,             def.tray.show_tray,
            "show_tray should fall back to default");
        assert_eq!(cfg.overlay.dim_opacity,        def.overlay.dim_opacity,
            "dim_opacity should fall back to default");
        assert_eq!(cfg.overlay.duration_seconds,   def.overlay.duration_seconds,
            "duration_seconds should fall back to default");
        assert_eq!(cfg.overlay.allow_skip,         def.overlay.allow_skip,
            "allow_skip should fall back to default");
        assert_eq!(cfg.overlay.skip_after_seconds, def.overlay.skip_after_seconds,
            "skip_after_seconds should fall back to default");
        assert_eq!(cfg.overlay.animation_style,    def.overlay.animation_style,
            "animation_style should fall back to default");

        // Reminders absent from the file → full default set
        assert_eq!(
            cfg.reminders.len(),
            def.reminders.len(),
            "absent reminders section should yield the default reminders"
        );
    }

    /// An entirely empty config file deserialises to the full defaults.
    #[test]
    fn empty_config_file_is_all_defaults() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap(); // empty TOML is valid

        let cfg = load_from_path(&path).expect("empty config should load");
        assert_eq!(cfg, ZenGuardConfig::default());
    }

    // ── Test 3: Invalid TOML returns a descriptive error ───────────────────────

    /// Malformed TOML produces an `Err` whose message mentions the file path
    /// and the parse failure.
    #[test]
    fn invalid_toml_returns_descriptive_error() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");
        // Deliberately broken TOML
        std::fs::write(&path, "not valid toml [[[").unwrap();

        let result = load_from_path(&path);
        assert!(result.is_err(), "broken TOML should return Err");

        let msg = format!("{:#}", result.unwrap_err());
        // The error chain must mention the path so the user knows which file
        // is at fault.
        assert!(
            msg.contains("config.toml"),
            "error should mention the config file path; got:\n{msg}"
        );
    }

    /// A TOML file with a structurally valid TOML syntax but an unknown enum
    /// variant should produce a clear error message.
    #[test]
    fn unknown_animation_style_returns_error() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");
        let toml = r#"
[overlay]
animation_style = "does_not_exist"
"#;
        std::fs::write(&path, toml).unwrap();
        let result = load_from_path(&path);
        assert!(
            result.is_err(),
            "unknown animation_style variant should fail to parse"
        );
    }

    // ── Test 4: Absent file falls back to bundled default ─────────────────────

    /// When the config file does not exist, the bundled default is written to
    /// disk and returned.
    #[test]
    fn absent_file_writes_and_returns_bundled_default() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("config.toml");

        assert!(!path.exists(), "pre-condition: file must be absent");

        let cfg = load_from_path(&path).expect("absent file should load defaults");

        // The file should now have been created
        assert!(path.exists(), "default config should be written to disk");

        // Spot-check a few defaults that are defined in default_config.toml
        assert!(!cfg.reminders.is_empty(), "default reminders should be present");
        assert_eq!(cfg.overlay.duration_seconds, 20);
        assert!(cfg.overlay.allow_skip);
        assert_eq!(cfg.overlay.skip_after_seconds, 5);
    }

    // ── Test 5: Bundled TOML itself is valid ───────────────────────────────────

    /// Ensures `config/default_config.toml` (embedded at compile time) is
    /// always syntactically and structurally valid.
    #[test]
    fn bundled_default_toml_is_valid() {
        let result: Result<ZenGuardConfig, _> = toml::from_str(BUNDLED_DEFAULT_TOML);
        assert!(
            result.is_ok(),
            "bundled default_config.toml must always be valid: {:?}",
            result.err()
        );
        let cfg = result.unwrap();
        assert!(
            !cfg.reminders.is_empty(),
            "bundled config must define at least one reminder"
        );
    }
}
