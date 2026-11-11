use crate::{
    FontId, ForegroundDependency, GlyphId, PreparedRasterStyle, RasterStyleRequest,
    RasterizedGlyph, RenderGlyphParams, Result, TextRenderingMode,
};
use anyhow::Context as _;
use linebender_resource_handle::Blob;
use skrifa::{
    FontRef, GlyphId as SkrifaGlyphId, MetadataProvider as _, Tag,
    bitmap::{BitmapFormat, BitmapStrikes},
    instance::NormalizedCoord,
    raw::{
        TableProvider as _,
        tables::{colr::Colr, sbix::Sbix, svg::SVGDocumentRecord},
    },
};

/// Shared ownership of immutable font or collection bytes.
pub use linebender_resource_handle::Blob as FontDataBlob;

/// One design-space variation coordinate for an exact font instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontVariation {
    /// OpenType variation-axis tag.
    pub tag: Tag,
    /// Design-space value consumed by native font APIs.
    pub value: f32,
}

impl FontVariation {
    /// Creates a design-space variation coordinate from an OpenType axis tag.
    pub const fn new(tag: [u8; 4], value: f32) -> Self {
        Self {
            tag: Tag::new(&tag),
            value,
        }
    }
}

/// Synthetic styling attached to an exact font instance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontSynthesis {
    /// Whether the rasterizer should synthesize a heavier outline.
    pub embolden: bool,
    /// Synthetic clockwise skew in degrees.
    pub skew_degrees: Option<f32>,
}

/// The immutable face and instance selected during shaping for a glyph.
#[derive(Clone, Copy)]
pub struct RasterFace<'a> {
    /// Canonical identity of the full face, variation, and synthesis combination.
    pub font_id: FontId,
    /// Stable identity shared by every face and instance backed by the same source bytes.
    pub source_id: u64,
    /// Owning handle for the original font or collection bytes.
    pub source: &'a Blob<u8>,
    /// Face index within a TTC or OTC collection.
    pub face_index: u32,
    /// Normalized variation coordinates in axis order.
    pub normalized_coords: &'a [NormalizedCoord],
    /// Design-space variation coordinates.
    pub variations: &'a [FontVariation],
    /// Synthetic styling selected during font matching.
    pub synthesis: FontSynthesis,
    /// Whether the face advertises an OpenType color glyph table.
    pub has_color_glyphs: bool,
}

impl RasterFace<'_> {
    /// Returns the original font or collection bytes.
    pub fn data(&self) -> &[u8] {
        self.source.as_ref()
    }

    /// Returns whether every supplied variation coordinate uses its axis default.
    pub fn has_default_variations(&self) -> Result<bool> {
        let font = FontRef::from_index(self.data(), self.face_index)
            .context("cannot inspect variation axes in the selected face")?;

        Ok(variations_are_default(&font, self.variations))
    }

    /// Returns the preferred color artwork format supported by the rasterizer.
    pub fn supported_color_glyph_kind(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> Result<Option<ColorGlyphKind>> {
        let font = FontRef::from_index(self.data(), self.face_index)
            .context("cannot inspect color glyph data in the selected face")?;

        Ok(first_supported_color_kind(
            ColorGlyphClassifier::new(font).available_kinds(glyph_id),
            supports,
        ))
    }
}

/// The native artwork format for a color glyph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorGlyphKind {
    /// OpenType COLRv0 layers.
    ColrV0,
    /// An OpenType COLRv1 paint graph.
    ColrV1,
    /// A CBDT bitmap strike.
    Cbdt,
    /// An Apple sbix bitmap strike.
    Sbix,
    /// An SVG document embedded in the font.
    Svg,
}

/// Rasterizes glyphs from the exact font instance selected during shaping.
pub trait GlyphRasterizer: Send {
    /// Returns whether this rasterizer can preserve the glyph's native color artwork.
    fn supports_color_glyph(&self, _kind: ColorGlyphKind) -> bool {
        true
    }

    /// Reduces a scene request to the settings which alter cached raster pixels.
    fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle;

    /// Rasterizes one glyph from the exact face selected during shaping.
    fn rasterize(
        &mut self,
        face: RasterFace<'_>,
        params: &RenderGlyphParams,
    ) -> Result<RasterizedGlyph>;

