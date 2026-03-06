//! GTK4 full-screen overlay window builder.
//!
//! [`OverlayWindow`] constructs a borderless, full-screen [`gtk4::Window`]
//! containing:
//!
//! - The animated eye SVG (`assets/eye_blink.svg`) via [`gtk4::Picture`],
//!   with the `.eye-animation` CSS class so the animation CSS takes effect.
//! - The reminder icon (emoji from [`crate::config::ReminderConfig::icon`]),
//!   title (`label`), and message.
//! - A live countdown label updated every second via `glib::timeout_add_local`.
//! - An optional "Skip →" button that appears after
//!   [`crate::config::OverlayConfig::skip_after_seconds`] when
//!   [`crate::config::OverlayConfig::allow_skip`] is `true`.
//!
//! # CSS architecture
//!
//! Two [`gtk4::CssProvider`]s are applied to the default display when the
//! window is shown and removed when it is destroyed:
//!
//! 1. **Overlay CSS** — window background, typography, skip-button styling.
//!    The `{{DIM_OPACITY}}` placeholder is replaced with the config value.
//! 2. **Animation CSS** — the `.eye-animation` keyframe, sourced from
//!    [`super::animation`].
//!
//! # Thread safety
//!
//! All methods must be called from the **GTK main thread**.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk4::prelude::*;
use tracing::{debug, warn};

use super::animation::{load_animation_css, AnimationManager};
use crate::config::OverlayConfig;
use crate::scheduler::reminder::Reminder;

// ── Overlay CSS template ──────────────────────────────────────────────────────

/// Window / typography CSS.  `{{DIM_OPACITY}}` is replaced at build time.
const OVERLAY_CSS_TEMPLATE: &str = "
/* Transparent window surface — alpha compositing shows the dim overlay */
window.brevyx-window {
    background: none;
}

/* Main backdrop */
.brevyx-overlay {
    background-color: rgba(8, 8, 18, {{DIM_OPACITY}});
}

/* Reminder icon (emoji) */
.brevyx-icon {
    font-size: 52px;
    margin-bottom: 4px;
}

/* Reminder title */
.brevyx-title {
    font-size: 26px;
    font-weight: bold;
    color: #e0e0f2;
    margin-top: 18px;
    margin-bottom: 6px;
}

/* Reminder body message */
.brevyx-message {
    font-size: 17px;
    color: #8888b4;
    margin-top: 4px;
    margin-bottom: 4px;
}

/* Live countdown */
.brevyx-countdown {
    font-size: 13px;
    color: #406880;
    margin-top: 14px;
    letter-spacing: 2px;
}

/* Skip button — pill style, subtle */
.brevyx-skip {
    margin:           20px;
    padding:          9px 30px;
    border-radius:    22px;
    background-color: rgba(255, 255, 255, 0.07);
    color:            #6868a0;
    border:           1px solid rgba(255, 255, 255, 0.09);
    font-size:        13px;
}

.brevyx-skip:hover {
    background-color: rgba(255, 255, 255, 0.13);
    color:            #a0a0c8;
    border-color:     rgba(255, 255, 255, 0.16);
}
";

// ── OverlayWindow ─────────────────────────────────────────────────────────────

/// A full-screen GTK4 overlay for a single reminder display.
///
/// # Lifecycle
/// 1. Call [`OverlayWindow::build`] (creates the window off-screen).
/// 2. Call [`OverlayWindow::present`] to show it.
/// 3. The window auto-dismisses after [`OverlayConfig::duration_seconds`].
///    The `on_closed` callback fires when the window is fully destroyed.
///
/// Keeping the `OverlayWindow` value alive is necessary for the underlying
/// GTK widget to remain on screen.  Drop it (or allow the `on_closed`
/// callback to do so) to release all resources.
pub struct OverlayWindow {
    /// The GTK4 window.  Dropping this decrements the GObject refcount;
    /// keep the struct alive as long as the window should be visible.
    window: gtk4::Window,
    /// Kept alive so the overlay CSS provider persists until `connect_destroy`.
    _style_provider: Rc<gtk4::CssProvider>,
    /// Kept alive so the animation CSS provider persists until `connect_destroy`.
    _anim_manager: Rc<AnimationManager>,
}

