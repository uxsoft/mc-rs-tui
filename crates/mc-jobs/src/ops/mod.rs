pub mod copy;
pub mod delete;
pub mod r#move;

pub use copy::CopyJob;
pub use delete::DeleteJob;
pub use r#move::MoveJob;

use mc_core::{Error, VPath};

/// Append a single path component to a [`VPath`]'s last layer.
pub(crate) fn child_of(parent: &VPath, name: &str) -> Result<VPath, Error> {
    let layer = parent
        .last()
        .cloned()
        .ok_or_else(|| Error::InvalidPath("empty vpath".into()))?;
    let mut new_layer = layer;
    new_layer.sub.push(name);
    let mut new = parent.clone();
    new.pop_layer();
    new.push_layer(new_layer);
    Ok(new)
}
