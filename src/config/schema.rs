//! Strongly-typed configuration schema for ZenGuard.
//!
//! All types derive [`serde::Serialize`] / [`serde::Deserialize`] and
//! implement [`Default`].  The `#[serde(default)]` attribute on every struct
//! ensures that fields missing from a user's config file are transparently
//! filled in from the corresponding `Default` implementation.
//!
//! # Separation of concerns
//! This module is intentionally dependency-light (only `serde`).  Future GUI
//! settings panels, CLI tools, or D-Bus interfaces can import it without
//! pulling in GTK4 or async symbols.
//!
//! # TOML representation
//! A complete example lives in `config/default_config.toml`.  Key
//! serialisation conventions:
//! - Enum variants use `snake_case` (`blink_eye`, `breathe`).
//! - The `Custom` animation variant serialises as an inline table:
//!   `animation_style = { custom = "/path/to/animation.css" }`.
//! - Reminders are an array of tables: `[[reminders]]`.

use serde::{Deserialize, Serialize};

// ── Root ──────────────────────────────────────────────────────────────────────

/// Root configuration object, deserialised from
/// `~/.config/zenguard/config.toml`.
///
/// Every field has a sane default so partial configs are fully supported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZenGuardConfig {
    /// General daemon behaviour (log level, autostart).
    pub general: GeneralConfig,

    /// System-tray icon settings.
    pub tray: TrayConfig,

    /// Overlay window appearance and behaviour.
    pub overlay: OverlayConfig,

    /// The list of active reminder definitions.
    ///
    /// If omitted from the config file the four built-in reminders
    /// (20-20-20 eye rest, water, walk, break) are used.
    pub reminders: Vec<ReminderConfig>,
}

impl Default for ZenGuardConfig {
    fn default() -> Self {
        Self {
            general:   GeneralConfig::default(),
            tray:      TrayConfig::default(),
            overlay:   OverlayConfig::default(),
            reminders: default_reminders(),
        }
    }
}

// ── General ───────────────────────────────────────────────────────────────────

/// General daemon-level settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Tracing log level passed to `RUST_LOG`.
    ///
    /// Accepted values: `trace`, `debug`, `info`, `warn`, `error`.
    /// Can be overridden at runtime with the `RUST_LOG` environment variable.
    pub log_level: String,

    /// Whether ZenGuard registers itself as a systemd user service on install.
    pub autostart: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            autostart: true,
        }
    }
}

// ── Tray ──────────────────────────────────────────────────────────────────────

/// System-tray icon configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrayConfig {
    /// Show a system-tray icon.
    ///
    /// Requires `libayatana-appindicator3` on the host system.
    pub show_tray: bool,

    /// Start the daemon in paused state (no reminders until manually resumed
    /// via the tray menu).
    pub pause_on_startup: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            show_tray:        true,
            pause_on_startup: false,
        }
    }
}

// ── Overlay ───────────────────────────────────────────────────────────────────

/// Full-screen overlay window settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfig {
    /// CSS animation style shown on the eye graphic.
    pub animation_style: AnimationStyle,

    /// Overlay background opacity (0.0 = fully transparent, 1.0 = opaque).
    ///
    /// Values between `0.85` and `0.95` work best on composited desktops.
    pub dim_opacity: f64,

    /// How long the overlay is displayed before auto-dismissing, in seconds.
    pub duration_seconds: u64,

    /// Whether the user may press "Skip" to close the overlay early.
    pub allow_skip: bool,

    /// Seconds into the countdown before the Skip button becomes visible.
    ///
    /// Has no effect when `allow_skip` is `false`.
    pub skip_after_seconds: u64,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            animation_style:   AnimationStyle::default(),
            dim_opacity:       0.92,
            duration_seconds:  20,
            allow_skip:        true,
            skip_after_seconds: 5,
        }
    }
}

// ── AnimationStyle ─────────────────────────────────────────────────────────────

/// The animation played on the eye graphic during an overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationStyle {
    /// Periodic eye-blink effect (`assets/animations/blink.css`).
    BlinkEye,

    /// Slow breathing / opacity pulse (`assets/animations/breathe.css`).
    Breathe,

    /// Load a custom CSS animation from the given file path.
    ///
    /// TOML example: `animation_style = { custom = "/home/user/my_anim.css" }`
    Custom(String),
}

impl Default for AnimationStyle {
    fn default() -> Self {
        AnimationStyle::BlinkEye
    }
}

// ── ReminderConfig ─────────────────────────────────────────────────────────────

/// Configuration for a single reminder.
///
/// Both built-in reminders (look-away, water, walk, break) and user-defined
/// custom reminders share this schema.
///
/// TOML example:
/// ```toml
/// [[reminders]]
/// id               = "look_away"
/// label            = "Eye Rest"
/// message          = "Look 20 feet away for 20 seconds."
/// interval_minutes = 20
/// enabled          = true
/// icon             = "👁"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderConfig {
    /// Stable machine-readable identifier, e.g. `"look_away"`.
    ///
    /// Used as a key when persisting scheduler state across restarts.
    pub id: String,

    /// Short human-readable name shown in the tray tooltip and overlay title.
    pub label: String,

    /// The body text shown on the full-screen overlay.
    pub message: String,

    /// How often to trigger this reminder, in minutes.
    pub interval_minutes: u64,

    /// Whether this reminder participates in scheduling.
    ///
    /// Disabled reminders are retained in the config so the user can
    /// re-enable them without having to retype the values.
    pub enabled: bool,

    /// Optional emoji character or absolute path to a PNG/SVG icon.
    ///
    /// Displayed above the reminder message on the overlay.  If `None`,
    /// a generic clock emoji is used.
    pub icon: Option<String>,
}

impl Default for ReminderConfig {
    fn default() -> Self {
        Self {
            id:               String::new(),
            label:            String::new(),
            message:          String::new(),
            interval_minutes: 20,
            enabled:          true,
            icon:             None,
        }
    }
}

// ── Built-in default reminders ─────────────────────────────────────────────────

/// Returns the four standard wellness reminders used when the user has not
/// provided a `[[reminders]]` section in their config file.
pub fn default_reminders() -> Vec<ReminderConfig> {
    vec![
        ReminderConfig {
            id:               "look_away".into(),
            label:            "Eye Rest (20-20-20)".into(),
            message:          "Look at something 20 feet away for 20 seconds \
                               to reduce eye strain."
                              .into(),
            interval_minutes: 20,
            enabled:          true,
            icon:             Some("👁".into()),
        },
        ReminderConfig {
            id:               "drink_water".into(),
            label:            "Drink Water".into(),
            message:          "Drink a glass of water. Staying hydrated \
                               keeps your mind sharp!"
                              .into(),
            interval_minutes: 45,
            enabled:          true,
            icon:             Some("💧".into()),
        },
        ReminderConfig {
            id:               "take_walk".into(),
            label:            "Take a Walk".into(),
            message:          "Stand up and walk around for a few minutes. \
                               Your body will thank you."
                              .into(),
            interval_minutes: 60,
            enabled:          true,
            icon:             Some("🚶".into()),
        },
        ReminderConfig {
            id:               "take_break".into(),
            label:            "Take a Break".into(),
            message:          "Step away from your desk for a proper break. \
                               Rest is productive."
                              .into(),
            interval_minutes: 90,
            enabled:          true,
            icon:             Some("🌿".into()),
        },
    ]
}
