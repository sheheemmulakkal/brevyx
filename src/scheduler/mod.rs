//! Reminder scheduler for ZenGuard.
//!
//! Spawns one `tokio` interval task per enabled [`ReminderConfig`] entry and
//! fires [`Reminder`] values over a `tokio::sync::mpsc` channel when an
//! interval elapses and the daemon is not paused.
//!
//! On config hot-reload all tasks are gracefully cancelled and re-spawned with
//! the updated intervals, guaranteeing no duplicate fires and no missed
//! transitions.
//!
//! # Design patterns
//!
//! ## Observer (Reactive) pattern
//! The scheduler *observes* configuration changes through a
//! `tokio::sync::watch` channel (the *subject*).  When the config author
//! (file-watcher) publishes a new value, the scheduler (the *observer*)
//! automatically cancels its current tasks and restarts with fresh intervals —
//! without any polling or external coordination.
//!
//! ## Command pattern
//! [`Reminder`] values are *command objects* placed onto the mpsc channel.
//! The overlay subsystem (the *invoker*) dequeues them independently of the
//! scheduler's timing logic.  See [`reminder::Reminder`] for details.
//!
//! ## Handle Object pattern
//! [`PauseHandle`] is a lightweight, `Clone`-able handle that provides typed
//! pause/resume control over the scheduler after `run()` has consumed it.
//! Multiple handles can be distributed to independent components (tray menu,
//! D-Bus, tests) without coupling them to the `Scheduler` itself.
//!
//! # Usage
//! ```ignore
//! # use zenguard::scheduler::Scheduler;
//! # use tokio::sync::mpsc;
//! // config_rx: watch::Receiver<ZenGuardConfig> from config::watch_config()
//! // tx:        mpsc::Sender<Reminder> — give the rx half to the overlay
//! let scheduler = Scheduler::new(config_rx, tx);
//! let pause     = scheduler.pause_handle();     // grab before run() consumes self
//! tokio::spawn(scheduler.run());
//!
//! // From the tray thread, later:
//! pause.pause();
//! pause.resume();
//! ```

pub mod reminder;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

use crate::config::{ReminderConfig, ZenGuardConfig};
use reminder::Reminder;

// ── PauseHandle ────────────────────────────────────────────────────────────────

/// A cloneable, thread-safe handle for pausing and resuming the scheduler.
///
/// Obtained via [`Scheduler::pause_handle`] **before** [`Scheduler::run`]
/// consumes the scheduler.  Distribute as many clones as needed to
/// independent components (tray menu, D-Bus interface, tests).
///
/// # Design
/// Implements the **Handle Object** pattern: the handle provides safe, shared
/// access to a single `AtomicBool` across ownership boundaries without
/// exposing the implementation detail.  Uses `Ordering::Relaxed` because
/// pause/resume is a best-effort, latency-tolerant operation — there is no
/// need for happens-before guarantees between the flag write and the next
/// timer tick.
#[derive(Clone, Debug)]
pub struct PauseHandle(Arc<AtomicBool>);

