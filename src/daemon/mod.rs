//! Daemon lifecycle management for Brevyx.
//!
//! The [`Daemon`] struct is the top-level orchestrator.  It owns or holds
//! handles to every subsystem and is constructed once inside the GTK4
//! `activate` signal handler.
//!
//! # Subsystem wiring
//!
//! ```text
//! Daemon::start()
//!   │
//!   ├─ config::watch_config()      → watch::Receiver<BrevyxConfig>
//!   │                                   ↓ (cloned into each consumer)
//!   ├─ Scheduler::new(cfg_rx, tx)  → tokio task (background thread pool)
//!   │       ↓ mpsc::Sender<Reminder>
//!   ├─ OverlayController::new(rx)  → glib timer on GTK main thread
//!   │
//!   ├─ tray::spawn_tray()          → dedicated thread (GTK3 main loop)
//!   │
//!   └─ glib::unix_signal_add_local (SIGTERM / SIGINT)
//! ```
//!
//! # State transitions
//!
//! ```text
//! Running ──pause()──▶ Paused ──resume()──▶ Running
//!     │                  │
//!     └──stop()──────────┘──────────────────▶ Stopped
//! ```
//!
//! # Thread safety
//! [`Daemon`] is intentionally `!Sync`.  All methods must be called from the
//! **GTK main thread**.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use glib::ControlFlow;
use gtk4::prelude::*;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{self, BrevyxConfig};
use crate::overlay::OverlayController;
use crate::scheduler::reminder::Reminder;
use crate::scheduler::{PauseHandle, Scheduler};
use crate::tray;

/// The buffer depth of the scheduler → overlay reminder channel.
///
/// Excess reminders are dropped by the overlay controller (one-at-a-time
/// invariant), so a small buffer is sufficient.
const REMINDER_CHANNEL_DEPTH: usize = 8;

// ── DaemonState ───────────────────────────────────────────────────────────────

/// Current operational state of the Brevyx daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// Scheduler is active; reminders fire on schedule.
    Running,
    /// Scheduler ticks continue but no reminders are delivered.
    Paused,
    /// Daemon has been stopped; no further state transitions are valid.
    Stopped,
}

// ── Daemon ────────────────────────────────────────────────────────────────────

/// Top-level daemon orchestrator.
///
/// Holds handles to all subsystems and drives state transitions.
/// Constructed via [`Daemon::start`]; kept alive by the GTK application.
///
/// # Drop behaviour
/// Dropping the `Daemon` aborts the scheduler task and releases all channels,
/// which causes the overlay controller's glib poll timer to stop at the next
/// tick (channel disconnect).
pub struct Daemon {
    state: Cell<DaemonState>,
    pause_handle: PauseHandle,
    tray_handle: tray::TrayHandle,

    // ── Kept alive for the daemon's lifetime ─────────────────────────────────
    /// Live config receiver — closing this would stop the scheduler.
    _config_rx: watch::Receiver<BrevyxConfig>,
    /// Scheduler background task — aborted on drop / stop.
    sched_task: JoinHandle<()>,
}

