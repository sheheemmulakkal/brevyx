//! Resolved reminder types produced by the scheduler.
//!
//! A [`Reminder`] is the fully-resolved, ready-to-display reminder derived
//! from a [`crate::config::ReminderConfig`].  It is the unit of communication
//! between the scheduler and the overlay subsystem.
//!
//! # Design patterns
//!
//! ## Command pattern
//! [`Reminder`] is a *command object*: it captures everything needed to
//! execute one reminder display (id, kind, message, full config snapshot)
//! without the receiver needing to consult any external state.  The
//! `tokio::sync::mpsc` channel acts as the command queue.
//!
//! ## Conversion traits
//! [`ReminderKind`] implements [`From<&str>`] and [`Reminder`] implements
//! [`From<&ReminderConfig>`], following the idiomatic Rust conversion-trait
//! pattern.  Both of the named constructors (`from_config`) simply delegate
//! to the blanket `From` impls for discoverability.

use std::fmt;

use crate::config::ReminderConfig;

// ── ReminderKind ───────────────────────────────────────────────────────────────

/// Semantic category for a reminder, inferred from the reminder's `id` field.
///
/// Built-in reminder types have dedicated variants so that downstream code
/// (e.g. the overlay) can apply type-specific icons or copy without fragile
/// string comparisons.  User-defined reminders fall under
/// [`ReminderKind::Custom`].
#[derive(Debug, Clone, PartialEq)]
pub enum ReminderKind {
    /// 20-20-20 eye-rest: look 20 feet away for 20 seconds.
    LookAway,

    /// Hydration reminder: drink a glass of water.
    DrinkWater,

    /// Movement reminder: stand up and walk around for a few minutes.
    TakeWalk,

    /// Longer break: step completely away from the desk.
    TakeBreak,

    /// User-defined reminder, carrying the config [`ReminderConfig::id`].
    Custom(String),
}

/// Converts a config `id` string to the appropriate [`ReminderKind`].
///
/// The four well-known built-in IDs are matched literally; everything else
/// becomes [`ReminderKind::Custom`].
///
/// # Example
/// ```
/// use zenguard::scheduler::reminder::ReminderKind;
/// assert_eq!(ReminderKind::from("look_away"), ReminderKind::LookAway);
/// assert_eq!(ReminderKind::from("unknown"),   ReminderKind::Custom("unknown".into()));
/// ```
impl From<&str> for ReminderKind {
    fn from(id: &str) -> Self {
        match id {
            "look_away"   => ReminderKind::LookAway,
            "drink_water" => ReminderKind::DrinkWater,
            "take_walk"   => ReminderKind::TakeWalk,
            "take_break"  => ReminderKind::TakeBreak,
            other         => ReminderKind::Custom(other.to_owned()),
        }
    }
}

impl fmt::Display for ReminderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReminderKind::LookAway      => write!(f, "Eye Rest"),
            ReminderKind::DrinkWater    => write!(f, "Drink Water"),
            ReminderKind::TakeWalk      => write!(f, "Take a Walk"),
            ReminderKind::TakeBreak     => write!(f, "Take a Break"),
            ReminderKind::Custom(label) => write!(f, "Reminder({})", label),
        }
    }
}

// ── Reminder ───────────────────────────────────────────────────────────────────

/// A fully-resolved, ready-to-display reminder — the *command object* produced
/// by the scheduler and consumed by the overlay subsystem.
///
/// Carries a complete snapshot of all presentation data so that the receiver
/// needs no further config lookups.
///
/// # Design
/// Implements the **Command** pattern: the scheduler creates `Reminder` values
/// and sends them through a `tokio::sync::mpsc` channel (the command queue).
/// The overlay (the *invoker*) dequeues and displays them independently of the
/// scheduler's timing logic.
#[derive(Debug, Clone)]
pub struct Reminder {
    /// Stable identifier copied from the originating [`ReminderConfig::id`].
    pub id: String,

    /// Semantic kind, inferred from [`Reminder::id`] at construction time.
    pub kind: ReminderKind,

    /// Body text for the full-screen overlay, copied from
    /// [`ReminderConfig::message`].
    pub message: String,

