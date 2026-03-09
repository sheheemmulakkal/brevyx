//! System-tray icon for Brevyx.
//!
//! # Backend
//! Uses the `appindicator3` crate, which wraps the Ayatana AppIndicator
//! library (`libayatana-appindicator3`), the standard Ubuntu/GNOME tray
//! protocol.
//!
//! Build requirement (only when the `tray` feature is enabled):
//! ```sh
//! sudo apt install libgtk-3-dev libayatana-appindicator3-dev
//! ```
//!
//! # Feature gate
//! The entire implementation is behind `#[cfg(feature = "tray")]`.  When the
//! feature is disabled every public function and type compiles to a no-op stub
//! so the rest of the codebase can call into this module unconditionally.
//!
//! # Thread model
//! The tray icon runs in a **dedicated OS thread** with its own GLib main
//! context and a GTK3 (`gtk`) main loop.  This keeps it isolated from the
//! GTK4 main thread.  Communication channels:
//!
//! - **Daemon → Tray**: `Arc<Mutex<TrayState>>` polled every second.
//! - **Tray → Daemon**: direct calls to [`PauseHandle`] (atomic) and an
//!   `on_quit` closure that schedules `app.quit()` via
//!   `glib::MainContext::default().invoke(...)` (cross-thread safe).
//!
//! # Menu structure
//! ```text
//! ● Brevyx — Active        (non-interactive label)
//! ──────────────────
//!   Pause                    (hidden when paused)
//!   Resume                   (hidden when running)
//! ──────────────────
//!   Settings  (coming soon)  (insensitive)
//! ──────────────────
//!   Quit
//! ```

use tracing::debug;

#[cfg(feature = "tray")]
use std::sync::{Arc, Mutex};

use crate::scheduler::PauseHandle;

// ── TrayState (shared between GTK main thread and tray thread) ────────────────

/// Runtime state mirrored into the tray thread for menu label sync.
#[derive(Debug, Default, Clone)]
#[cfg_attr(not(feature = "tray"), allow(dead_code))]
struct TrayState {
    paused: bool,
}

// ── TrayHandle ────────────────────────────────────────────────────────────────

/// Opaque handle for updating the tray icon state from the daemon.
///
/// Cheap to clone.  All methods are no-ops when the `tray` Cargo feature is
/// disabled, so callers need no conditional compilation.
pub struct TrayHandle {
    #[cfg(feature = "tray")]
    state: Arc<Mutex<TrayState>>,

    // Phantom unit field so the struct is always non-empty without the feature.
    #[cfg(not(feature = "tray"))]
    _private: (),
}

impl TrayHandle {
    /// Reflects a new paused/running state into the tray menu.
    ///
    /// The tray thread picks up the change within ~1 second (poll interval).
    pub fn set_paused(&self, paused: bool) {
        #[cfg(feature = "tray")]
        {
            debug!("TrayHandle::set_paused({})", paused);
            if let Ok(mut s) = self.state.lock() {
                s.paused = paused;
            }
        }

        #[cfg(not(feature = "tray"))]
        let _ = paused;
    }
}

// ── spawn_tray ────────────────────────────────────────────────────────────────

/// Spawns the tray subsystem and returns a [`TrayHandle`] for state updates.
///
/// When the `tray` feature is disabled this is a no-op that returns an inert
/// handle.
///
/// # Parameters
/// - `pause_handle` — used by the Pause/Resume menu items to toggle the
///   scheduler directly (no round-trip through the daemon needed).
/// - `on_quit`      — called on the tray thread when the user clicks Quit;
///   **must** schedule `app.quit()` on the GTK4 main thread (e.g. via
///   `glib::MainContext::default().invoke(...)`).
pub fn spawn_tray(pause_handle: PauseHandle, on_quit: impl Fn() + Send + 'static) -> TrayHandle {
    #[cfg(feature = "tray")]
    {
        spawn_tray_impl(pause_handle, on_quit)
    }

    #[cfg(not(feature = "tray"))]
    {
        let _ = (pause_handle, on_quit);
        debug!("Tray feature disabled — no system-tray icon");
        TrayHandle { _private: () }
    }
}

// ── Feature-gated implementation ─────────────────────────────────────────────

