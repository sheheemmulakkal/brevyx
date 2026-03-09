//! System-tray icon for Brevyx.
//!
//! # Backend
//! Uses [`ksni`], which implements the **StatusNotifierItem** D-Bus protocol
//! directly — no GTK3 or system library headers required.  Works on both X11
//! and Wayland without conflicting with the GTK4 main thread.
//!
//! # Thread model
//! `ksni::run_in_background` spawns its own thread and owns the tray state.
//! The [`TrayHandle`] returned here wraps the ksni handle for state updates.
//!
//! # Menu structure
//! ```text
//! ● Brevyx                     (non-interactive label)
//! ──────────────────
//!   Pause / Resume
//! ──────────────────
//!   Quit
//! ```

use tracing::{debug, info};

use crate::scheduler::PauseHandle;

// ── BrevyxTray (ksni Tray impl) ───────────────────────────────────────────────

struct BrevyxTray {
    pause_handle: PauseHandle,
    quit_tx: std::sync::mpsc::SyncSender<()>,
}

impl ksni::Tray for BrevyxTray {
    fn icon_name(&self) -> String {
        // Use a standard freedesktop icon as fallback; themed "brevyx" icon
        // will be picked up automatically if installed to the icon theme path.
        "dialog-information".into()
    }

    fn title(&self) -> String {
        "Brevyx".into()
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let paused = self.pause_handle.is_paused();
        let toggle_label = if paused { "Resume" } else { "Pause" };

        vec![
            StandardItem {
                label: "Brevyx".into(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: toggle_label.into(),
                activate: Box::new(|tray: &mut BrevyxTray| {
                    if tray.pause_handle.is_paused() {
                        tray.pause_handle.resume();
                        info!("Tray: daemon resumed");
                    } else {
                        tray.pause_handle.pause();
                        info!("Tray: daemon paused");
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut BrevyxTray| {
                    info!("Tray: quit selected");
                    let _ = tray.quit_tx.send(());
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

// ── TrayHandle ────────────────────────────────────────────────────────────────

/// Opaque handle for interacting with the tray icon after it is spawned.
pub struct TrayHandle {
    _handle: Option<ksni::Handle<BrevyxTray>>,
}

impl TrayHandle {
    /// Notifies the tray that the pause state changed so the menu label
    /// refreshes on the next user interaction.
    ///
    /// With `ksni` the menu reads `pause_handle.is_paused()` directly, so
    /// no explicit state sync is required.  This method is kept for API
    /// compatibility with call sites in the daemon.
    pub fn set_paused(&self, paused: bool) {
        debug!("TrayHandle::set_paused({paused}) — menu reads live state");
    }
}

// ── spawn_tray ────────────────────────────────────────────────────────────────

/// Spawns the tray icon in a background thread and returns a [`TrayHandle`].
///
/// If no StatusNotifierHost is available (e.g. no compatible system tray
/// extension is running), the icon is silently absent — the daemon continues
/// normally.
///
/// # Parameters
/// - `pause_handle` — shared atomic flag; the Pause/Resume menu item toggles
///   it directly without a round-trip through the GTK4 main thread.
/// - `on_quit`      — called when the user clicks Quit. The caller is
///   responsible for routing this to `app.quit()` on the GTK4 main thread
///   (e.g. via the sync-channel + glib poll pattern in the daemon).
pub fn spawn_tray(pause_handle: PauseHandle, on_quit: impl Fn() + Send + 'static) -> TrayHandle {
    let (quit_tx, quit_rx) = std::sync::mpsc::sync_channel::<()>(1);

    // Forward quit signal from the ksni callback to the on_quit closure on a
    // tiny dedicated thread, keeping the ksni tray thread unblocked.
    std::thread::Builder::new()
        .name("brevyx-tray-quit".into())
        .spawn(move || {
            if quit_rx.recv().is_ok() {
                on_quit();
            }
        })
        .expect("failed to spawn tray-quit thread");

    let service = ksni::TrayService::new(BrevyxTray {
        pause_handle,
        quit_tx,
    });
    let handle = service.handle();
    // `spawn` runs the D-Bus service on a background thread.
    // If no StatusNotifierHost is present the icon is simply absent —
    // the thread keeps running silently until the process exits.
    service.spawn();
    info!("System tray icon active (StatusNotifierItem)");

    TrayHandle {
        _handle: Some(handle),
    }
}