    /// Returns the platform's default mode for ordinary text.
    fn recommended_mode(&self) -> TextRenderingMode {
        TextRenderingMode::Subpixel
    }
}

/// Color glyph data parsed once for all glyphs in a shaped run.
pub struct ColorGlyphClassifier<'a> {
    colr: Option<Colr<'a>>,
    sbix: Option<Sbix<'a>>,
    cbdt_strikes: Option<BitmapStrikes<'a>>,
    svg_records: &'a [SVGDocumentRecord],
}

impl ColorGlyphClassifier<'_> {
    /// Parses color tables for repeated glyph queries on one face.
    pub fn new(font: FontRef<'_>) -> ColorGlyphClassifier<'_> {
        let svg_records = font
            .svg()
            .ok()
            .and_then(|svg| svg.svg_document_list().ok())
            .map(|documents| documents.document_records())
            .unwrap_or_default();

        ColorGlyphClassifier {
            colr: font.colr().ok(),
            sbix: font.sbix().ok(),
            cbdt_strikes: BitmapStrikes::with_format(&font, BitmapFormat::Cbdt),
            svg_records,
        }
    }

    /// Returns every artwork format available for this glyph in preference order.
    pub fn available_kinds(&self, glyph_id: GlyphId) -> impl Iterator<Item = ColorGlyphKind> {
        let skrifa_id = SkrifaGlyphId::new(glyph_id.0);

        let has_colr_v1 = self.colr.as_ref().is_some_and(|colr| {
            colr.v1_base_glyph(skrifa_id)
                .is_ok_and(|glyph| glyph.is_some())
        });

        let has_colr_v0 = self.colr.as_ref().is_some_and(|colr| {
            colr.v0_base_glyph(skrifa_id)
                .is_ok_and(|glyph| glyph.is_some())
        });

        let has_sbix_bitmap = self
            .sbix
            .as_ref()
            .is_some_and(|sbix| sbix_has_glyph(sbix, skrifa_id));

        let has_cbdt_bitmap = self
            .cbdt_strikes
            .as_ref()
            .is_some_and(|strikes| strikes.iter().any(|strike| strike.get(skrifa_id).is_some()));

        let has_svg = self.svg_records.iter().any(|record| {
            let start = record.start_glyph_id().to_u32();
            let end = record.end_glyph_id().to_u32();

            (start..=end).contains(&glyph_id.0)
        });

        [
            has_colr_v1.then_some(ColorGlyphKind::ColrV1),
            has_colr_v0.then_some(ColorGlyphKind::ColrV0),
            has_sbix_bitmap.then_some(ColorGlyphKind::Sbix),
            has_cbdt_bitmap.then_some(ColorGlyphKind::Cbdt),
            has_svg.then_some(ColorGlyphKind::Svg),
        ]
        .into_iter()
        .flatten()
    }

    /// Returns which foreground components affect the supported artwork.
    pub fn foreground_dependency(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> ForegroundDependency {
        let Some(kind) = first_supported_color_kind(self.available_kinds(glyph_id), supports)
        else {
            return ForegroundDependency::Full;
        };

        match kind {
            ColorGlyphKind::Cbdt | ColorGlyphKind::Sbix => ForegroundDependency::AlphaOnly,
            ColorGlyphKind::ColrV0 if self.colr_v0_has_fixed_palette(glyph_id) => {
                ForegroundDependency::AlphaOnly
            }
            _ => ForegroundDependency::Full,
        }
    }

    fn colr_v0_has_fixed_palette(&self, glyph_id: GlyphId) -> bool {
        let Some(colr) = self.colr.as_ref() else {
            return false;
        };

        let Some(mut layers) = colr
            .v0_base_glyph(SkrifaGlyphId::new(glyph_id.0))
            .ok()
            .flatten()
        else {
            return false;
        };

        layers.all(|layer_idx| {
            colr.v0_layer(layer_idx)
                .is_ok_and(|(_glyph_id, palette_idx)| palette_idx != u16::MAX)
        })
    }
}

fn sbix_has_glyph(sbix: &Sbix<'_>, glyph_id: SkrifaGlyphId) -> bool {
    (0..sbix.strikes().len()).any(|strike_idx| {
        sbix.strikes()
            .get(strike_idx)
            .ok()
            .and_then(|strike| strike.glyph_data(glyph_id).ok())
            .flatten()
            .is_some()
    })
}

fn first_supported_color_kind(
    kinds: impl IntoIterator<Item = ColorGlyphKind>,
    mut supports: impl FnMut(ColorGlyphKind) -> bool,
) -> Option<ColorGlyphKind> {
    kinds.into_iter().find(|&kind| supports(kind))
}

fn variations_are_default(font: &FontRef<'_>, variations: &[FontVariation]) -> bool {
    let axes = font.axes();

    variations.iter().all(|variation| {
        axes.iter()
            .any(|axis| axis.tag() == variation.tag && axis.default_value() == variation.value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::raw::{FontData, FontRead};

    #[test]
    fn supported_color_artwork_is_selected_when_a_preferred_format_is_unsupported() {
        let available = [ColorGlyphKind::ColrV1, ColorGlyphKind::ColrV0];
        let selected = first_supported_color_kind(available, |kind| kind == ColorGlyphKind::ColrV0);

        assert_eq!(selected, Some(ColorGlyphKind::ColrV0));
    }

    #[test]
    fn colr_v0_foreground_dependency_checks_every_layer() {
        let mut table = Vec::new();
        table.extend_from_slice(&0u16.to_be_bytes());
        table.extend_from_slice(&3u16.to_be_bytes());
        table.extend_from_slice(&14u32.to_be_bytes());
        table.extend_from_slice(&32u32.to_be_bytes());
        table.extend_from_slice(&2u16.to_be_bytes());

        for (glyph_id, first_layer) in [(1u16, 0u16), (2, 1), (4, 2)] {
            table.extend_from_slice(&glyph_id.to_be_bytes());
            table.extend_from_slice(&first_layer.to_be_bytes());
            table.extend_from_slice(&1u16.to_be_bytes());
        }

        for (glyph_id, palette_idx) in [(10u16, 0u16), (11, u16::MAX)] {
            table.extend_from_slice(&glyph_id.to_be_bytes());
            table.extend_from_slice(&palette_idx.to_be_bytes());
        }

        let colr = <Colr<'_> as FontRead<'_>>::read(FontData::new(&table)).unwrap();

        let classifier = ColorGlyphClassifier {
            colr: Some(colr),
            sbix: None,
            cbdt_strikes: None,
            svg_records: &[],
        };

        assert!(classifier.colr_v0_has_fixed_palette(GlyphId(1)));
        assert!(!classifier.colr_v0_has_fixed_palette(GlyphId(2)));
        assert!(!classifier.colr_v0_has_fixed_palette(GlyphId(3)));
        assert!(!classifier.colr_v0_has_fixed_palette(GlyphId(4)));
    }

    #[test]
    fn sbix_records_are_artwork_even_when_their_payload_is_a_native_reference() {
        for graphic_type in [*b"flip", *b"dupe"] {
            let mut table = Vec::new();
            table.extend_from_slice(&1u16.to_be_bytes());
            table.extend_from_slice(&1u16.to_be_bytes());
            table.extend_from_slice(&1u32.to_be_bytes());
            table.extend_from_slice(&12u32.to_be_bytes());
            table.extend_from_slice(&20u16.to_be_bytes());
            table.extend_from_slice(&72u16.to_be_bytes());
            table.extend_from_slice(&16u32.to_be_bytes());
            table.extend_from_slice(&16u32.to_be_bytes());
            table.extend_from_slice(&26u32.to_be_bytes());
            table.extend_from_slice(&0i16.to_be_bytes());
            table.extend_from_slice(&0i16.to_be_bytes());
            table.extend_from_slice(&graphic_type);
            table.extend_from_slice(&0u16.to_be_bytes());

            let sbix = Sbix::read(FontData::new(&table), 2).unwrap();

            assert!(!sbix_has_glyph(&sbix, SkrifaGlyphId::new(0)));
            assert!(sbix_has_glyph(&sbix, SkrifaGlyphId::new(1)));
        }
    }
}
