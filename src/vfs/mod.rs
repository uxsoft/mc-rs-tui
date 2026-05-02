//! Virtual filesystem trait and built-in backends.

pub mod local;
pub mod registry;
pub mod trait_;

pub use registry::Registry;
pub use trait_::{Capabilities, Vfs, WriteOpts};
