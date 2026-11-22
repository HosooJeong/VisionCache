//! Window-scoped capture, adaptive image observations, and desktop input.

pub mod desktop;
pub mod image;
pub mod mcp;
pub mod session;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;
