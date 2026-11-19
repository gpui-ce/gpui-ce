use anyhow::{Context as _, Result, bail, ensure};
use fontique::{Blob, Synthesis};
use gpui::{
    Bounds, ColorGlyphClassifier, ColorGlyphKind, FontId, FontMetrics, FontSynthesis,
    FontVariation, ForegroundDependency, GlyphId, RasterFace, Size, point, size,
};
use skrifa::{
    FontRef, MetadataProvider as _, Tag,
    instance::{NormalizedCoord, Size as SkrifaSize},
};
use std::collections::HashMap;

const CANONICAL_FONT_ID_BIT: usize = 1 << (usize::BITS - 1);

/// Synthetic outline changes which affect raster output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SynthesisKey {
    embolden: bool,
    skew_bits: Option<u32>,
}

impl From<Synthesis> for SynthesisKey {
    fn from(synthesis: Synthesis) -> Self {
        Self {
            embolden: synthesis.embolden(),
            skew_bits: synthesis.skew().map(f32::to_bits),
        }
    }
}

/// Full identity of a selected font instance.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FontKey {
    source_id: u64,
    face_index: u32,
    normalized_coords: Vec<NormalizedCoord>,
    synthesis: SynthesisKey,
}

/// A concrete font face and variation instance selected for GPUI.
#[derive(Clone, Debug)]
pub(crate) struct LoadedFont {
    /// Immutable bytes containing the selected face.
    pub(crate) data: Blob<u8>,
    /// Face index within the font collection.
    pub(crate) index: u32,
    /// Normalized variation coordinates in axis order.
    pub(crate) normalized_coords: Vec<NormalizedCoord>,
    /// Design-space coordinates for native font APIs.
    pub(crate) variations: Vec<FontVariation>,
    /// Synthetic styling requested for the selected face.
    pub(crate) synthesis: Synthesis,
    /// Whether this face advertises color glyph data.
    pub(crate) has_color_glyphs: bool,
}

impl LoadedFont {
    fn skrifa_ref(&self) -> Result<FontRef<'_>> {
        FontRef::from_index(self.data.as_ref(), self.index)
            .context("Skrifa could not parse the stored font face")
    }

    /// Builds a glyph-level view of the face's color artwork.
    pub(crate) fn color_glyphs(&self) -> Result<ColorGlyphClassifier<'_>> {
        let font = self.skrifa_ref()?;

        Ok(ColorGlyphClassifier::new(font))
    }

    pub(crate) fn foreground_dependency(
        &self,
        glyph_id: GlyphId,
        supports: impl FnMut(ColorGlyphKind) -> bool,
    ) -> Result<ForegroundDependency> {
        Ok(self
            .color_glyphs()?
            .foreground_dependency(glyph_id, supports))
    }

    /// Reads global metrics in font units from the canonical bytes.
    pub(crate) fn metrics(&self) -> Result<FontMetrics> {
        let font = self.skrifa_ref()?;
        let metrics = font.metrics(SkrifaSize::unscaled(), self.normalized_coords.as_slice());

        Ok(FontMetrics {
            units_per_em: metrics.units_per_em.into(),
            ascent: metrics.ascent,
            descent: -metrics.descent,
            line_gap: metrics.leading,
            underline_position: metrics.underline.map_or(0.0, |underline| underline.offset),
            underline_thickness: metrics
                .underline
                .map_or(0.0, |underline| underline.thickness),
            cap_height: metrics.cap_height.unwrap_or(metrics.ascent),
            x_height: metrics.x_height.unwrap_or(metrics.ascent),
            bounding_box: Bounds {
                origin: point(0.0, 0.0),
                size: size(
                    metrics.max_width.unwrap_or(0.0),
                    metrics.ascent - metrics.descent,
                ),
            },
        })
    }

    /// Maps a Unicode scalar to a nominal glyph using the canonical bytes.
    pub(crate) fn glyph_for_char(&self, character: char) -> Result<Option<GlyphId>> {
        Ok(self
            .skrifa_ref()?
            .charmap()
            .map(character)
            .map(|glyph| GlyphId(glyph.to_u32())))
    }

    /// Returns the unscaled advance for a glyph.
    pub(crate) fn advance(&self, glyph_id: GlyphId) -> Result<Size<f32>> {
        let font = self.skrifa_ref()?;
        let metrics = font.glyph_metrics(SkrifaSize::unscaled(), self.normalized_coords.as_slice());
        let glyph_id = skrifa::GlyphId::new(glyph_id.0);

        Ok(size(metrics.advance_width(glyph_id).unwrap_or(0.0), 0.0))
    }

    pub(crate) fn raster_face(&self, font_id: FontId) -> RasterFace<'_> {
        RasterFace {
            font_id,
            source_id: self.data.id(),
            source: &self.data,
            face_index: self.index,
            normalized_coords: &self.normalized_coords,
            variations: &self.variations,
            synthesis: FontSynthesis {
                embolden: self.synthesis.embolden(),
                skew_degrees: self.synthesis.skew(),
            },
            has_color_glyphs: self.has_color_glyphs,
        }
    }

    pub(crate) fn data_identity(&self) -> u64 {
        self.data.id()
    }

    /// Returns the glyph's control bounds in font units.
    pub(crate) fn glyph_bounds(&self, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let font = self.skrifa_ref()?;
        let bounds = font
            .glyph_metrics(SkrifaSize::unscaled(), self.normalized_coords.as_slice())
            .bounds(skrifa::GlyphId::new(glyph_id.0));
        let Some(bounds) = bounds else {
            return Ok(Bounds::default());
        };

        Ok(Bounds {
            origin: point(bounds.x_min, bounds.y_min),
            size: size(bounds.x_max - bounds.x_min, bounds.y_max - bounds.y_min),
        })
    }
}

