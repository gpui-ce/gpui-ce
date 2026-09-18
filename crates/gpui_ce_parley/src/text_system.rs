use crate::{
    CatalogState, FaceFamily, FaceRequest, FontCatalog, FontStore, GlyphRasterizer,
    SwashGlyphRasterizer, SystemFonts,
};
use anyhow::{Context as _, Result};
use gpui::{
    Bounds, CaretAffinity, CaretPosition, Font, FontId, FontMetrics, GlyphId, InlineBoxRequest,
    InlineLayout, InlineLayoutRequest, InlineTextMetrics, InlineVisualLine, LineLayout,
    PaintFragment, PaintStyle, Pixels, PlatformTextLayout, PlatformTextSystem, PositionedInlineBox,
    PreparedRasterStyle, RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, ShapedGlyph, Size,
    TextAlign, TextLayoutRequest, TextMovement, TextRenderingMode, TextRun, TextSelectionKind,
    VisualDirection, VisualLine, align_inline_boxes, point, px, size,
};
use parking_lot::{Mutex, RwLock};
use parley::setting::Tag;
use parley::{
    Affinity, Alignment, AlignmentOptions, CHROMIUM_LINE_BREAK_OVERRIDE, Cluster, Cursor,
    FontContext, FontFamily, FontFamilyName, FontFeature, FontFeatures, FontStyle, FontWeight,
    GenericFamily, InlineBox, InlineBoxKind, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, Selection, StyleProperty,
};
use skrifa::instance::NormalizedCoord;
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation as _;

struct ParleyState {
    fonts: FontContext,
    layout: LayoutContext<PaintStyle>,
}

struct ParleyLayoutResult {
    layout: LineLayout,
    inline_lines: Vec<InlineVisualLine>,
    inline_boxes: Vec<PositionedInlineBox>,
    size: Size<Pixels>,
}

fn inline_alignment_offset(text_align: TextAlign, lines: &[InlineVisualLine]) -> Pixels {
    let Some(line) = lines.first() else {
        return Pixels::ZERO;
    };
    match text_align {
        TextAlign::Left => Pixels::ZERO,
        TextAlign::Center => line.origin.x + line.size.width / 2.,
        TextAlign::Right => line.origin.x + line.size.width,
    }
}

#[derive(Clone, Debug)]
struct ParleyLayout {
    layout: Layout<PaintStyle>,
    text_len: usize,
    caret_stops: Vec<ParleyCaretStop>,
    graphemes: Vec<std::ops::Range<usize>>,
}

#[derive(Clone, Copy, Debug)]
struct ParleyCaretStop {
    cursor: Cursor,
    block: f64,
    inline: f64,
}

impl ParleyLayout {
    fn new(layout: Layout<PaintStyle>, text: &str) -> Self {
        let graphemes = text
            .grapheme_indices(true)
            .map(|(start, grapheme)| start..start + grapheme.len())
            .collect::<Vec<_>>();
        let caret_stops = Self::collect_caret_stops(&layout, text, &graphemes);
        Self {
            layout,
            text_len: text.len(),
            caret_stops,
            graphemes,
        }
    }

    fn affinity(affinity: CaretAffinity) -> Affinity {
        match affinity {
            CaretAffinity::Downstream => Affinity::Downstream,
            CaretAffinity::Upstream => Affinity::Upstream,
        }
    }

    fn caret_position(cursor: Cursor) -> CaretPosition {
        CaretPosition::new(
            cursor.index(),
            match cursor.affinity() {
                Affinity::Downstream => CaretAffinity::Downstream,
                Affinity::Upstream => CaretAffinity::Upstream,
            },
        )
    }

    fn cursor(&self, caret: CaretPosition) -> Cursor {
        Cursor::from_byte_index(&self.layout, caret.index, Self::affinity(caret.affinity))
    }

    fn cursor_position(layout: &Layout<PaintStyle>, cursor: Cursor) -> (f64, f64) {
        let geometry = cursor.geometry(layout, 0.0);
        (geometry.y0, geometry.x0)
    }

    fn collect_caret_stops(
        layout: &Layout<PaintStyle>,
        text: &str,
        graphemes: &[std::ops::Range<usize>],
    ) -> Vec<ParleyCaretStop> {
        let boundaries = std::iter::once(0)
            .chain(graphemes.iter().map(|range| range.end))
            .chain(std::iter::once(text.len()));

        let mut stops = Vec::with_capacity((graphemes.len() + 2) * 2);
        for index in boundaries {
            for affinity in [Affinity::Downstream, Affinity::Upstream] {
                let cursor = Cursor::from_byte_index(layout, index, affinity);
                let (block, inline) = Self::cursor_position(layout, cursor);
                stops.push(ParleyCaretStop {
                    cursor,
                    block,
                    inline,
                });
            }
        }
        stops.sort_by(|left, right| {
            left.block
                .total_cmp(&right.block)
                .then_with(|| left.inline.total_cmp(&right.inline))
        });

        let mut unique_stops: Vec<ParleyCaretStop> = Vec::with_capacity(stops.len());
        for mut stop in stops {
            if let Some(previous) = unique_stops.last_mut()
                && previous.block == stop.block
                && previous.inline == stop.inline
            {
                let geometry = stop.cursor.geometry(layout, 0.0);
                let hit = Cursor::from_point(
                    layout,
                    stop.inline as f32,
                    ((geometry.y0 + geometry.y1) * 0.5) as f32,
                );
                if Self::cursor_position(layout, hit) == (stop.block, stop.inline) {
                    previous.cursor = hit;
                }
                continue;
            }

            let geometry = stop.cursor.geometry(layout, 0.0);
            let hit = Cursor::from_point(
                layout,
                stop.inline as f32,
                ((geometry.y0 + geometry.y1) * 0.5) as f32,
            );
            if Self::cursor_position(layout, hit) == (stop.block, stop.inline) {
                stop.cursor = hit;
            }
            unique_stops.push(stop);
        }
        unique_stops
    }

    fn caret_stop_index(&self, cursor: Cursor) -> Option<usize> {
        let (block, inline) = Self::cursor_position(&self.layout, cursor);
        self.caret_stops
            .binary_search_by(|stop| {
                stop.block
                    .total_cmp(&block)
                    .then_with(|| stop.inline.total_cmp(&inline))
            })
            .ok()
    }

    fn adjacent_caret_stop(
        &self,
        cursor: Cursor,
        direction: VisualDirection,
    ) -> Option<ParleyCaretStop> {
        let index = self.caret_stop_index(cursor)?;
        match direction {
            VisualDirection::Left => index
                .checked_sub(1)
                .and_then(|index| self.caret_stops.get(index)),
            VisualDirection::Right => self.caret_stops.get(index + 1),
        }
        .copied()
    }

    fn direct_visual_move(&self, cursor: Cursor, direction: VisualDirection) -> Cursor {
        match direction {
            VisualDirection::Left => cursor.previous_visual(&self.layout),
            VisualDirection::Right => cursor.next_visual(&self.layout),
        }
    }

    fn opposite(direction: VisualDirection) -> VisualDirection {
        match direction {
            VisualDirection::Left => VisualDirection::Right,
            VisualDirection::Right => VisualDirection::Left,
        }
    }

    fn native_y_for_line(&self, line_ix: usize) -> f32 {
        self.layout
            .get(line_ix)
            .map(|line| {
                let metrics = line.metrics();
                (metrics.block_min_coord + metrics.block_max_coord) * 0.5
            })
            .unwrap_or_else(|| self.layout.height())
    }
}

impl PlatformTextLayout for ParleyLayout {
    fn len(&self) -> usize {
        self.text_len
    }

    fn line_count(&self) -> usize {
        self.layout.len()
    }

