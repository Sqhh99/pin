//! Pin library: OS-agnostic core plus Windows-specific glue.
//!
//! Public surface is intentionally minimal — exported for integration tests
//! and to give the binary a single entry point ([`run`]).

pub mod overlay;
pub mod pinned;
pub mod resources;
pub mod win;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod selection;
#[cfg(windows)]
pub mod tray;

#[cfg(windows)]
pub fn run() -> anyhow::Result<()> {
    app::run()
}