impl OverlayWindow {
    /// Builds the overlay window and wires up all timers and event handlers.
    ///
    /// The window is **not** visible after this call; invoke
    /// [`OverlayWindow::present`] to show it.
    ///
    /// # Parameters
    /// - `reminder`    — the reminder to display.
    /// - `overlay_cfg` — appearance and timing settings.
    /// - `on_closed`   — called **once** when the window is fully destroyed,
    ///   regardless of whether it was auto-dismissed or skipped.
    pub fn build(
        reminder: &Reminder,
        overlay_cfg: &OverlayConfig,
        on_closed: impl Fn() + 'static,
    ) -> Self {
        // ── CSS providers ─────────────────────────────────────────────────────
        let overlay_css = OVERLAY_CSS_TEMPLATE.replace(
            "{{DIM_OPACITY}}",
            &format!("{:.2}", overlay_cfg.dim_opacity),
        );

        let anim_css = load_animation_css(
            &overlay_cfg.animation_style,
            overlay_cfg.duration_seconds as u32,
        );

        let style_provider = Rc::new(gtk4::CssProvider::new());
        style_provider.load_from_string(&overlay_css);

        let anim_manager = Rc::new(AnimationManager::new(&anim_css));

        if let Some(display) = gdk4::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &*style_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            anim_manager.apply(&display);
        }

        // ── Window ────────────────────────────────────────────────────────────
        let window = gtk4::Window::new();
        window.set_decorated(false);
        window.fullscreen();
        window.add_css_class("brevyx-window");

        // ── Root box (fills the window, carries the dim background) ───────────
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.add_css_class("brevyx-overlay");

        // ── Flexible top spacer ───────────────────────────────────────────────
        let top_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        top_spacer.set_vexpand(true);
        root.append(&top_spacer);

        // ── Centered content ──────────────────────────────────────────────────
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content.set_halign(gtk4::Align::Center);
        content.set_margin_start(48);
        content.set_margin_end(48);

        // Icon (emoji or default clock)
        let icon_text = reminder.config.icon.as_deref().unwrap_or("⏰");
        let icon_label = gtk4::Label::new(Some(icon_text));
        icon_label.add_css_class("brevyx-icon");
        icon_label.set_halign(gtk4::Align::Center);
        content.append(&icon_label);

        // Animated eye SVG
        let eye_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        eye_box.set_halign(gtk4::Align::Center);
        eye_box.set_margin_top(8);
        eye_box.set_margin_bottom(8);
        if let Some(picture) = build_eye_picture() {
            eye_box.append(&picture);
        }
        content.append(&eye_box);

        // Title
        let title_label = gtk4::Label::new(Some(&reminder.config.label));
        title_label.add_css_class("brevyx-title");
        title_label.set_halign(gtk4::Align::Center);
        content.append(&title_label);

        // Message
        let msg_label = gtk4::Label::new(Some(&reminder.message));
        msg_label.add_css_class("brevyx-message");
        msg_label.set_halign(gtk4::Align::Center);
        msg_label.set_wrap(true);
        msg_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        msg_label.set_max_width_chars(60);
        content.append(&msg_label);

        // Countdown
        let duration_secs = overlay_cfg.duration_seconds;
        let countdown = gtk4::Label::new(Some(&fmt_countdown(duration_secs)));
        countdown.add_css_class("brevyx-countdown");
        countdown.set_halign(gtk4::Align::Center);
        content.append(&countdown);

        root.append(&content);

        // ── Flexible bottom spacer ────────────────────────────────────────────
        let bot_spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        bot_spacer.set_vexpand(true);
        root.append(&bot_spacer);

        // ── Skip button ───────────────────────────────────────────────────────
        let skip_btn = gtk4::Button::with_label("Skip \u{2192}");
        skip_btn.add_css_class("brevyx-skip");
        skip_btn.set_halign(gtk4::Align::Center);
        skip_btn.set_visible(false);
        root.append(&skip_btn);

        window.set_child(Some(&root));

        // ── Countdown tick (1 s interval) ─────────────────────────────────────
        {
            let remaining = Rc::new(Cell::new(duration_secs));
            let cd_weak = countdown.downgrade();
            glib::timeout_add_local(Duration::from_secs(1), move || {
                let Some(label) = cd_weak.upgrade() else {
                    return ControlFlow::Break;
                };
                let r = remaining.get();
                if r == 0 {
                    return ControlFlow::Break;
                }
                let new_r = r.saturating_sub(1);
                remaining.set(new_r);
                label.set_text(&fmt_countdown(new_r));
                ControlFlow::Continue
            });
        }