impl Daemon {
    /// Initialises all subsystems and returns a running [`Daemon`].
    ///
    /// Must be called from the **GTK main thread** because it registers glib
    /// timeout sources and signal handlers.
    ///
    /// # Parameters
    /// - `app`         — GTK4 application; `app.quit()` is called on SIGTERM/SIGINT.
    /// - `cfg`         — initial configuration snapshot.
    /// - `config_path` — path forwarded to the config file-watcher.
    /// - `rt`          — Tokio runtime handle; the scheduler is spawned here.
    pub fn start(
        app: &gtk4::Application,
        cfg: BrevyxConfig,
        config_path: PathBuf,
        rt: &tokio::runtime::Handle,
    ) -> Result<Self> {
        info!(version = env!("CARGO_PKG_VERSION"), "Daemon starting");

        // ── Config hot-reload watcher ─────────────────────────────────────────
        let config_rx = config::watch_config(config_path, cfg.clone())
            .context("starting config file watcher")?;

        // ── Reminder channel (scheduler → overlay) ────────────────────────────
        let (reminder_tx, reminder_rx) = mpsc::channel::<Reminder>(REMINDER_CHANNEL_DEPTH);

        // ── Scheduler ────────────────────────────────────────────────────────
        let scheduler = Scheduler::new(config_rx.clone(), reminder_tx);
        let pause_handle = scheduler.pause_handle();
        let sched_task = rt.spawn(scheduler.run());

        // ── Overlay controller (GTK main thread) ──────────────────────────────
        OverlayController::new(reminder_rx, config_rx.clone()).start();

        // ── System tray ───────────────────────────────────────────────────────
        //
        // `gtk4::Application` is !Send, so it cannot be captured in the tray
        // thread's `on_quit` closure.  Instead we relay the signal through a
        // sync channel and poll it on the GTK main thread via a glib timer.
        let (quit_tx, quit_rx) = std::sync::mpsc::channel::<()>();
        let tray_handle = tray::spawn_tray(pause_handle.clone(), move || {
            let _ = quit_tx.send(());
        });

        // ── Tray quit-signal poll ────────────────────────────────────────────
        //
        // Poll the sync channel every 200 ms on the GTK main thread.  When
        // the tray "Quit" item is clicked, this timer calls app.quit().
        {
            let app_for_quit = app.clone();
            glib::timeout_add_local(Duration::from_millis(200), move || {
                match quit_rx.try_recv() {
                    Ok(()) => {
                        info!("Tray requested quit — shutting down");
                        app_for_quit.quit();
                        ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => ControlFlow::Break,
                }
            });
        }

        // ── Initial pause state ───────────────────────────────────────────────
        if cfg.tray.pause_on_startup {
            pause_handle.pause();
            tray_handle.set_paused(true);
            info!("Daemon started in paused state (pause_on_startup = true)");
        }

        // ── POSIX signal handlers ─────────────────────────────────────────────
        //
        // `glib::unix_signal_add_local` integrates with the GLib event loop so
        // the callback runs safely on the GTK main thread.
        //
        //   SIGTERM = 15 (graceful stop requested by systemd / kill)
        //   SIGINT  =  2 (Ctrl-C in terminal)
        let app1 = app.clone();
        glib::unix_signal_add_local(libc_signum::SIGTERM, move || {
            info!("SIGTERM received — requesting application quit");
            app1.quit();
            ControlFlow::Break
        });
        let app2 = app.clone();
        glib::unix_signal_add_local(libc_signum::SIGINT, move || {
            info!("SIGINT received — requesting application quit");
            app2.quit();
            ControlFlow::Break
        });

        info!("Daemon started — all subsystems active");

        Ok(Self {
            state: Cell::new(DaemonState::Running),
            pause_handle,
            tray_handle,
            _config_rx: config_rx,
            sched_task,
        })
    }

    /// Pauses reminder delivery.
    ///
    /// The scheduler continues ticking internally but suppresses sends.
    /// Idempotent if already paused.
    pub fn pause(&self) {
        if self.state.get() == DaemonState::Stopped {
            warn!("pause() called on stopped daemon — ignoring");
            return;
        }
        self.state.set(DaemonState::Paused);
        self.pause_handle.pause();
        self.tray_handle.set_paused(true);
        info!("Daemon paused");
    }

    /// Resumes reminder delivery.
    ///
    /// The next naturally-scheduled interval tick will fire a reminder.
    /// Idempotent if already running.
    pub fn resume(&self) {
        if self.state.get() == DaemonState::Stopped {
            warn!("resume() called on stopped daemon — ignoring");
            return;
        }
        self.state.set(DaemonState::Running);
        self.pause_handle.resume();
        self.tray_handle.set_paused(false);
        info!("Daemon resumed");
    }

    /// Stops the daemon and requests the GTK application to quit.
    ///
    /// Aborts the scheduler task so in-flight reminders are discarded.
    /// After this call, the `Daemon` value should be dropped.
    pub fn stop(&self, app: &gtk4::Application) {
        if self.state.get() == DaemonState::Stopped {
            return;
        }
        self.state.set(DaemonState::Stopped);
        self.sched_task.abort();
        info!("Daemon stopped — requesting GTK quit");
        app.quit();
    }

    /// Returns the current [`DaemonState`].
    pub fn state(&self) -> DaemonState {
        self.state.get()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Abort the scheduler task so it doesn't linger after the daemon is
        // released (e.g. during application shutdown).
        self.sched_task.abort();
        info!("Daemon dropped — scheduler task aborted");
    }
}

// ── Signal number constants ───────────────────────────────────────────────────
//
// Using literal values avoids a `libc` dependency.  These are stable POSIX
// values on Linux (and all platforms we target).
mod libc_signum {
    pub const SIGINT: i32 = 2;
    pub const SIGTERM: i32 = 15;
}

// ── Periodic tray-state refresh ───────────────────────────────────────────────
//
// The tray thread caches pause state in an Arc<Mutex<TrayState>>.  This timer
// is a safety net that keeps the tray in sync even if a race condition causes
// a missed update.  Only registered when the tray feature is enabled.
#[cfg(feature = "tray")]
pub(crate) fn register_tray_sync_timer(pause_handle: PauseHandle, tray_handle: tray::TrayHandle) {
    glib::timeout_add_local(Duration::from_secs(5), move || {
        tray_handle.set_paused(pause_handle.is_paused());
        ControlFlow::Continue
    });
}
