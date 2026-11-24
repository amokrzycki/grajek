//! The small JSON file that carries playback preferences, queue, history and favorites
//! across restarts.

mod file;
mod saved;
mod saver;

pub use file::open;
pub use saved::{DEFAULT_VOLUME, Saved, SavedQueueItem};
pub use saver::spawn_saver;

#[cfg(test)]
pub(crate) use file::{load, write};

#[cfg(test)]
mod tests;