    fn size(&self) -> Size<Pixels> {
        size(px(self.layout.width()), px(self.layout.height()))
    }

    fn index_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
    ) -> std::result::Result<usize, usize> {
        let closest = self
            .caret_from_point(point, line_height)
            .unwrap_or_else(|caret| caret)
            .index;
        if point.y < Pixels::ZERO || line_height <= Pixels::ZERO {
            return Err(closest);
        }
        let line_ix = (point.y / line_height) as usize;
        let Some(line) = self.layout.get(line_ix) else {
            return Err(closest);
        };
        let metrics = line.metrics();
        let left = metrics.inline_min_coord + metrics.offset;
        let right = left + metrics.advance;
        if f32::from(point.x) < left || f32::from(point.x) >= right {
            return Err(closest);
        }
        Cluster::from_point(
            &self.layout,
            point.x.into(),
            self.native_y_for_line(line_ix),
        )
        .map(|(cluster, _)| cluster.text_range().start)
        .ok_or(closest)
    }

    fn caret_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
    ) -> std::result::Result<CaretPosition, CaretPosition> {
        let line_ix = if line_height > px(0.0) && point.y >= Pixels::ZERO {
            (point.y / line_height) as usize
        } else {
            0
        };
        let caret = Self::caret_position(Cursor::from_point(
            &self.layout,
            point.x.into(),
            self.native_y_for_line(line_ix),
        ));
        let Some(line) = self.layout.get(line_ix) else {
            return Err(caret);
        };
        let metrics = line.metrics();
        let left = metrics.inline_min_coord + metrics.offset;
        let right = left + metrics.advance;
        if point.y >= Pixels::ZERO
            && line_height > Pixels::ZERO
            && f32::from(point.x) >= left
            && f32::from(point.x) < right
        {
            Ok(caret)
        } else {
            Err(caret)
        }
    }

    fn caret_geometry(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        if caret.index > self.len() {
            return None;
        }
        let cursor = self.cursor(caret);
        let geometry = cursor.geometry(&self.layout, 0.0);
        let line_ix = self
            .layout
            .lines()
            .position(|line| {
                let metrics = line.metrics();
                geometry.y0 as f32 >= metrics.block_min_coord
                    && (geometry.y0 as f32) < metrics.block_max_coord
            })
            .unwrap_or_else(|| self.layout.len().saturating_sub(1));
        Some(Bounds::from_corners(
            point(px(geometry.x0 as f32), line_height * line_ix),
            point(px(geometry.x1 as f32), line_height * (line_ix + 1)),
        ))
    }

    fn refresh_caret(&self, caret: CaretPosition) -> CaretPosition {
        Self::caret_position(self.cursor(caret))
    }

    fn move_visual(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let cursor = self.cursor(caret);
        let adjacent = self.adjacent_caret_stop(cursor, direction)?;
        let moved = self.direct_visual_move(cursor, direction);
        let moved_position = Self::cursor_position(&self.layout, moved);
        let inverse = self.direct_visual_move(moved, Self::opposite(direction));
        let inverse_position = Self::cursor_position(&self.layout, inverse);
        let current_position = Self::cursor_position(&self.layout, cursor);
        if moved_position == (adjacent.block, adjacent.inline)
            && inverse_position == current_position
        {
            Some(Self::caret_position(moved))
        } else {
            Some(Self::caret_position(adjacent.cursor))
        }
    }

    fn selection_geometry(
        &self,
        range: std::ops::Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        let anchor = Cursor::from_byte_index(&self.layout, range.start, Affinity::Downstream);
        let focus = Cursor::from_byte_index(&self.layout, range.end, Affinity::Upstream);
        Selection::new(anchor, focus)
            .geometry(&self.layout)
            .into_iter()
            .map(|(geometry, line_ix)| {
                Bounds::from_corners(
                    point(px(geometry.x0 as f32), line_height * line_ix),
                    point(px(geometry.x1 as f32), line_height * (line_ix + 1)),
                )
            })
            .collect()
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<std::ops::Range<usize>> {
        self.graphemes
            .iter()
            .rev()
            .find(|range| range.start < caret.index)
            .cloned()
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<std::ops::Range<usize>> {
        self.graphemes
            .iter()
            .find(|range| range.end > caret.index)
            .cloned()
    }

    fn move_caret(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> (CaretPosition, Option<Pixels>) {
        let cursor = self.cursor(caret);
        let moved = match movement {
            TextMovement::VisualLeft => {
                return (
                    self.move_visual(caret, VisualDirection::Left)
                        .unwrap_or(caret),
                    None,
                );
            }
            TextMovement::VisualRight => {
                return (
                    self.move_visual(caret, VisualDirection::Right)
                        .unwrap_or(caret),
                    None,
                );
            }
            TextMovement::VisualWordLeft => cursor.previous_visual_word(&self.layout),
            TextMovement::VisualWordRight => cursor.next_visual_word(&self.layout),
            TextMovement::VisualLineStart => Selection::from(cursor)
                .line_start(&self.layout, false)
                .focus(),
            TextMovement::VisualLineEnd => Selection::from(cursor)
                .line_end(&self.layout, false)
                .focus(),
            TextMovement::HardLineStart => Selection::from(cursor)
                .hard_line_start(&self.layout, false)
                .focus(),
            TextMovement::HardLineEnd => Selection::from(cursor)
                .hard_line_end(&self.layout, false)
                .focus(),
            TextMovement::VisualUp | TextMovement::VisualDown => {
                let delta = if movement == TextMovement::VisualUp {
                    -1
                } else {
                    1
                };
                let geometry = cursor.geometry(&self.layout, 0.0);
                let line_ix = self
                    .layout
                    .lines()
                    .position(|line| {
                        let metrics = line.metrics();
                        geometry.y0 as f32 >= metrics.block_min_coord
                            && (geometry.y0 as f32) < metrics.block_max_coord
                    })
                    .unwrap_or_else(|| self.layout.len().saturating_sub(1));
                let target_ix = line_ix
                    .checked_add_signed(delta)
                    .filter(|&target_ix| self.layout.get(target_ix).is_some());
                let Some(target_ix) = target_ix else {
                    let selection = Selection::from(cursor);
                    let moved = if delta < 0 {
                        selection.previous_line(&self.layout, false)
                    } else {
                        selection.next_line(&self.layout, false)
                    };
                    return (Self::caret_position(moved.focus()), preferred_x);
                };
                let x = preferred_x
                    .map_or_else(|| cursor.geometry(&self.layout, 0.0).x0 as f32, f32::from);
                let moved = Cursor::from_point(&self.layout, x, self.native_y_for_line(target_ix));
                return (Self::caret_position(moved), Some(px(x)));
            }
        };
        (Self::caret_position(moved), None)
    }

    fn selection_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> std::ops::Range<usize> {
        let line_ix = if line_height > Pixels::ZERO && point.y >= Pixels::ZERO {
            (point.y / line_height) as usize
        } else {
            0
        };
        let y = self.native_y_for_line(line_ix);
        match kind {
            TextSelectionKind::Word => Selection::word_from_point(&self.layout, point.x.into(), y),
            TextSelectionKind::VisualLine => {
                Selection::line_from_point(&self.layout, point.x.into(), y)
            }
            TextSelectionKind::HardLine => {
                Selection::hard_line_from_point(&self.layout, point.x.into(), y)
            }
        }
        .text_range()
    }
}

impl ParleyState {
    fn new(system_fonts: SystemFonts) -> (Self, FontCatalog) {
        let collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: true,
            system_fonts: system_fonts == SystemFonts::Load,
        });
        let source_cache = fontique::SourceCache::new_shared();
        let catalog = FontCatalog::from_shared(collection.clone(), source_cache.clone());
        (
            Self {
                fonts: FontContext {
                    collection,
                    source_cache,
                },
                layout: LayoutContext::new(),
            },
            catalog,
        )
    }
}

