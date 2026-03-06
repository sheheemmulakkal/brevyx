//! CSS animation loader and switcher for the overlay eye graphic.
//!
//! Loads the appropriate [`gtk4::CssProvider`] for the configured
//! [`crate::config::AnimationStyle`]:
//!
//! | Style               | Source                                          |
//! |---------------------|-------------------------------------------------|
//! | [`AnimationStyle::BlinkEye`] | `assets/animations/blink.css` (bundled) |
//! | [`AnimationStyle::Breathe`]  | `assets/animations/breathe.css` (bundled) |
//! | [`AnimationStyle::Custom`]   | arbitrary path on disk                  |
//!
//! In all cases the `{{DURATION}}` placeholder inside the CSS source is
//! replaced with the actual overlay duration (in seconds) before the CSS
//! is handed to GTK4.  This keeps the animation cycle length in sync with
//! the overlay auto-dismiss timer.
//!
//! # Usage
//! ```no_run
//! use brevyx::config::AnimationStyle;
//! use brevyx::overlay::animation::{AnimationManager, load_animation_css};
//!
//! let css     = load_animation_css(&AnimationStyle::BlinkEye, 20);
//! let manager = AnimationManager::new(&css);
//!
//! // Apply to the default display (must be called from the GTK main thread):
//! if let Some(display) = gdk4::Display::default() {
//!     manager.apply(&display);
//! }
//! ```

use tracing::{debug, warn};

use crate::config::AnimationStyle;

// ── Bundled CSS assets ────────────────────────────────────────────────────────

const BLINK_CSS: &str = include_str!("../../assets/animations/blink.css");
const BREATHE_CSS: &str = include_str!("../../assets/animations/breathe.css");

/// The placeholder token inside each CSS template that is replaced with the
/// actual animation duration at load time.
const DURATION_PLACEHOLDER: &str = "{{DURATION}}";

// ── Public API ────────────────────────────────────────────────────────────────

/// Returns the CSS string for `style`, with the `{{DURATION}}` placeholder
/// replaced by `duration_secs` seconds.
///
/// For [`AnimationStyle::Custom`] the file is read from disk; if the read
/// fails the function falls back to the built-in blink animation and logs a
/// warning.
///
/// The returned string is ready to be passed directly to
/// [`gtk4::CssProvider::load_from_string`].
pub fn load_animation_css(style: &AnimationStyle, duration_secs: u32) -> String {
    let duration_value = format!("{}s", duration_secs);

    let template = match style {
        AnimationStyle::BlinkEye => BLINK_CSS.to_owned(),
        AnimationStyle::Breathe => BREATHE_CSS.to_owned(),
        AnimationStyle::Custom(path) => match std::fs::read_to_string(path) {
            Ok(css) => {
                debug!(path = %path, "Loaded custom animation CSS");
                css
            }
            Err(err) => {
                warn!(
                    path  = %path,
                    error = %err,
                    "Failed to read custom animation CSS; \
                     falling back to built-in blink"
                );
                BLINK_CSS.to_owned()
            }
        },
    };

    template.replace(DURATION_PLACEHOLDER, &duration_value)
}

// ── AnimationManager ──────────────────────────────────────────────────────────

/// Owns a [`gtk4::CssProvider`] loaded with the animation CSS and provides
/// methods to attach/detach it from the default GTK display.
///
/// # Lifecycle
/// 1. Construct with [`AnimationManager::new`] (loads the CSS into the provider).
/// 2. Call [`AnimationManager::apply`] once the GTK display is available.
/// 3. Call [`AnimationManager::remove`] when the overlay is torn down so the
///    animation CSS doesn't leak to future windows.
///
/// # Thread safety
/// All methods must be called from the **GTK main thread**.
pub struct AnimationManager {
    provider: gtk4::CssProvider,
}

impl AnimationManager {
    /// Creates a new `AnimationManager` and loads `css` into the provider.
    ///
    /// Does **not** apply the CSS to any display yet; call
    /// [`AnimationManager::apply`] when ready to show the animation.
    pub fn new(css: &str) -> Self {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(css);
        Self { provider }
    }

    /// Applies this provider to `display` at
    /// [`gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION`] priority.
    ///
    /// All widgets with the `.eye-animation` CSS class on this display will
    /// pick up the animation immediately.
    pub fn apply(&self, display: &gdk4::Display) {
        gtk4::style_context_add_provider_for_display(
            display,
            &self.provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        debug!("Animation CSS provider applied to display");
    }

    /// Removes this provider from `display`.
    ///
    /// Should be called when the overlay window is destroyed to avoid
    /// leaving stale CSS in the display's style cascade.
    pub fn remove(&self, display: &gdk4::Display) {
        gtk4::style_context_remove_provider_for_display(display, &self.provider);
        debug!("Animation CSS provider removed from display");
    }

    /// Returns a reference to the underlying [`gtk4::CssProvider`].
    pub fn provider(&self) -> &gtk4::CssProvider {
        &self.provider
    }
}
