#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

mod handle;
mod slab;
mod store;

pub use handle::FileIoHandle;
pub use store::{BlockStore, FileStore, HeaderStore};
