//! ZenGuard library crate.
//!
//! All subsystems are exposed as public modules so that the binary crate
//! (`src/main.rs`) and integration tests (`tests/`) can import them directly.

pub mod app;
pub mod config;
pub mod daemon;
pub mod error;
pub mod overlay;
pub mod scheduler;
pub mod tray;
