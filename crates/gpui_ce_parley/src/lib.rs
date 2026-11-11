//! Shared text support built on the Parley text stack.

mod catalog;
mod store;
mod text_system;

pub use catalog::SystemFonts;
pub use store::{BitmapFallbackGlyphRasterizer, SwashGlyphRasterizer};
pub use text_system::ParleyTextSystem;

pub(crate) use store::FontStore;
