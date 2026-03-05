//! Full-screen overlay window controller for ZenGuard.
//!
//! # Responsibilities
//! - Owns the [`tokio::sync::mpsc`] receiver that the [`crate::scheduler`]
//!   pushes [`crate::scheduler::reminder::Reminder`] values onto.
//! - Polls the channel every 200 ms from the **GTK main thread** via
//!   [`glib::timeout_add_local`] (no `Send` requirement).
//! - Instantiates an [`window::OverlayWindow`] for each incoming reminder and
//!   presents it on screen.
//! - Enforces the **no-stack invariant**: at most one overlay is shown at a
//!   time.  Reminders that arrive while an overlay is active are silently
//!   dropped (logged at `debug`).
//! - Reads the live [`crate::config::OverlayConfig`] from a
//!   [`tokio::sync::watch`] receiver so that config hot-reloads take effect
//!   on the *next* overlay without restarting the controller.
//!
//! # Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Tokio task (scheduler)                                 │
//! │  tokio::sync::mpsc::Sender<Reminder>  ──────────────┐  │
//! └──────────────────────────────────────────────────────│──┘
//!                                                        │ channel
//! ┌──────────────────────────────────────────────────────▼──┐
//! │  GTK main thread                                        │
//! │  glib::timeout_add_local (200 ms poll)                  │
//! │    try_recv() → Some(reminder) → OverlayWindow::build() │
//! │                                → window.present()       │
//! │    on_closed callback ←── window destroyed              │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Thread safety
//!
//! [`OverlayController::start`] **must** be called from the GTK main thread.
//! The internal [`std::rc::Rc`] / [`std::cell::RefCell`] types are
//! intentionally `!Send`; they should never leave the GTK main thread.

pub mod animation;
pub mod window;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::config::ZenGuardConfig;
use crate::scheduler::reminder::Reminder;

// ── OverlayController ─────────────────────────────────────────────────────────

/// Receives reminders from the scheduler and presents them as full-screen
/// GTK4 overlay windows.
///
/// # Usage
/// ```no_run
/// use tokio::sync::{mpsc, watch};
/// use zenguard::config::ZenGuardConfig;
/// use zenguard::overlay::OverlayController;
///
/// let (tx, rx)       = mpsc::channel(8);
/// let (cfg_tx, cfg_rx) = watch::channel(ZenGuardConfig::default());
///
/// // Must be called from the GTK main thread (e.g. inside activate()):
/// OverlayController::new(rx, cfg_rx).start();
/// ```
pub struct OverlayController {
    reminder_rx: mpsc::Receiver<Reminder>,
    config_rx: watch::Receiver<ZenGuardConfig>,
}

impl OverlayController {
    /// Creates a new `OverlayController`.
    ///
    /// # Parameters
    /// - `reminder_rx` — receiving half of the scheduler's mpsc channel.
    /// - `config_rx`   — watch receiver for live config changes.  The overlay
    ///   config is snapshotted at the moment each reminder fires, so hot-reloads
    ///   take effect without restarting the controller.
    pub fn new(
        reminder_rx: mpsc::Receiver<Reminder>,
        config_rx: watch::Receiver<ZenGuardConfig>,
    ) -> Self {
        Self {
            reminder_rx,
            config_rx,
        }
    }

    /// Registers the GTK main-thread poll timer and transfers ownership of the
    /// controller into the closure.
    ///
    /// The timer fires every 200 ms and calls
    /// [`mpsc::Receiver::try_recv`] to check for pending reminders.  It stops
    /// automatically when the sender is dropped (channel disconnected).
    ///
    /// # Panics
    /// Must be called from the GTK main thread; panics otherwise (glib will
    /// reject the `timeout_add_local` call).
    pub fn start(self) {
        info!("OverlayController starting poll timer");

        // ── Shared state ─────────────────────────────────────────────────────
        // `active`  — true while an overlay window is visible.
        // `current` — holds the OverlayWindow so the GTK window stays alive.
        let active: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let current: Rc<RefCell<Option<window::OverlayWindow>>> = Rc::new(RefCell::new(None));

        // Wrap the receiver in Rc<RefCell> so the closure can hold it without
        // a Send requirement.  All access happens on the GTK main thread.
        let rx = Rc::new(RefCell::new(self.reminder_rx));
        let cfg_rx = self.config_rx;

        glib::timeout_add_local(Duration::from_millis(200), move || {
            let poll_result = rx.borrow_mut().try_recv();

            match poll_result {
                Ok(reminder) => {
                    if active.get() {
                        debug!(
                            id = %reminder.id,
                            "Overlay already visible — reminder dropped"
                        );
                        return ControlFlow::Continue;
                    }

                    // Snapshot the overlay config at fire-time so live
                    // hot-reloads are visible on the next reminder.
                    let overlay_cfg = cfg_rx.borrow().overlay.clone();

                    info!(id = %reminder.id, kind = %reminder.kind, "Showing overlay");
                    active.set(true);

                    // Clone handles for the on_closed callback.
                    let active_c = Rc::clone(&active);
                    let current_c = Rc::clone(&current);

                    let overlay =
                        window::OverlayWindow::build(&reminder, &overlay_cfg, move || {
                            debug!("Overlay closed — controller ready for next reminder");
                            active_c.set(false);
                            // Drop the OverlayWindow — releases GTK resources.
                            *current_c.borrow_mut() = None;
                        });

                    overlay.present();

                    // Store the overlay so the GTK window stays alive.
                    // This assignment happens before any GTK events are
                    // processed, so the window cannot close between present()
                    // and here.
                    *current.borrow_mut() = Some(overlay);
                }

                Err(TryRecvError::Empty) => {
                    // No reminder pending — nothing to do this tick.
                }

                Err(TryRecvError::Disconnected) => {
                    warn!("Reminder channel disconnected — overlay controller stopping");
                    return ControlFlow::Break;
                }
            }

            ControlFlow::Continue
        });
    }
}