/// Canonical storage for concrete font faces and variation instances.
#[derive(Default)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    ids_by_key: HashMap<FontKey, FontId>,
}

impl FontStore {
    /// Interns a face using Fontique's selected variation and synthesis settings.
    pub(crate) fn intern_synthesized(
        &mut self,
        data: Blob<u8>,
        index: u32,
        synthesis: Synthesis,
    ) -> Result<FontId> {
        let normalized_coords = {
            let font = FontRef::from_index(data.as_ref(), index)
                .context("cannot intern a font face Skrifa cannot parse")?;
            font.axes()
                .location(synthesis.variation_settings().iter().copied())
                .coords()
                .to_vec()
        };

        self.intern(data, index, &normalized_coords, synthesis, &[])
    }

    /// Interns a selected font instance and returns its canonical GPUI ID.
    pub(crate) fn intern(
        &mut self,
        data: Blob<u8>,
        index: u32,
        normalized_coords: &[NormalizedCoord],
        synthesis: Synthesis,
        shaping_variations: &[FontVariation],
    ) -> Result<FontId> {
        let font = FontRef::from_index(data.as_ref(), index)
            .context("cannot intern a font face Skrifa cannot parse")?;
        let axis_count = font.axes().len();
        ensure!(
            normalized_coords.len() <= axis_count,
            "shaped location has more coordinates than the selected face"
        );
        let mut canonical_coords = normalized_coords.to_vec();
        canonical_coords.resize(axis_count, NormalizedCoord::default());
        let variations =
            verified_design_variations(&font, &canonical_coords, synthesis, shaping_variations)?;
        let key = FontKey {
            source_id: data.id(),
            face_index: index,
            normalized_coords: canonical_coords.clone(),
            synthesis: synthesis.into(),
        };

        if let Some(id) = self.ids_by_key.get(&key) {
            return Ok(*id);
        }

        if self.fonts.len() >= CANONICAL_FONT_ID_BIT {
            bail!("canonical font store exhausted its FontId namespace");
        }

        let id = FontId(CANONICAL_FONT_ID_BIT | self.fonts.len());
        let has_color_glyphs = [*b"CBDT", *b"sbix", *b"COLR", *b"SVG "]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(&tag)).is_some());
        self.fonts.push(LoadedFont {
            data,
            index,
            normalized_coords: canonical_coords,
            variations,
            synthesis,
            has_color_glyphs,
        });

        self.ids_by_key.insert(key, id);
        Ok(id)
    }

    /// Returns the stored font for a canonical ID.
    pub(crate) fn get(&self, id: FontId) -> Option<&LoadedFont> {
        canonical_index(id).and_then(|index| self.fonts.get(index))
    }
}

/// Rebuilds the design-space settings used by shaping, then verifies their normalized location.
fn verified_design_variations(
    font: &FontRef<'_>,
    normalized_coords: &[NormalizedCoord],
    synthesis: Synthesis,
    shaping_variations: &[FontVariation],
) -> Result<Vec<FontVariation>> {
    let axes = font.axes();

    let mut variations = axes
        .iter()
        .map(|axis| FontVariation {
            tag: axis.tag(),
            value: axis.default_value(),
        })
        .collect::<Vec<_>>();

    let settings = synthesis.variation_settings().iter().copied().chain(
        shaping_variations
            .iter()
            .map(|variation| (variation.tag, variation.value)),
    );

    for (tag, value) in settings {
        let Some(axis) = axes.get_by_tag(tag) else {
            continue;
        };

        variations[axis.index()].value = value.clamp(axis.min_value(), axis.max_value());
    }

    let location = axes.location(
        variations
            .iter()
            .map(|variation| (variation.tag, variation.value)),
    );

    ensure!(
        location.coords() == normalized_coords,
        "native font coordinates differ from the shaped instance: design {variations:?}, native {:?}, shaped {normalized_coords:?}",
        location.coords()
    );

    Ok(variations)
}

