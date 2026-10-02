//! Pin library: platform-free domain logic plus Windows-specific glue.
//!
//! Layers (each depends only on the ones above it):
//! - [`domain`]: pure logic, unit-tested on any host.
//! - [`resources`]: embedded images and pixel helpers.
//! - [`win`]: thin Win32 wrappers (RAII handles, window queries).
//! - `ui`: windows Pin owns — picker, overlay badge, tray (views).
//! - `autostart`: settings file + `Run` registry glue.
//! - `app`: the controller; owns all state, runs the message loop.
//!
//! Only what the binary and integration tests need is public.

pub mod domain;
pub mod resources;

#[cfg(windows)]
pub mod win;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
pub use app::run;
