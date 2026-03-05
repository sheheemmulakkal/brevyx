//! Integration test: scheduler → reminder channel pipeline.
//!
//! These tests exercise the public API of the `zenguard` library crate
//! without requiring a running GTK4 display.  The overlay is represented by
//! the receiving end of the `tokio::sync::mpsc` channel — a "null sink" that
//! proves reminders are delivered at the correct times.
//!
//! All tests use `start_paused = true` for deterministic time control via
//! `tokio::time::advance`.

use std::time::Duration;

use tokio::sync::{mpsc, watch};
use zenguard::config::{ReminderConfig, ZenGuardConfig};
use zenguard::scheduler::reminder::{Reminder, ReminderKind};
use zenguard::scheduler::{PauseHandle, Scheduler};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Builds a [`ZenGuardConfig`] with a single enabled reminder at the given
/// interval, using an otherwise-default config.
fn single_reminder_config(id: &str, interval_minutes: u64) -> ZenGuardConfig {
    ZenGuardConfig {
        reminders: vec![ReminderConfig {
            id: id.to_owned(),
            label: id.to_owned(),
            message: format!("Integration test reminder: {id}"),
            interval_minutes,
            enabled: true,
            icon: None,
        }],
        ..Default::default()
    }
}

/// Spawns a [`Scheduler`] on the current Tokio runtime and returns the
/// reminder receiver and pause handle.
fn start_scheduler(
    cfg: ZenGuardConfig,
) -> (
    mpsc::Receiver<Reminder>,
    PauseHandle,
    watch::Sender<ZenGuardConfig>,
) {
    let (cfg_tx, cfg_rx) = watch::channel(cfg);
    let (tx, rx) = mpsc::channel::<Reminder>(16);

    let scheduler = Scheduler::new(cfg_rx, tx);
    let pause_handle = scheduler.pause_handle();
    tokio::spawn(scheduler.run());

    (rx, pause_handle, cfg_tx)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// The scheduler delivers a reminder through the mpsc channel at the correct
/// interval.  This is the simplest end-to-end proof that the
/// `Scheduler → Sender<Reminder> → Receiver<Reminder>` pipeline is intact.
#[tokio::test(start_paused = true)]
async fn reminder_delivered_after_interval() {
    let cfg = single_reminder_config("look_away", 5); // 5-minute interval
    let (mut rx, _pause, _cfg_tx) = start_scheduler(cfg);

    // Discard the first-tick no-op (interval fires immediately at t=0).
    tokio::time::advance(Duration::from_millis(1)).await;

    // Just before the deadline — nothing yet.
    tokio::time::advance(Duration::from_secs(5 * 60 - 1)).await;
    assert!(rx.try_recv().is_err(), "no reminder before 5 minutes");

    // Cross the 5-minute mark.
    tokio::time::advance(Duration::from_secs(2)).await;
    let r = rx.recv().await.expect("reminder should arrive");

    assert_eq!(r.id, "look_away");
    assert_eq!(r.kind, ReminderKind::LookAway);
    assert!(!r.message.is_empty());

    // Only one reminder, no burst.
    assert!(rx.try_recv().is_err(), "no burst after first reminder");
}

/// A paused scheduler does not deliver reminders even when intervals elapse.
/// When resumed, the NEXT natural tick delivers exactly one reminder.
#[tokio::test(start_paused = true)]
async fn pause_suppresses_and_resume_restores() {
    let cfg = single_reminder_config("drink_water", 5);
    let (mut rx, pause, _cfg_tx) = start_scheduler(cfg);

    tokio::time::advance(Duration::from_millis(1)).await;

    // Pause before the first fire.
    pause.pause();
    assert!(pause.is_paused());

    // Two full intervals pass — neither fires.
    tokio::time::advance(Duration::from_secs(10 * 60 + 1)).await;
    assert!(rx.try_recv().is_err(), "no reminder while paused");

    // Resume.  Next tick is at t ≈ 15 min.
    pause.resume();
    assert!(!pause.is_paused());

    tokio::time::advance(Duration::from_secs(5 * 60)).await;
    rx.recv().await.expect("reminder fires after resume");

    // No burst — exactly one.
    assert!(rx.try_recv().is_err(), "no burst after resume");
}

/// A config hot-reload replaces the interval without duplicate fires.
/// After the reload the new interval is respected and the old deadline
/// is cancelled.
#[tokio::test(start_paused = true)]
async fn config_hot_reload_changes_interval() {
    let (cfg_tx, cfg_rx) = watch::channel(single_reminder_config("take_walk", 10));
    let (tx, mut rx) = mpsc::channel::<Reminder>(16);
    tokio::spawn(Scheduler::new(cfg_rx, tx).run());

    tokio::time::advance(Duration::from_millis(1)).await;

    // 4 min into the 10-min cycle.
    tokio::time::advance(Duration::from_secs(4 * 60)).await;
    assert!(rx.try_recv().is_err(), "no fire at 4 min");

    // Hot-reload: switch to 5-min interval.
    cfg_tx.send(single_reminder_config("take_walk", 5)).unwrap();
    tokio::time::advance(Duration::from_millis(1)).await; // let scheduler restart

    // Old 10-min deadline is gone; new fire at ~4 min + 5 min = 9 min.
    tokio::time::advance(Duration::from_secs(4 * 60 + 59)).await;
    assert!(rx.try_recv().is_err(), "no fire before new 5-min interval");

    tokio::time::advance(Duration::from_secs(2)).await;
    let r = rx.recv().await.expect("fires at new 5-min interval");
    assert_eq!(r.id, "take_walk");

    assert!(rx.try_recv().is_err(), "no duplicate fire");
    drop(cfg_tx);
}

/// Multiple reminders in the config each fire on their own independent
/// interval.  The two deliveries are correctly separated.
#[tokio::test(start_paused = true)]
async fn multiple_reminders_fire_independently() {
    let cfg = ZenGuardConfig {
        reminders: vec![
            ReminderConfig {
                id: "fast".into(),
                label: "Fast".into(),
                message: "Fast reminder".into(),
                interval_minutes: 2,
                enabled: true,
                icon: None,
            },
            ReminderConfig {
                id: "slow".into(),
                label: "Slow".into(),
                message: "Slow reminder".into(),
                interval_minutes: 5,
                enabled: true,
                icon: None,
            },
        ],
        ..Default::default()
    };

    let (cfg_tx, cfg_rx) = watch::channel(cfg);
    let (tx, mut rx) = mpsc::channel::<Reminder>(16);
    tokio::spawn(Scheduler::new(cfg_rx, tx).run());

    // Allow the scheduler to initialise both interval tasks before advancing
    // virtual time so their timers are registered at t ≈ 0.
    tokio::time::advance(Duration::from_millis(1)).await;

    // Advance in 1-minute steps over 10 minutes.  After each step, yield once
    // so woken tasks get an executor turn to complete their `send()` and
    // re-register the next interval timer.  This is required because
    // MissedTickBehavior::Skip means a single large advance would fire each
    // task only once; small steps let each task chain its subsequent ticks.
    //
    // Expected fires over [0, 10 min]:
    //   "fast" (2-min interval) at ≈ 2, 4, 6, 8, 10 min  →  5 fires
    //   "slow" (5-min interval) at ≈ 5, 10 min            →  2 fires
    let mut fast_count = 0usize;
    let mut slow_count = 0usize;
    for _ in 0..10 {
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        while let Ok(r) = rx.try_recv() {
            match r.id.as_str() {
                "fast" => fast_count += 1,
                "slow" => slow_count += 1,
                other => panic!("unexpected reminder id: {other}"),
            }
        }
    }

    assert!(
        fast_count >= 4,
        "expected ≥ 4 'fast' reminders in 10 min, got {fast_count}"
    );
    assert_eq!(
        slow_count, 2,
        "expected exactly 2 'slow' reminders in 10 min, got {slow_count}"
    );

    drop(cfg_tx);
}

/// The `ZenGuardConfig::default()` overlay settings are the values documented
/// in the spec (20 s duration, allow_skip, skip_after = 5 s).  Verified here
/// so a future schema change is caught by the integration suite.
#[test]
fn default_overlay_config_matches_spec() {
    let cfg = ZenGuardConfig::default();
    let o = &cfg.overlay;

    assert_eq!(o.duration_seconds, 20, "default duration");
    assert!(o.allow_skip, "allow_skip default");
    assert_eq!(o.skip_after_seconds, 5, "skip_after default");
    assert!(
        (o.dim_opacity - 0.92).abs() < f64::EPSILON,
        "dim_opacity default"
    );
}

/// Disabled reminders are not delivered, even if their interval elapses.
#[tokio::test(start_paused = true)]
async fn disabled_reminder_is_never_delivered() {
    let cfg = ZenGuardConfig {
        reminders: vec![ReminderConfig {
            id: "disabled".into(),
            label: "Disabled".into(),
            message: "Should never fire".into(),
            interval_minutes: 1,
            enabled: false, // ← disabled
            icon: None,
        }],
        ..Default::default()
    };

    let (cfg_tx, cfg_rx) = watch::channel(cfg);
    let (tx, mut rx) = mpsc::channel::<Reminder>(8);
    tokio::spawn(Scheduler::new(cfg_rx, tx).run());

    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::time::advance(Duration::from_secs(5 * 60)).await;

    assert!(
        rx.try_recv().is_err(),
        "disabled reminder must never reach the channel"
    );

    drop(cfg_tx);
}
