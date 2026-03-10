# Contributing to Brevyx

This document is aimed at developers who want to build, test, or contribute to Brevyx.

---

## Table of contents

1. [Dev environment setup](#dev-environment-setup)
2. [Build commands](#build-commands)
3. [Architecture overview](#architecture-overview)
4. [Module guide](#module-guide)
5. [Test suite](#test-suite)
6. [Key patterns and gotchas](#key-patterns-and-gotchas)
7. [How to add things](#how-to-add-things)
8. [Code quality gates](#code-quality-gates)
9. [CI pipeline](#ci-pipeline)

---

## Dev environment setup

### System packages

```bash
# Minimum required (GTK4 build-time headers + D-Bus)
sudo apt install \
  libgtk-4-dev \
  libdbus-1-dev \
  pkg-config \
  build-essential \
  librsvg2-common

# Optional: required only if you want to run a build with the legacy
# appindicator path (not the default; ksni is used instead)
# sudo apt install libgtk-3-dev libayatana-appindicator3-dev
```

### Rust toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# includes rustfmt and clippy by default on the stable channel
```

Brevyx targets stable Rust. No nightly features are used.

---

## Build commands

```bash
make build       # debug build  (cargo build)
make release     # release build (cargo build --release)
make test        # run all unit + integration tests
make lint        # cargo clippy -- -D warnings
make fmt         # auto-format with cargo fmt
make check       # fmt check + clippy + tests — mirrors CI
make clean       # remove target/
```

Or use `cargo` directly:

```bash
cargo build                        # debug
cargo build --release              # release
cargo run -- --log-level debug     # run locally (requires a display)
cargo test                         # all tests (no display required)
cargo clippy -- -D warnings
cargo fmt --check
```

> Tests do **not** require a running GTK4 display — all GTK-dependent code is
> excluded from the test binary via `#[cfg(not(test))]` guards or separate
> integration tests that only exercise the scheduler/config layer.

---

## Architecture overview

```
main.rs
  └─ app::build_and_run(initial_cfg, config_path)
       ├─ Tokio runtime (multi-thread, background)
       └─ gtk4::Application ("com.brevyx.app")
            └─ activate signal
                 └─ daemon::Daemon::start()
                      ├─ config watcher (notify, background thread)
                      ├─ scheduler::Scheduler  ──────────────────────► mpsc::Sender<Reminder>
                      │    (one tokio task per enabled reminder)              │
                      ├─ tray (ksni, blocking thread)                         │
                      │    └─ sends TrayCommand over std::sync::mpsc          │
                      └─ overlay::OverlayController (GTK main thread)◄────────┘
                           └─ gtk4::Window per reminder (auto-dismisses)
```

### Thread model

| Thread | What runs there |
|--------|-----------------|
| Main (GTK) | `gtk4::Application`, `OverlayController`, `glib::timeout_add_local` timers |
| Tokio pool | `Scheduler` tasks, config hot-reload watcher |
| Dedicated thread | `ksni` tray (blocks on D-Bus) |

Communication between threads always goes through channels — never shared mutable state:

- `Scheduler → OverlayController`: `tokio::sync::mpsc::channel::<Reminder>`
- `Daemon → Scheduler` (config reload): `tokio::sync::watch::channel::<BrevyxConfig>`
- `Tray → Daemon` (pause/resume/quit): `std::sync::mpsc::sync_channel::<TrayCommand>`
- `Daemon → Tray` (pause state init): `PauseHandle(Arc<AtomicBool>)`

---

## Module guide

```
src/
  main.rs          CLI entry point — parses args, loads config, calls app::build_and_run
  lib.rs           Re-exports all public modules for the integration test crate
  error.rs         Top-level AppError (thiserror)
  app.rs           GTK4 Application bootstrap; Tokio runtime construction
  daemon/
    mod.rs         Daemon orchestrator — owns Scheduler, OverlayController, Tray, watcher
  config/
    schema.rs      BrevyxConfig, OverlayConfig, ReminderConfig, AnimationStyle (serde)
    mod.rs         load_or_create(), watch(), default_config_path()
    default_config.toml  Embedded default written on first run
  scheduler/
    reminder.rs    Reminder value type, ReminderKind, Display, From<&ReminderConfig>
    mod.rs         Scheduler, PauseHandle; one tokio::time::interval task per reminder
  overlay/
    animation.rs   load_animation_css(), AnimationManager (apply/remove CssProvider)
    window.rs      OverlayWindow::build() — full-screen gtk4::Window with countdown
    mod.rs         OverlayController — polls mpsc, manages window lifetime
  tray/
    mod.rs         BrevyxTray (ksni::Tray impl), TrayCommand enum
assets/
  eye_blink.svg            Eye SVG embedded at compile time (include_bytes!)
  animations/
    blink.css              Built-in blink animation (uses {{DURATION}} placeholder)
    breathe.css            Built-in breathe animation
config/
  default_config.toml      Shipped default config (embedded in binary)
tests/
  pipeline.rs              Integration tests: Scheduler → mpsc channel (no GTK)
```

---

## Test suite

```
cargo test
```

### Unit tests (18)

| Location | Count | What they cover |
|----------|-------|-----------------|
| `src/config/mod.rs` | 8 | Parse defaults, load from file, missing file, bad TOML |
| `src/scheduler/reminder.rs` | 5 | ReminderKind detection, Display, From conversions |
| `src/scheduler/mod.rs` | 4 | Interval fires, pause/resume, hot-reload, disabled reminder |

### Integration tests (6, in `tests/pipeline.rs`)

These exercise the full `Scheduler → mpsc::Sender<Reminder>` pipeline without
any GTK dependency:

| Test | What it proves |
|------|----------------|
| `reminder_delivered_after_interval` | Basic delivery at correct time |
| `pause_suppresses_and_resume_restores` | Pause/resume semantics |
| `config_hot_reload_changes_interval` | watch channel propagation |
| `multiple_reminders_fire_independently` | Independent intervals don't interfere |
| `default_overlay_config_matches_spec` | Default values match docs |
| `disabled_reminder_is_never_delivered` | `enabled = false` is respected |

### Deterministic timer tests

All async timer tests use `#[tokio::test(start_paused = true)]` with
`tokio::time::advance`. This is mandatory for deterministic, fast execution —
do not use real `sleep` in tests.

Key subtlety: after `tokio::time::advance(d).await` the woken task needs one
more executor turn to complete its `sender.send().await` before returning
from `run()`.

- **Correct** for "should fire": `rx.recv().await` (suspends, lets task finish)
- **Correct** for "should NOT fire": `rx.try_recv()` (non-blocking assertion)

---

## Key patterns and gotchas

### glib 0.20 channel API

`glib::MainContext::channel`, `glib::Sender`, and `glib::Receiver` **do not
exist** in glib 0.20. Use `std::sync::mpsc::sync_channel` with
`glib::timeout_add_local` polling instead:

```rust
// Producer (any thread)
let (tx, rx) = std::sync::mpsc::sync_channel::<Cmd>(8);

// Consumer (GTK main thread)
glib::timeout_add_local(Duration::from_millis(200), move || {
    while let Ok(cmd) = rx.try_recv() {
        handle(cmd);
    }
    glib::ControlFlow::Continue
});
```

### GTK objects and thread safety

GTK4 widgets are **not** `Send`. Keep all widget construction and manipulation
on the main thread. Use channels to pass data across thread boundaries.

`Rc<RefCell<T>>` is idiomatic for GTK main-thread state. `Arc<Mutex<T>>` is
for data that genuinely crosses into Tokio tasks.

### ControlFlow in glib timers

`glib::timeout_add_local` callbacks return `glib::ControlFlow`:

```rust
glib::ControlFlow::Continue  // keep firing
glib::ControlFlow::Break     // unregister the timer (fire once)
```

Use `Break` for one-shot timers (auto-dismiss countdown, skip-button reveal).

### ksni tray

The tray runs on a dedicated blocking thread via `std::thread::spawn`. It
communicates back to the daemon via `std::sync::mpsc`. Do not try to call
GTK APIs from that thread.

### Config hot-reload

`notify` watches the **parent directory** (not the file directly) to handle
editors that use atomic renames (e.g. Vim `:w`). The watcher thread sends
updates over `tokio::sync::watch`. The scheduler re-spawns its interval tasks
when the watch channel yields a new config value.

### CSS animation placeholder

The `{{DURATION}}` token in `blink.css` / `breathe.css` is replaced at runtime
by `overlay::animation::load_animation_css()` with the overlay duration string
(e.g. `"20s"`). Custom animations must include this token for the cycle to stay
in sync with the countdown.

---

## How to add things

### New built-in reminder kind

1. Add a variant to `ReminderKind` in `src/scheduler/reminder.rs`.
2. Add the matching string pattern to the `From<&str> for ReminderKind` impl.
3. Add the variant to the `Display` impl (human-readable label).
4. Add a `[[reminders]]` block in `config/default_config.toml` with the new ID.
5. Add a unit test in `src/scheduler/reminder.rs` for the new kind.

### New animation style

1. Create `assets/animations/<name>.css` using `{{DURATION}}` as the duration
   token (see existing files for reference).
2. Add a variant to `AnimationStyle` in `src/config/schema.rs`.
3. Handle the new variant in `src/overlay/animation.rs::load_animation_css()`.
4. Document it in the `README.md` animation-styles table.

### New config field

1. Add the field to the relevant struct in `src/config/schema.rs` with a
   `#[serde(default = "...")]` attribute.
2. Provide a named default function (e.g. `fn default_foo() -> T`).
3. Update `config/default_config.toml` with the new key + value.
4. Add a test in `src/config/mod.rs` asserting the default value.
5. Update the configuration reference table in `README.md`.

### New tray menu item

1. Add a variant to `TrayCommand` in `src/tray/mod.rs`.
2. Add an `ksni::MenuItem` entry in `BrevyxTray::menu()`.
3. Handle the new command in the `TrayCommand` match arm inside
   `daemon::Daemon::start`.

---

## Code quality gates

All PRs must pass the same checks that CI enforces:

```bash
cargo fmt --check          # zero formatting drift
cargo clippy -- -D warnings  # zero lint warnings
cargo test                 # all tests green
```

Run them together with:

```bash
make check
```

No `unwrap()` or `expect()` in production paths (`src/` outside `#[cfg(test)]`
blocks). Use `?`, `anyhow::Context`, or `tracing::error!` + graceful
degradation.

---

## CI pipeline

Defined in `.github/workflows/ci.yml`. Three parallel jobs:

| Job | What it runs |
|-----|-------------|
| `fmt` | `cargo fmt --check` |
| `clippy` | `cargo clippy -- -D warnings` (with GTK4 + D-Bus headers installed) |
| `test` | `cargo test` (with GTK4 + D-Bus headers installed) |

The CI workflow is also declared as `workflow_call` so that `release.yml` can
gate releases on it passing.

All three jobs install `libgtk-4-dev` and `libdbus-1-dev` via `apt` and use
`actions/cache` keyed on `Cargo.lock` to speed up subsequent runs.
