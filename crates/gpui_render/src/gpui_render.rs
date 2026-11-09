//! Shared typed rendering and shader contracts.

pub mod artifacts;
pub mod blur;
mod instances;
pub mod path_types;
pub mod shaders;
mod target_stack;

#[cfg(all(test, feature = "swash"))]
#[path = "../../gpui_ce_parley/test_support/overflow.rs"]
mod overflow;

pub use instances::InstanceRange;
pub use target_stack::TargetStack;

#[cfg(feature = "swash")]
mod swash_glyph_rasterizer;

#[cfg(feature = "swash")]
pub use swash_glyph_rasterizer::{BitmapFallbackGlyphRasterizer, SwashGlyphRasterizer};