impl PauseHandle {
    fn new(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    /// Pause reminder delivery.
    ///
    /// In-flight `tokio::time::interval` ticks continue, but no [`Reminder`]
    /// is sent to the channel until [`Self::resume`] is called.
    pub fn pause(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Resume reminder delivery.
    ///
    /// The next naturally-scheduled tick (at the next `interval_minutes`
    /// boundary) will fire a reminder.  Ticks that elapsed while paused are
    /// silently discarded via `MissedTickBehavior::Skip`.
    pub fn resume(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    /// Returns `true` if the scheduler is currently paused.
    pub fn is_paused(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

// ── Scheduler ─────────────────────────────────────────────────────────────────

/// Drives periodic reminder delivery based on the live configuration.
///
/// # Lifecycle
/// 1. Construct with [`Scheduler::new`].
/// 2. Optionally clone a [`PauseHandle`] via [`Scheduler::pause_handle`].
/// 3. Pass ownership to [`Scheduler::run`] and `tokio::spawn` the result.
///
/// [`Scheduler::run`] loops forever until the config watch channel closes
/// (i.e. the config sender is dropped at daemon shutdown).
pub struct Scheduler {
    /// Receives updated configs from the file-watcher (Observer pattern subject).
    config_rx: watch::Receiver<ZenGuardConfig>,
    /// Channel endpoint; cloned into each per-reminder task (Command pattern queue).
    sender: mpsc::Sender<Reminder>,
    /// Shared pause flag, also held by every distributed [`PauseHandle`].
    paused: Arc<AtomicBool>,
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    ///
    /// # Parameters
    /// - `config_rx` — watch receiver produced by
    ///   [`crate::config::watch_config`].  The scheduler restarts its tasks
    ///   automatically whenever a new value is published.
    /// - `sender` — sending half of a `tokio::sync::mpsc` channel; the caller
    ///   owns the receiving half and processes [`Reminder`] values as they
    ///   arrive.
    pub fn new(config_rx: watch::Receiver<ZenGuardConfig>, sender: mpsc::Sender<Reminder>) -> Self {
        Self {
            config_rx,
            sender,
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a [`PauseHandle`] that shares ownership of the internal pause
    /// flag.
    ///
    /// Call this **before** [`Scheduler::run`] because `run` consumes `self`.
    /// Distribute clones of the returned handle freely — each clone controls
    /// the same underlying flag.
    #[must_use]
    pub fn pause_handle(&self) -> PauseHandle {
        PauseHandle::new(Arc::clone(&self.paused))
    }

    /// Convenience: pause this scheduler directly (equivalent to
    /// `self.pause_handle().pause()`).
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    /// Convenience: resume this scheduler directly.
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    /// Returns `true` if the scheduler is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Runs the scheduler loop until the config watch channel closes.
    ///
    /// Meant to be passed to `tokio::spawn`:
    /// ```ignore
    /// tokio::spawn(scheduler.run());
    /// ```
    ///
    /// # Loop invariant
    /// On each iteration:
    /// 1. Read and mark-seen the current config via `borrow_and_update`.
    /// 2. Spawn one `tokio::time::interval` task per *enabled* reminder.
    /// 3. Block on `config_rx.changed()` (Observer pattern).
    /// 4. Abort all tasks, drain the `JoinSet`, and restart from step 1.
    ///
    /// `MissedTickBehavior::Skip` ensures that tasks aborted mid-interval do
    /// not burst-fire on restart.
    pub async fn run(mut self) {
        info!("Scheduler starting");

        loop {
            // Snapshot enabled reminders and mark the value as seen so that
            // `changed()` won't immediately fire again for the same value.
            let reminders: Vec<ReminderConfig> = {
                let cfg = self.config_rx.borrow_and_update();
                cfg.reminders
                    .iter()
                    .filter(|r| r.enabled)
                    .cloned()
                    .collect()
            };

            info!(count = reminders.len(), "Spawning reminder tasks");

            let mut set: JoinSet<()> = JoinSet::new();

            for cfg in reminders {
                let paused = Arc::clone(&self.paused);
                let sender = self.sender.clone();

                set.spawn(async move {
                    // Guard against zero-minute intervals.
                    let secs = cfg.interval_minutes.saturating_mul(60).max(1);
                    let period = Duration::from_secs(secs);

                    let mut interval = tokio::time::interval(period);
                    // Skip ticks that fire while the task is occupied (e.g.
                    // awaiting a slow mpsc send) to prevent burst-firing on
                    // resume or after restart.
                    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

                    // The first tick fires immediately at creation time.
                    // Discard it so reminders don't fire the instant the
                    // daemon starts.
                    interval.tick().await;

                    loop {
                        interval.tick().await;

                        if paused.load(Ordering::Relaxed) {
                            debug!(id = %cfg.id, "Reminder suppressed (paused)");
                            continue;
                        }

                        let reminder = Reminder::from_config(&cfg);
                        info!(
                            id   = %reminder.id,
                            kind = %reminder.kind,
                            "Firing reminder: {reminder}",
                        );

                        if sender.send(reminder).await.is_err() {
                            // Receiver dropped — daemon is shutting down.
                            debug!(id = %cfg.id, "mpsc receiver closed; exiting task");
                            break;
                        }
                    }
                });
            }

            // Block until the config changes (Observer) or the channel closes.
            match self.config_rx.changed().await {
                Ok(()) => {
                    info!("Config changed — restarting scheduler tasks");
                    set.abort_all();
                    // Drain JoinSet so aborted task handles are freed before
                    // the next iteration re-uses `set`.
                    while set.join_next().await.is_some() {}
                    // Loop to step 1 with the new config.
                }
                Err(_) => {
                    warn!("Config watch channel closed; scheduler exiting");
                    set.abort_all();
                    while set.join_next().await.is_some() {}
                    break;
                }
            }
        }

        info!("Scheduler stopped");
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReminderConfig, ZenGuardConfig};
    use tokio::sync::{mpsc, watch};

    // ── Test helpers ───────────────────────────────────────────────────────────

    /// Build a [`ZenGuardConfig`] with a single enabled reminder.
    fn one_reminder(id: &str, interval_minutes: u64) -> ZenGuardConfig {
        ZenGuardConfig {
            reminders: vec![ReminderConfig {
                id: id.to_owned(),
                label: id.to_owned(),
                message: format!("Reminder: {id}"),
                interval_minutes,
                enabled: true,
                icon: None,
            }],
            ..Default::default()
        }
    }

    // ── Test 1: fires after correct interval ───────────────────────────────────

    /// The scheduler sends exactly one [`Reminder`] after one full interval
    /// has elapsed.  Uses `start_paused = true` so time is fully deterministic.
    #[tokio::test(start_paused = true)]
    async fn fires_after_correct_interval() {
        let (watch_tx, watch_rx) = watch::channel(one_reminder("look_away", 5));
        let (tx, mut rx) = mpsc::channel::<Reminder>(8);

        tokio::spawn(Scheduler::new(watch_rx, tx).run());

        // Advance a tiny amount so the spawned task can initialise and discard
        // its immediate first tick.
        tokio::time::advance(Duration::from_millis(1)).await;

        // Just before the 5-minute mark — nothing should have arrived yet.
        tokio::time::advance(Duration::from_secs(5 * 60 - 1)).await;
        assert!(rx.try_recv().is_err(), "should not fire before 5 min");

        // Cross the threshold.  Use recv().await instead of try_recv() so the
        // test suspends and lets the spawned task complete its sender.send()
        // before we inspect the channel.
        tokio::time::advance(Duration::from_secs(1)).await;
        let reminder = rx.recv().await.expect("channel closed unexpectedly");
        assert_eq!(reminder.id, "look_away");
        assert_eq!(reminder.kind, reminder::ReminderKind::LookAway);

        // No second fire yet.
        assert!(rx.try_recv().is_err(), "should not double-fire");

        drop(watch_tx);
    }

    // ── Test 2: pausing stops firing ───────────────────────────────────────────

    /// While paused, reminders are suppressed even as intervals continue to
    /// tick internally.
    #[tokio::test(start_paused = true)]
    async fn pausing_stops_firing() {
        let (watch_tx, watch_rx) = watch::channel(one_reminder("eye_rest", 5));
        let (tx, mut rx) = mpsc::channel::<Reminder>(8);

        let scheduler = Scheduler::new(watch_rx, tx);
        let handle = scheduler.pause_handle();
        tokio::spawn(scheduler.run());

        tokio::time::advance(Duration::from_millis(1)).await;

        // Pause before the first natural fire.
        handle.pause();
        assert!(handle.is_paused());

        // Advance well past the interval.
        tokio::time::advance(Duration::from_secs(5 * 60 + 1)).await;
        assert!(
            rx.try_recv().is_err(),
            "no reminder should arrive while paused"
        );

        drop(watch_tx);
    }

    // ── Test 3: resuming restores firing ───────────────────────────────────────

    /// After pausing and resuming, the scheduler fires at the next naturally-
    /// scheduled tick.  Ticks missed while paused are discarded (no burst).
    #[tokio::test(start_paused = true)]
    async fn resuming_restores_firing() {
        let (watch_tx, watch_rx) = watch::channel(one_reminder("water", 5));
        let (tx, mut rx) = mpsc::channel::<Reminder>(8);

        let scheduler = Scheduler::new(watch_rx, tx);
        let handle = scheduler.pause_handle();
        tokio::spawn(scheduler.run());

        tokio::time::advance(Duration::from_millis(1)).await;

        // Pause before the first natural fire.
        handle.pause();

        // Let two full intervals elapse — both ticks fire internally but
        // MissedTickBehavior::Skip discards them; no reminder is sent.
        tokio::time::advance(Duration::from_secs(10 * 60 + 1)).await;
        assert!(rx.try_recv().is_err(), "still no reminder while paused");

        // Resume — the interval's next scheduled tick is at t ≈ 15 min.
        handle.resume();
        assert!(!handle.is_paused());

        // Advance to the next tick and confirm the reminder fires exactly once.
        tokio::time::advance(Duration::from_secs(5 * 60)).await;
        rx.recv()
            .await
            .expect("should fire at next tick after resume");

        // No burst — only one reminder.
        assert!(rx.try_recv().is_err(), "no burst after resume");

        drop(watch_tx);
    }

    // ── Test 4: config reload updates interval ─────────────────────────────────

    /// Sending a new config atomically replaces all timers: old intervals are
    /// cancelled and new ones start fresh with the updated period.
    /// No reminder fires at the old deadline after the reload.
    #[tokio::test(start_paused = true)]
    async fn config_reload_updates_interval_without_duplicate_fires() {
        // Start with a 10-minute interval.
        let (watch_tx, watch_rx) = watch::channel(one_reminder("eye_rest", 10));
        let (tx, mut rx) = mpsc::channel::<Reminder>(8);
        tokio::spawn(Scheduler::new(watch_rx, tx).run());

        // Let the scheduler initialise its tasks.
        tokio::time::advance(Duration::from_millis(1)).await;

        // 4 minutes in — old timer hasn't fired yet (fires at 10 min).
        tokio::time::advance(Duration::from_secs(4 * 60)).await;
        assert!(
            rx.try_recv().is_err(),
            "no fire at 4 min (old 10-min timer)"
        );

        // Reload config: switch to a 5-minute interval.
        watch_tx.send(one_reminder("eye_rest", 5)).unwrap();

        // Give the scheduler a moment to process the change, abort the old task,
        // and spawn a fresh task whose first tick fires immediately and is discarded.
        tokio::time::advance(Duration::from_millis(1)).await;

        // At this point the new 5-min timer has just started (at t ≈ 4 min).
        // The old 10-min deadline (t = 10 min) is gone.
        // New fire expected at t ≈ 4 min + 5 min = 9 min.

        // Advance to 4 min 59 s into the new interval — still before the new fire.
        tokio::time::advance(Duration::from_secs(4 * 60 + 59)).await;
        assert!(
            rx.try_recv().is_err(),
            "no fire before new 5-min interval elapses"
        );

        // Cross the new 5-minute threshold.
        tokio::time::advance(Duration::from_secs(2)).await;
        rx.recv()
            .await
            .expect("should fire at the new 5-min interval");

        // Confirm no duplicate or ghost fire from the old 10-min timer.
        assert!(rx.try_recv().is_err(), "no duplicate or ghost fire");

        drop(watch_tx);
    }
}