/// Shapes with Parley and paints exact font instances with an injected glyph rasterizer.
pub struct ParleyTextSystem {
    catalog: FontCatalog,
    fonts: RwLock<FontStore>,
    rasterizer: Mutex<Box<dyn GlyphRasterizer>>,
    parley: Mutex<ParleyState>,
    system_font_fallback: String,
    additional_fallbacks: Vec<String>,
}

impl ParleyTextSystem {
    /// Creates a text system using GPUI's default system-font family.
    pub fn new(system_fonts: SystemFonts) -> Self {
        Self::new_with_system_font(system_fonts, ".SystemUIFont")
    }

    /// Creates a text system with the concrete family used for GPUI's system-font alias.
    pub fn new_with_system_font(
        system_fonts: SystemFonts,
        system_font_fallback: impl Into<String>,
    ) -> Self {
        Self::new_with_rasterizer(
            system_fonts,
            system_font_fallback,
            SwashGlyphRasterizer::default(),
        )
    }

    /// Creates a text system which delegates only glyph rasterization to `rasterizer`.
    pub fn new_with_rasterizer(
        system_fonts: SystemFonts,
        system_font_fallback: impl Into<String>,
        rasterizer: impl GlyphRasterizer + 'static,
    ) -> Self {
        let (parley, catalog) = ParleyState::new(system_fonts);
        Self {
            catalog,
            fonts: RwLock::new(FontStore::default()),
            rasterizer: Mutex::new(Box::new(rasterizer)),
            parley: Mutex::new(parley),
            system_font_fallback: system_font_fallback.into(),
            additional_fallbacks: Vec::new(),
        }
    }

    /// Creates a deterministic text system without operating-system fonts.
    pub fn without_system_fonts() -> Self {
        Self::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans")
    }

    /// Adds fallback families after those supplied by each text style.
    pub fn with_fallback_families(
        mut self,
        families: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.additional_fallbacks = families.into_iter().map(Into::into).collect();
        self
    }

    fn resolve_canonical_font(&self, descriptor: &Font) -> Result<FontId> {
        let mut families = Vec::with_capacity(
            1 + descriptor
                .fallbacks
                .as_ref()
                .map_or(0, |fallbacks| fallbacks.fallback_list().len()),
        );
        push_face_families(
            &mut families,
            descriptor.family.as_ref(),
            &self.system_font_fallback,
        );
        if let Some(fallbacks) = &descriptor.fallbacks {
            for family in fallbacks.fallback_list() {
                push_face_families(&mut families, family, &self.system_font_fallback);
            }
        }
        for family in &self.additional_fallbacks {
            push_face_families(&mut families, family, &self.system_font_fallback);
        }
        let resolved = self
            .catalog
            .resolve(&FaceRequest {
                families: &families,
                weight: descriptor.weight.0,
                style: descriptor.style,
                character: None,
            })
            .with_context(|| format!("Fontique could not resolve '{}'", descriptor.family))?;
        let font_id = self.fonts.write().intern_synthesized(
            resolved.data,
            resolved.index,
            resolved.synthesis,
        )?;
        Ok(font_id)
    }