#[cfg(feature = "tray")]
fn spawn_tray_impl(pause_handle: PauseHandle, on_quit: impl Fn() + Send + 'static) -> TrayHandle {
    use appindicator3::prelude::AppIndicatorExt;
    use appindicator3::{Indicator, IndicatorCategory, IndicatorStatus};
    use glib::ControlFlow;
    use gtk::prelude::*;
    use std::time::Duration;

    let state = Arc::new(Mutex::new(TrayState::default()));
    let state_for_thread = Arc::clone(&state);
    let state_for_poll = Arc::clone(&state);

    if let Err(e) = std::thread::Builder::new()
        .name("brevyx-tray".into())
        .spawn(move || {
            // Initialise GTK3 on this dedicated thread.
            // The tray uses GTK3 menus (appindicator3 requirement).
            if gtk::init().is_err() {
                tracing::error!("GTK3 init failed in tray thread — no tray icon");
                return;
            }

            // ── AppIndicator ─────────────────────────────────────────────────
            let mut indicator = Indicator::new("brevyx", "", IndicatorCategory::ApplicationStatus);
            // Prefer a themed icon; fall back gracefully if absent.
            if let Some(data_dir) = dirs::data_local_dir() {
                let icon_dir = data_dir.join("brevyx");
                if let Some(path) = icon_dir.to_str() {
                    indicator.set_icon_theme_path(path);
                }
            }
            indicator.set_icon_full("brevyx", "Brevyx");
            indicator.set_status(IndicatorStatus::Active);

            // ── GTK3 menu ────────────────────────────────────────────────────
            let menu = gtk::Menu::new();

            // Non-interactive title label
            let title = gtk::MenuItem::with_label("Brevyx — Active");
            title.set_sensitive(false);
            menu.append(&title);
            menu.append(&gtk::SeparatorMenuItem::new());

            // Pause
            let pause_item = gtk::MenuItem::with_label("Pause");
            {
                let ph = pause_handle.clone();
                let st = Arc::clone(&state_for_thread);
                pause_item.connect_activate(move |_| {
                    ph.pause();
                    if let Ok(mut s) = st.lock() {
                        s.paused = true;
                    }
                    tracing::info!("Tray: daemon paused");
                });
            }
            menu.append(&pause_item);

            // Resume
            let resume_item = gtk::MenuItem::with_label("Resume");
            {
                let ph = pause_handle.clone();
                let st = Arc::clone(&state_for_thread);
                resume_item.connect_activate(move |_| {
                    ph.resume();
                    if let Ok(mut s) = st.lock() {
                        s.paused = false;
                    }
                    tracing::info!("Tray: daemon resumed");
                });
            }
            menu.append(&resume_item);

            menu.append(&gtk::SeparatorMenuItem::new());

            // Settings (placeholder)
            let settings = gtk::MenuItem::with_label("Settings (coming soon)");
            settings.set_sensitive(false);
            menu.append(&settings);

            menu.append(&gtk::SeparatorMenuItem::new());

            // Quit
            let quit_item = gtk::MenuItem::with_label("Quit");
            quit_item.connect_activate(move |_| {
                tracing::info!("Tray: quit selected");
                on_quit();
            });
            menu.append(&quit_item);

            menu.show_all();
            indicator.set_menu(Some(&menu));

            // ── Sync pause/resume visibility (1 s poll) ───────────────────────
            //
            // Reflects changes from daemon::pause() / resume() that go through
            // the shared TrayState rather than the menu's own click handlers.
            {
                let pi = pause_item.downgrade();
                let ri = resume_item.downgrade();
                glib::timeout_add_local(Duration::from_secs(1), move || {
                    let (Some(p), Some(r)) = (pi.upgrade(), ri.upgrade()) else {
                        return ControlFlow::Break;
                    };
                    if let Ok(s) = state_for_poll.lock() {
                        p.set_visible(!s.paused);
                        r.set_visible(s.paused);
                    }
                    ControlFlow::Continue
                });
            }

            // Run the GTK3 main loop for this thread.
            gtk::main();
            tracing::debug!("Tray thread exiting");
        })
    {
        tracing::error!("Failed to spawn tray thread: {e} — no system-tray icon");
    }

    TrayHandle { state }
}
