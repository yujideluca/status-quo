//! Reusable tray interface without platform, filesystem, process or network dependencies.
mod browser;
pub use browser::*;
mod layout;
pub use layout::*;
mod ui;
pub use ui::*;