    fn parley_layout(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        wrap: Option<(Pixels, Option<usize>)>,
        inline_boxes: &[InlineBoxRequest],
        line_height: Option<Pixels>,
        inline_text_metrics: Option<InlineTextMetrics>,
        text_align: Option<TextAlign>,
    ) -> Result<ParleyLayoutResult> {
        let mut expected_start = 0usize;
        let mut run_ranges = Vec::with_capacity(runs.len());
        for run in runs {
            let Some(end) = expected_start.checked_add(run.len) else {
                anyhow::bail!("text run length overflowed the input range");
            };
            if end > text.len() {
                anyhow::bail!("text runs extend past the input text");
            }
            let range = expected_start..end;
            if !text.is_char_boundary(range.start) || !text.is_char_boundary(range.end) {
                anyhow::bail!("text runs do not align with the input text");
            }
            expected_start = range.end;
            run_ranges.push(range);
        }
        if expected_start != text.len() {
            anyhow::bail!("text runs do not cover the input text");
        }

        let family_lists = runs
            .iter()
            .map(|run| {
                let descriptor = &run.font;
                let mut families = Vec::new();
                push_parley_families(
                    &mut families,
                    descriptor.family.as_ref(),
                    &self.system_font_fallback,
                );
                if let Some(fallbacks) = &descriptor.fallbacks {
                    for family in fallbacks.fallback_list() {
                        push_parley_families(&mut families, family, &self.system_font_fallback);
                    }
                }
                families.extend(
                    self.additional_fallbacks
                        .iter()
                        .map(|family| FontFamilyName::Named(Cow::Borrowed(family.as_str()))),
                );
                families
            })
            .collect::<Vec<_>>();
        let feature_lists = runs
            .iter()
            .map(|run| {
                let descriptor = &run.font;
                descriptor
                    .features
                    .tag_value_list()
                    .iter()
                    .map(|(tag, value)| {
                        let tag = Tag::parse(tag)
                            .with_context(|| format!("invalid OpenType feature tag '{tag}'"))?;
                        let value = (*value).try_into().with_context(|| {
                            format!("OpenType feature '{tag}' value is larger than u16")
                        })?;
                        Ok(FontFeature::new(tag, value))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;

        let mut state = self.parley.lock();
        let ParleyState { fonts, layout } = &mut *state;
        let mut builder = layout.ranged_builder(fonts, text, 1.0, false);
        builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
        builder.push_default(StyleProperty::FontSize(f32::from(font_size)));
        if let Some(line_height) = line_height {
            builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(f32::from(
                line_height,
            ))));
        }
        for (run_index, run) in runs.iter().enumerate() {
            let descriptor = &run.font;
            let range = run_ranges[run_index].clone();
            builder.push(
                StyleProperty::FontFamily(FontFamily::from(family_lists[run_index].as_slice())),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(descriptor.weight.0)),
                range.clone(),
            );
            builder.push(
                StyleProperty::FontStyle(match descriptor.style {
                    gpui::FontStyle::Normal => FontStyle::Normal,
                    gpui::FontStyle::Italic => FontStyle::Italic,
                    gpui::FontStyle::Oblique => FontStyle::Oblique(None),
                }),
                range.clone(),
            );
            if !feature_lists[run_index].is_empty() {
                builder.push(
                    StyleProperty::FontFeatures(FontFeatures::from(
                        feature_lists[run_index].as_slice(),
                    )),
                    range.clone(),
                );
            }
            if let Some(letter_spacing) = run.letter_spacing {
                builder.push(
                    StyleProperty::LetterSpacing(f32::from(letter_spacing)),
                    range.clone(),
                );
            }
            let paint_style = PaintStyle::from(run);
            builder.push(StyleProperty::Brush(paint_style.clone()), range.clone());
            if let Some(underline) = run.underline {
                builder.push(StyleProperty::Underline(true), range.clone());
                builder.push(
                    StyleProperty::UnderlineSize(Some(underline.thickness.into())),
                    range.clone(),
                );
                builder.push(
                    StyleProperty::UnderlineBrush(Some(paint_style.clone())),
                    range.clone(),
                );
            }
            if let Some(strikethrough) = run.strikethrough {
                builder.push(StyleProperty::Strikethrough(true), range.clone());
                builder.push(
                    StyleProperty::StrikethroughSize(Some(strikethrough.thickness.into())),
                    range.clone(),
                );
                builder.push(
                    StyleProperty::StrikethroughBrush(Some(paint_style)),
                    range.clone(),
                );
            }
        }

        for inline_box in inline_boxes {
            if inline_box.index > text.len() || !text.is_char_boundary(inline_box.index) {
                anyhow::bail!("inline box index does not align with the input text");
            }
            builder.push_inline_box(InlineBox {
                id: inline_box.id,
                kind: InlineBoxKind::InFlow,
                index: inline_box.index,
                width: f32::from(inline_box.size.width),
                height: f32::from(inline_box.size.height),
            });
        }

        let mut layout = builder.build(text);
        if let Some((wrap_width, max_lines)) = wrap {
            if let Some(max_lines) = max_lines {
                let mut breaker = layout.break_lines();
                breaker.state_mut().set_layout_max_advance(f32::MAX);
                breaker
                    .state_mut()
                    .set_line_max_advance(f32::from(wrap_width));
                for _ in 0..max_lines.saturating_sub(1) {
                    if breaker.break_next().is_none() {
                        break;
                    }
                }
                breaker.break_remaining(f32::MAX);
            } else {
                layout.break_all_lines(Some(f32::from(wrap_width)));
            }
        } else {
            layout.break_all_lines(None);
        }
        if let Some(text_align) = text_align {
            let alignment = match text_align {
                TextAlign::Left => Alignment::Left,
                TextAlign::Center => Alignment::Center,
                TextAlign::Right => Alignment::Right,
            };
            layout.align(alignment, AlignmentOptions::default());
        }
        let mut visual_lines = Vec::new();
        let mut paint_fragments = Vec::new();
        let mut positioned_inline_boxes = Vec::new();
        let mut inline_lines = Vec::new();
        let mut inline_line_metrics = Vec::new();
        let mut width = px(0.0);
        let mut ascent = px(0.0);
        let mut descent = px(0.0);
        let mut saw_line = false;
        for (line_index, line) in layout.lines().enumerate() {
            saw_line = true;
            let fragment_start = paint_fragments.len();
            let metrics = *line.metrics();
            let line_x = px(metrics.inline_min_coord + metrics.offset);
            let mut text_metrics = inline_text_metrics.unwrap_or_default();
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    let PositionedLayoutItem::InlineBox(inline_box) = item else {
                        unreachable!();
                    };
                    positioned_inline_boxes.push(PositionedInlineBox {
                        id: inline_box.id,
                        line_index,
                        bounds: Bounds::new(
                            point(px(inline_box.x), px(inline_box.y)),
                            size(px(inline_box.width), px(inline_box.height)),
                        ),
                    });
                    continue;
                };
                let run = glyph_run.run();
                let normalized_coords = run
                    .normalized_coords()
                    .iter()
                    .copied()
                    .map(NormalizedCoord::from_bits)
                    .collect::<Vec<_>>();
                let font_id = self.fonts.write().intern(
                    run.font().data.clone(),
                    run.font().index,
                    &normalized_coords,
                    run.synthesis(),
                )?;
                let baseline = glyph_run.baseline();
                let run_metrics = glyph_run.run().metrics();
                text_metrics.ascent = text_metrics.ascent.max(px(run_metrics.ascent));
                text_metrics.descent = text_metrics.descent.max(px(run_metrics.descent));
                let parley_style = glyph_run.style();
                let mut paint_style = parley_style.brush.clone();
                let underline_offset = parley_style.underline.as_ref().map(|decoration| {
                    let mut underline = decoration.brush.underline.unwrap_or_default();
                    underline.thickness = px(decoration.size.unwrap_or(run_metrics.underline_size));
                    paint_style.underline = Some(underline);
                    px(decoration.offset.unwrap_or(run_metrics.underline_offset))
                });
                let strikethrough_offset = parley_style.strikethrough.as_ref().map(|decoration| {
                    let mut strikethrough = decoration.brush.strikethrough.unwrap_or_default();
                    strikethrough.thickness =
                        px(decoration.size.unwrap_or(run_metrics.strikethrough_size));
                    paint_style.strikethrough = Some(strikethrough);
                    px(decoration
                        .offset
                        .unwrap_or(run_metrics.strikethrough_offset))
                });
                let glyphs = {
                    let fonts = self.fonts.read();
                    let color_glyphs = fonts
                        .get(font_id)
                        .context("canonical font missing after interning")?
                        .color_glyphs()?;
                    let rasterizer = self.rasterizer.lock();
                    glyph_run
                        .positioned_glyphs()
                        .map(|glyph| {
                            let id = GlyphId(glyph.id);
                            ShapedGlyph {
                                id,
                                position: point(px(glyph.x) - line_x, px(glyph.y - baseline)),
                                is_emoji: color_glyphs
                                    .kind(id)
                                    .is_some_and(|kind| rasterizer.supports_color_glyph(kind)),
                            }
                        })
                        .collect()
                };
                let start = px(glyph_run.offset()) - line_x;
                paint_fragments.push(PaintFragment {
                    font_id,
                    glyphs,
                    x_range: start..start + px(glyph_run.advance()),
                    style: paint_style,
                    underline_offset,
                    strikethrough_offset,
                });
            }

            let parley_text_range = line.text_range();
            let text_range =
                parley_text_range.start.min(text.len())..parley_text_range.end.min(text.len());

            visual_lines.push(VisualLine {
                text_range,
                fragment_range: fragment_start..paint_fragments.len(),
                advance: px(metrics.advance),
            });
            inline_lines.push(InlineVisualLine {
                origin: point(line_x, px(metrics.block_min_coord)),
                size: size(
                    px(metrics.advance),
                    px(metrics.block_max_coord - metrics.block_min_coord),
                ),
                baseline: px(metrics.baseline - metrics.block_min_coord),
            });
            inline_line_metrics.push(text_metrics);
            width = width.max(px(metrics.advance));
            ascent = ascent.max(px(metrics.ascent));
            descent = descent.max(px(metrics.descent));
        }
        if !saw_line {
            anyhow::bail!("Parley produced no line");
        }

        let platform_layout = ParleyLayout::new(layout.clone(), text);
        let mut size = size(px(layout.width()), px(layout.height()));
        if let (Some(text_metrics), Some(line_height)) = (inline_text_metrics, line_height) {
            align_inline_boxes(
                &mut inline_lines,
                &mut positioned_inline_boxes,
                &mut size,
                inline_boxes,
                &inline_line_metrics,
                text_metrics,
                line_height,
            );
        }
        let line_layout = LineLayout {
            font_size,
            width,
            ascent,
            descent,
            visual_lines: visual_lines.iter().cloned().collect(),
            paint_fragments,
            len: text.len(),
            platform_layout: std::sync::Arc::new(platform_layout),
        };
        Ok(ParleyLayoutResult {
            layout: line_layout,
            inline_lines,
            inline_boxes: positioned_inline_boxes,
            size,
        })
    }
}

fn push_face_families<'a>(
    families: &mut Vec<FaceFamily<'a>>,
    name: &'a str,
    system_font_fallback: &'a str,
) {
    if name == ".SystemUIFont" {
        families.push(FaceFamily::SystemUi);
        families.push(FaceFamily::Named(system_font_fallback));
    } else {
        families.push(FaceFamily::Named(canonical_family(
            name,
            system_font_fallback,
        )));
    }
}

fn push_parley_families<'a>(
    families: &mut Vec<FontFamilyName<'a>>,
    name: &'a str,
    system_font_fallback: &str,
) {
    if name == ".SystemUIFont" {
        families.push(FontFamilyName::Generic(GenericFamily::SystemUi));
        families.push(FontFamilyName::Named(Cow::Owned(
            system_font_fallback.to_string(),
        )));
    } else {
        families.push(FontFamilyName::Named(Cow::Owned(
            canonical_family(name, system_font_fallback).to_string(),
        )));
    }
}

fn canonical_family<'a>(name: &'a str, system: &'a str) -> &'a str {
    match name {
        ".SystemUIFont" => system,
        ".ZedSans" | "Zed Plex Sans" => "IBM Plex Sans",
        ".ZedMono" | "Zed Plex Mono" => "Lilex",
        _ => name,
    }
}

