//! Read-only native adapters. Pure parsers are shared with fixture verification.
mod parse;
pub use parse::*;
mod logs;
pub use logs::*;
mod accounts;
pub use accounts::*;
mod github;
pub use github::*;
mod local;
pub use local::*;
mod lifecycle;
pub use lifecycle::*;