        // ── Auto-dismiss timer ────────────────────────────────────────────────
        {
            let win_weak = window.downgrade();
            // Fire one second after the countdown reaches zero to ensure the
            // "0 SECONDS" state is briefly visible before the window closes.
            let dismiss_after = duration_secs.saturating_add(1);
            glib::timeout_add_local(Duration::from_secs(dismiss_after), move || {
                if let Some(w) = win_weak.upgrade() {
                    debug!("Auto-dismissing overlay ({}s elapsed)", duration_secs);
                    w.close();
                }
                ControlFlow::Break
            });
        }

        // ── Skip button visibility + click ────────────────────────────────────
        if overlay_cfg.allow_skip {
            // Show the Skip button after the configured delay.
            // Clamp to at least 100 ms so the button appears after the first
            // render frame even when skip_after_seconds is 0.
            let skip_after_ms = overlay_cfg.skip_after_seconds.saturating_mul(1000).max(100);
            let skip_weak = skip_btn.downgrade();
            glib::timeout_add_local(Duration::from_millis(skip_after_ms), move || {
                if let Some(btn) = skip_weak.upgrade() {
                    btn.set_visible(true);
                }
                ControlFlow::Break
            });

            // Close the window when the user clicks Skip.
            let win_weak = window.downgrade();
            skip_btn.connect_clicked(move |_| {
                if let Some(w) = win_weak.upgrade() {
                    debug!("User skipped overlay");
                    w.close();
                }
            });
        }

        // ── CSS cleanup + on_closed callback ─────────────────────────────────
        //
        // We hook into `close-request` rather than `destroy` because GTK4
        // windows hide (not destroy) by default when closed, which means
        // `connect_destroy` is unreliable for detecting overlay dismissal.
        //
        // Returning `Propagation::Stop` prevents GTK from doing its own
        // hide/destroy; we instead explicitly hide the window ourselves after
        // cleaning up CSS providers and notifying the overlay controller.
        //
        // Both providers are Rc-cloned into this closure so they stay alive
        // for the duration of the signal handler regardless of when the
        // OverlayWindow struct is dropped by the controller.
        let sp_clone = Rc::clone(&style_provider);
        let am_clone = Rc::clone(&anim_manager);
        window.connect_close_request(move |w| {
            if let Some(display) = gdk4::Display::default() {
                gtk4::style_context_remove_provider_for_display(&display, &*sp_clone);
                am_clone.remove(&display);
            }
            // Reset the controller's `active` flag and drop the OverlayWindow
            // before hiding so the next reminder can be shown immediately.
            on_closed();
            w.set_visible(false);
            glib::Propagation::Stop
        });

        Self {
            window,
            _style_provider: style_provider,
            _anim_manager: anim_manager,
        }
    }

    /// Makes the overlay window visible on screen.
    ///
    /// Must be called from the GTK main thread after [`OverlayWindow::build`].
    pub fn present(&self) {
        self.window.present();
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Loads the bundled eye SVG and returns a [`gtk4::Picture`] with the
/// `.eye-animation` CSS class.  Returns `None` if the texture cannot be
/// decoded (e.g. librsvg not installed), allowing the overlay to render
/// gracefully without the eye graphic.
fn build_eye_picture() -> Option<gtk4::Picture> {
    const SVG: &[u8] = include_bytes!("../../assets/eye_blink.svg");

    let bytes = glib::Bytes::from_static(SVG);
    let texture = gdk4::Texture::from_bytes(&bytes)
        .map_err(|e| warn!("Could not decode eye SVG: {e}"))
        .ok()?;

    let picture = gtk4::Picture::new();
    picture.set_paintable(Some(&texture));
    picture.set_can_shrink(true);
    picture.set_width_request(240);
    picture.set_height_request(120);
    picture.add_css_class("eye-animation");
    Some(picture)
}

/// Formats a countdown value as an uppercase seconds string suitable for
/// the countdown label (e.g. `"20 SECONDS"`, `"1 SECOND"`, `"0 SECONDS"`).
fn fmt_countdown(secs: u64) -> String {
    if secs == 1 {
        "1 SECOND".to_owned()
    } else {
        format!("{secs} SECONDS")
    }
}
