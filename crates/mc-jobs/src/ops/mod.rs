pub mod copy;
pub mod delete;
pub mod r#move;

pub use copy::CopyJob;
pub use delete::DeleteJob;
pub use r#move::MoveJob;

use mc_core::{Error, VPath};

/// Append a single path component to a [`VPath`]'s last layer.
/// Wrapper around [`VPath::child`] that returns the rich job-side error.
pub(crate) fn child_of(parent: &VPath, name: &str) -> Result<VPath, Error> {
    parent
        .child(name)
        .ok_or_else(|| Error::InvalidPath(format!("invalid child name {name:?}")))
}
