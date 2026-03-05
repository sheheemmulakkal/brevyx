//! GTK4 application bootstrap for ZenGuard.
//!
//! # Responsibilities
//! - Create the [`gtk4::Application`] with application ID `com.zenguard.app`.
//! - Build and manage a [`tokio::runtime::Runtime`] alongside the GTK4 event
//!   loop: Tokio runs on a background thread pool; GTK4 owns the main thread.
//! - Wire the `activate` signal to [`crate::daemon::Daemon::start`].
//! - Ensure the daemon is kept alive for the entire application lifetime and
//!   cleanly released on `shutdown`.
//!
//! # Thread model
//!
//! ```text
//! Main thread          Background thread pool (Tokio)
//! ───────────          ──────────────────────────────
//! gtk4::Application    Scheduler tasks
//! GTK4 event loop      Config watcher (notify thread)
//! OverlayController    Signal handling (tokio::signal)
//! Daemon (on main)
//! ```
//!
//! The [`tokio::runtime::Handle`] is passed into the GTK `activate` closure so
//! that subsystems like the scheduler can spawn async tasks without the main
//! thread being part of the Tokio thread pool.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result};
use gtk4::prelude::*;
use tracing::{error, info};

use crate::config::ZenGuardConfig;
use crate::daemon::Daemon;

/// Builds and runs the GTK4 application until the user quits.
///
/// This function blocks the calling thread until the GTK4 event loop exits.
/// It returns only after all subsystems have been cleanly stopped.
///
/// # Parameters
/// - `initial_cfg`  — first-boot configuration, already loaded from disk.
/// - `config_path`  — absolute path to the config file; forwarded to the
///   config hot-reload watcher.
///
/// # Errors
/// Returns an error if the Tokio runtime cannot be constructed.  All other
/// errors are logged and cause the application to request a clean exit.
pub fn build_and_run(initial_cfg: ZenGuardConfig, config_path: PathBuf) -> Result<()> {
    // ── Tokio runtime ─────────────────────────────────────────────────────────
    //
    // Built first so that its Handle can be cloned into the GTK activate
    // closure without the Runtime itself crossing thread boundaries.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building Tokio runtime")?;
    let rt_handle = rt.handle().clone();

    // ── GTK4 Application ──────────────────────────────────────────────────────
    let app = gtk4::Application::builder()
        .application_id("com.zenguard.app")
        .build();

    // Clone values captured by the activate closure.
    let cfg = initial_cfg;
    let path = config_path;

    app.connect_activate(move |gtk_app| {
        info!("GTK4 activate — wiring daemon subsystems");

        // Hold the application open indefinitely.
        //
        // GTK4 exits automatically when the last window closes.  ZenGuard is
        // a windowless daemon (overlays are short-lived and transient), so
        // without this guard the process would quit immediately after activate.
        // `hold()` returns an `ApplicationHoldGuard` (RAII) that calls
        // `g_application_release` when dropped.  We wrap it in
        // `Rc<RefCell<Option<_>>>` so the `Fn` shutdown closure can take and
        // drop it without requiring `FnOnce`.
        let hold_guard: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> =
            Rc::new(RefCell::new(Some(gtk_app.hold())));

        // `daemon_slot` keeps the Daemon alive for the application lifetime.
        // Using Rc<RefCell<...>> is safe because all access happens on the
        // GTK main thread; no Send requirement.
        let daemon_slot: Rc<RefCell<Option<Daemon>>> = Rc::new(RefCell::new(None));

        match Daemon::start(gtk_app, cfg.clone(), path.clone(), &rt_handle) {
            Ok(daemon) => {
                *daemon_slot.borrow_mut() = Some(daemon);
            }
            Err(e) => {
                error!("Daemon failed to start: {e:#}");
                *hold_guard.borrow_mut() = None; // release hold so the app can exit
                gtk_app.quit();
                return;
            }
        }

        // Drop the hold guard and daemon when the application shuts down.
        let slot_for_shutdown = Rc::clone(&daemon_slot);
        let hold_for_shutdown = Rc::clone(&hold_guard);
        gtk_app.connect_shutdown(move |_| {
            info!("GTK4 shutdown signal — releasing daemon");
            *slot_for_shutdown.borrow_mut() = None;
            *hold_for_shutdown.borrow_mut() = None; // drops ApplicationHoldGuard
        });
    });

    // Blocks until the GTK4 event loop exits (app.quit() or last window closed).
    app.run_with_args::<String>(&[]);

    // Drop the Tokio runtime.  This performs a blocking shutdown that waits
    // for all spawned tasks to complete or be cancelled.
    info!("GTK4 event loop exited — shutting down Tokio runtime");
    drop(rt);

    Ok(())
}