    /// Snapshot of the originating config entry.
    ///
    /// Carried along so the overlay can access `label`, `icon`, and other
    /// presentation details without a separate config lookup.
    pub config: ReminderConfig,
}

/// Constructs a [`Reminder`] from a [`ReminderConfig`] snapshot.
///
/// # Example
/// ```
/// use zenguard::config::ReminderConfig;
/// use zenguard::scheduler::reminder::{Reminder, ReminderKind};
///
/// let cfg = ReminderConfig { id: "look_away".into(), ..Default::default() };
/// let r   = Reminder::from(&cfg);
/// assert_eq!(r.kind, ReminderKind::LookAway);
/// ```
impl From<&ReminderConfig> for Reminder {
    fn from(config: &ReminderConfig) -> Self {
        Self {
            id:      config.id.clone(),
            kind:    ReminderKind::from(config.id.as_str()),
            message: config.message.clone(),
            config:  config.clone(),
        }
    }
}

impl Reminder {
    /// Constructs a `Reminder` from a [`ReminderConfig`] snapshot.
    ///
    /// Convenience alias for `Reminder::from(config)`.
    #[inline]
    pub fn from_config(config: &ReminderConfig) -> Self {
        Self::from(config)
    }
}

impl fmt::Display for Reminder {
    /// Formats the reminder as `[id] Kind: message` for logging.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.id, self.kind, self.message)
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReminderConfig;

    fn cfg(id: &str) -> ReminderConfig {
        ReminderConfig {
            id:      id.to_owned(),
            message: format!("Test message for {id}"),
            ..Default::default()
        }
    }

    // ── ReminderKind::from(&str) ───────────────────────────────────────────────

    #[test]
    fn builtin_ids_map_to_correct_kinds() {
        assert_eq!(ReminderKind::from("look_away"),   ReminderKind::LookAway);
        assert_eq!(ReminderKind::from("drink_water"), ReminderKind::DrinkWater);
        assert_eq!(ReminderKind::from("take_walk"),   ReminderKind::TakeWalk);
        assert_eq!(ReminderKind::from("take_break"),  ReminderKind::TakeBreak);
    }

    #[test]
    fn unknown_id_becomes_custom() {
        let kind = ReminderKind::from("my_custom_reminder");
        assert_eq!(kind, ReminderKind::Custom("my_custom_reminder".into()));
    }

    // ── Reminder::from / from_config ───────────────────────────────────────────

    #[test]
    fn from_config_populates_all_fields() {
        let config = ReminderConfig {
            id:      "look_away".into(),
            label:   "Eye Rest".into(),
            message: "Look away!".into(),
            ..Default::default()
        };
        let r = Reminder::from_config(&config);
        assert_eq!(r.id,      "look_away");
        assert_eq!(r.kind,    ReminderKind::LookAway);
        assert_eq!(r.message, "Look away!");
        assert_eq!(r.config,  config);
    }

    #[test]
    fn from_trait_and_from_config_are_identical() {
        let c  = cfg("drink_water");
        let r1 = Reminder::from(&c);
        let r2 = Reminder::from_config(&c);
        // Structural equality on the fields we care about (Reminder itself is
        // not PartialEq because ReminderConfig clone equality is sufficient).
        assert_eq!(r1.id,   r2.id);
        assert_eq!(r1.kind, r2.kind);
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn display_contains_id_kind_and_message() {
        let r = Reminder::from_config(&ReminderConfig {
            id:      "look_away".into(),
            message: "Look away!".into(),
            ..Default::default()
        });
        let s = r.to_string();
        assert!(s.contains("look_away"), "display should contain id");
        assert!(s.contains("Eye Rest"),  "display should contain kind");
        assert!(s.contains("Look away!"), "display should contain message");
    }

    #[test]
    fn reminder_kind_display_values() {
        assert_eq!(ReminderKind::LookAway.to_string(),              "Eye Rest");
        assert_eq!(ReminderKind::DrinkWater.to_string(),            "Drink Water");
        assert_eq!(ReminderKind::TakeWalk.to_string(),              "Take a Walk");
        assert_eq!(ReminderKind::TakeBreak.to_string(),             "Take a Break");
        assert_eq!(ReminderKind::Custom("x".into()).to_string(),    "Reminder(x)");
    }
}