fn canonical_index(id: FontId) -> Option<usize> {
    (id.0 & CANONICAL_FONT_ID_BIT != 0).then_some(id.0 & !CANONICAL_FONT_ID_BIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_fonts::SOURCE_SERIF;

    fn steep_avar_font_data() -> Vec<u8> {
        let mut data = SOURCE_SERIF.data.to_vec();
        let table_count = u16::from_be_bytes(data[4..6].try_into().unwrap()) as usize;
        let avar_offset = (0..table_count)
            .find_map(|table_idx| {
                let record = 12 + table_idx * 16;
                (&data[record..record + 4] == b"avar").then(|| {
                    u32::from_be_bytes(data[record + 8..record + 12].try_into().unwrap()) as usize
                })
            })
            .unwrap();
        assert_eq!(
            &data[avar_offset + 14..avar_offset + 18],
            &[0xe0, 0x00, 0xd7, 0x8e]
        );
        data[avar_offset + 16..avar_offset + 18].copy_from_slice(&(-15_565i16).to_be_bytes());

        data
    }

    #[test]
    fn design_variations_preserve_original_coordinates_and_reject_mismatches() {
        let steep_data = steep_avar_font_data();
        let weight = [FontVariation::new(*b"wght", 340.0)];
        let weight_and_size = [
            FontVariation::new(*b"wght", 340.0),
            FontVariation::new(*b"opsz", 48.0),
        ];
        let mismatch = [FontVariation::new(*b"wght", 341.0)];

        for (name, data, shaped, requested, valid) in [
            ("defaults", SOURCE_SERIF.data, &[][..], &[][..], true),
            ("weight", SOURCE_SERIF.data, &weight[..], &weight[..], true),
            (
                "multiple axes",
                SOURCE_SERIF.data,
                &weight_and_size[..],
                &weight_and_size[..],
                true,
            ),
            (
                "steep avar",
                steep_data.as_slice(),
                &weight[..],
                &weight[..],
                true,
            ),
            (
                "mismatch",
                SOURCE_SERIF.data,
                &weight[..],
                &mismatch[..],
                false,
            ),
        ] {
            let font = FontRef::new(data).unwrap();
            let axes = font.axes();
            let target = axes.location(
                shaped
                    .iter()
                    .map(|variation| (variation.tag, variation.value)),
            );
            let result =
                verified_design_variations(&font, target.coords(), Synthesis::default(), requested);
            assert_eq!(result.is_ok(), valid, "{name}: {result:?}");

            if !valid {
                continue;
            }

            let variations = result.unwrap();
            let actual = axes.location(
                variations
                    .iter()
                    .map(|variation| (variation.tag, variation.value)),
            );
            assert_eq!(actual.coords(), target.coords(), "{name}");

            for setting in requested {
                assert_eq!(
                    variations
                        .iter()
                        .find(|variation| variation.tag == setting.tag)
                        .map(|variation| variation.value),
                    Some(setting.value),
                    "{name}",
                );
            }
        }
    }

    #[test]
    fn interning_deduplicates_only_equivalent_font_instances() {
        let data = Blob::from(SOURCE_SERIF.data.to_vec());
        let mut store = FontStore::default();
        let first = store
            .intern_synthesized(data.clone(), 0, Synthesis::default())
            .unwrap();
        let duplicate = store
            .intern_synthesized(data.clone(), 0, Synthesis::default())
            .unwrap();
        assert_eq!(first, duplicate);

        let font = FontRef::new(data.as_ref()).unwrap();
        let explicit_defaults = vec![NormalizedCoord::default(); font.axes().len()];
        let equivalent = store
            .intern(
                data.clone(),
                0,
                &explicit_defaults,
                Synthesis::default(),
                &[],
            )
            .unwrap();
        assert_eq!(first, equivalent);
        assert!(
            store
                .intern(
                    data.clone(),
                    0,
                    &explicit_defaults,
                    Synthesis::default(),
                    &[FontVariation::new(*b"wght", 340.0)],
                )
                .is_err()
        );

        let varied_settings = [FontVariation::new(*b"wght", 700.0)];
        let varied_location = font.axes().location(
            varied_settings
                .iter()
                .map(|variation| (variation.tag, variation.value)),
        );
        let varied = store
            .intern(
                data.clone(),
                0,
                varied_location.coords(),
                Synthesis::default(),
                &varied_settings,
            )
            .unwrap();
        assert_ne!(first, varied);

        let copied_source = store
            .intern_synthesized(
                Blob::from(SOURCE_SERIF.data.to_vec()),
                0,
                Synthesis::default(),
            )
            .unwrap();
        assert_ne!(first, copied_source);
        assert!(store.get(first).is_some());
        assert!(store.get(FontId(0)).is_none());
    }
}
