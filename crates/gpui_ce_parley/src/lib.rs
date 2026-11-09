//! Shared text support built on the Parley text stack.

mod catalog;
mod store;
mod text_system;

#[cfg(test)]
#[path = "../test_support/overflow.rs"]
mod overflow;

pub use catalog::SystemFonts;
pub use text_system::ParleyTextSystem;

pub(crate) use store::FontStore;