impl PlatformTextSystem for ParleyTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let blobs = fonts
            .iter()
            .map(|bytes| fontique::Blob::from(bytes.as_ref().to_vec()))
            .collect::<Vec<_>>();
        let mut state = self.catalog.state.write();
        let mut next: CatalogState = state.clone();
        next.register_blobs(&blobs)?;
        *state = next;
        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names = self.catalog.family_names();
        names.extend([".SystemUIFont", ".ZedSans", ".ZedMono"].map(str::to_owned));
        names.sort_unstable();
        names.dedup();
        names
    }

    fn font_generation(&self) -> u64 {
        self.catalog.generation()
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.resolve_canonical_font(descriptor)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.fonts
            .read()
            .get(font_id)
            .expect("Parley FontId missing from its store")
            .metrics()
            .expect("stored font failed Skrifa metrics")
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.fonts
            .read()
            .get(font_id)
            .context("Parley FontId missing from its store")?
            .glyph_bounds(glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.fonts
            .read()
            .get(font_id)
            .context("Parley FontId missing from its store")?
            .advance(glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.fonts.read().get(font_id)?.glyph_for_char(ch).ok()?
    }

    fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        let fonts = self.fonts.read();
        let font = fonts
            .get(params.font_id)
            .context("Parley FontId missing from its store")?;
        let data_identity = font.data_identity();
        let face_index = font.index;
        let variations = font.variations.clone();
        self.rasterizer
            .lock()
            .rasterize(font.raster_face(params.font_id), params)
            .with_context(|| {
                format!(
                    "native rasterization failed for FontId {:?}, data identity {data_identity}, face index {face_index}, variations {variations:?}",
                    params.font_id
                )
            })
    }

    fn prepare_raster_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        self.rasterizer.lock().prepare_style(request)
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        self.rasterizer.lock().recommended_mode()
    }

    fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
        let wrap = (request.wrap_width.is_some() || request.line_clamp.is_some()).then_some((
            request.wrap_width.unwrap_or(Pixels::MAX),
            request.line_clamp,
        ));
        self.parley_layout(
            request.text,
            request.font_size,
            request.runs,
            wrap,
            &[],
            None,
            None,
            None,
        )
        .expect("Parley failed to lay out a validated GPUI document")
        .layout
    }

    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        let wrap = (request.wrap_width.is_some() || request.line_clamp.is_some()).then_some((
            request.wrap_width.unwrap_or(Pixels::MAX),
            request.line_clamp,
        ));
        let result = self
            .parley_layout(
                request.text,
                request.font_size,
                request.runs,
                wrap,
                request.boxes,
                Some(request.line_height),
                Some(request.text_metrics),
                Some(request.text_align),
            )
            .expect("Parley failed to lay out a validated GPUI inline document");
        InlineLayout {
            layout: std::sync::Arc::new(result.layout),
            alignment_offset: inline_alignment_offset(request.text_align, &result.inline_lines),
            lines: result.inline_lines,
            boxes: result.inline_boxes,
            size: result.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FontSynthesis, FontVariation, RasterFace};
    use gpui::{
        CaretSelection, FontFallbacks, FontFeatures as GpuiFontFeatures, FontStyle, FontWeight,
        GlyphRenderMode, RasterizedGlyphFormat, StrikethroughStyle, TextSystem, UnderlineStyle,
        VerticalAlign, WindowTextSystem, font, hsla,
    };
    use std::sync::Arc;

    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const IBM_PLEX_SEMIBOLD: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf");
    const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");
    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");
    const NOTO_COLOR_EMOJI: &[u8] =
        include_bytes!("../../../assets/fonts/noto-color-emoji/NotoColorEmoji.subset.ttf");

    fn test_system() -> Arc<ParleyTextSystem> {
        let system = Arc::new(
            ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans")
                .with_fallback_families([
                    "IBM Plex Sans",
                    "Lilex",
                    "Source Serif 4",
                    "Noto Color Emoji",
                ]),
        );
        system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD),
                Cow::Borrowed(LILEX),
                Cow::Borrowed(SOURCE_SERIF),
                Cow::Borrowed(NOTO_COLOR_EMOJI),
            ])
            .unwrap();
        system
    }

    fn text_run(text: &str, family: &str) -> TextRun {
        TextRun {
            len: text.len(),
            font: font(family),
            ..Default::default()
        }
    }

    fn layout_line(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size,
            runs,
            wrap_width: None,
            line_clamp: None,
        })
    }

    fn layout_wrapped(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Pixels,
        line_clamp: Option<usize>,
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size,
            runs,
            wrap_width: Some(wrap_width),
            line_clamp,
        })
    }

    fn wrapped(layout: LineLayout, width: Pixels) -> gpui::WrappedLineLayout {
        gpui::WrappedLineLayout {
            layout: Arc::new(layout),
            wrap_width: Some(width),
        }
    }

    fn positioned_box_bounds(
        layout: &InlineLayout,
        inline_box: &PositionedInlineBox,
    ) -> Bounds<Pixels> {
        assert!(
            inline_box.line_index < layout.lines.len(),
            "inline box {} refers to missing line {}",
            inline_box.id,
            inline_box.line_index
        );
        inline_box.bounds
    }

    fn assert_inline_geometry_is_contained(layout: &InlineLayout, width: Pixels) {
        let epsilon = px(0.01);
        for (line_index, line) in layout.lines.iter().enumerate() {
            assert!(
                line.origin.x + line.size.width <= width + epsilon,
                "line {line_index} extends past the available width"
            );
            assert!(
                line.origin.y + line.size.height <= layout.size.height + epsilon,
                "line {line_index} extends past the layout height"
            );
        }
        for inline_box in &layout.boxes {
            let bounds = positioned_box_bounds(layout, inline_box);
            let line = layout.lines[inline_box.line_index];
            assert!(
                bounds.right() <= width + epsilon,
                "inline box {} extends past the available width",
                inline_box.id
            );
            assert!(
                bounds.origin.y + epsilon >= line.origin.y
                    && bounds.bottom() <= line.origin.y + line.size.height + epsilon,
                "inline box {} is outside its assigned line",
                inline_box.id
            );
        }
    }

    fn assert_document_contract(text: &str, layout: &LineLayout) {
        assert_eq!(layout.len, text.len());
        assert!(!layout.visual_lines.is_empty());
        assert_eq!(layout.visual_lines[0].text_range.start, 0);
        assert_eq!(
            layout.visual_lines.last().unwrap().text_range.end,
            text.len()
        );
        assert_eq!(layout.visual_lines[0].fragment_range.start, 0);
        assert_eq!(
            layout.visual_lines.last().unwrap().fragment_range.end,
            layout.paint_fragments.len()
        );

        for pair in layout.visual_lines.windows(2) {
            assert_eq!(pair[0].text_range.end, pair[1].text_range.start);
            assert_eq!(pair[0].fragment_range.end, pair[1].fragment_range.start);
        }
        for line in &layout.visual_lines {
            assert!(text.is_char_boundary(line.text_range.start));
            assert!(text.is_char_boundary(line.text_range.end));
            assert!(f32::from(line.advance).is_finite() && line.advance >= Pixels::ZERO);
            for fragment in &layout.paint_fragments[line.fragment_range.clone()] {
                assert!(f32::from(fragment.x_range.start).is_finite());
                assert!(f32::from(fragment.x_range.end).is_finite());
                assert!(fragment.x_range.start <= fragment.x_range.end);
                assert!(fragment.x_range.start >= -layout.font_size * 2.0);
                assert!(fragment.x_range.end <= line.advance + layout.font_size * 2.0);
                for glyph in &fragment.glyphs {
                    assert!(f32::from(glyph.position.x).is_finite());
                    assert!(f32::from(glyph.position.y).is_finite());
                }
            }
        }

        let line_height = px(24.0);
        let wrapped = wrapped(layout.clone_for_test(), px(120.0));
        let mut caret = wrapped
            .closest_caret_for_position(point(px(-100.0), line_height * 0.5), line_height)
            .unwrap_err();
        let mut seen = Vec::new();
        let max_steps = text.chars().count() * 4 + wrapped.line_count() * 4 + 8;
        for _ in 0..max_steps {
            assert!(text.is_char_boundary(caret.index));
            assert!(!seen.contains(&caret), "visual caret traversal cycled");
            seen.push(caret);
            let Some(next) = wrapped.next_visual_caret(caret) else {
                break;
            };
            caret = next;
        }
        assert!(wrapped.next_visual_caret(caret).is_none());
        for _ in 0..max_steps {
            let Some(previous) = wrapped.previous_visual_caret(caret) else {
                break;
            };
            caret = previous;
        }
        assert!(wrapped.previous_visual_caret(caret).is_none());

        for caret in seen {
            let bounds = wrapped
                .position_for_caret(caret, line_height)
                .expect("native caret must have geometry");
            assert!(f32::from(bounds.x).is_finite() && f32::from(bounds.y).is_finite());
        }
        if text.chars().any(|character| !character.is_whitespace()) {
            assert!(
                !wrapped
                    .selection_bounds(0..text.len(), line_height)
                    .is_empty()
            );
        }
    }

    #[test]
    fn inline_layout_flows_boxes_with_styled_wrapped_text() {
        let system = test_system();
        let text = "alpha beta gamma delta";
        let split = "alpha beta ".len();
        let first_color = hsla(0.0, 0.8, 0.4, 1.0);
        let second_color = hsla(0.6, 0.8, 0.4, 1.0);
        let runs = [
            TextRun {
                len: split,
                color: first_color,
                font: font("IBM Plex Sans"),
                ..Default::default()
            },
            TextRun {
                len: text.len() - split,
                color: second_color,
                font: font("Source Serif 4"),
                ..Default::default()
            },
        ];
        let boxes = [
            InlineBoxRequest {
                id: 7,
                index: "alpha ".len(),
                size: size(px(28.0), px(32.0)),
                vertical_align: VerticalAlign::Baseline,
            },
            InlineBoxRequest {
                id: 9,
                index: split,
                size: size(px(18.0), px(14.0)),
                vertical_align: VerticalAlign::Middle,
            },
            InlineBoxRequest {
                id: 11,
                index: "alpha beta gamma ".len(),
                size: size(px(20.0), px(18.0)),
                vertical_align: VerticalAlign::Top,
            },
            InlineBoxRequest {
                id: 13,
                index: "alpha beta gamma ".len(),
                size: size(px(16.0), px(28.0)),
                vertical_align: VerticalAlign::Bottom,
            },
        ];
        let text_metrics = InlineTextMetrics {
            ascent: px(14.0),
            descent: px(4.0),
            x_height: px(8.0),
        };

        let request = InlineLayoutRequest {
            text,
            runs: &runs,
            boxes: &boxes,
            font_size: px(18.0),
            line_height: px(24.0),
            text_metrics,
            wrap_width: Some(px(160.0)),
            line_clamp: None,
            text_align: TextAlign::Center,
        };
        let layout = system.layout_inline(request);

        assert!(layout.lines.len() >= 2);
        assert_eq!(layout.lines.len(), layout.layout.visual_lines.len());
        assert_eq!(
            layout
                .boxes
                .iter()
                .map(|inline_box| inline_box.id)
                .collect::<Vec<_>>(),
            vec![7, 9, 11, 13]
        );
        assert!(layout.size.width <= px(160.0));
        assert!(layout.lines.iter().any(|line| line.origin.x > Pixels::ZERO));
        assert!(
            layout
                .layout
                .paint_fragments
                .iter()
                .any(|fragment| fragment.style.color == first_color)
        );
        assert!(
            layout
                .layout
                .paint_fragments
                .iter()
                .any(|fragment| fragment.style.color == second_color)
        );

        let box_and_line = |id| {
            let inline_box = layout
                .boxes
                .iter()
                .find(|inline_box| inline_box.id == id)
                .unwrap();
            let line = &layout.lines[inline_box.line_index];
            let bounds = positioned_box_bounds(&layout, inline_box);
            (bounds, line)
        };
        let (baseline_box, baseline_line) = box_and_line(7);
        assert!(
            (baseline_box.bottom() - (baseline_line.origin.y + baseline_line.baseline)).abs()
                < px(0.01)
        );
        let (middle_box, middle_line) = box_and_line(9);
        assert!(
            (middle_box.center().y
                - (middle_line.origin.y + middle_line.baseline - text_metrics.x_height / 2.))
                .abs()
                < px(0.01)
        );
        let (top_box, top_line) = box_and_line(11);
        assert!((top_box.origin.y - top_line.origin.y).abs() < px(0.01));
        let (bottom_box, bottom_line) = box_and_line(13);
        assert!(
            (bottom_box.bottom() - (bottom_line.origin.y + bottom_line.size.height)).abs()
                < px(0.01)
        );

        for lines in layout.lines.windows(2) {
            assert!(lines[0].origin.y + lines[0].size.height <= lines[1].origin.y);
        }
        assert_inline_geometry_is_contained(&layout, px(160.0));
    }

    #[test]
    fn inline_layout_supports_documents_containing_only_boxes() {
        let system = test_system();
        let boxes = [
            InlineBoxRequest {
                id: 1,
                index: 0,
                size: size(px(30.0), px(12.0)),
                vertical_align: VerticalAlign::Baseline,
            },
            InlineBoxRequest {
                id: 2,
                index: 0,
                size: size(px(20.0), px(36.0)),
                vertical_align: VerticalAlign::Middle,
            },
        ];

        let layout = system.layout_inline(InlineLayoutRequest {
            text: "",
            runs: &[],
            boxes: &boxes,
            font_size: px(18.0),
            line_height: px(24.0),
            text_metrics: InlineTextMetrics {
                ascent: px(14.0),
                descent: px(4.0),
                x_height: px(8.0),
            },
            wrap_width: Some(px(24.0)),
            line_clamp: None,
            text_align: TextAlign::Left,
        });

        assert_eq!(layout.boxes.len(), 2);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.layout.paint_fragments.len(), 0);
        assert_eq!(
            layout
                .boxes
                .iter()
                .map(|inline_box| inline_box.line_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(layout.size.width, px(30.0));
        assert!(layout.size.height >= px(48.0));
        assert_inline_geometry_is_contained(&layout, layout.size.width);
    }

    trait CloneLineLayoutForTest {
        fn clone_for_test(&self) -> LineLayout;
    }

    impl CloneLineLayoutForTest for LineLayout {
        fn clone_for_test(&self) -> LineLayout {
            LineLayout {
                font_size: self.font_size,
                width: self.width,
                ascent: self.ascent,
                descent: self.descent,
                visual_lines: self.visual_lines.clone(),
                paint_fragments: self.paint_fragments.clone(),
                len: self.len,
                platform_layout: self.platform_layout.clone(),
            }
        }
    }

    #[test]
    fn document_layout_contract_covers_scripts_breaks_wrapping_and_clamping() {
        let system = test_system();
        let cases = [
            ("empty", "", px(120.0), None, 1usize),
            ("hard breaks", "one\ntwo\nthree", px(500.0), None, 3),
            ("trailing break", "one\n", px(500.0), None, 2),
            ("latin wrap", "one two three four five", px(54.0), None, 2),
            (
                "clamped",
                "one two three four five six",
                px(45.0),
                Some(2),
                2,
            ),
            ("mixed bidi", "abc אבג العربية xyz", px(500.0), None, 1),
            ("cjk", "日本語中文テキスト", px(500.0), None, 1),
            ("thai", "ภาษาไทย ก้ กี", px(500.0), None, 1),
            ("ligatures", "office affine ffi", px(500.0), None, 1),
            ("emoji", "👩🏽‍💻 family 👨‍👩‍👧‍👦 🇬🇧", px(500.0), None, 1),
        ];

        for (name, text, width, max_lines, minimum_lines) in cases {
            let runs = (!text.is_empty())
                .then(|| text_run(text, "IBM Plex Sans"))
                .into_iter()
                .collect::<Vec<_>>();
            let layout = layout_wrapped(&system, text, px(18.0), &runs, width, max_lines);
            assert_document_contract(text, &layout);
            if max_lines.is_some() {
                assert_eq!(layout.visual_lines.len(), minimum_lines, "{name}");
            } else {
                assert!(layout.visual_lines.len() >= minimum_lines, "{name}");
            }
        }

        let layout = layout_line(&system, "", px(18.0), &[]);
        assert_document_contract("", &layout);
        assert_eq!(layout.visual_lines[0].text_range, 0..0);
    }

    #[test]
    fn wrapping_does_not_move_breaks_forward_as_width_shrinks() {
        let system = test_system();
        let cases = [
            (
                "code punctuation",
                "Lilex regular: fn main() { println!(\"hello\"); }",
                "Lilex",
            ),
            (
                "prose punctuation",
                "One sentence with punctuation, followed by another.",
                "IBM Plex Sans",
            ),
            (
                "nested delimiters",
                "call(value, other_value) } trailing",
                "IBM Plex Sans",
            ),
            (
                "mixed scripts",
                "English العربية 日本語 punctuation.",
                "IBM Plex Sans",
            ),
        ];

        for (name, text, family) in cases {
            let runs = [text_run(text, family)];
            let mut previous_end = text.len();
            for half_width in (200..=1200).rev() {
                let width = px(half_width as f32 / 2.0);
                let layout = layout_wrapped(&system, text, px(16.0), &runs, width, None);
                let first_line_end = layout.visual_lines[0].text_range.end;
                assert!(
                    first_line_end <= previous_end,
                    "{name}: first break moved forward at {width:?}: {previous_end} -> {first_line_end}"
                );
                previous_end = first_line_end;
            }
        }
    }

    #[test]
    fn styled_document_uses_parley_paint_runs_without_changing_geometry() {
        let system = test_system();
        let text = "office café العربية";
        let first_end = "office ".len();
        let second_end = first_end + "café ".len();
        let mut first_font = font("Source Serif 4");
        first_font.features = GpuiFontFeatures::disable_ligatures();
        first_font.fallbacks = Some(FontFallbacks::from_fonts(vec![
            "IBM Plex Sans".into(),
            "Noto Color Emoji".into(),
        ]));
        let base_runs = vec![
            TextRun {
                len: first_end,
                font: first_font,
                letter_spacing: Some(px(0.4)),
                color: hsla(0.0, 0.8, 0.4, 1.0),
                ..Default::default()
            },
            TextRun {
                len: second_end - first_end,
                font: font("IBM Plex Sans").bold(),
                color: hsla(0.35, 0.7, 0.35, 1.0),
                ..Default::default()
            },
            TextRun {
                len: text.len() - second_end,
                font: font("IBM Plex Sans"),
                color: hsla(0.6, 0.8, 0.45, 1.0),
                ..Default::default()
            },
        ];
        let mut decorated_runs = base_runs.clone();
        decorated_runs[0].background_color = Some(hsla(0.1, 0.5, 0.5, 0.3));
        decorated_runs[0].underline = Some(UnderlineStyle {
            thickness: px(1.5),
            color: None,
            wavy: true,
        });
        decorated_runs[1].strikethrough = Some(StrikethroughStyle {
            thickness: px(1.0),
            color: None,
        });

        let plain = layout_line(&system, text, px(20.0), &base_runs);
        let decorated = layout_line(&system, text, px(20.0), &decorated_runs);
        let geometry = |layout: &LineLayout| {
            layout
                .paint_fragments
                .iter()
                .flat_map(|fragment| {
                    fragment
                        .glyphs
                        .iter()
                        .map(move |glyph| (fragment.font_id, glyph.id, glyph.position))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(plain.width, decorated.width);
        assert_eq!(geometry(&plain), geometry(&decorated));
        assert!(decorated.paint_fragments.iter().any(|fragment| {
            fragment.style.background_color.is_some() && fragment.style.underline.is_some()
        }));
        assert!(
            decorated
                .paint_fragments
                .iter()
                .any(|fragment| fragment.style.strikethrough.is_some())
        );
    }

    #[test]
    fn font_registration_invalidates_layouts_and_distinguishes_requested_instances() {
        let backend = Arc::new(
            ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans")
                .with_fallback_families(["IBM Plex Sans"]),
        );
        backend.add_fonts(vec![Cow::Borrowed(IBM_PLEX)]).unwrap();
        let text_system = Arc::new(TextSystem::new(backend.clone()));
        let window_text_system = WindowTextSystem::new(text_system);
        let text = "registered later";
        let run = text_run(text, "Source Serif 4");
        let before = window_text_system
            .shape_text(text, px(18.0), std::slice::from_ref(&run), None, None)
            .unwrap();
        let fallback_id = before.paint_fragments[0].font_id;

        backend
            .add_fonts(vec![Cow::Borrowed(SOURCE_SERIF)])
            .unwrap();
        let after = window_text_system
            .shape_text(text, px(18.0), &[run], None, None)
            .unwrap();
        assert_ne!(fallback_id, after.paint_fragments[0].font_id);

        backend
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SEMIBOLD)])
            .unwrap();
        let regular = backend.font_id(&font("IBM Plex Sans")).unwrap();
        let bold = backend.font_id(&font("IBM Plex Sans").bold()).unwrap();
        assert_ne!(regular, bold);

        let mut variable = font("Source Serif 4");
        variable.weight = FontWeight(725.0);
        variable.style = FontStyle::Oblique;
        let variable_id = backend.font_id(&variable).unwrap();
        let source_serif_regular = backend.font_id(&font("Source Serif 4")).unwrap();
        assert_ne!(source_serif_regular, variable_id);
    }

    #[test]
    fn native_interaction_handles_bidi_atomic_clusters_and_semantic_selection() {
        let system = test_system();
        for text in ["👩🏽‍💻", "👨‍👩‍👧‍👦", "🇬🇧", "ก้"] {
            let layout = layout_line(&system, text, px(22.0), &[text_run(text, "IBM Plex Sans")]);
            let wrapped = wrapped(layout, px(500.0));
            assert_eq!(
                wrapped.logical_cluster_after(CaretPosition::default()),
                Some(0..text.len()),
                "{text:?} must be one editable cluster"
            );
        }

        let single_line_text = "abcd";
        let single_line = wrapped(
            layout_line(
                &system,
                single_line_text,
                px(20.0),
                &[text_run(single_line_text, "IBM Plex Sans")],
            ),
            px(500.0),
        );
        let middle = CaretPosition::new(2, CaretAffinity::Downstream);
        assert_eq!(
            single_line
                .move_caret(middle, TextMovement::VisualUp, None)
                .0
                .index,
            0
        );
        assert_eq!(
            single_line
                .move_caret(middle, TextMovement::VisualDown, None)
                .0
                .index,
            single_line_text.len()
        );

        let text = "abc אבג العربية xyz";
        let layout = wrapped(
            layout_wrapped(
                &system,
                text,
                px(20.0),
                &[text_run(text, "IBM Plex Sans")],
                px(90.0),
                None,
            ),
            px(90.0),
        );
        let line_height = px(26.0);
        let start = layout
            .closest_caret_for_position(point(px(-10.0), px(10.0)), line_height)
            .unwrap_err();
        let end = layout
            .closest_caret_for_position(point(px(10_000.0), px(10.0)), line_height)
            .unwrap_err();
        let selection = CaretSelection::new(end, start);
        let collapsed_left = layout.move_selection(
            selection,
            TextMovement::VisualLeft,
            false,
            None,
            line_height,
        );
        assert!(collapsed_left.selection.is_empty());
        assert_eq!(collapsed_left.selection.focus, start);
        let collapsed_right = layout.move_selection(
            selection,
            TextMovement::VisualRight,
            false,
            None,
            line_height,
        );
        assert_eq!(collapsed_right.selection.focus, end);

        let word = layout.move_selection(
            CaretSelection::collapsed(start),
            TextMovement::VisualWordRight,
            true,
            None,
            line_height,
        );
        assert_eq!(word.selection.anchor, start);
        assert_ne!(word.selection.focus, start);
        let down = layout.move_selection(
            CaretSelection::collapsed(word.selection.focus),
            TextMovement::VisualDown,
            false,
            None,
            line_height,
        );
        assert!(down.preferred_x.is_some());
        let maintained_x = layout
            .move_selection(
                down.selection,
                TextMovement::VisualDown,
                false,
                down.preferred_x,
                line_height,
            )
            .preferred_x;
        assert_eq!(maintained_x, down.preferred_x);
        let selection = layout.selection_from_point(
            point(px(12.0), px(10.0)),
            line_height,
            TextSelectionKind::Word,
        );
        assert!(!selection.is_empty());
        assert!(text.is_char_boundary(selection.start));
        assert!(text.is_char_boundary(selection.end));
    }

    #[test]
    fn rasterizes_monochrome_and_color_glyphs_with_expected_buffer_formats() {
        let system = test_system();

        let mixed_color_face = layout_line(
            &system,
            "1 1",
            px(24.0),
            &[text_run("1 1", "Noto Color Emoji")],
        );
        let color_flags = mixed_color_face
            .paint_fragments
            .iter()
            .flat_map(|fragment| fragment.glyphs.iter().map(|glyph| glyph.is_emoji))
            .collect::<Vec<_>>();
        assert_eq!(color_flags, [true, false, true]);

        for (text, mode, expected_format) in [
            (
                "A",
                GlyphRenderMode::Grayscale,
                RasterizedGlyphFormat::AlphaMask,
            ),
            (
                "A",
                GlyphRenderMode::Subpixel,
                RasterizedGlyphFormat::BgraSubpixelMask,
            ),
            (
                "😀",
                GlyphRenderMode::Color,
                RasterizedGlyphFormat::BgraColor,
            ),
        ] {
            let expect_color = mode == GlyphRenderMode::Color;
            let layout = layout_line(&system, text, px(24.0), &[text_run(text, "IBM Plex Sans")]);
            let glyph = layout
                .paint_fragments
                .iter()
                .flat_map(|fragment| {
                    fragment
                        .glyphs
                        .iter()
                        .map(move |glyph| (fragment.font_id, glyph))
                })
                .find(|(_, glyph)| glyph.is_emoji == expect_color)
                .unwrap();
            let raster = system
                .rasterize_glyph(&RenderGlyphParams {
                    font_id: glyph.0,
                    glyph_id: glyph.1.id,
                    font_size: px(24.0),
                    subpixel_variant: Default::default(),
                    scale_factor: 1.0,
                    raster_style: PreparedRasterStyle::independent(mode),
                })
                .unwrap();
            assert_eq!(raster.bounds.size, raster.size);
            assert!(raster.size.width.0 > 0 && raster.size.height.0 > 0);
            let channels = match raster.format {
                gpui::RasterizedGlyphFormat::AlphaMask => 1,
                gpui::RasterizedGlyphFormat::BgraSubpixelMask
                | gpui::RasterizedGlyphFormat::BgraColor => 4,
            };
            assert_eq!(raster.format, expected_format);
            assert_eq!(
                raster.pixels.len(),
                raster.size.width.0 as usize * raster.size.height.0 as usize * channels
            );
            match expected_format {
                RasterizedGlyphFormat::AlphaMask => {
                    assert!(raster.pixels.iter().any(|&alpha| alpha > 0));
                }
                RasterizedGlyphFormat::BgraSubpixelMask => {
                    assert!(
                        raster
                            .pixels
                            .chunks_exact(4)
                            .any(|pixel| { pixel[0] != pixel[1] || pixel[1] != pixel[2] })
                    );
                }
                RasterizedGlyphFormat::BgraColor => {
                    assert!(raster.pixels.chunks_exact(4).any(|pixel| {
                        pixel[3] > 128
                            && (pixel[0] != pixel[1]
                                || pixel[1] != pixel[2]
                                || pixel[0] != pixel[2])
                    }));
                }
            }
        }
    }

    #[test]
    fn native_backend_receives_the_face_instance_selected_during_shaping() {
        let seen = Arc::new(Mutex::new(None));
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "Source Serif 4",
            RecordingRasterizer { seen: seen.clone() },
        );
        system.add_fonts(vec![Cow::Borrowed(SOURCE_SERIF)]).unwrap();

        let text = "A";
        let layout = layout_line(
            &system,
            text,
            px(22.0),
            &[TextRun {
                len: text.len(),
                font: font("Source Serif 4").bold().italic(),
                ..Default::default()
            }],
        );
        let fragment = &layout.paint_fragments[0];
        let font_id = fragment.font_id;
        let params = RenderGlyphParams {
            font_id,
            glyph_id: fragment.glyphs[0].id,
            font_size: px(22.0),
            subpixel_variant: point(2, 0),
            scale_factor: 1.5,
            raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
        };

        let raster = system.rasterize_glyph(&params).unwrap();
        assert_eq!(raster.format, RasterizedGlyphFormat::AlphaMask);
        assert_eq!(raster.size, Size::default());
        assert!(raster.pixels.is_empty());

        let seen = seen.lock().clone().expect("rasterizer saw a face");
        assert_eq!(seen.font_id, font_id);
        assert_eq!(seen.face_index, 0);
        assert!(seen.data_matches);
        assert!(!seen.has_color_glyphs);
        assert_eq!(seen.synthesis.embolden, false);
        assert_eq!(seen.synthesis.skew_degrees, Some(14.0));
        let weight = seen
            .variations
            .iter()
            .find(|variation| variation.tag == skrifa::Tag::new(b"wght"))
            .expect("weight design coordinate");
        assert!((weight.value - 700.0).abs() < 0.05, "{weight:?}");
    }

    #[derive(Clone, Debug)]
    struct SeenRasterFace {
        font_id: FontId,
        face_index: u32,
        data_matches: bool,
        variations: Vec<FontVariation>,
        synthesis: FontSynthesis,
        has_color_glyphs: bool,
    }

    struct RecordingRasterizer {
        seen: Arc<Mutex<Option<SeenRasterFace>>>,
    }

    impl GlyphRasterizer for RecordingRasterizer {
        fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
            PreparedRasterStyle::independent(request.requested_mode)
        }

        fn rasterize(
            &mut self,
            face: RasterFace<'_>,
            _params: &RenderGlyphParams,
        ) -> Result<RasterizedGlyph> {
            *self.seen.lock() = Some(SeenRasterFace {
                font_id: face.font_id,
                face_index: face.face_index,
                data_matches: face.data == SOURCE_SERIF,
                variations: face.variations.to_vec(),
                synthesis: face.synthesis,
                has_color_glyphs: face.has_color_glyphs,
            });
            Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask))
        }
    }
}
