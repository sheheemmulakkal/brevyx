//! Full-screen overlay window controller for Brevyx.
//!
//! # Responsibilities
//! - Owns the [`tokio::sync::mpsc`] receiver that the [`crate::scheduler`]
//!   pushes [`crate::scheduler::reminder::Reminder`] values onto.
//! - Polls the channel every 200 ms from the **GTK main thread** via
//!   [`glib::timeout_add_local`] (no `Send` requirement).
//! - Instantiates an [`window::OverlayWindow`] for each incoming reminder and
//!   presents it on screen.
//! - Shows overlays sequentially: at most one overlay is shown at a time.
//!   Reminders that arrive while an overlay is active are queued (FIFO) and
//!   shown in order after the current overlay closes.
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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::config::{BrevyxConfig, DisplayMode};
use crate::scheduler::reminder::Reminder;

// ── OverlayController ─────────────────────────────────────────────────────────

/// Receives reminders from the scheduler and presents them as full-screen
/// GTK4 overlay windows.
///
/// # Usage
/// ```no_run
/// use tokio::sync::{mpsc, watch};
/// use brevyx::config::BrevyxConfig;
/// use brevyx::overlay::OverlayController;
///
/// let (tx, rx)       = mpsc::channel(8);
/// let (cfg_tx, cfg_rx) = watch::channel(BrevyxConfig::default());
///
/// // Must be called from the GTK main thread (e.g. inside activate()):
/// OverlayController::new(rx, cfg_rx).start();
/// ```
pub struct OverlayController {
    reminder_rx: mpsc::Receiver<Reminder>,
    config_rx: watch::Receiver<BrevyxConfig>,
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
        config_rx: watch::Receiver<BrevyxConfig>,
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
        // `queue`   — reminders waiting to be shown (FIFO).
        // `current` — holds the active OverlayWindow so the GTK window stays alive.
        //             `None` means no overlay is currently on screen.
        let queue: Rc<RefCell<VecDeque<Reminder>>> = Rc::new(RefCell::new(VecDeque::new()));
        let current: Rc<RefCell<Option<window::OverlayWindow>>> = Rc::new(RefCell::new(None));

        // Wrap the receiver in Rc<RefCell> so the closure can hold it without
        // a Send requirement.  All access happens on the GTK main thread.
        let rx = Rc::new(RefCell::new(self.reminder_rx));
        let cfg_rx = self.config_rx;

        glib::timeout_add_local(Duration::from_millis(200), move || {
            // 1. Drain all newly-arrived reminders into the local queue.
            loop {
                match rx.borrow_mut().try_recv() {
                    Ok(reminder) => {
                        debug!(id = %reminder.id, "Queuing reminder");
                        queue.borrow_mut().push_back(reminder);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        warn!("Reminder channel disconnected — overlay controller stopping");
                        return ControlFlow::Break;
                    }
                }
            }

            // 2. If no overlay is showing, show the next item(s) from the queue.
            if current.borrow().is_none() {
                let cfg_snapshot = cfg_rx.borrow().overlay.clone();

                let overlay = match cfg_snapshot.display_mode {
                    // ── Sequential: show one reminder at a time ───────────────
                    DisplayMode::Sequential => {
                        let next = queue.borrow_mut().pop_front();
                        next.map(|reminder| {
                            let remaining = queue.borrow().len();
                            info!(
                                id = %reminder.id,
                                kind = %reminder.kind,
                                queued = remaining,
                                "Showing overlay (sequential)"
                            );
                            let current_c = Rc::clone(&current);
                            let ov = window::OverlayWindow::build(&reminder, &cfg_snapshot, move || {
                                debug!("Overlay closed — ready for next reminder");
                                *current_c.borrow_mut() = None;
                            });
                            ov
                        })
                    }

                    // ── Simultaneous: drain the whole queue into one overlay ──
                    DisplayMode::Simultaneous => {
                        if queue.borrow().is_empty() {
                            None
                        } else {
                            let batch: Vec<_> = queue.borrow_mut().drain(..).collect();
                            info!(
                                count = batch.len(),
                                "Showing overlay (simultaneous)"
                            );
                            let current_c = Rc::clone(&current);
                            let ov = window::OverlayWindow::build_multi(&batch, &cfg_snapshot, move || {
                                debug!("Multi-overlay closed — ready for next batch");
                                *current_c.borrow_mut() = None;
                            });
                            Some(ov)
                        }
                    }
                };

                if let Some(ov) = overlay {
                    ov.present();
                    *current.borrow_mut() = Some(ov);
                }
            }

            ControlFlow::Continue
        });
    }
}
