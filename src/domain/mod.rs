//! Platform-free application logic.
//!
//! Nothing in this module touches Win32, so everything here is unit-tested on
//! any host (including the Linux CI job). The Windows glue in [`crate::win`],
//! `ui` and `app` translates OS handles and messages into these types.

pub mod geometry;
pub mod pinned;
pub mod settings;
pub mod window;
