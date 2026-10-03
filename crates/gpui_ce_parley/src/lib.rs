//! Shared text support built on the Parley text stack.

mod catalog;
#[cfg(test)]
mod font_fixtures {
    include!("font_fixtures.rs");
    include!("font_fixtures_tests.rs");
}
mod store;
mod text_system;

pub use catalog::SystemFonts;
pub use fontique::Blob as FontDataBlob;
pub use store::{
    BitmapFallbackGlyphRasterizer, ColorGlyphKind, FontSynthesis, FontVariation, GlyphRasterizer,
    RasterFace, SwashGlyphRasterizer,
};
pub use text_system::ParleyTextSystem;

pub(crate) use catalog::{CatalogState, FaceFamily, FaceRequest, FontCatalog};
pub(crate) use store::FontStore;
