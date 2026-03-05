# ZenGuard

A production-grade Ubuntu wellness daemon with animated GTK4 reminders.
Inspired by [LookAway](https://www.lookaway.app/) (macOS) — now for Linux.

ZenGuard runs silently as a systemd user service and shows full-screen animated
overlay popups at configurable intervals, reminding you to rest your eyes, drink
water, move, and take breaks.

---

## Screenshot

> *Screenshot placeholder — replace with an actual screenshot after first boot.*
>
> `assets/screenshots/overlay.png`

---

## Features

- **20-20-20 rule** — eye-rest overlay every 20 minutes
- **Hydration, movement, break** reminders on independent schedules
- **Animated eye SVG** with two built-in CSS animations (blink / breathe) plus
  support for fully custom animations
- **Hot-reload config** — edit `~/.config/zenguard/config.toml` and changes
  take effect within seconds; no restart needed
- **System-tray icon** (Pause / Resume / Quit) via Ayatana AppIndicator
- **Skip button** — configurable delay before it appears (or disable entirely
  for no-skip mode)
- **systemd user service** with automatic restart on failure
- Zero `unwrap()` in production paths; structured logging via `tracing`

---

## Installation

### Prerequisites

```bash
# GTK4 development headers + SVG loader (required)
sudo apt install libgtk-4-dev pkg-config build-essential librsvg2-common

# For the system-tray icon (optional — enables --features tray)
sudo apt install libgtk-3-dev libayatana-appindicator3-dev
```

Rust toolchain: install from <https://rustup.rs> if not already present.

### Install

```bash
git clone https://github.com/yourname/zenguard.git
cd zenguard
chmod +x install.sh
./install.sh
```

The script will:

1. Build the release binary (`cargo build --release`)
2. Install the binary to `~/.local/bin/zenguard`
3. Copy assets to `~/.local/share/zenguard/`
4. Write the default config to `~/.config/zenguard/config.toml` (if absent)
5. Install and enable the systemd user service (`zenguard.service`)

To install without the systemd service (e.g. for manual launches):

```bash
./install.sh --no-service
```

### Uninstall

```bash
./uninstall.sh                # removes binary, assets, service; keeps config
./uninstall.sh --purge-config # also removes ~/.config/zenguard/
```

---

## Running manually

```bash
zenguard                        # uses ~/.config/zenguard/config.toml
zenguard --config /path/to.toml # custom config file
zenguard --log-level debug      # verbose logging
zenguard --help
zenguard --version
```

---

## Service management

```bash
systemctl --user status  zenguard   # show current state
systemctl --user stop    zenguard   # stop until next login
systemctl --user start   zenguard   # start
systemctl --user restart zenguard   # restart after config change

journalctl --user -u zenguard -f    # follow live logs
journalctl --user -u zenguard -n 50 # last 50 log lines
```

---

## Configuration reference

Config file: `~/.config/zenguard/config.toml`

ZenGuard writes this file on first run if it does not exist.
Changes are picked up automatically (inotify hot-reload, ~1 s latency).

### `[general]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `log_level` | string | `"info"` | Tracing log level: `trace` \| `debug` \| `info` \| `warn` \| `error`. Can also be set via `RUST_LOG` env var. |
| `autostart` | bool | `true` | Whether `install.sh` registers the systemd user service. Has no effect after install. |

### `[tray]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `show_tray` | bool | `true` | Show a system-tray icon. Requires the `tray` Cargo feature and `libayatana-appindicator3` on the host. |
| `pause_on_startup` | bool | `false` | Start the daemon in paused state. No reminders fire until you click **Resume** in the tray menu. |

### `[overlay]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `animation_style` | string or table | `"blink_eye"` | Animation played on the eye graphic. See [Animation styles](#animation-styles). |
| `dim_opacity` | float | `0.92` | Background opacity of the full-screen overlay (0.0 = transparent, 1.0 = opaque). Values between `0.85` and `0.95` look best on composited desktops. |
| `duration_seconds` | integer | `20` | How long the overlay is displayed before auto-dismissing. |
| `allow_skip` | bool | `true` | Show a **Skip** button that closes the overlay early. |
| `skip_after_seconds` | integer | `5` | Seconds into the countdown before the Skip button appears. Set to `0` to show it immediately. Has no effect when `allow_skip = false`. |

#### Animation styles

| Value | Effect |
|-------|--------|
| `"blink_eye"` | Periodic double-blink — the eye closes briefly twice per cycle. |
| `"breathe"` | Slow opacity + scale pulse — guides a calming breath. |
| `{ custom = "/absolute/path/to/animation.css" }` | Load a custom CSS file. |

### `[[reminders]]`

Each `[[reminders]]` table defines one independent reminder.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | string | `""` | Stable machine-readable key. Built-in IDs: `look_away`, `drink_water`, `take_walk`, `take_break`. Any other value becomes a custom reminder. |
| `label` | string | `""` | Short title shown on the overlay and tray tooltip. |
| `message` | string | `""` | Body text shown on the full-screen overlay. |
| `interval_minutes` | integer | `20` | How often (in minutes) to trigger this reminder. |
| `enabled` | bool | `true` | Set to `false` to silence a reminder without removing it from the config. |
| `icon` | string or absent | absent | Emoji or absolute path to a PNG/SVG displayed above the message. Falls back to a clock emoji if omitted. |

---

## Adding a custom reminder

Append a `[[reminders]]` block to your config file. Changes take effect within
~1 second — no restart needed.

```toml
[[reminders]]
id               = "posture_check"
label            = "Posture Check"
message          = "Sit up straight and relax your shoulders."
interval_minutes = 30
enabled          = true
icon             = "🪑"
```

To temporarily silence any reminder without deleting its config, set
`enabled = false`.

---

## Writing a custom animation

Custom animations are standard CSS keyframes applied to the `.eye-animation`
CSS class that wraps the eye SVG.

### Rules

1. Define a `@keyframes` block with any name.
2. Apply it to `.eye-animation` using `animation-name`, `animation-duration`,
   `animation-iteration-count: infinite`, and your chosen timing function.
3. Use the token `{{DURATION}}` as the value of `animation-duration`.
   ZenGuard replaces it at runtime with the configured overlay duration (e.g.
   `20s`), keeping the animation cycle in sync with the countdown timer.
4. Save the file with an absolute path and reference it in the config:

```toml
[overlay]
animation_style = { custom = "/home/you/.config/zenguard/pulse.css" }
```

### Example — colour fade

```css
/*
 * ZenGuard — Custom "warm pulse" animation
 *
 * The token {{DURATION}} is replaced at runtime.  Do not remove it.
 */

@keyframes warm-pulse {
    0%, 100% { opacity: 0.6; filter: hue-rotate(0deg);   }
    50%       { opacity: 1.0; filter: hue-rotate(30deg);  }
}

.eye-animation {
    animation-name:            warm-pulse;
    animation-duration:        {{DURATION}};
    animation-timing-function: ease-in-out;
    animation-iteration-count: infinite;
    animation-fill-mode:       both;
}
```

Properties you can animate: `opacity`, `transform` (scale, rotate), `filter`
(hue-rotate, brightness), `color`. Avoid layout-affecting properties
(`width`, `height`, `margin`) as they cause unnecessary reflows.

---

## Troubleshooting

### Overlay does not appear

- Run `zenguard --log-level debug` in a terminal and watch for errors.
- Ensure the GTK4 display is available (`echo $DISPLAY` or `$WAYLAND_DISPLAY`).
- If using Wayland, confirm `XDG_RUNTIME_DIR` is set in the service environment:
  ```bash
  systemctl --user show-environment
  ```
- Check that the intervals have actually elapsed — default eye-rest is 20 min.

### No system-tray icon

- Confirm `show_tray = true` in config.
- Ensure `libayatana-appindicator3` is installed:
  ```bash
  dpkg -l libayatana-appindicator3-1
  ```
- ZenGuard must have been built with `--features tray` (the standard
  `install.sh` does **not** enable this by default — add it manually if needed):
  ```bash
  cargo build --release --features tray
  ```

### Eye SVG not visible

- Install the gdk-pixbuf SVG loader plugin:
  ```bash
  sudo apt install librsvg2-common
  ```
- ZenGuard degrades gracefully — reminders still appear without the eye graphic.

### Service fails to start at login

- Check logs: `journalctl --user -u zenguard -n 30`
- The service has a 3-second `ExecStartPre` delay so the display is ready;
  increase it in `~/.config/systemd/user/zenguard.service` if your session
  initialises slowly.
- Verify `~/.local/bin` is on your `PATH`:
  ```bash
  echo $PATH | grep -q "$HOME/.local/bin" && echo "OK" || echo "MISSING"
  ```

### Config changes not picked up

- Confirm inotify watches are available:
  ```bash
  cat /proc/sys/fs/inotify/max_user_watches  # should be > 0
  ```
- Some editors (notably Vim's `:w`) do atomic renames — ZenGuard watches the
  parent **directory** rather than the file directly, so these are handled
  correctly.

---

## Roadmap

| Feature | Status |
|---------|--------|
| GTK4 settings panel (GUI config editor) | Planned |
| Multi-monitor aware overlays | Planned |
| Theme support (light / dark / custom palettes) | Planned |
| D-Bus interface (pause/resume/skip programmatically) | Planned |
| Wayland layer-shell integration (true top-of-stack overlay) | Planned |
| Per-reminder animation overrides | Planned |
| Statistics panel (how many reminders taken vs skipped) | Planned |

---

## License

MIT — see `LICENSE` for full text.

Copyright (c) 2024 ZenGuard Contributors
