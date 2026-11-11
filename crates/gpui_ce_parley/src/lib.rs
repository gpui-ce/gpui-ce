//! Shared text support built on the Parley text stack.

mod catalog;
mod store;
mod text_system;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use catalog::SystemFonts;
pub use text_system::ParleyTextSystem;

pub(crate) use store::FontStore;
