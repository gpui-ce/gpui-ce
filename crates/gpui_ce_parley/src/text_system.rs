#[cfg(test)]
use crate::{FontSynthesis, FontVariation, RasterFace};

#[cfg(test)]
use gpui::{
    AppContext, CaretSelection, Context, FontFallbacks, FontFeatures as GpuiFontFeatures,
    FontStyle as GpuiFontStyle, FontWeight as GpuiFontWeight, GlyphRenderMode, HeadlessAppContext,
    HighlightStyle, Hsla, IntoElement, RasterizedGlyphFormat, Render, ScaledPixels,
    StrikethroughStyle, Styled, StyledText, TextSystem, UnderlineStyle, VerticalAlign, Window,
    WindowHandle, WindowTextSystem, div, font, hsla, prelude::*,
};

#[cfg(test)]
use std::{cell::Cell, cell::RefCell, rc::Rc, sync::atomic::AtomicUsize};

use crate::{
    ColorGlyphKind, FontStore, GlyphRasterizer, SwashGlyphRasterizer, SystemFonts,
    catalog::{family_names, new_font_context, register_font_blobs, resolve_face},
};

use anyhow::{Context as _, Result};
use gpui::{
    Bounds, CaretAffinity, CaretMovement, CaretPosition, Font, FontId, FontMetrics, GlyphId,
    InlineBidiScope, InlineBoxRequest, InlineLayout, InlineLayoutRequest, InlineTextMetrics,
    InlineTextStyle, InlineVisualLine, LineLayout, PaintFragment, PaintStyle, ParagraphDirection,
    Pixels, PlatformTextLayout, PlatformTextSystem, Point, PositionedInlineBox,
    PreparedRasterStyle, RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, ResolvedDirection,
    ShapedGlyph, Size, TextAlign, TextBoundary as Boundary, TextDirection as Direction,
    TextLayoutOptions, TextLayoutRequest, TextMovement, TextRenderingMode, TextRun,
    TextSelectionKind, UnicodeBidi, VisualDirection, VisualLine, align_inline_boxes, point, px,
    size,
};

use parking_lot::{Mutex, RwLock};
use parley::setting::Tag;
use parley::{
    Affinity, Alignment, AlignmentOptions, CHROMIUM_LINE_BREAK_OVERRIDE, Cluster, Cursor,
    FontContext, FontFamily, FontFamilyName, FontFeature, FontFeatures, FontStyle,
    FontVariation as ParleyFontVariation, FontWeight, GenericFamily, InlineBox, InlineBoxKind,
    Layout, LayoutContext, Line, LineHeight, PositionedLayoutItem, Selection, StyleProperty,
};
use skrifa::instance::NormalizedCoord;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    ops::Range,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use unicode_segmentation::UnicodeSegmentation as _;

mod paragraphs;

use paragraphs::{
    ParagraphLayout, ParleyDocumentLayout, is_paragraph_separator, local_range, paragraph_ranges,
};

struct ParleyState {
    font_context: FontContext,
    layout_context: LayoutContext<ParleyBrush>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ParleyBrush {
    source_run: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ParleyShapingRun {
    len: usize,
    font: Font,
    letter_spacing: Option<Pixels>,
}

impl From<&TextRun> for ParleyShapingRun {
    fn from(run: &TextRun) -> Self {
        Self {
            len: run.len,
            font: run.font.clone(),
            letter_spacing: run.letter_spacing,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ParagraphCacheKey {
    font_generation: u64,
    text: Arc<str>,
    font_size: Pixels,
    runs: Vec<ParleyShapingRun>,
    inline_boxes: Vec<InlineBoxRequest>,
    text_styles: Vec<InlineTextStyle>,
    line_height: Option<Pixels>,
}

struct BoundedCache<Key, Value, const CAPACITY: usize> {
    entries: HashMap<Key, Value>,
    insertion_order: VecDeque<Key>,
}

impl<Key, Value, const CAPACITY: usize> Default for BoundedCache<Key, Value, CAPACITY> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }
}

impl<Key, Value, const CAPACITY: usize> BoundedCache<Key, Value, CAPACITY>
where
    Key: Clone + Eq + Hash,
    Value: Clone,
{
    fn get(&self, key: &Key) -> Option<Value> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: Key, value: Value) {
        if self.entries.contains_key(&key) {
            return;
        }

        if self.entries.len() == CAPACITY
            && let Some(expired) = self.insertion_order.pop_front()
        {
            self.entries.remove(&expired);
        }

        self.insertion_order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.insertion_order.clear();
    }
}

type ParagraphCache = BoundedCache<ParagraphCacheKey, Layout<ParleyBrush>, 512>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ParagraphResultCacheKey {
    paragraph: ParagraphCacheKey,
    source_map: SourceMap,
    wrap: Option<(Pixels, Option<usize>)>,
    inline_text_metrics: Option<InlineTextMetrics>,
    text_align: TextAlign,
    alignment_width: Option<Pixels>,
}

type ParagraphResultCache = BoundedCache<ParagraphResultCacheKey, ParleyLayoutResult, 512>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceMap {
    source_to_backend: Vec<usize>,
    backend_to_source: Vec<usize>,
}

impl SourceMap {
    fn source_index(&self, backend_idx: usize) -> usize {
        self.backend_to_source
            .get(backend_idx)
            .copied()
            .unwrap_or_else(|| *self.backend_to_source.last().unwrap_or(&0))
    }

    fn backend_index(&self, source_idx: usize) -> usize {
        self.source_to_backend
            .get(source_idx)
            .copied()
            .unwrap_or_else(|| *self.source_to_backend.last().unwrap_or(&0))
    }

    fn source_caret(&self, caret: CaretPosition) -> CaretPosition {
        CaretPosition::new(self.source_index(caret.index), caret.affinity)
    }

    fn backend_caret(&self, caret: CaretPosition) -> CaretPosition {
        CaretPosition::new(self.backend_index(caret.index), caret.affinity)
    }

    fn source_range(&self, range: Range<usize>) -> Range<usize> {
        self.source_index(range.start)..self.source_index(range.end)
    }

    fn backend_range(&self, range: Range<usize>) -> Range<usize> {
        self.backend_index(range.start)..self.backend_index(range.end)
    }
}

#[derive(Debug)]
struct SourceMappedLayout {
    source_len: usize,
    map: SourceMap,
    inner: Arc<dyn PlatformTextLayout>,
}

impl SourceMappedLayout {
    fn source_caret_for_point(
        &self,
        caret: CaretPosition,
        point: Point<Pixels>,
        line_height: Pixels,
    ) -> CaretPosition {
        let mapped = self.map.source_caret(caret);
        if self.inner.len() == self.source_len {
            return mapped;
        }

        let start = CaretPosition::attached_to_next_cluster(0);
        if self.source_len == 0 {
            return start;
        }

        let end = CaretPosition::attached_to_previous_cluster(self.source_len);
        let mapped = match mapped.index {
            0 => start,
            idx if idx == self.source_len => end,
            _ => return mapped,
        };

        // A synthetic bidi control can map a visual edge to the other source endpoint.
        let distance = |endpoint| {
            self.inner
                .caret_geometry(self.map.backend_caret(endpoint), line_height)
                .map(|bounds| (bounds.origin - point).magnitude())
        };

        match (distance(start), distance(end)) {
            (Some(start_distance), Some(end_distance)) if end_distance < start_distance => end,
            (Some(_), Some(_)) => start,
            _ => mapped,
        }
    }
}

impl PlatformTextLayout for SourceMappedLayout {
    fn len(&self) -> usize {
        self.source_len
    }

    fn line_count(&self) -> usize {
        self.inner.line_count()
    }

    fn size(&self) -> Size<Pixels> {
        self.inner.size()
    }

    fn index_from_point(&self, point: Point<Pixels>, line_height: Pixels) -> Result<usize, usize> {
        self.inner
            .index_from_point(point, line_height)
            .map(|idx| self.map.source_index(idx))
            .map_err(|idx| self.map.source_index(idx))
    }

    fn caret_from_point(
        &self,
        point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.inner
            .caret_from_point(point, line_height)
            .map(|caret| self.source_caret_for_point(caret, point, line_height))
            .map_err(|caret| self.source_caret_for_point(caret, point, line_height))
    }

    fn caret_geometry(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        self.inner
            .caret_geometry(self.map.backend_caret(caret), line_height)
    }

    fn refresh_caret(&self, caret: CaretPosition) -> CaretPosition {
        self.map
            .source_caret(self.inner.refresh_caret(self.map.backend_caret(caret)))
    }

    fn move_visual(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let source_start = caret.index.min(self.source_len);
        let mut backend_caret = self.map.backend_caret(caret);

        for _attempt in 0..=self.inner.len() {
            backend_caret = self.inner.move_visual(backend_caret, direction)?;
            let source_caret = self.map.source_caret(backend_caret);

            if source_caret.index != source_start {
                return Some(source_caret);
            }
        }

        None
    }

    fn selection_geometry(&self, range: Range<usize>, line_height: Pixels) -> Vec<Bounds<Pixels>> {
        self.inner
            .selection_geometry(self.map.backend_range(range), line_height)
    }

    fn inline_geometry(&self, range: Range<usize>) -> Vec<(Bounds<Pixels>, usize)> {
        self.inner.inline_geometry(self.map.backend_range(range))
    }

    fn inline_geometry_for_ranges(
        &self,
        ranges: &[Range<usize>],
    ) -> Vec<Vec<(Bounds<Pixels>, usize)>> {
        let ranges = ranges
            .iter()
            .cloned()
            .map(|range| self.map.backend_range(range))
            .collect::<Vec<_>>();

        self.inner.inline_geometry_for_ranges(&ranges)
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        let mut backend_caret = self.map.backend_caret(caret);

        for _attempt in 0..=self.inner.len() {
            let range = self.inner.logical_cluster_before(backend_caret)?;
            let source = self.map.source_range(range.clone());

            if !source.is_empty() {
                return Some(source);
            }

            backend_caret.index = range.start;
        }

        None
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        let mut backend_caret = self.map.backend_caret(caret);

        for _attempt in 0..=self.inner.len() {
            let range = self.inner.logical_cluster_after(backend_caret)?;
            let source = self.map.source_range(range.clone());

            if !source.is_empty() {
                return Some(source);
            }

            backend_caret.index = range.end;
        }

        None
    }

    fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> CaretMovement {
        let direction = match (movement.direction, movement.boundary) {
            (Direction::Left, Boundary::Cluster) => Some(VisualDirection::Left),
            (Direction::Right, Boundary::Cluster) => Some(VisualDirection::Right),
            _ => None,
        };

        if let Some(direction) = direction {
            return CaretMovement {
                caret: self.move_visual(caret, direction).unwrap_or(caret),
                preferred_x: None,
            };
        }

        let CaretMovement { caret, preferred_x } =
            self.inner
                .caret_movement(self.map.backend_caret(caret), movement, preferred_x);

        CaretMovement {
            caret: self.map.source_caret(caret),
            preferred_x,
        }
    }

    fn selection_from_point(
        &self,
        point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        self.map
            .source_range(self.inner.selection_from_point(point, line_height, kind))
    }
}

struct PreparedBidiText {
    text: String,
    runs: Vec<TextRun>,
    run_sources: Vec<usize>,
    text_styles: Vec<InlineTextStyle>,
    inline_boxes: Vec<InlineBoxRequest>,
    map: SourceMap,
}

#[derive(Clone)]
struct BidiControls {
    start: usize,
    end: usize,
    open: Vec<char>,
    close: Vec<char>,
}

fn controls_for_scope(
    range: Range<usize>,
    direction: ResolvedDirection,
    unicode_bidi: UnicodeBidi,
) -> Option<BidiControls> {
    let (directional_embed, directional_isolate, directional_override) = match direction {
        ResolvedDirection::LeftToRight => ('\u{202a}', '\u{2066}', '\u{202d}'),
        ResolvedDirection::RightToLeft => ('\u{202b}', '\u{2067}', '\u{202e}'),
    };
    let (open, close) = match unicode_bidi {
        UnicodeBidi::Normal => return None,
        UnicodeBidi::Embed => (vec![directional_embed], vec!['\u{202c}']),
        UnicodeBidi::Isolate => (vec![directional_isolate], vec!['\u{2069}']),
        UnicodeBidi::BidiOverride => (vec![directional_override], vec!['\u{202c}']),
        UnicodeBidi::IsolateOverride => (
            vec![directional_isolate, directional_override],
            vec!['\u{202c}', '\u{2069}'],
        ),
        UnicodeBidi::Plaintext => (vec!['\u{2068}'], vec!['\u{2069}']),
    };

    Some(BidiControls {
        start: range.start,
        end: range.end,
        open,
        close,
    })
}

fn prepare_bidi_text(
    text: &str,
    runs: &[TextRun],
    text_styles: &[InlineTextStyle],
    inline_boxes: &[InlineBoxRequest],
    direction: ParagraphDirection,
    unicode_bidi: UnicodeBidi,
    bidi_scopes: &[InlineBidiScope],
) -> PreparedBidiText {
    let root_direction = match direction {
        ParagraphDirection::Auto => None,
        ParagraphDirection::LeftToRight => Some(ResolvedDirection::LeftToRight),
        ParagraphDirection::RightToLeft => Some(ResolvedDirection::RightToLeft),
    };
    let mut controls = bidi_scopes
        .iter()
        .filter(|scope| {
            scope.range.start <= scope.range.end
                && scope.range.end <= text.len()
                && text.is_char_boundary(scope.range.start)
                && text.is_char_boundary(scope.range.end)
        })
        .filter_map(|scope| {
            controls_for_scope(scope.range.clone(), scope.direction, scope.unicode_bidi)
        })
        .collect::<Vec<_>>();

    if let Some(root_direction) = root_direction
        && let Some(root) = controls_for_scope(0..text.len(), root_direction, unicode_bidi)
    {
        controls.push(root);
    }

    controls.sort_by_key(|scope| (scope.start, std::cmp::Reverse(scope.end)));

    let mut prepared = String::new();
    let mut backend_to_source = vec![0];
    let mut source_to_backend = vec![0; text.len() + 1];

    let append_synthetic = |character: char,
                            source_idx: usize,
                            prepared: &mut String,
                            backend_to_source: &mut Vec<usize>| {
        prepared.push(character);
        backend_to_source.extend(std::iter::repeat_n(source_idx, character.len_utf8()));
    };

    if let Some(root_direction) = root_direction {
        let marker = match root_direction {
            ResolvedDirection::LeftToRight => '\u{200e}',
            ResolvedDirection::RightToLeft => '\u{200f}',
        };
        append_synthetic(marker, 0, &mut prepared, &mut backend_to_source);
    }

    for (source_idx, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        for scope in controls.iter().rev().filter(|scope| {
            scope.start < scope.end && scope.end == source_idx && scope.end < text.len()
        }) {
            for control in &scope.close {
                append_synthetic(*control, source_idx, &mut prepared, &mut backend_to_source);
            }
        }

        for scope in controls.iter().filter(|scope| scope.start == source_idx) {
            for control in &scope.open {
                append_synthetic(*control, source_idx, &mut prepared, &mut backend_to_source);
            }
        }

        source_to_backend[source_idx] = prepared.len();

        if source_idx == text.len() {
            break;
        }

        for scope in controls
            .iter()
            .filter(|scope| scope.start == source_idx && scope.end == source_idx)
        {
            for control in &scope.close {
                append_synthetic(*control, source_idx, &mut prepared, &mut backend_to_source);
            }
        }

        prepared.push(character);
        for byte_offset in 1..=character.len_utf8() {
            backend_to_source.push(source_idx + byte_offset);
            source_to_backend[source_idx + byte_offset] =
                prepared.len() - character.len_utf8() + byte_offset;
        }
    }

    for scope in controls
        .iter()
        .rev()
        .filter(|scope| scope.end == text.len())
    {
        for control in &scope.close {
            append_synthetic(*control, text.len(), &mut prepared, &mut backend_to_source);
        }
    }

    let map = SourceMap {
        source_to_backend,
        backend_to_source,
    };
    let (runs, run_sources) = remap_runs(&prepared, &map, runs, text.len());
    let text_styles = text_styles
        .iter()
        .map(|style| InlineTextStyle {
            range: map.backend_range(style.range.clone()),
            ..style.clone()
        })
        .collect();
    let inline_boxes = inline_boxes
        .iter()
        .map(|inline_box| InlineBoxRequest {
            index: map.backend_index(inline_box.index),
            ..*inline_box
        })
        .collect();

    PreparedBidiText {
        text: prepared,
        runs,
        run_sources,
        text_styles,
        inline_boxes,
        map,
    }
}

fn remap_runs(
    text: &str,
    map: &SourceMap,
    source_runs: &[TextRun],
    source_len: usize,
) -> (Vec<TextRun>, Vec<usize>) {
    let mut run_ends = Vec::with_capacity(source_runs.len());
    let mut end = 0;

    for run in source_runs {
        end += run.len;
        run_ends.push(end);
    }

    let mut runs: Vec<TextRun> = Vec::new();
    let mut run_sources = Vec::new();

    for (backend_idx, character) in text.char_indices() {
        let source_idx = map
            .source_index(backend_idx)
            .min(source_len.saturating_sub(1));
        let source_run = run_ends
            .partition_point(|run_end| *run_end <= source_idx)
            .min(source_runs.len().saturating_sub(1));

        if run_sources.last().copied() == Some(source_run) {
            runs.last_mut().unwrap().len += character.len_utf8();
        } else if let Some(run) = source_runs.get(source_run) {
            let mut run = run.clone();
            run.len = character.len_utf8();
            runs.push(run);
            run_sources.push(source_run);
        }
    }

    (runs, run_sources)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RasterStyleCacheKey {
    color: RasterStyleColorKey,
    requested_mode: gpui::GlyphRenderMode,
    foreground_dependency: gpui::ForegroundDependency,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum RasterStyleColorKey {
    Scene([u32; 4]),
    Normalized(gpui::Rgba8),
}

impl From<RasterStyleRequest> for RasterStyleCacheKey {
    fn from(request: RasterStyleRequest) -> Self {
        let color = if request.requested_mode == gpui::GlyphRenderMode::Color {
            RasterStyleColorKey::Normalized(request.normalized_color())
        } else {
            RasterStyleColorKey::Scene([
                request.scene_color.red.to_bits(),
                request.scene_color.green.to_bits(),
                request.scene_color.blue.to_bits(),
                request.scene_color.alpha.to_bits(),
            ])
        };

        Self {
            color,
            requested_mode: request.requested_mode,
            foreground_dependency: request.foreground_dependency,
        }
    }
}

type RasterStyleCache = BoundedCache<RasterStyleCacheKey, PreparedRasterStyle, 256>;

#[derive(Clone)]
struct ParleyLayoutResult {
    layout: LineLayout,
    inline_lines: Vec<InlineVisualLine>,
    inline_boxes: Vec<PositionedInlineBox>,
    size: Size<Pixels>,
}

fn inline_alignment_offset(
    text_align: TextAlign,
    direction: ResolvedDirection,
    lines: &[InlineVisualLine],
) -> Pixels {
    let Some(line) = lines.first() else {
        return Pixels::ZERO;
    };

    match text_align {
        TextAlign::Start if direction.is_rtl() => line.origin.x + line.size.width,
        TextAlign::Start | TextAlign::Left => Pixels::ZERO,
        TextAlign::Center => line.origin.x + line.size.width / 2.,
        TextAlign::End if direction.is_rtl() => Pixels::ZERO,
        TextAlign::End | TextAlign::Right => line.origin.x + line.size.width,
    }
}

#[derive(Clone, Debug)]
struct ParleyLayout {
    layout: Layout<ParleyBrush>,
    inline_lines: Vec<InlineVisualLine>,
    text: Arc<str>,
    caret_stops: OnceLock<ParleyCaretStops>,
    graphemes: OnceLock<Vec<std::ops::Range<usize>>>,
}

#[derive(Clone, Copy, Debug)]
struct ParleyCaretStop {
    caret: CaretPosition,
    block: f64,
    inline: f64,
}

#[derive(Clone, Debug)]
struct ParleyCaretStops {
    stops: Vec<ParleyCaretStop>,
    indices: HashMap<CaretPosition, usize>,
}

struct ParleyClusterGeometry {
    text_range: Range<usize>,
    bounds: Bounds<Pixels>,
    line_idx: usize,
}

fn visit_line_clusters(
    line: Line<'_, ParleyBrush>,
    mut visit: impl FnMut(Cluster<'_, ParleyBrush>, Range<f32>),
) {
    let mut previous_run_idx = None;

    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            continue;
        };

        let run = glyph_run.run();

        if previous_run_idx == Some(run.index()) {
            continue;
        }

        previous_run_idx = Some(run.index());
        let mut inline = glyph_run.offset();

        for cluster in run.visual_clusters() {
            let advance = inline..inline + cluster.advance();
            inline = advance.end;
            visit(cluster, advance);
        }
    }
}

impl ParleyLayout {
    fn new(
        layout: Layout<ParleyBrush>,
        text: Arc<str>,
        inline_lines: Vec<InlineVisualLine>,
    ) -> Self {
        Self {
            layout,
            inline_lines,
            text,
            caret_stops: OnceLock::new(),
            graphemes: OnceLock::new(),
        }
    }

    fn graphemes(&self) -> &[Range<usize>] {
        self.graphemes.get_or_init(|| {
            self.text
                .grapheme_indices(true)
                .map(|(start, grapheme)| start..start + grapheme.len())
                .collect()
        })
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

    fn cursor_position(layout: &Layout<ParleyBrush>, cursor: Cursor) -> (f64, f64) {
        let geometry = cursor.geometry(layout, 0.0);
        (geometry.y0, geometry.x0)
    }

    fn collect_caret_stops(
        layout: &Layout<ParleyBrush>,
        text_len: usize,
        graphemes: &[std::ops::Range<usize>],
    ) -> ParleyCaretStops {
        let mut stops = Vec::with_capacity((graphemes.len() + 2) * 2);
        let mut indices = HashMap::with_capacity((graphemes.len() + 2) * 2);
        let grapheme_boundaries = std::iter::once(0)
            .chain(graphemes.iter().map(|range| range.end))
            .collect::<HashSet<_>>();

        for line in layout.lines() {
            let block = f64::from(line.metrics().block_min_coord);
            visit_line_clusters(line, |cluster, advance| {
                let range = cluster.text_range();
                let (left, right) = if cluster.is_rtl() {
                    (
                        CaretPosition::new(range.end, CaretAffinity::Upstream),
                        CaretPosition::new(range.start, CaretAffinity::Downstream),
                    )
                } else {
                    (
                        CaretPosition::new(range.start, CaretAffinity::Downstream),
                        CaretPosition::new(range.end, CaretAffinity::Upstream),
                    )
                };

                for (caret, inline) in [(left, advance.start), (right, advance.end)] {
                    if grapheme_boundaries.contains(&caret.index) {
                        Self::push_caret_stop(
                            &mut stops,
                            &mut indices,
                            caret,
                            block,
                            f64::from(inline),
                        );
                    }
                }
            });
        }

        if stops.is_empty() {
            let cursor = Cursor::from_byte_index(layout, text_len, Affinity::Upstream);
            let (block, inline) = Self::cursor_position(layout, cursor);
            Self::push_caret_stop(
                &mut stops,
                &mut indices,
                Self::caret_position(cursor),
                block,
                inline,
            );
        }

        ParleyCaretStops { stops, indices }
    }

    fn push_caret_stop(
        stops: &mut Vec<ParleyCaretStop>,
        indices: &mut HashMap<CaretPosition, usize>,
        caret: CaretPosition,
        block: f64,
        inline: f64,
    ) {
        if stops.last().is_some_and(|stop| {
            Self::same_coordinate(stop.block, block) && Self::same_coordinate(stop.inline, inline)
        }) {
            indices.insert(caret, stops.len() - 1);

            return;
        }

        indices.insert(caret, stops.len());
        stops.push(ParleyCaretStop {
            caret,
            block,
            inline,
        });
    }

    fn same_coordinate(left: f64, right: f64) -> bool {
        let scale = 1.0 + left.abs().max(right.abs());

        (left - right).abs() <= f64::from(f32::EPSILON) * scale * 4.0
    }

    fn caret_stop_index(&self, caret: CaretPosition) -> Option<usize> {
        let stops = self.caret_stops.get_or_init(|| {
            Self::collect_caret_stops(&self.layout, self.text.len(), self.graphemes())
        });
        if let Some(idx) = stops.indices.get(&caret).copied() {
            return Some(idx);
        }

        let cursor = self.cursor(caret);
        if let Some(idx) = stops.indices.get(&Self::caret_position(cursor)).copied() {
            return Some(idx);
        }

        let (block, inline) = Self::cursor_position(&self.layout, cursor);

        let search = stops.stops.binary_search_by(|stop| {
            stop.block
                .total_cmp(&block)
                .then_with(|| stop.inline.total_cmp(&inline))
        });

        search.ok().or_else(|| {
            let insertion_idx = search.unwrap_err();
            [insertion_idx.checked_sub(1), Some(insertion_idx)]
                .into_iter()
                .flatten()
                .find(|idx| {
                    stops.stops.get(*idx).is_some_and(|stop| {
                        Self::same_coordinate(stop.block, block)
                            && Self::same_coordinate(stop.inline, inline)
                    })
                })
        })
    }

    fn adjacent_caret_stop(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<ParleyCaretStop> {
        let idx = self.caret_stop_index(caret)?;
        let caret_stops = self.caret_stops.get()?;

        match direction {
            VisualDirection::Left => idx
                .checked_sub(1)
                .and_then(|idx| caret_stops.stops.get(idx)),
            VisualDirection::Right => caret_stops.stops.get(idx + 1),
        }
        .copied()
    }

    fn native_y_for_line(&self, line_idx: usize) -> f32 {
        self.layout
            .get(line_idx)
            .map(|line| {
                let metrics = line.metrics();
                (metrics.block_min_coord + metrics.block_max_coord) * 0.5
            })
            .unwrap_or_else(|| self.layout.height())
    }

    fn line_index_from_point(point: Point<Pixels>, line_height: Pixels) -> usize {
        if line_height > Pixels::ZERO && point.y >= Pixels::ZERO {
            (point.y / line_height) as usize
        } else {
            0
        }
    }

    fn line_index_at_block(&self, block: f32) -> usize {
        self.layout
            .lines()
            .position(|line| {
                let metrics = line.metrics();

                block >= metrics.block_min_coord && block < metrics.block_max_coord
            })
            .unwrap_or_else(|| self.layout.len().saturating_sub(1))
    }

    fn inline_cluster_geometry(&self) -> Vec<ParleyClusterGeometry> {
        let mut geometries = Vec::new();

        for (line_idx, line) in self.layout.lines().enumerate() {
            let Some(inline_line) = self.inline_lines.get(line_idx).copied() else {
                continue;
            };
            visit_line_clusters(line, |cluster, advance| {
                let left = px(advance.start);
                let right = px(advance.end);

                if right > left {
                    geometries.push(ParleyClusterGeometry {
                        text_range: cluster.text_range(),
                        bounds: Bounds::new(
                            point(left, inline_line.origin.y),
                            size(right - left, inline_line.size.height),
                        ),
                        line_idx,
                    });
                }
            });
        }

        geometries
    }

    fn append_inline_geometry(
        output: &mut Vec<(Bounds<Pixels>, usize)>,
        geometry: &ParleyClusterGeometry,
    ) {
        if let Some((previous, previous_line_idx)) = output.last_mut()
            && *previous_line_idx == geometry.line_idx
            && previous.origin.y == geometry.bounds.origin.y
            && previous.size.height == geometry.bounds.size.height
            && geometry.bounds.origin.x <= previous.right()
        {
            *previous = previous.union(&geometry.bounds);

            return;
        }

        output.push((geometry.bounds, geometry.line_idx));
    }
}

impl PlatformTextLayout for ParleyLayout {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn line_count(&self) -> usize {
        self.layout.len()
    }

    fn size(&self) -> Size<Pixels> {
        let width = if self.text.is_empty() && self.layout.inline_boxes().is_empty() {
            Pixels::ZERO
        } else {
            px(self.layout.width())
        };

        size(width, px(self.layout.height()))
    }

    fn index_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
    ) -> std::result::Result<usize, usize> {
        let closest = self
            .caret_from_point(point, line_height)
            .map_err(|caret| caret.index)?
            .index;

        Cluster::from_point(
            &self.layout,
            point.x.into(),
            self.native_y_for_line(Self::line_index_from_point(point, line_height)),
        )
        .map(|(cluster, _)| cluster.text_range().start)
        .ok_or(closest)
    }

    fn caret_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
    ) -> std::result::Result<CaretPosition, CaretPosition> {
        let line_idx = Self::line_index_from_point(point, line_height);

        let caret = Self::caret_position(Cursor::from_point(
            &self.layout,
            point.x.into(),
            self.native_y_for_line(line_idx),
        ));

        if self.text.is_empty() {
            return Err(caret);
        }

        let Some(line) = self.layout.get(line_idx) else {
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

        if self.text.is_empty() && self.layout.inline_boxes().is_empty() {
            return Some(Bounds::new(
                point(self.inline_lines[0].origin.x, Pixels::ZERO),
                size(Pixels::ZERO, line_height),
            ));
        }

        let cursor = self.cursor(caret);
        let geometry = cursor.geometry(&self.layout, 0.0);
        let line_idx = self.line_index_at_block(geometry.y0 as f32);

        Some(Bounds::from_corners(
            point(px(geometry.x0 as f32), line_height * line_idx),
            point(px(geometry.x1 as f32), line_height * (line_idx + 1)),
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
        if self.text.is_empty() && self.layout.inline_boxes().is_empty() {
            return None;
        }

        self.adjacent_caret_stop(caret, direction)
            .map(|stop| stop.caret)
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
            .map(|(geometry, line_idx)| {
                Bounds::from_corners(
                    point(px(geometry.x0 as f32), line_height * line_idx),
                    point(px(geometry.x1 as f32), line_height * (line_idx + 1)),
                )
            })
            .collect()
    }

    fn inline_geometry(&self, range: std::ops::Range<usize>) -> Vec<(Bounds<Pixels>, usize)> {
        self.inline_geometry_for_ranges(std::slice::from_ref(&range))
            .pop()
            .unwrap()
    }

    fn inline_geometry_for_ranges(
        &self,
        ranges: &[Range<usize>],
    ) -> Vec<Vec<(Bounds<Pixels>, usize)>> {
        let geometries = self.inline_cluster_geometry();
        let mut owners = vec![Vec::new(); geometries.len()];
        let mut geometry_order = (0..geometries.len()).collect::<Vec<_>>();
        geometry_order.sort_unstable_by_key(|idx| geometries[*idx].text_range.start);

        let mut range_order = (0..ranges.len())
            .filter(|idx| !ranges[*idx].is_empty())
            .collect::<Vec<_>>();
        range_order.sort_unstable_by_key(|idx| ranges[*idx].start);

        let mut active_ranges: Vec<usize> = Vec::new();
        let mut next_range = 0;

        for geometry_idx in geometry_order {
            let text_range = &geometries[geometry_idx].text_range;
            active_ranges.retain(|range_idx| ranges[*range_idx].end > text_range.start);

            while let Some(range_idx) = range_order.get(next_range).copied()
                && ranges[range_idx].start < text_range.end
            {
                if ranges[range_idx].end > text_range.start {
                    active_ranges.push(range_idx);
                }

                next_range += 1;
            }

            owners[geometry_idx].extend(active_ranges.iter().copied());
        }

        let mut output = vec![Vec::new(); ranges.len()];

        for (geometry, owners) in geometries.iter().zip(owners) {
            for range_idx in owners {
                Self::append_inline_geometry(&mut output[range_idx], geometry);
            }
        }

        output
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<std::ops::Range<usize>> {
        self.graphemes()
            .iter()
            .rev()
            .find(|range| range.start < caret.index)
            .cloned()
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<std::ops::Range<usize>> {
        self.graphemes()
            .iter()
            .find(|range| range.end > caret.index)
            .cloned()
    }

    fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> CaretMovement {
        let cursor = self.cursor(caret);
        let moved = match (movement.direction, movement.boundary) {
            (Direction::Left | Direction::Right, Boundary::Cluster) => {
                let direction = if movement.direction == Direction::Left {
                    VisualDirection::Left
                } else {
                    VisualDirection::Right
                };

                return CaretMovement {
                    caret: self.move_visual(caret, direction).unwrap_or(caret),
                    preferred_x: None,
                };
            }
            (Direction::Left, Boundary::Word) => cursor.previous_visual_word(&self.layout),
            (Direction::Right, Boundary::Word) => cursor.next_visual_word(&self.layout),
            (Direction::Start, Boundary::VisualLine) => Selection::from(cursor)
                .line_start(&self.layout, false)
                .focus(),
            (Direction::End, Boundary::VisualLine) => Selection::from(cursor)
                .line_end(&self.layout, false)
                .focus(),
            (Direction::Start, Boundary::HardLine) => Selection::from(cursor)
                .hard_line_start(&self.layout, false)
                .focus(),
            (Direction::End, Boundary::HardLine) => Selection::from(cursor)
                .hard_line_end(&self.layout, false)
                .focus(),
            (Direction::Start | Direction::End, Boundary::Document) => {
                let delta = if movement.direction == Direction::Start {
                    isize::MIN
                } else {
                    isize::MAX
                };

                Selection::from(cursor)
                    .move_lines(&self.layout, delta, false)
                    .focus()
            }
            (Direction::Up | Direction::Down, Boundary::VisualLine) => {
                let delta = if movement.direction == Direction::Up {
                    -1
                } else {
                    1
                };

                let geometry = cursor.geometry(&self.layout, 0.0);
                let line_idx = self.line_index_at_block(geometry.y0 as f32);
                let target_idx = line_idx
                    .checked_add_signed(delta)
                    .filter(|&target_idx| self.layout.get(target_idx).is_some());
                let Some(target_idx) = target_idx else {
                    let selection = Selection::from(cursor);
                    let moved = if delta < 0 {
                        selection.previous_line(&self.layout, false)
                    } else {
                        selection.next_line(&self.layout, false)
                    };

                    return CaretMovement {
                        caret: Self::caret_position(moved.focus()),
                        preferred_x,
                    };
                };

                let x = preferred_x
                    .map_or_else(|| cursor.geometry(&self.layout, 0.0).x0 as f32, f32::from);
                let moved = Cursor::from_point(&self.layout, x, self.native_y_for_line(target_idx));
                return CaretMovement {
                    caret: Self::caret_position(moved),
                    preferred_x: Some(px(x)),
                };
            }
            _ => cursor,
        };

        CaretMovement {
            caret: Self::caret_position(moved),
            preferred_x: None,
        }
    }

    fn selection_from_point(
        &self,
        point: gpui::Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> std::ops::Range<usize> {
        let line_idx = Self::line_index_from_point(point, line_height);

        let y = self.native_y_for_line(line_idx);
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
    fn new(system_fonts: SystemFonts) -> Self {
        Self {
            font_context: new_font_context(system_fonts),
            layout_context: LayoutContext::new(),
        }
    }
}

/// Shapes with Parley and paints exact font instances with an injected glyph rasterizer.
pub struct ParleyTextSystem {
    font_instances: RwLock<FontStore>,
    font_generation: AtomicU64,
    rasterizer: Mutex<Box<dyn GlyphRasterizer>>,
    raster_styles: Mutex<RasterStyleCache>,
    foreground_dependencies: Mutex<HashMap<(FontId, GlyphId), gpui::ForegroundDependency>>,
    color_glyph_formats: Vec<ColorGlyphKind>,
    recommended_rendering_mode: TextRenderingMode,
    automatic_optical_sizing: bool,
    parley: Mutex<ParleyState>,
    paragraph_cache: Mutex<ParagraphCache>,
    paragraph_result_cache: Mutex<ParagraphResultCache>,
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
        let parley = ParleyState::new(system_fonts);
        let color_glyph_formats = [
            ColorGlyphKind::ColrV0,
            ColorGlyphKind::ColrV1,
            ColorGlyphKind::Cbdt,
            ColorGlyphKind::Sbix,
            ColorGlyphKind::Svg,
        ]
        .into_iter()
        .filter(|kind| rasterizer.supports_color_glyph(*kind))
        .collect();
        let recommended_rendering_mode = rasterizer.recommended_mode();

        Self {
            font_instances: RwLock::new(FontStore::default()),
            font_generation: AtomicU64::new(0),
            rasterizer: Mutex::new(Box::new(rasterizer)),
            raster_styles: Mutex::default(),
            foreground_dependencies: Mutex::default(),
            color_glyph_formats,
            recommended_rendering_mode,
            automatic_optical_sizing: false,
            parley: Mutex::new(parley),
            paragraph_cache: Mutex::default(),
            paragraph_result_cache: Mutex::default(),
            system_font_fallback: system_font_fallback.into(),
            additional_fallbacks: Vec::new(),
        }
    }

    /// Enables explicit optical sizing from each shaped run's logical font size.
    pub fn with_automatic_optical_sizing(mut self) -> Self {
        self.automatic_optical_sizing = true;

        self
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

    fn font_families<'a>(
        &'a self,
        descriptor: &'a Font,
    ) -> impl Iterator<Item = FontFamilyName<'a>> {
        std::iter::once(descriptor.family.as_ref())
            .chain(
                descriptor
                    .fallbacks
                    .iter()
                    .flat_map(|fallbacks| fallbacks.fallback_list().iter().map(String::as_str)),
            )
            .flat_map(|name| face_families(name, &self.system_font_fallback))
    }

    fn resolve_canonical_font(&self, descriptor: &Font) -> Result<FontId> {
        let families = self
            .font_families(descriptor)
            .chain(
                self.additional_fallbacks
                    .iter()
                    .flat_map(|name| face_families(name, &self.system_font_fallback)),
            )
            .collect::<Vec<_>>();

        let resolved = {
            let mut state = self.parley.lock();

            resolve_face(
                &mut state.font_context,
                &families,
                descriptor.weight.0,
                descriptor.style,
            )
            .with_context(|| format!("Fontique could not resolve '{}'", descriptor.family))?
        };

        self.font_instances.write().intern_synthesized(
            resolved.blob,
            resolved.index,
            resolved.synthesis,
        )
    }

    fn parley_layout(
        &self,
        request: TextLayoutRequest<'_>,
        inline_boxes: &[InlineBoxRequest],
        text_styles: &[InlineTextStyle],
        line_height: Option<Pixels>,
        inline_text_metrics: Option<InlineTextMetrics>,
        bidi_scopes: &[InlineBidiScope],
    ) -> Result<ParleyLayoutResult> {
        // Parley 0.11 resolves one base direction per layout, including across newlines.
        if !request.text.chars().any(is_paragraph_separator) {
            return self.parley_paragraph_layout(
                request,
                inline_boxes,
                text_styles,
                line_height,
                inline_text_metrics,
                bidi_scopes,
            );
        }

        let TextLayoutRequest {
            text,
            font_size,
            runs,
            options,
        } = request;
        let run_ranges = run_ranges(runs);

        let mut paragraphs = Vec::new();
        let mut visual_lines = Vec::new();
        let mut paint_fragments = Vec::new();
        let mut inline_lines = Vec::new();
        let mut positioned_inline_boxes = Vec::new();

        let mut document_size = Size::<Pixels>::default();
        let mut width = Pixels::ZERO;
        let mut ascent = Pixels::ZERO;
        let mut descent = Pixels::ZERO;

        for source in paragraph_ranges(text) {
            let first_run = run_ranges.partition_point(|range| range.end <= source.content.start);
            let (mut paragraph_run_sources, mut paragraph_runs): (Vec<_>, Vec<_>) = run_ranges
                .iter()
                .enumerate()
                .skip(first_run)
                .take_while(|(_idx, range)| range.start < source.content.end)
                .filter_map(|(idx, range)| {
                    let local = local_range(range, &source.content)?;

                    Some((
                        idx,
                        TextRun {
                            len: local.len(),
                            ..runs[idx].clone()
                        },
                    ))
                })
                .unzip();
            let first_style =
                text_styles.partition_point(|style| style.range.end <= source.content.start);
            let mut paragraph_styles = text_styles[first_style..]
                .iter()
                .take_while(|style| style.range.start < source.content.end)
                .filter_map(|style| {
                    let range = local_range(&style.range, &source.content)?;

                    Some(InlineTextStyle {
                        range,
                        ..style.clone()
                    })
                })
                .collect::<Vec<_>>();

            if source.content.is_empty() {
                if let Some(run) = runs.get(first_run.min(runs.len().saturating_sub(1))) {
                    paragraph_run_sources.push(first_run.min(runs.len().saturating_sub(1)));
                    paragraph_runs.push(TextRun {
                        len: 0,
                        ..run.clone()
                    });
                }

                let style_idx = source.content.start.min(text.len().saturating_sub(1));
                paragraph_styles.extend(
                    text_styles
                        .iter()
                        .filter(|style| style.range.contains(&style_idx))
                        .map(|style| InlineTextStyle {
                            range: 0..0,
                            ..style.clone()
                        }),
                );
            }

            let first_box =
                inline_boxes.partition_point(|inline_box| inline_box.index < source.content.start);
            let paragraph_boxes = inline_boxes[first_box..]
                .iter()
                .take_while(|inline_box| {
                    inline_box.index < source.separator.end
                        || source.separator.is_empty() && inline_box.index == text.len()
                })
                .map(|inline_box| InlineBoxRequest {
                    index: inline_box.index.min(source.content.end) - source.content.start,
                    ..*inline_box
                })
                .collect::<Vec<_>>();
            let mut result = self.parley_paragraph_layout(
                TextLayoutRequest {
                    text: &text[source.content.clone()],
                    runs: &paragraph_runs,
                    options: TextLayoutOptions {
                        // Clamping stops soft wrapping after the budget, but preserves hard breaks.
                        line_clamp: options
                            .line_clamp
                            .map(|count| count.saturating_sub(visual_lines.len())),
                        ..options
                    },
                    ..request
                },
                &paragraph_boxes,
                &paragraph_styles,
                line_height,
                inline_text_metrics,
                &bidi_scopes
                    .iter()
                    .filter_map(|scope| {
                        local_range(&scope.range, &source.content).map(|range| InlineBidiScope {
                            range,
                            direction: scope.direction,
                            unicode_bidi: scope.unicode_bidi,
                        })
                    })
                    .collect::<Vec<_>>(),
            )?;

            for fragment in &mut result.layout.paint_fragments {
                fragment.source_run = paragraph_run_sources[fragment.source_run];
            }

            let first_line = visual_lines.len();
            let first_fragment = paint_fragments.len();
            let block_offset = document_size.height;

            for line in &mut result.layout.visual_lines {
                line.text_range.start += source.content.start;
                line.text_range.end += source.content.start;
                line.fragment_range.start += first_fragment;
                line.fragment_range.end += first_fragment;
            }

            if let Some(line) = result.layout.visual_lines.last_mut() {
                // Keep separators in document indices without adding native trailing rows.
                line.text_range.end = source.separator.end;
            }

            for line in &mut result.inline_lines {
                line.origin.y += block_offset;
            }

            for inline_box in &mut result.inline_boxes {
                inline_box.line_index += first_line;
                inline_box.bounds.origin.y += block_offset;
            }

            width = width.max(result.layout.width);
            ascent = ascent.max(result.layout.ascent);
            descent = descent.max(result.layout.descent);
            document_size.width = document_size.width.max(result.size.width);
            document_size.height += result.size.height;

            let last_line = result.inline_lines.last().unwrap();
            let newline_width = (result.layout.ascent + result.layout.descent) * 0.25;
            let newline = if result.layout.visual_lines[0].direction.is_rtl() {
                last_line.origin.x - newline_width..last_line.origin.x
            } else {
                let newline_x = last_line.origin.x + last_line.size.width;

                newline_x..newline_x + newline_width
            };

            paragraphs.push(ParagraphLayout {
                source,
                first_line,
                block_offset,
                native: result.layout.platform_layout,
                newline,
            });
            visual_lines.extend(result.layout.visual_lines);
            paint_fragments.extend(result.layout.paint_fragments);
            inline_lines.extend(result.inline_lines);
            positioned_inline_boxes.extend(result.inline_boxes);
        }

        let platform_layout = ParleyDocumentLayout::new(paragraphs, text, document_size);

        Ok(ParleyLayoutResult {
            layout: LineLayout {
                font_size,
                width,
                ascent,
                descent,
                visual_lines: visual_lines.into_iter().collect(),
                paint_fragments,
                len: text.len(),
                platform_layout: std::sync::Arc::new(platform_layout),
            },
            inline_lines,
            inline_boxes: positioned_inline_boxes,
            size: document_size,
        })
    }

    fn parley_paragraph_layout(
        &self,
        request: TextLayoutRequest<'_>,
        inline_boxes: &[InlineBoxRequest],
        text_styles: &[InlineTextStyle],
        line_height: Option<Pixels>,
        inline_text_metrics: Option<InlineTextMetrics>,
        bidi_scopes: &[InlineBidiScope],
    ) -> Result<ParleyLayoutResult> {
        let TextLayoutRequest {
            text,
            font_size,
            runs,
            options,
        } = request;
        let TextLayoutOptions {
            wrap_width,
            line_clamp,
            text_align,
            alignment_width,
            direction,
            unicode_bidi,
        } = options;
        let wrap = (wrap_width.is_some() || line_clamp.is_some())
            .then_some((wrap_width.unwrap_or(Pixels::MAX), line_clamp));
        let source_text_len = text.len();
        let source_runs = runs;
        let prepared = prepare_bidi_text(
            text,
            runs,
            text_styles,
            inline_boxes,
            direction,
            unicode_bidi,
            bidi_scopes,
        );
        let text = prepared.text.as_str();
        let runs = prepared.runs.as_slice();
        let text_styles = prepared.text_styles.as_slice();
        let inline_boxes = prepared.inline_boxes.as_slice();
        let run_ranges = run_ranges(runs);
        let line_height = if text.is_empty() {
            text_styles
                .last()
                .map(|style| style.line_height)
                .or(line_height)
        } else {
            line_height
        };
        let paragraph_text: Arc<str> = text.into();
        let cache_key = ParagraphCacheKey {
            font_generation: self.font_generation.load(Ordering::Acquire),
            text: paragraph_text.clone(),
            font_size,
            runs: runs.iter().map(ParleyShapingRun::from).collect(),
            inline_boxes: inline_boxes.to_vec(),
            text_styles: text_styles.to_vec(),
            line_height,
        };
        let result_cache_key = ParagraphResultCacheKey {
            paragraph: cache_key.clone(),
            source_map: prepared.map.clone(),
            wrap,
            inline_text_metrics,
            text_align,
            alignment_width,
        };

        if let Some(mut result) = self.paragraph_result_cache.lock().get(&result_cache_key) {
            for fragment in &mut result.layout.paint_fragments {
                fragment.style = PaintStyle::from(&source_runs[fragment.source_run]);
            }

            return Ok(result);
        }

        let mut layout = if let Some(layout) = self.paragraph_cache.lock().get(&cache_key) {
            layout
        } else {
            let family_lists =
                runs.iter()
                    .map(|run| {
                        self.font_families(&run.font)
                            .chain(self.additional_fallbacks.iter().map(|family| {
                                FontFamilyName::Named(Cow::Borrowed(family.as_str()))
                            }))
                            .collect::<Vec<_>>()
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
            let inline_optical_settings = self.automatic_optical_sizing.then(|| {
                text_styles
                    .iter()
                    .map(|style| {
                        [ParleyFontVariation::new(
                            Tag::new(b"opsz"),
                            f32::from(style.font_size),
                        )]
                    })
                    .collect::<Vec<_>>()
            });

            let mut state = self.parley.lock();
            let ParleyState {
                font_context,
                layout_context,
            } = &mut *state;
            let mut builder = layout_context.ranged_builder(font_context, text, 1.0, false);
            builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
            builder.push_default(StyleProperty::FontSize(f32::from(font_size)));

            let default_optical_settings = self.automatic_optical_sizing.then(|| {
                [ParleyFontVariation::new(
                    Tag::new(b"opsz"),
                    f32::from(font_size),
                )]
            });
            if let Some(settings) = &default_optical_settings {
                builder.push_default(StyleProperty::FontVariations(settings.as_slice().into()));
            }

            if let Some(line_height) = line_height {
                builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(f32::from(
                    line_height,
                ))));
            }

            let mut push_style = |property, range| {
                if text.is_empty() {
                    builder.push_default(property);
                } else {
                    builder.push(property, range);
                }
            };

            for (run_idx, run) in runs.iter().enumerate() {
                let descriptor = &run.font;
                let range = run_ranges[run_idx].clone();
                if range.is_empty() && !text.is_empty() {
                    continue;
                }

                for property in [
                    StyleProperty::FontFamily(FontFamily::from(family_lists[run_idx].as_slice())),
                    StyleProperty::FontWeight(FontWeight::new(descriptor.weight.0)),
                    StyleProperty::FontStyle(match descriptor.style {
                        gpui::FontStyle::Normal => FontStyle::Normal,
                        gpui::FontStyle::Italic => FontStyle::Italic,
                        gpui::FontStyle::Oblique => FontStyle::Oblique(None),
                    }),
                    StyleProperty::Brush(ParleyBrush {
                        source_run: run_idx,
                    }),
                ] {
                    push_style(property, range.clone());
                }

                if !feature_lists[run_idx].is_empty() {
                    push_style(
                        StyleProperty::FontFeatures(FontFeatures::from(
                            feature_lists[run_idx].as_slice(),
                        )),
                        range.clone(),
                    );
                }

                if let Some(letter_spacing) = run.letter_spacing {
                    push_style(
                        StyleProperty::LetterSpacing(f32::from(letter_spacing)),
                        range.clone(),
                    );
                }
            }

            for (style_idx, style) in text_styles.iter().enumerate() {
                push_style(
                    StyleProperty::FontSize(f32::from(style.font_size)),
                    style.range.clone(),
                );

                if let Some(settings) = &inline_optical_settings {
                    push_style(
                        StyleProperty::FontVariations(settings[style_idx].as_slice().into()),
                        style.range.clone(),
                    );
                }

                push_style(
                    StyleProperty::LineHeight(LineHeight::Absolute(f32::from(style.line_height))),
                    style.range.clone(),
                );
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

            let layout = builder.build(text);
            self.paragraph_cache
                .lock()
                .insert(cache_key, layout.clone());

            layout
        };

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
        } else if let Some(alignment_width) = alignment_width {
            let mut breaker = layout.break_lines();
            breaker.state_mut().set_layout_max_advance(f32::INFINITY);
            breaker.state_mut().set_line_max_advance(f32::INFINITY);

            while breaker.break_next().is_some() {
                breaker.set_prior_line_width(f32::from(alignment_width));
            }

            breaker.finish();
        } else {
            layout.break_all_lines(None);
        }

        // Parley uses an unbounded line width for empty layouts. Align their empty
        // row here so centered and right-aligned carets stay inside the container.
        let empty_alignment = (text.is_empty() && inline_boxes.is_empty()).then(|| {
            let width = alignment_width
                .or_else(|| wrap.map(|(width, _max_lines)| width))
                .filter(|width| *width < Pixels::MAX)
                .unwrap_or_default();

            match text_align {
                TextAlign::Start if layout.is_rtl() => width,
                TextAlign::End if !layout.is_rtl() => width,
                TextAlign::Right => width,
                TextAlign::Center => width / 2.,
                _ => Pixels::ZERO,
            }
        });

        if empty_alignment.is_none() {
            let alignment = match text_align {
                TextAlign::Start => Alignment::Start,
                TextAlign::End => Alignment::End,
                TextAlign::Left => Alignment::Left,
                TextAlign::Center => Alignment::Center,
                TextAlign::Right => Alignment::Right,
            };

            layout.align(
                alignment,
                AlignmentOptions {
                    align_when_overflowing: true,
                },
            );
        }

        let paragraph_direction = if layout.is_rtl() {
            ResolvedDirection::RightToLeft
        } else {
            ResolvedDirection::LeftToRight
        };
        let mut visual_lines = Vec::new();
        let mut paint_fragments = Vec::new();

        let mut positioned_inline_boxes = Vec::new();
        let mut inline_lines = Vec::new();
        let mut inline_line_metrics = Vec::new();
        let mut inline_text_bounds = Vec::new();

        let mut width = px(0.0);
        let mut ascent = px(0.0);
        let mut descent = px(0.0);

        for (line_idx, line) in layout.lines().enumerate() {
            let fragment_start = paint_fragments.len();
            let metrics = *line.metrics();
            let line_x =
                empty_alignment.unwrap_or_else(|| px(metrics.inline_min_coord + metrics.offset));
            let line_advance = if empty_alignment.is_some() {
                Pixels::ZERO
            } else {
                px(metrics.advance)
            };

            let mut text_metrics = inline_text_metrics.unwrap_or_default();
            let leading =
                ((line_height.unwrap_or_default() - text_metrics.ascent - text_metrics.descent)
                    / 2.)
                    .max(Pixels::ZERO);

            let mut text_top = -text_metrics.ascent - leading;
            let mut text_bottom = text_metrics.descent + leading;

            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    let PositionedLayoutItem::InlineBox(inline_box) = item else {
                        unreachable!();
                    };

                    positioned_inline_boxes.push(PositionedInlineBox {
                        id: inline_box.id,
                        line_index: line_idx,
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
                let shaping_variations = self
                    .automatic_optical_sizing
                    .then(|| [crate::FontVariation::new(*b"opsz", run.font_size())]);
                let font_id = self.font_instances.write().intern(
                    run.font().data.clone(),
                    run.font().index,
                    &normalized_coords,
                    run.synthesis(),
                    shaping_variations.as_ref().map_or(&[], |settings| settings),
                )?;

                let baseline = glyph_run.baseline();
                let run_metrics = glyph_run.run().metrics();

                // Resolve leading from the input ranges. Parley 0.11 can report the
                // following style's line height in a shaped run's cached metrics.
                let range = run.text_range();
                let first_style =
                    text_styles.partition_point(|style| style.range.end <= range.start);
                let run_line_height = text_styles[first_style..]
                    .iter()
                    .take_while(|style| style.range.start < range.end)
                    .map(|style| f32::from(style.line_height))
                    .reduce(f32::max)
                    .unwrap_or_else(|| {
                        line_height
                            .map(f32::from)
                            .unwrap_or(run_metrics.line_height)
                    });

                let half_leading =
                    ((run_line_height - run_metrics.ascent - run_metrics.descent) / 2.).max(0.);

                text_metrics.ascent = text_metrics.ascent.max(px(run_metrics.ascent));
                text_metrics.descent = text_metrics.descent.max(px(run_metrics.descent));
                text_top = text_top.min(px(-run_metrics.ascent - half_leading));
                text_bottom = text_bottom.max(px(run_metrics.descent + half_leading));

                if text.is_empty() {
                    continue;
                }

                let glyphs = {
                    let font_instances = self.font_instances.read();
                    let color_glyphs = font_instances
                        .get(font_id)
                        .context("canonical font missing after interning")?
                        .color_glyphs()?;
                    glyph_run
                        .positioned_glyphs()
                        .map(|glyph| {
                            let glyph_id = GlyphId(glyph.id);
                            ShapedGlyph {
                                id: glyph_id,
                                position: point(px(glyph.x) - line_x, px(glyph.y - baseline)),
                                is_emoji: color_glyphs
                                    .available_kinds(glyph_id)
                                    .any(|kind| self.color_glyph_formats.contains(&kind)),
                            }
                        })
                        .collect()
                };

                let start = px(glyph_run.offset()) - line_x;
                let source_run = prepared.run_sources[glyph_run.style().brush.source_run];
                paint_fragments.push(PaintFragment {
                    source_run,
                    font_id,
                    font_size: px(run.font_size()),
                    glyphs,
                    x_range: start..start + px(glyph_run.advance()),
                    style: PaintStyle::from(&source_runs[source_run]),
                    underline_offset: Some(px(run_metrics.underline_offset)),
                    strikethrough_offset: Some(px(run_metrics.strikethrough_offset)),
                });
            }

            let parley_text_range = line.text_range();
            let text_range = prepared.map.source_range(
                parley_text_range.start.min(text.len())..parley_text_range.end.min(text.len()),
            );

            visual_lines.push(VisualLine {
                text_range,
                fragment_range: fragment_start..paint_fragments.len(),
                advance: line_advance,
                offset: line_x,
                direction: paragraph_direction,
            });

            inline_lines.push(InlineVisualLine {
                origin: point(line_x, px(metrics.block_min_coord)),
                size: size(
                    line_advance,
                    px(metrics.block_max_coord - metrics.block_min_coord),
                ),
                baseline: px(metrics.baseline - metrics.block_min_coord),
            });

            inline_line_metrics.push(text_metrics);
            inline_text_bounds.push((text_top, text_bottom));

            width = width.max(line_advance);
            ascent = ascent.max(px(metrics.ascent));
            descent = descent.max(px(metrics.descent));
        }

        if visual_lines.is_empty() {
            anyhow::bail!("Parley produced no line");
        }

        let mut size = size(px(layout.width()), px(layout.height()));

        if empty_alignment.is_some() {
            size.width = Pixels::ZERO;
        }

        if let (Some(text_metrics), Some(line_height)) = (inline_text_metrics, line_height) {
            align_inline_boxes(
                &mut inline_lines,
                &mut positioned_inline_boxes,
                &mut size,
                inline_boxes,
                &inline_line_metrics,
                &inline_text_bounds,
                text_metrics,
                line_height,
            );
        }

        let platform_layout = ParleyLayout::new(layout, paragraph_text, inline_lines.clone());
        let platform_layout: Arc<dyn PlatformTextLayout> = Arc::new(SourceMappedLayout {
            source_len: source_text_len,
            map: prepared.map,
            inner: Arc::new(platform_layout),
        });
        let line_layout = LineLayout {
            font_size,
            width,
            ascent,
            descent,
            visual_lines: visual_lines.into_iter().collect(),
            paint_fragments,
            len: source_text_len,
            platform_layout,
        };

        let result = ParleyLayoutResult {
            layout: line_layout,
            inline_lines,
            inline_boxes: positioned_inline_boxes,
            size,
        };
        self.paragraph_result_cache
            .lock()
            .insert(result_cache_key, result.clone());

        Ok(result)
    }
}

fn run_ranges(runs: &[TextRun]) -> Vec<Range<usize>> {
    let mut start = 0;

    runs.iter()
        .map(|run| {
            let range = start..start + run.len;
            start = range.end;

            range
        })
        .collect()
}

fn face_families<'a>(
    name: &'a str,
    system_font_fallback: &'a str,
) -> impl Iterator<Item = FontFamilyName<'a>> {
    (name == ".SystemUIFont")
        .then_some(FontFamilyName::Generic(GenericFamily::SystemUi))
        .into_iter()
        .chain(std::iter::once(FontFamilyName::Named(Cow::Borrowed(
            canonical_family(name, system_font_fallback),
        ))))
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
            .into_iter()
            .map(|bytes| fontique::Blob::from(bytes.into_owned()))
            .collect::<Vec<_>>();

        {
            let mut state = self.parley.lock();
            register_font_blobs(&mut state.font_context, &blobs)?;

            let _previous_generation = self.font_generation.fetch_add(1, Ordering::Release);
            self.paragraph_cache.lock().clear();
            self.paragraph_result_cache.lock().clear();
        }

        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names = {
            let mut state = self.parley.lock();

            family_names(&mut state.font_context)
        };
        names.extend([".SystemUIFont", ".ZedSans", ".ZedMono"].map(str::to_owned));
        names.sort_unstable();
        names.dedup();

        names
    }

    fn font_generation(&self) -> u64 {
        self.font_generation.load(Ordering::Acquire)
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.resolve_canonical_font(descriptor)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.font_instances
            .read()
            .get(font_id)
            .expect("Parley FontId missing from its store")
            .metrics()
            .expect("stored font failed Skrifa metrics")
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.font_instances
            .read()
            .get(font_id)
            .context("Parley FontId missing from its store")?
            .glyph_bounds(glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.font_instances
            .read()
            .get(font_id)
            .context("Parley FontId missing from its store")?
            .advance(glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, character: char) -> Option<GlyphId> {
        self.font_instances
            .read()
            .get(font_id)?
            .glyph_for_char(character)
            .ok()?
    }

    fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        let font_instances = self.font_instances.read();
        let font = font_instances
            .get(params.font_id)
            .context("Parley FontId missing from its store")?;
        let data_identity = font.data_identity();
        let face_idx = font.index;
        let variations = font.variations.clone();
        self.rasterizer
            .lock()
            .rasterize(font.raster_face(params.font_id), params)
            .with_context(|| {
                format!(
                    "glyph rasterization failed for FontId {:?}, data identity {data_identity}, face index {face_idx}, variations {variations:?}",
                    params.font_id
                )
            })
    }

    fn prepare_raster_style(&self, mut request: RasterStyleRequest) -> PreparedRasterStyle {
        request.foreground_dependency = if request.requested_mode == gpui::GlyphRenderMode::Color {
            let dependency_key = (request.font_id, request.glyph_id);
            if let Some(dependency) = self
                .foreground_dependencies
                .lock()
                .get(&dependency_key)
                .copied()
            {
                dependency
            } else {
                let dependency = self
                    .font_instances
                    .read()
                    .get(request.font_id)
                    .and_then(|font| {
                        font.foreground_dependency(request.glyph_id, |kind| {
                            self.color_glyph_formats.contains(&kind)
                        })
                        .ok()
                    })
                    .unwrap_or(gpui::ForegroundDependency::Full);
                self.foreground_dependencies
                    .lock()
                    .insert(dependency_key, dependency);

                dependency
            }
        } else {
            gpui::ForegroundDependency::Full
        };

        let key = request.into();
        if let Some(style) = self.raster_styles.lock().get(&key) {
            return style;
        }

        let style = self.rasterizer.lock().prepare_style(request);
        self.raster_styles.lock().insert(key, style);
        style
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        self.recommended_rendering_mode
    }

    fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
        self.parley_layout(request, &[], &[], None, None, &[])
            .expect("Parley failed to lay out a validated GPUI document")
            .layout
    }

    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        let result = self
            .parley_layout(
                TextLayoutRequest {
                    text: request.text,
                    font_size: request.font_size,
                    runs: request.runs,
                    options: request.options,
                },
                request.boxes,
                request.text_styles,
                Some(request.line_height),
                Some(request.text_metrics),
                request.bidi_scopes,
            )
            .expect("Parley failed to lay out a validated GPUI inline document");

        let alignment_offset = inline_alignment_offset(
            request.options.text_align,
            result.layout.visual_lines[0].direction,
            &result.inline_lines,
        );

        InlineLayout {
            layout: std::sync::Arc::new(result.layout),
            alignment_offset,
            lines: result.inline_lines,
            boxes: result.inline_boxes,
            size: result.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IBM_PLEX: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const IBM_PLEX_SEMIBOLD: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf");
    const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");
    const SOURCE_SERIF: &[u8] =
        include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");
    const NOTO_COLOR_EMOJI: &[u8] =
        include_bytes!("../../../assets/fonts/noto-color-emoji/NotoColorEmoji.subset.ttf");
    const NOTO_ARABIC: &[u8] =
        include_bytes!("../../../assets/fonts/noto-sans-arabic/NotoSansArabic-Regular.ttf");
    const NOTO_HEBREW: &[u8] =
        include_bytes!("../../../assets/fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf");

    fn test_system() -> Arc<ParleyTextSystem> {
        let system = Arc::new(
            ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans")
                .with_fallback_families([
                    "IBM Plex Sans",
                    "Lilex",
                    "Source Serif 4",
                    "Noto Color Emoji",
                    "Noto Sans Arabic",
                    "Noto Sans Hebrew",
                ]),
        );
        system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD),
                Cow::Borrowed(LILEX),
                Cow::Borrowed(SOURCE_SERIF),
                Cow::Borrowed(NOTO_COLOR_EMOJI),
                Cow::Borrowed(NOTO_ARABIC),
                Cow::Borrowed(NOTO_HEBREW),
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
            options: TextLayoutOptions {
                text_align: TextAlign::Left,
                ..Default::default()
            },
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
            options: TextLayoutOptions {
                wrap_width: Some(wrap_width),
                line_clamp,
                text_align: TextAlign::Left,
                ..Default::default()
            },
        })
    }

    fn layout_directional(
        system: &ParleyTextSystem,
        text: &str,
        direction: ParagraphDirection,
        text_align: TextAlign,
        alignment_width: Option<Pixels>,
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size: px(16.0),
            runs: &[text_run(text, "IBM Plex Sans")],
            options: TextLayoutOptions {
                alignment_width,
                text_align,
                direction,
                ..Default::default()
            },
        })
    }

    #[test]
    fn explicit_rtl_sets_the_base_direction_without_rtl_characters() {
        let system = test_system();

        for text in ["English 123!", "123 …", ""] {
            let layout = layout_directional(
                &system,
                text,
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                Some(px(240.0)),
            );

            assert_eq!(layout.len, text.len());
            assert_eq!(layout.platform_layout.len(), text.len());
            assert_eq!(
                layout.visual_lines[0].direction,
                ResolvedDirection::RightToLeft
            );
            let caret = layout
                .platform_layout
                .caret_geometry(CaretPosition::new(0, CaretAffinity::Downstream), px(24.0))
                .unwrap();
            assert!(caret.origin.x > px(100.0), "text={text:?}, caret={caret:?}");
        }
    }

    #[test]
    fn directional_hit_testing_preserves_source_endpoint_carets() {
        let system = test_system();
        let line_height = px(24.0);

        for (text, direction) in [
            ("English 123", ParagraphDirection::RightToLeft),
            ("مرحبا", ParagraphDirection::LeftToRight),
            ("English 123", ParagraphDirection::LeftToRight),
            ("مرحبا", ParagraphDirection::RightToLeft),
        ] {
            let layout =
                layout_directional(&system, text, direction, TextAlign::Start, Some(px(240.0)));

            for expected in [
                CaretPosition::attached_to_next_cluster(0),
                CaretPosition::attached_to_previous_cluster(text.len()),
            ] {
                let expected_bounds = layout
                    .platform_layout
                    .caret_geometry(expected, line_height)
                    .unwrap();
                let hit = layout
                    .platform_layout
                    .caret_from_point(
                        expected_bounds.origin + point(px(0.0), line_height / 2.0),
                        line_height,
                    )
                    .unwrap_or_else(|caret| caret);

                assert_eq!(hit, expected, "text={text:?}, direction={direction:?}");
            }
        }
    }

    #[test]
    fn logical_text_alignment_uses_the_paragraph_direction() {
        let system = test_system();
        let rtl_start = layout_directional(
            &system,
            "English",
            ParagraphDirection::RightToLeft,
            TextAlign::Start,
            Some(px(240.0)),
        );
        let rtl_end = layout_directional(
            &system,
            "English",
            ParagraphDirection::RightToLeft,
            TextAlign::End,
            Some(px(240.0)),
        );
        let ltr_start = layout_directional(
            &system,
            "English",
            ParagraphDirection::LeftToRight,
            TextAlign::Start,
            Some(px(240.0)),
        );

        let line_height = px(24.0);
        let rtl_start_selection = rtl_start
            .platform_layout
            .selection_geometry(0.."English".len(), line_height);
        let rtl_end_selection = rtl_end
            .platform_layout
            .selection_geometry(0.."English".len(), line_height);
        let ltr_start_selection = ltr_start
            .platform_layout
            .selection_geometry(0.."English".len(), line_height);
        assert!(
            rtl_start_selection[0].origin.x > px(100.0),
            "start={rtl_start_selection:?}, end={rtl_end_selection:?}, fragments={:?}",
            rtl_start.paint_fragments
        );
        assert!(rtl_end_selection[0].origin.x < px(1.0));
        assert!(ltr_start_selection[0].origin.x < px(1.0));

        let caret = rtl_start
            .platform_layout
            .caret_geometry(
                CaretPosition::new(0, CaretAffinity::Downstream),
                line_height,
            )
            .unwrap();
        assert!(caret.origin.x > px(100.0));
    }

    #[test]
    fn bidi_scope_controls_preserve_source_byte_indices() {
        let system = test_system();
        let text = "A אב B";
        let hebrew = text.find('א').unwrap()..text.find('ב').unwrap() + 'ב'.len_utf8();
        let runs = [text_run(text, "IBM Plex Sans")];
        let scopes = [InlineBidiScope {
            range: hebrew,
            direction: ResolvedDirection::RightToLeft,
            unicode_bidi: UnicodeBidi::IsolateOverride,
        }];
        let layout = system.layout_inline(InlineLayoutRequest {
            text,
            font_size: px(16.0),
            runs: &runs,
            boxes: &[],
            text_styles: &[],
            line_height: px(24.0),
            text_metrics: InlineTextMetrics::default(),
            options: TextLayoutOptions {
                alignment_width: Some(px(240.0)),
                direction: ParagraphDirection::LeftToRight,
                ..Default::default()
            },
            bidi_scopes: &scopes,
        });

        assert_eq!(layout.layout.len, text.len());
        assert_eq!(layout.layout.platform_layout.len(), text.len());
        assert_eq!(
            layout
                .layout
                .platform_layout
                .logical_cluster_after(CaretPosition::new(0, CaretAffinity::Downstream)),
            Some(0..1)
        );

        for (idx, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), '\0')))
        {
            let caret = layout
                .layout
                .platform_layout
                .refresh_caret(CaretPosition::new(idx, CaretAffinity::Downstream));
            assert!(caret.index <= text.len());
        }

        let selection = layout
            .layout
            .platform_layout
            .selection_geometry(0..text.len(), px(24.0));
        assert!(!selection.is_empty());
    }

    #[test]
    fn result_cache_keeps_source_maps_for_authored_directional_controls() {
        let system = test_system();
        let explicit = layout_directional(
            &system,
            "a",
            ParagraphDirection::LeftToRight,
            TextAlign::Left,
            None,
        );
        let source = "\u{200e}a";
        let authored_control = system.layout_text(TextLayoutRequest {
            text: source,
            font_size: px(16.0),
            runs: &[text_run(source, "IBM Plex Sans")],
            options: TextLayoutOptions {
                text_align: TextAlign::Left,
                ..Default::default()
            },
        });

        assert_eq!(explicit.platform_layout.len(), 1);
        assert_eq!(authored_control.len, source.len());
        assert_eq!(authored_control.platform_layout.len(), source.len());
    }

    #[test]
    fn rtl_bidi_override_reverses_visual_cluster_positions() {
        let system = test_system();
        let text = "abc";
        let runs = [text_run(text, "IBM Plex Sans")];
        let scopes = [InlineBidiScope {
            range: 0..text.len(),
            direction: ResolvedDirection::RightToLeft,
            unicode_bidi: UnicodeBidi::IsolateOverride,
        }];
        let layout = system.layout_inline(InlineLayoutRequest {
            text,
            font_size: px(16.0),
            runs: &runs,
            boxes: &[],
            text_styles: &[],
            line_height: px(24.0),
            text_metrics: InlineTextMetrics::default(),
            options: TextLayoutOptions {
                direction: ParagraphDirection::LeftToRight,
                ..Default::default()
            },
            bidi_scopes: &scopes,
        });
        let positions = [0..1, 1..2, 2..3].map(|range| {
            layout.layout.platform_layout.inline_geometry(range)[0]
                .0
                .origin
                .x
        });

        assert!(positions[0] > positions[1]);
        assert!(positions[1] > positions[2]);
    }

    #[test]
    fn plaintext_resolves_each_hard_paragraph_and_neutral_lines_fall_back_to_ltr() {
        let system = test_system();
        let text = "مرحبا\n123 …\nEnglish\n";
        let layout = system.layout_text(TextLayoutRequest {
            text,
            font_size: px(16.0),
            runs: &[text_run(text, "IBM Plex Sans")],
            options: TextLayoutOptions {
                alignment_width: Some(px(240.0)),
                unicode_bidi: UnicodeBidi::Plaintext,
                ..Default::default()
            },
        });

        assert_eq!(
            layout
                .visual_lines
                .iter()
                .map(|line| line.direction)
                .collect::<Vec<_>>(),
            vec![
                ResolvedDirection::RightToLeft,
                ResolvedDirection::LeftToRight,
                ResolvedDirection::LeftToRight,
                ResolvedDirection::LeftToRight,
            ]
        );
        assert!(layout.visual_lines[0].offset > px(100.0));
        assert_eq!(layout.visual_lines[1].offset, Pixels::ZERO);
        assert_eq!(layout.visual_lines[3].text_range, text.len()..text.len());
    }

    fn shaped_text_layout(layout: LineLayout, width: Pixels) -> gpui::ShapedTextLayout {
        gpui::ShapedTextLayout {
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
        for (line_idx, line) in layout.lines.iter().enumerate() {
            assert!(
                line.origin.x + line.size.width <= width + epsilon,
                "line {line_idx} extends past {width:?}: {line:?}"
            );
            assert!(
                line.origin.y + line.size.height <= layout.size.height + epsilon,
                "line {line_idx} extends past the layout height"
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
                for glyph in fragment.glyphs.iter() {
                    assert!(f32::from(glyph.position.x).is_finite());
                    assert!(f32::from(glyph.position.y).is_finite());
                }
            }
        }

        let line_height = px(24.0);
        let wrapped = shaped_text_layout(layout.clone(), px(120.0));
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

        assert!(
            wrapped.previous_visual_caret(caret).is_none(),
            "visual traversal did not stop at the left edge of {text:?}: {caret:?}"
        );

        for caret in seen {
            let bounds = wrapped
                .visual_position_for_caret(caret, line_height)
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
    fn inline_ranges_forward_font_metrics_and_keep_native_geometry() {
        let system = test_system();

        let text = "small\nBIG words\nend";
        let runs = [TextRun {
            len: text.len(),
            font: font("IBM Plex Sans"),
            ..Default::default()
        }];

        let text_styles = [InlineTextStyle {
            range: 6..15,
            font_size: px(30.),
            line_height: px(46.),
        }];

        let layout = system.layout_inline(InlineLayoutRequest {
            text,
            runs: &runs,
            text_styles: &text_styles,
            boxes: &[],
            font_size: px(14.),
            line_height: px(20.),
            text_metrics: InlineTextMetrics {
                ascent: px(11.),
                descent: px(3.),
                x_height: px(7.),
            },

            options: TextLayoutOptions {
                wrap_width: Some(px(300.)),
                text_align: TextAlign::Left,
                ..Default::default()
            },
            bidi_scopes: &[],
        });

        assert_eq!(layout.lines.len(), 3);
        assert!(
            layout.lines[1].size.height >= px(45.99),
            "{:?}",
            layout.lines
        );
        assert!(
            layout
                .layout
                .paint_fragments
                .iter()
                .any(|fragment| fragment.font_size == px(30.))
        );
        assert!(
            layout
                .layout
                .paint_fragments
                .iter()
                .any(|fragment| fragment.font_size == px(14.))
        );

        let native = layout.layout.platform_layout.inline_geometry(16..19);
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].1, 2);
        assert!(native[0].0.origin.y >= px(65.99), "{native:?}");

        let selection = layout
            .layout
            .platform_layout
            .selection_geometry(16..19, px(20.));
        assert_eq!(
            selection[0].origin.y,
            px(40.),
            "ordinary selection retains its existing row policy"
        );

        assert!(
            layout
                .layout
                .platform_layout
                .inline_geometry(5..6)
                .is_empty(),
            "newlines add no span hit region"
        );
        assert!(
            layout
                .layout
                .platform_layout
                .inline_geometry(6..6)
                .is_empty()
        );
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
            text_styles: &[],
            text,
            runs: &runs,
            boxes: &boxes,
            font_size: px(18.0),
            line_height: px(24.0),
            text_metrics,
            options: TextLayoutOptions {
                wrap_width: Some(px(160.0)),
                text_align: TextAlign::Center,
                ..Default::default()
            },
            bidi_scopes: &[],
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

        let box_and_line = |box_id| {
            let inline_box = layout
                .boxes
                .iter()
                .find(|inline_box| inline_box.id == box_id)
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
            text_styles: &[],
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
            options: TextLayoutOptions {
                wrap_width: Some(px(24.0)),
                text_align: TextAlign::Left,
                ..Default::default()
            },
            bidi_scopes: &[],
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

    #[test]
    fn paragraph_direction_does_not_leak_across_newlines() {
        let system = test_system();
        let text = "שלום עולם\nabc אבג def\nx (مرحبا) y";
        let runs = [text_run(text, "IBM Plex Sans")];
        let document = shaped_text_layout(layout_line(&system, text, px(18.0), &runs), px(500.0));

        for (left, right) in [("abc", "def"), ("x", "y")] {
            let left_position = document
                .position_for_index(text.find(left).unwrap(), px(24.0))
                .unwrap();
            let right_position = document
                .position_for_index(text.find(right).unwrap(), px(24.0))
                .unwrap();

            assert_eq!(left_position.y, right_position.y);
            assert!(
                left_position.x < right_position.x,
                "{left} must precede {right}"
            );
        }
    }

    #[test]
    fn paragraph_paint_and_carets_match_independent_layouts() {
        let system = test_system();
        let sample =
            "שלום עולם\nمرحبا بالعالم\nabc אבג def\nx (مرحبا) y\nEnglish ثم عربي ثم English";

        for text in [
            sample.to_string(),
            format!("Latin first\n{sample}\n\n"),
            "אבג\u{2028}abc אבג def\n123 (45)\n\u{2067}אבג\u{2069} abc\nمرحبا".to_string(),
        ] {
            for width in [px(500.), px(90.)] {
                let document = layout_wrapped(
                    &system,
                    &text,
                    px(18.),
                    &[text_run(&text, "IBM Plex Sans")],
                    width,
                    None,
                );
                assert_document_contract(&text, &document);

                let mut first_line = 0;

                for source in paragraph_ranges(&text) {
                    let content = &text[source.content.clone()];
                    let independent = layout_wrapped(
                        &system,
                        content,
                        px(18.),
                        &[text_run(content, "IBM Plex Sans")],
                        width,
                        None,
                    );

                    for (idx, expected) in independent.visual_lines.iter().enumerate() {
                        let actual = &document.visual_lines[first_line + idx];
                        assert_eq!(actual.advance, expected.advance);
                        assert_eq!(
                            document.paint_fragments[actual.fragment_range.clone()],
                            independent.paint_fragments[expected.fragment_range.clone()],
                            "{content:?} at {width:?}"
                        );
                    }

                    for (idx, _character) in content
                        .char_indices()
                        .chain(std::iter::once((content.len(), '\0')))
                    {
                        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
                            let local = CaretPosition::new(idx, affinity);
                            let global = CaretPosition::new(source.content.start + idx, affinity);
                            let mut expected = independent
                                .platform_layout
                                .caret_geometry(local, px(24.))
                                .unwrap();
                            expected.origin.y += px(24.) * first_line;

                            assert_eq!(
                                document.platform_layout.caret_geometry(global, px(24.)),
                                Some(expected)
                            );
                        }
                    }

                    first_line += independent.visual_lines.len();
                }

                assert_eq!(first_line, document.visual_lines.len());
                assert!(
                    document
                        .paint_fragments
                        .iter()
                        .flat_map(|fragment| fragment.glyphs.iter())
                        .all(|glyph| glyph.id != GlyphId(0)),
                    "fixtures must cover the sample"
                );
            }
        }

        let arabic = "مرحبا بالعالم";
        let layout = layout_line(
            &system,
            arabic,
            px(24.),
            &[text_run(arabic, "Noto Sans Arabic")],
        );
        let fragment = &layout.paint_fragments[0];
        let nominal = arabic
            .chars()
            .map(|character| system.glyph_for_char(fragment.font_id, character).unwrap())
            .collect::<Vec<_>>();

        assert!(
            fragment
                .glyphs
                .iter()
                .any(|glyph| !nominal.contains(&glyph.id)),
            "Arabic must use contextual forms"
        );
    }

    #[test]
    fn paragraph_interaction_preserves_breaks_and_visual_traversal() {
        let system = test_system();
        let text = "שלום עולם\r\n\nabc אבג def\u{2029}مرحبا بالعالم\n";
        let layout = layout_wrapped(
            &system,
            text,
            px(18.),
            &[text_run(text, "IBM Plex Sans")],
            px(100.),
            None,
        );
        let native = &layout.platform_layout;
        let line_height = px(24.);
        let geometry = |caret| native.caret_geometry(caret, line_height).unwrap();
        let mut caret = native
            .caret_from_point(point(px(-100.), px(12.)), line_height)
            .unwrap_err();
        let mut steps = 0;

        while let Some(next) = native.move_visual(caret, VisualDirection::Right) {
            let previous = native
                .move_visual(next, VisualDirection::Left)
                .unwrap_or_else(|| panic!("cannot reverse {caret:?} -> {next:?}"));
            assert_eq!(
                geometry(previous),
                geometry(caret),
                "visual movement must reverse {caret:?} -> {next:?} -> {previous:?}"
            );

            let bounds = geometry(next);
            let hit = native
                .caret_from_point(
                    point(bounds.origin.x, bounds.origin.y + line_height / 2.),
                    line_height,
                )
                .unwrap_or_else(|caret| caret);
            assert_eq!(geometry(hit), bounds);
            assert!(text.is_char_boundary(next.index));
            caret = next;
            steps += 1;
            assert!(steps < text.len() * 4);
        }

        assert!(steps > text.chars().count() / 2);

        for source in paragraph_ranges(text) {
            if source.separator.is_empty() {
                continue;
            }

            let before = CaretPosition::new(source.separator.start, CaretAffinity::Downstream);
            let after = CaretPosition::new(source.separator.end, CaretAffinity::Downstream);
            assert_eq!(
                native.logical_cluster_after(before),
                Some(source.separator.clone())
            );
            assert_eq!(
                native.logical_cluster_before(after),
                Some(source.separator.clone())
            );
            assert!(
                !native
                    .selection_geometry(source.separator.clone(), line_height)
                    .is_empty()
            );
            assert!(native.inline_geometry(source.separator.clone()).is_empty());

            let position = geometry(before).origin + point(px(0.), line_height / 2.);
            let selected =
                native.selection_from_point(position, line_height, TextSelectionKind::HardLine);
            assert_eq!(selected, source.content.start..source.separator.end);

            for (movement, expected) in [
                (
                    Direction::Start.with_boundary(Boundary::HardLine),
                    source.content.start,
                ),
                (
                    Direction::End.with_boundary(Boundary::HardLine),
                    source.content.end,
                ),
            ] {
                assert_eq!(
                    native.caret_movement(before, movement, None).caret.index,
                    expected
                );
            }
        }

        let crlf = text.find('\r').unwrap();
        let inside = CaretPosition::new(crlf + 1, CaretAffinity::Downstream);
        assert_eq!(native.refresh_caret(inside).index, crlf);

        let start = CaretPosition::default();
        let down = native.caret_movement(
            start,
            Direction::Down.with_boundary(Boundary::VisualLine),
            None,
        );
        assert_eq!(
            geometry(down.caret).origin.y,
            geometry(start).origin.y + line_height
        );
        assert_eq!(down.preferred_x, Some(geometry(start).origin.x));

        let mut vertical_caret = start;
        let mut preferred_x = None;

        for line_idx in 1..native.line_count() {
            let moved = native.caret_movement(
                vertical_caret,
                Direction::Down.with_boundary(Boundary::VisualLine),
                preferred_x,
            );
            vertical_caret = moved.caret;
            preferred_x = moved.preferred_x;

            assert_eq!(geometry(vertical_caret).origin.y, line_height * line_idx);
            assert_eq!(preferred_x, Some(geometry(start).origin.x));
        }

        for line_idx in (0..native.line_count() - 1).rev() {
            let moved = native.caret_movement(
                vertical_caret,
                Direction::Up.with_boundary(Boundary::VisualLine),
                preferred_x,
            );
            vertical_caret = moved.caret;
            preferred_x = moved.preferred_x;

            assert_eq!(geometry(vertical_caret).origin.y, line_height * line_idx);
        }

        let document_start = native.caret_movement(
            CaretPosition::attached_to_previous_cluster(text.len()),
            Direction::Start.with_boundary(Boundary::Document),
            None,
        );
        assert_eq!(
            document_start.caret,
            CaretPosition::attached_to_next_cluster(0)
        );

        let document_end = native.caret_movement(
            CaretPosition::attached_to_next_cluster(0),
            Direction::End.with_boundary(Boundary::Document),
            None,
        );
        assert_eq!(
            document_end.caret,
            CaretPosition::attached_to_previous_cluster(text.len())
        );

        for direction in [VisualDirection::Left, VisualDirection::Right] {
            let movement = match direction {
                VisualDirection::Left => Direction::Left.with_boundary(Boundary::Word),
                VisualDirection::Right => Direction::Right.with_boundary(Boundary::Word),
            };
            let edge_x = match direction {
                VisualDirection::Left => px(-100.),
                VisualDirection::Right => px(10_000.),
            };
            let empty_row = native
                .caret_from_point(point(edge_x, line_height * 1.5), line_height)
                .unwrap_or_else(|caret| caret);
            let word = native.caret_movement(empty_row, movement, None).caret;
            assert_ne!(geometry(word).origin.y, geometry(empty_row).origin.y);
        }

        let selected = native.selection_geometry(0..text.len(), line_height);
        assert!(
            selected
                .windows(2)
                .all(|pair| pair[0].origin.y <= pair[1].origin.y)
        );

        let mixed = "abc אבג\nאבג abc\n";
        let layout = layout_line(&system, mixed, px(18.), &[text_run(mixed, "IBM Plex Sans")]);

        for (line_idx, source) in paragraph_ranges(mixed).into_iter().take(2).enumerate() {
            let selection = layout
                .platform_layout
                .selection_geometry(source.separator, line_height);
            let bounds = selection[0];
            assert_eq!(bounds.origin.y, line_height * line_idx);

            if line_idx == 0 {
                assert!(bounds.origin.x >= layout.visual_lines[line_idx].advance);
            } else {
                assert!(bounds.right() <= Pixels::ZERO);
            }
        }
    }

    #[test]
    fn paragraph_inline_layout_preserves_styles_empty_rows_and_boundary_boxes() {
        let system = test_system();
        let text = "אבג\r\nalpha مرحبا omega\r\n\r\n";
        let split = text.find("مرحبا").unwrap();
        let runs = [
            TextRun {
                len: split,
                color: hsla(0.2, 0.8, 0.5, 1.),
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    ..Default::default()
                }),
                ..text_run(text, "IBM Plex Sans")
            },
            TextRun {
                len: text.len() - split,
                color: hsla(0.7, 0.8, 0.5, 1.),
                ..text_run(text, "IBM Plex Sans")
            },
        ];
        let styles = [InlineTextStyle {
            range: split..text.len(),
            font_size: px(26.),
            line_height: px(48.),
        }];
        let separator = text.find('\r').unwrap();
        let boxes = [0, separator, separator + 1, separator + 2, text.len()]
            .into_iter()
            .enumerate()
            .map(|(idx, index)| InlineBoxRequest {
                id: idx as u64,
                index,
                size: size(px(10.), px(12.)),
                vertical_align: VerticalAlign::Baseline,
            })
            .collect::<Vec<_>>();

        for text_align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
            let inline = system.layout_inline(InlineLayoutRequest {
                text,
                runs: &runs,
                text_styles: &styles,
                boxes: &boxes,
                font_size: px(18.),
                line_height: px(28.),
                text_metrics: InlineTextMetrics {
                    ascent: px(16.),
                    descent: px(4.),
                    x_height: px(9.),
                },
                options: TextLayoutOptions {
                    wrap_width: Some(px(260.)),
                    text_align,
                    ..Default::default()
                },
                bidi_scopes: &[],
            });
            let mut ids = inline
                .boxes
                .iter()
                .map(|inline_box| inline_box.id)
                .collect::<Vec<_>>();
            ids.sort_unstable();

            assert_eq!(ids, vec![0, 1, 2, 3, 4]);
            assert_inline_geometry_is_contained(&inline, px(260.));
            assert_eq!(inline.layout.len, text.len());

            for inline_box in &inline.boxes {
                let expected_line = match inline_box.id {
                    0..=2 => 0,
                    3 => 1,
                    4 => inline.lines.len() - 1,
                    _ => unreachable!(),
                };

                assert_eq!(inline_box.line_index, expected_line);
            }

            for line in inline.lines.iter().rev().take(2) {
                assert!(
                    line.size.height >= px(47.99),
                    "empty rows must retain styled line height: {line:?}"
                );
            }

            let empty_line = inline.lines[inline.lines.len() - 2];
            let empty_caret = inline
                .layout
                .platform_layout
                .caret_geometry(
                    CaretPosition::new(text.len() - 2, CaretAffinity::Downstream),
                    px(28.),
                )
                .unwrap();
            assert_eq!(empty_line.size.width, Pixels::ZERO);
            assert_eq!(empty_caret.origin.x, empty_line.origin.x);

            assert!(
                inline
                    .layout
                    .paint_fragments
                    .iter()
                    .any(|fragment| fragment.style.color == runs[0].color
                        && fragment.style.underline.is_some())
            );
            assert!(
                inline
                    .layout
                    .paint_fragments
                    .iter()
                    .any(|fragment| fragment.style.color == runs[1].color
                        && fragment.font_size == px(26.))
            );

            let regions = inline
                .layout
                .platform_layout
                .inline_geometry(split..text.len());
            assert!(
                regions
                    .iter()
                    .all(|(bounds, idx)| bounds.origin.y >= inline.lines[*idx].origin.y)
            );
        }
    }

    #[test]
    fn paragraph_composition_preserves_existing_latin_wrapping_and_clamping() {
        let system = test_system();
        let text = "one two three four\nfive six seven eight\nnine ten\n";
        let runs = [text_run(text, "IBM Plex Sans")];

        for max_lines in [None, Some(0), Some(1), Some(3), Some(8)] {
            let document = layout_wrapped(&system, text, px(18.), &runs, px(80.), max_lines);
            let previous = system
                .parley_paragraph_layout(
                    TextLayoutRequest {
                        text,
                        font_size: px(18.),
                        runs: &runs,
                        options: TextLayoutOptions {
                            wrap_width: Some(px(80.)),
                            line_clamp: max_lines,
                            ..TextLayoutOptions::default()
                        },
                    },
                    &[],
                    &[],
                    None,
                    None,
                    &[],
                )
                .unwrap();

            assert_eq!(
                document
                    .visual_lines
                    .iter()
                    .map(|line| line.text_range.clone())
                    .collect::<Vec<_>>(),
                previous
                    .layout
                    .visual_lines
                    .iter()
                    .map(|line| line.text_range.clone())
                    .collect::<Vec<_>>()
            );
            assert_document_contract(text, &document);
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
        assert_eq!(layout.width, Pixels::ZERO);
        assert!(layout.paint_fragments.is_empty());
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
    fn paragraph_caches_preserve_shaping_across_widths_and_edits() {
        let system = test_system();
        let text = "one two three four five";
        let runs = [text_run(text, "IBM Plex Sans")];
        layout_wrapped(&system, text, px(18.0), &runs, px(80.0), None);
        layout_wrapped(&system, text, px(18.0), &runs, px(140.0), None);

        assert_eq!(system.paragraph_cache.lock().entries.len(), 1);
        assert_eq!(system.paragraph_result_cache.lock().entries.len(), 2);

        let first = "stable paragraph\nfirst ending";
        let second = "stable paragraph\nsecond ending";
        layout_wrapped(
            &system,
            first,
            px(18.0),
            &[text_run(first, "IBM Plex Sans")],
            px(120.0),
            None,
        );
        let cache_entries = system.paragraph_cache.lock().entries.len();
        layout_wrapped(
            &system,
            second,
            px(18.0),
            &[text_run(second, "IBM Plex Sans")],
            px(120.0),
            None,
        );

        assert_eq!(
            system.paragraph_cache.lock().entries.len(),
            cache_entries + 1,
            "the unchanged first paragraph should keep its cached shaping"
        );
    }

    #[test]
    fn paint_cache_preserves_identical_input_run_boundaries() {
        let system = test_system();
        let text = "left right";
        let mut left = text_run("left ", "IBM Plex Sans");
        let mut right = text_run("right", "IBM Plex Sans");
        let first = layout_line(&system, text, px(18.0), &[left.clone(), right.clone()]);

        left.color = hsla(0.0, 0.8, 0.4, 1.0);
        right.color = hsla(0.6, 0.8, 0.4, 1.0);
        let repainted = layout_line(&system, text, px(18.0), &[left.clone(), right.clone()]);

        assert!(Arc::ptr_eq(
            &first.platform_layout,
            &repainted.platform_layout
        ));
        assert!(
            repainted
                .paint_fragments
                .iter()
                .any(|fragment| fragment.style == PaintStyle::from(&left))
        );
        assert!(
            repainted
                .paint_fragments
                .iter()
                .any(|fragment| fragment.style == PaintStyle::from(&right))
        );
    }

    #[test]
    fn raster_policy_queries_are_cached_away_from_mutable_rasterization() {
        struct PolicyRasterizer {
            prepare_calls: Arc<AtomicUsize>,
            recommended_calls: Arc<AtomicUsize>,
        }

        impl GlyphRasterizer for PolicyRasterizer {
            fn prepare_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
                self.prepare_calls.fetch_add(1, Ordering::SeqCst);
                PreparedRasterStyle::independent(request.requested_mode)
            }

            fn rasterize(
                &mut self,
                _face: RasterFace<'_>,
                _params: &RenderGlyphParams,
            ) -> Result<RasterizedGlyph> {
                Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask))
            }

            fn recommended_mode(&self) -> TextRenderingMode {
                self.recommended_calls.fetch_add(1, Ordering::SeqCst);
                TextRenderingMode::Grayscale
            }
        }

        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let recommended_calls = Arc::new(AtomicUsize::new(0));
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "IBM Plex Sans",
            PolicyRasterizer {
                prepare_calls: prepare_calls.clone(),
                recommended_calls: recommended_calls.clone(),
            },
        );
        let request = RasterStyleRequest {
            font_id: FontId(1),
            glyph_id: GlyphId(1),
            scene_color: gpui::rgba(0x334455ff),
            requested_mode: GlyphRenderMode::Grayscale,
            foreground_dependency: gpui::ForegroundDependency::Full,
        };

        assert_eq!(
            system.prepare_raster_style(request),
            system.prepare_raster_style(RasterStyleRequest {
                font_id: FontId(2),
                glyph_id: GlyphId(2),
                ..request
            })
        );
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            system.recommended_rendering_mode(FontId(1), px(16.0)),
            TextRenderingMode::Grayscale
        );
        assert_eq!(
            system.recommended_rendering_mode(FontId(2), px(24.0)),
            TextRenderingMode::Grayscale
        );
        assert_eq!(recommended_calls.load(Ordering::SeqCst), 1);
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
        let source_serif_regular = backend.font_id(&font("Source Serif 4")).unwrap();
        assert_eq!(after.paint_fragments[0].font_id, source_serif_regular);

        backend
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SEMIBOLD)])
            .unwrap();
        let regular = backend.font_id(&font("IBM Plex Sans")).unwrap();
        let bold = backend.font_id(&font("IBM Plex Sans").bold()).unwrap();
        assert_ne!(regular, bold);

        let mut variable = font("Source Serif 4");
        variable.weight = GpuiFontWeight(725.0);
        variable.style = GpuiFontStyle::Oblique;
        let variable_id = backend.font_id(&variable).unwrap();
        assert_ne!(source_serif_regular, variable_id);
    }

    #[test]
    fn native_interaction_handles_bidi_atomic_clusters_and_semantic_selection() {
        let system = test_system();
        for text in ["👩🏽‍💻", "👨‍👩‍👧‍👦", "🇬🇧", "ก้"] {
            let layout = layout_line(&system, text, px(22.0), &[text_run(text, "IBM Plex Sans")]);
            let wrapped = shaped_text_layout(layout, px(500.0));
            assert_eq!(
                wrapped.logical_cluster_after(CaretPosition::default()),
                Some(0..text.len()),
                "{text:?} must be one editable cluster"
            );
            let end = wrapped
                .platform_layout
                .move_visual(CaretPosition::default(), VisualDirection::Right)
                .unwrap();
            assert_eq!(
                end.index,
                text.len(),
                "{text:?} must have no internal caret"
            );
            assert_eq!(
                wrapped
                    .platform_layout
                    .move_visual(end, VisualDirection::Left)
                    .unwrap()
                    .index,
                0,
                "{text:?} must be one visual caret step"
            );
        }

        let single_line_text = "abcd";
        let single_line = shaped_text_layout(
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
                .caret_movement(
                    middle,
                    Direction::Up.with_boundary(Boundary::VisualLine),
                    None,
                )
                .caret
                .index,
            0
        );
        assert_eq!(
            single_line
                .caret_movement(
                    middle,
                    Direction::Down.with_boundary(Boundary::VisualLine),
                    None,
                )
                .caret
                .index,
            single_line_text.len()
        );

        let text = "abc אבג العربية xyz";
        let layout = shaped_text_layout(
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
            Direction::Left.with_boundary(Boundary::Cluster),
            false,
            None,
            line_height,
        );
        assert!(collapsed_left.selection.is_empty());
        assert_eq!(collapsed_left.selection.focus, start);
        let collapsed_right = layout.move_selection(
            selection,
            Direction::Right.with_boundary(Boundary::Cluster),
            false,
            None,
            line_height,
        );
        assert_eq!(collapsed_right.selection.focus, end);

        let word = layout.move_selection(
            CaretSelection::collapsed(start),
            Direction::Right.with_boundary(Boundary::Word),
            true,
            None,
            line_height,
        );
        assert_eq!(word.selection.anchor, start);
        assert_ne!(word.selection.focus, start);
        let down = layout.move_selection(
            CaretSelection::collapsed(word.selection.focus),
            Direction::Down.with_boundary(Boundary::VisualLine),
            false,
            None,
            line_height,
        );
        assert!(down.preferred_x.is_some());
        let maintained_x = layout
            .move_selection(
                down.selection,
                Direction::Down.with_boundary(Boundary::VisualLine),
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
    fn bundled_emoji_sample_keeps_font_metrics_and_device_scaling() {
        const SAMPLE: &str = "Color emoji: 😀 🎉 🚀 💡 🔥 ✨";
        const EMOJI: [char; 6] = ['😀', '🎉', '🚀', '💡', '🔥', '✨'];

        let system = test_system();
        let noto_id = system.font_id(&font("Noto Color Emoji")).unwrap();

        for font_size in [px(16.0), px(24.0), px(32.0)] {
            let layout = layout_line(
                &system,
                SAMPLE,
                font_size,
                &[text_run(SAMPLE, "IBM Plex Sans")],
            );
            let emoji_glyphs = layout
                .paint_fragments
                .iter()
                .flat_map(|fragment| {
                    fragment
                        .glyphs
                        .iter()
                        .filter(|glyph| glyph.is_emoji)
                        .map(move |glyph| (fragment, glyph))
                })
                .collect::<Vec<_>>();

            assert_eq!(emoji_glyphs.len(), EMOJI.len());
            assert_eq!(layout.font_size, font_size);

            for (character, (fragment, glyph)) in EMOJI.into_iter().zip(emoji_glyphs) {
                assert_eq!(
                    fragment.font_id, noto_id,
                    "{character} selected another font"
                );
                assert_eq!(fragment.font_size, font_size);
                assert_eq!(glyph.position.y, Pixels::ZERO);
                assert_eq!(
                    glyph.id,
                    system.glyph_for_char(noto_id, character).unwrap(),
                    "{character} did not use its nominal bundled glyph"
                );

                let glyph_idx = fragment
                    .glyphs
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, glyph))
                    .unwrap();
                let shaped_advance = fragment
                    .glyphs
                    .get(glyph_idx + 1)
                    .map(|next| next.position.x - glyph.position.x)
                    .unwrap_or(fragment.x_range.end - glyph.position.x);
                assert!(shaped_advance > Pixels::ZERO);
                let raster_style = system.prepare_raster_style(RasterStyleRequest {
                    font_id: fragment.font_id,
                    glyph_id: glyph.id,
                    scene_color: gpui::rgba(0xffffffff),
                    requested_mode: GlyphRenderMode::Color,
                    foreground_dependency: gpui::ForegroundDependency::Full,
                });
                let mut scale_one_bounds: Option<[f32; 4]> = None;

                for scale_factor in [1.0, 1.5, 2.0] {
                    let raster = system
                        .rasterize_glyph(&RenderGlyphParams {
                            font_id: fragment.font_id,
                            glyph_id: glyph.id,
                            font_size: fragment.font_size,
                            subpixel_variant: point(0, 0),
                            scale_factor,
                            raster_style,
                        })
                        .unwrap();
                    raster.validate().unwrap();
                    assert_eq!(raster.format, RasterizedGlyphFormat::BgraColor);
                    assert!(
                        raster.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0),
                        "{character} produced no visible pixels"
                    );

                    let logical_bounds = [
                        raster.bounds.origin.x.0 as f32 / scale_factor,
                        raster.bounds.origin.y.0 as f32 / scale_factor,
                        raster.size.width.0 as f32 / scale_factor,
                        raster.size.height.0 as f32 / scale_factor,
                    ];

                    if let Some(reference) = scale_one_bounds {
                        for (actual, expected) in logical_bounds.into_iter().zip(reference) {
                            assert!(
                                (actual - expected).abs() <= 1.0,
                                "{character} changed logical raster bounds from {reference:?} to {logical_bounds:?} at scale {scale_factor}"
                            );
                        }
                    } else {
                        scale_one_bounds = Some(logical_bounds);
                    }
                }
            }
        }
    }

    #[test]
    fn bundled_joined_emoji_sequence_shapes_as_one_color_glyph() {
        let system = test_system();
        let text = "👩🏽‍💻";
        let layout = layout_line(
            &system,
            text,
            px(24.0),
            &[text_run(text, "Noto Color Emoji")],
        );
        let glyphs = layout
            .paint_fragments
            .iter()
            .flat_map(|fragment| fragment.glyphs.iter())
            .collect::<Vec<_>>();

        assert_eq!(glyphs.len(), 1);
        assert!(glyphs[0].is_emoji);
    }

    #[test]
    fn fixed_color_bitmaps_ignore_foreground_rgb_but_preserve_alpha() {
        let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
        system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX),
                Cow::Borrowed(NOTO_COLOR_EMOJI),
            ])
            .unwrap();
        let font_id = system.font_id(&font("Noto Color Emoji")).unwrap();
        let glyph_id = system.glyph_for_char(font_id, '😀').unwrap();
        let style = |color| {
            system.prepare_raster_style(RasterStyleRequest {
                font_id,
                glyph_id,
                scene_color: color,
                requested_mode: GlyphRenderMode::Color,
                foreground_dependency: gpui::ForegroundDependency::Full,
            })
        };

        let red = style(gpui::rgba(0xff0000cc));
        let blue = style(gpui::rgba(0x0000ffcc));
        let translucent = style(gpui::rgba(0x0000ff80));
        assert_eq!(red, blue);
        assert_eq!(
            red.foreground_dependency,
            gpui::ForegroundDependency::AlphaOnly
        );
        assert_ne!(red, translucent);
    }

    #[test]
    fn automatic_optical_sizing_tracks_logical_default_and_inline_sizes() {
        let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "Source Serif 4")
            .with_automatic_optical_sizing();
        system.add_fonts(vec![Cow::Borrowed(SOURCE_SERIF)]).unwrap();

        let optical_value = |font_id| {
            system
                .font_instances
                .read()
                .get(font_id)
                .unwrap()
                .variations
                .iter()
                .find(|variation| variation.tag == skrifa::Tag::new(b"opsz"))
                .unwrap()
                .value
        };

        for font_size in [px(12.0), px(48.0)] {
            let layout = layout_line(&system, "A", font_size, &[text_run("A", "Source Serif 4")]);
            let fragment = &layout.paint_fragments[0];
            assert_eq!(optical_value(fragment.font_id), f32::from(font_size));

            for scale_factor in [1.0, 2.0] {
                let params = RenderGlyphParams {
                    font_id: fragment.font_id,
                    glyph_id: fragment.glyphs[0].id,
                    font_size,
                    subpixel_variant: point(0, 0),
                    scale_factor,
                    raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
                };
                system.rasterize_glyph(&params).unwrap();
                assert_eq!(optical_value(fragment.font_id), f32::from(font_size));
            }
        }

        let text = "small large";
        let runs = [text_run(text, "Source Serif 4")];
        let text_styles = [InlineTextStyle {
            range: 6..text.len(),
            font_size: px(48.0),
            line_height: px(52.0),
        }];
        let inline = system.layout_inline(InlineLayoutRequest {
            text,
            runs: &runs,
            text_styles: &text_styles,
            boxes: &[],
            font_size: px(12.0),
            line_height: px(16.0),
            text_metrics: InlineTextMetrics::default(),
            options: TextLayoutOptions {
                text_align: TextAlign::Left,
                ..Default::default()
            },
            bidi_scopes: &[],
        });
        let small = inline
            .layout
            .paint_fragments
            .iter()
            .find(|fragment| fragment.font_size == px(12.0))
            .unwrap();
        let large = inline
            .layout
            .paint_fragments
            .iter()
            .find(|fragment| fragment.font_size == px(48.0))
            .unwrap();
        assert_ne!(small.font_id, large.font_id);
        assert_eq!(optical_value(small.font_id), 12.0);
        assert_eq!(optical_value(large.font_id), 48.0);
    }

    #[test]
    fn native_backend_receives_the_face_instance_selected_during_shaping() {
        let seen = Arc::new(Mutex::new(None));
        let system = ParleyTextSystem::new_with_rasterizer(
            SystemFonts::Skip,
            "Source Serif 4",
            RecordingRasterizer { seen: seen.clone() },
        )
        .with_automatic_optical_sizing();
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
        let optical_size = seen
            .variations
            .iter()
            .find(|variation| variation.tag == skrifa::Tag::new(b"opsz"))
            .expect("optical-size design coordinate");
        assert_eq!(optical_size.value, 22.0);
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
                data_matches: face.data() == SOURCE_SERIF,
                variations: face.variations.to_vec(),
                synthesis: face.synthesis,
                has_color_glyphs: face.has_color_glyphs,
            });

            Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask))
        }
    }

    mod inline_reflow {
        use super::*;

        const SCALE_FACTOR: f32 = 1.5;

        const INLINE: &str = "inline-translation-root";
        const FIRST_BOX: &str = "inline-translation-first-box";
        const SECOND_BOX: &str = "inline-translation-second-box";
        const NESTED: &str = "inline-translation-nested";

        fn first_group_color() -> Hsla {
            hsla(0.02, 0.75, 0.55, 1.0)
        }

        fn second_group_color() -> Hsla {
            hsla(0.58, 0.75, 0.55, 1.0)
        }

        fn highlighted(text: &'static str, color: Hsla) -> StyledText {
            StyledText::new(text).with_highlights([(
                0..text.len(),
                HighlightStyle {
                    background_color: Some(color),
                    ..Default::default()
                },
            )])
        }

        struct InlineTranslationView {
            width_offset: f32,
        }

        impl Render for InlineTranslationView {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                div().size_full().pl(px(11.)).pt(px(9.)).child(
                    div()
                        .block()
                        .w(px(160. + self.width_offset))
                        .p(px(7.))
                        .border_1()
                        .text_size(px(17.))
                        .line_height(px(23.))
                        .text_center()
                        .debug_selector(|| INLINE.into())
                        .child(highlighted("short line ", first_group_color()))
                        .child(
                            div()
                                .inline_flex()
                                .w(px(13.))
                                .h(px(16.))
                                .align_middle()
                                .debug_selector(|| FIRST_BOX.into()),
                        )
                        .child("\n")
                        .child(highlighted("secondline ", second_group_color()))
                        .child(
                            div()
                                .inline_flex()
                                .items_center()
                                .justify_center()
                                .w(px(29.))
                                .h(px(16.))
                                .align_middle()
                                .debug_selector(|| SECOND_BOX.into())
                                .child(div().w(px(5.)).h(px(7.)).debug_selector(|| NESTED.into())),
                        ),
                )
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq)]
        struct Geometry {
            inline: Bounds<Pixels>,
            text_groups: [Bounds<Pixels>; 2],
            first_box: Bounds<Pixels>,
            second_box: Bounds<Pixels>,
            nested: Bounds<Pixels>,
        }

        impl Geometry {
            fn read(
                cx: &mut HeadlessAppContext,
                window: WindowHandle<InlineTranslationView>,
            ) -> Self {
                let any_window = window.into();
                let mut debug_bounds = |selector| {
                    cx.debug_bounds(any_window, selector)
                        .unwrap()
                        .unwrap_or_else(|| panic!("missing debug bounds for {selector}"))
                };

                let inline = debug_bounds(INLINE);
                let first_box = debug_bounds(FIRST_BOX);
                let second_box = debug_bounds(SECOND_BOX);
                let nested = debug_bounds(NESTED);

                let text_groups = [
                    text_group_bounds(cx, any_window, first_group_color()),
                    text_group_bounds(cx, any_window, second_group_color()),
                ];

                Self {
                    inline,
                    text_groups,
                    first_box,
                    second_box,
                    nested,
                }
            }

            fn participants(self) -> [Bounds<Pixels>; 5] {
                [
                    self.text_groups[0],
                    self.text_groups[1],
                    self.first_box,
                    self.second_box,
                    self.nested,
                ]
            }

            fn nested_offset(self) -> Point<Pixels> {
                self.nested.origin - self.second_box.origin
            }

            fn assert_valid(self) {
                let epsilon = px(1.);

                assert!(self.text_groups[0].right() <= self.first_box.origin.x + epsilon);
                assert!(self.text_groups[1].right() <= self.second_box.origin.x + epsilon);
                assert!(self.text_groups[0].origin.y < self.text_groups[1].origin.y);

                for participant in self.participants() {
                    assert!(participant.origin.x >= self.inline.origin.x);
                    assert!(participant.origin.y >= self.inline.origin.y);
                    assert!(participant.right() <= self.inline.right());
                    assert!(participant.bottom() <= self.inline.bottom());
                }

                assert!(self.nested.origin.x >= self.second_box.origin.x);
                assert!(self.nested.origin.y >= self.second_box.origin.y);
                assert!(self.nested.right() <= self.second_box.right());
                assert!(self.nested.bottom() <= self.second_box.bottom());
            }

            fn assert_moved_as_one_group(self, before: Self, step: usize) {
                let expected_delta = self.text_groups[0].origin - before.text_groups[0].origin;

                for (participant, (actual, previous)) in self
                    .participants()
                    .into_iter()
                    .zip(before.participants())
                    .enumerate()
                {
                    assert_point_close(
                        actual.origin - previous.origin,
                        expected_delta,
                        &format!("participant {participant} at resize step {step}"),
                    );
                }
            }
        }

        fn text_group_bounds(
            cx: &mut HeadlessAppContext,
            window: gpui::AnyWindowHandle,
            color: Hsla,
        ) -> Bounds<Pixels> {
            let bounds = cx.solid_quad_bounds(window, color).unwrap();
            assert_eq!(bounds.len(), 1, "expected one rendered quad for {color:?}");

            logical_bounds(bounds[0])
        }

        fn logical_bounds(bounds: Bounds<ScaledPixels>) -> Bounds<Pixels> {
            bounds.map(|value| px(value.as_f32() / SCALE_FACTOR))
        }

        fn assert_close(actual: Pixels, expected: Pixels, context: &str) {
            assert!(
                (actual - expected).abs() < px(0.01),
                "{context}: expected {actual:?} to equal {expected:?}"
            );
        }

        fn assert_point_close(actual: Point<Pixels>, expected: Point<Pixels>, context: &str) {
            assert_close(actual.x, expected.x, context);
            assert_close(actual.y, expected.y, context);
        }

        fn update_view<View: Render>(
            cx: &mut HeadlessAppContext,
            window: WindowHandle<View>,
            edit: impl FnOnce(&mut View),
        ) {
            window
                .update(cx, |view, _window, cx| {
                    edit(view);
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
        }

        #[test]
        fn centered_inline_lines_move_as_one_group_during_fractional_resize() {
            let text_system =
                ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
            text_system
                .add_fonts(vec![Cow::Borrowed(IBM_PLEX)])
                .unwrap();

            let mut cx = HeadlessAppContext::new(Arc::new(text_system));

            let window = cx
                .open_window(size(px(340.), px(180.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| InlineTranslationView { width_offset: 0. })
                })
                .unwrap();

            cx.run_until_parked();

            let initial = Geometry::read(&mut cx, window);
            initial.assert_valid();

            for step in 1..=64 {
                update_view(&mut cx, window, |view| {
                    view.width_offset = step as f32 / 16.;
                });

                let translated = Geometry::read(&mut cx, window);
                translated.assert_valid();
                translated.assert_moved_as_one_group(initial, step);
                assert_point_close(
                    translated.nested_offset(),
                    initial.nested_offset(),
                    &format!("nested offset at resize step {step}"),
                );
            }

            update_view(&mut cx, window, |view| view.width_offset = 0.);
            assert_eq!(Geometry::read(&mut cx, window), initial);
        }

        fn headless() -> HeadlessAppContext {
            let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
            system
                .add_fonts(vec![
                    Cow::Borrowed(IBM_PLEX),
                    Cow::Borrowed(IBM_PLEX_SEMIBOLD),
                ])
                .unwrap();
            HeadlessAppContext::new(Arc::new(system))
        }

        type CapturedParagraphs = Rc<RefCell<Vec<(gpui::SharedString, Arc<InlineLayout>)>>>;

        struct ParagraphProbe {
            element: gpui::Div,
            painted: CapturedParagraphs,
            measurements: CapturedParagraphs,
            remeasure: bool,
        }

        impl IntoElement for ParagraphProbe {
            type Element = Self;

            fn into_element(self) -> Self {
                self
            }
        }

        impl gpui::Element for ParagraphProbe {
            type RequestLayoutState = gpui::DivFrameState;
            type PrepaintState = <gpui::Div as gpui::Element>::PrepaintState;

            fn id(&self) -> Option<gpui::ElementId> {
                None
            }

            fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
                None
            }

            fn request_layout(
                &mut self,
                global_id: Option<&gpui::GlobalElementId>,
                inspector_id: Option<&gpui::InspectorElementId>,
                window: &mut Window,
                cx: &mut gpui::App,
            ) -> (gpui::LayoutId, Self::RequestLayoutState) {
                let (layout_id, state) =
                    self.element
                        .request_layout(global_id, inspector_id, window, cx);

                if self.remeasure {
                    self.measurements.borrow_mut().clear();

                    for width in [80., 200., 2000., 80.] {
                        window.compute_layout(
                            layout_id,
                            size(
                                gpui::AvailableSpace::Definite(px(width)),
                                gpui::AvailableSpace::MaxContent,
                            ),
                            cx,
                        );
                        self.measurements
                            .borrow_mut()
                            .extend(state.measured_inline_paragraphs());
                    }
                }

                (layout_id, state)
            }

            fn prepaint(
                &mut self,
                global_id: Option<&gpui::GlobalElementId>,
                inspector_id: Option<&gpui::InspectorElementId>,
                bounds: Bounds<Pixels>,
                state: &mut Self::RequestLayoutState,
                window: &mut Window,
                cx: &mut gpui::App,
            ) -> Self::PrepaintState {
                self.element
                    .prepaint(global_id, inspector_id, bounds, state, window, cx)
            }

            fn paint(
                &mut self,
                global_id: Option<&gpui::GlobalElementId>,
                inspector_id: Option<&gpui::InspectorElementId>,
                bounds: Bounds<Pixels>,
                state: &mut Self::RequestLayoutState,
                prepaint: &mut Self::PrepaintState,
                window: &mut Window,
                cx: &mut gpui::App,
            ) {
                *self.painted.borrow_mut() = state.measured_inline_paragraphs();
                self.element
                    .paint(global_id, inspector_id, bounds, state, prepaint, window, cx);
            }
        }

        const OVERFLOW_TEXT: &str =
            "Begin café e\u{301} 👩‍👩‍👧‍👦 and several words that need room before the ending";
        const OVERFLOW_MIDDLE: &str = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW";
        const LINE_CLAMP_TEXT: &str = "A paragraph can wrap at word boundaries while keeping styled text, punctuation, and spacing together. Resize the window and this sample will follow the available width.";

        struct OverflowParagraph {
            text: &'static str,
            width: f32,
            direction: gpui::TruncateFrom,
            max_lines: Option<usize>,
            styled: bool,
            marker: bool,
            remeasure: bool,
            painted: CapturedParagraphs,
            measurements: CapturedParagraphs,
        }

        impl OverflowParagraph {
            fn new(direction: gpui::TruncateFrom) -> Self {
                Self {
                    text: OVERFLOW_TEXT,
                    width: 180.,
                    direction,
                    max_lines: None,
                    styled: false,
                    marker: true,
                    remeasure: false,
                    painted: Rc::default(),
                    measurements: Rc::default(),
                }
            }
        }

        impl Render for OverflowParagraph {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                let mut paragraph = div()
                    .block()
                    .w_full()
                    .min_w_0()
                    .font_family("IBM Plex Sans")
                    .text_size(px(14.))
                    .line_height(px(24.))
                    .text_color(first_group_color());

                if self.marker {
                    paragraph = match self.direction {
                        gpui::TruncateFrom::End => paragraph.text_ellipsis(),
                        gpui::TruncateFrom::Start => paragraph.text_ellipsis_start(),
                        gpui::TruncateFrom::Middle => paragraph.text_ellipsis_middle(),
                    };
                }

                paragraph = if let Some(count) = self.max_lines {
                    paragraph.line_clamp(count)
                } else {
                    paragraph.whitespace_nowrap()
                };

                paragraph = if self.styled {
                    paragraph.child(
                        div()
                            .inline()
                            .bg(first_group_color())
                            .child("begin ")
                            .child(
                                div()
                                    .inline()
                                    .text_size(px(30.))
                                    .line_height(px(46.))
                                    .text_color(second_group_color())
                                    .bg(second_group_color())
                                    .child(OVERFLOW_MIDDLE),
                            )
                            .child(" end"),
                    )
                } else {
                    paragraph.child(self.text)
                };

                div()
                    .size_full()
                    .items_start()
                    .child(div().w(px(self.width)).child(ParagraphProbe {
                        element: paragraph,
                        painted: self.painted.clone(),
                        measurements: self.measurements.clone(),
                        remeasure: self.remeasure,
                    }))
            }
        }

        fn assert_paragraph_fits(text: &str, layout: &InlineLayout, width: f32, max_lines: usize) {
            assert_eq!(layout.layout.len, text.len());
            assert!(
                layout.lines.len() <= max_lines,
                "{text:?}: {:?}",
                layout.lines
            );
            assert!(
                layout.layout.platform_layout.size().width <= px(width + 0.01),
                "{text:?}: {:?}",
                layout.layout.visual_lines
            );
        }

        #[test]
        fn block_paragraph_paints_grapheme_safe_end_start_and_middle_ellipsis() {
            let mut cx = headless();

            for direction in [
                gpui::TruncateFrom::End,
                gpui::TruncateFrom::Start,
                gpui::TruncateFrom::Middle,
            ] {
                let view = OverflowParagraph::new(direction);
                let painted = view.painted.clone();
                let window = cx
                    .open_window(size(px(360.), px(180.)), |window, cx| {
                        window.set_scale_factor(SCALE_FACTOR);

                        cx.new(|_cx| view)
                    })
                    .unwrap();
                cx.run_until_parked();

                let captured = painted.borrow();
                assert_eq!(captured.len(), 1);
                let (text, layout) = &captured[0];
                assert_paragraph_fits(text, layout, 180., 1);
                let (prefix, suffix) = text
                    .split_once('…')
                    .expect("paragraph must paint the marker");
                assert!(OVERFLOW_TEXT.starts_with(prefix) && OVERFLOW_TEXT.ends_with(suffix));
                let boundaries: Vec<_> = OVERFLOW_TEXT
                    .grapheme_indices(true)
                    .map(|(idx, _grapheme)| idx)
                    .chain([OVERFLOW_TEXT.len()])
                    .collect();
                assert!(boundaries.contains(&prefix.len()));
                assert!(boundaries.contains(&(OVERFLOW_TEXT.len() - suffix.len())));

                match direction {
                    gpui::TruncateFrom::End => assert!(suffix.is_empty() && !prefix.is_empty()),
                    gpui::TruncateFrom::Start => assert!(prefix.is_empty() && !suffix.is_empty()),
                    gpui::TruncateFrom::Middle => assert!(!prefix.is_empty() && !suffix.is_empty()),
                }

                assert!(
                    !cx.glyph_bounds(window.into(), first_group_color())
                        .unwrap()
                        .is_empty()
                );
            }
        }

        #[test]
        fn block_paragraph_places_requested_marker_on_second_clamped_line() {
            let mut cx = headless();
            let mut view = OverflowParagraph::new(gpui::TruncateFrom::End);
            view.max_lines = Some(2);
            view.width = 140.;
            let painted = view.painted.clone();
            let window = cx
                .open_window(size(px(360.), px(180.)), |_window, cx| cx.new(|_cx| view))
                .unwrap();
            cx.run_until_parked();

            {
                let captured = painted.borrow();
                let (text, layout) = &captured[0];
                assert_paragraph_fits(text, layout, 140., 2);
                assert_eq!(layout.lines.len(), 2);
                assert!(text.ends_with('…'));
                assert!(
                    layout.layout.visual_lines[1]
                        .text_range
                        .contains(&(text.len() - '…'.len_utf8()))
                );
                let geometry = layout
                    .layout
                    .platform_layout
                    .inline_geometry_for_ranges(&[text.len() - '…'.len_utf8()..text.len()]);
                assert!(!geometry[0].is_empty());
                assert!(geometry[0].iter().all(|(_bounds, line_idx)| *line_idx == 1));
            }

            window
                .update(&mut cx, |view, _window, cx| {
                    view.marker = false;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let captured = painted.borrow();
            assert_eq!(captured[0].0.as_ref(), OVERFLOW_TEXT);
            assert_eq!(captured[0].1.lines.len(), 2);
        }

        #[test]
        fn block_line_clamp_ignores_wrapped_trailing_whitespace() {
            let system = test_system();
            let runs = [text_run(LINE_CLAMP_TEXT, "IBM Plex Sans")];
            let width = (300..800)
                .map(|width| px(width as f32))
                .find(|width| {
                    let layout = system.layout_text(TextLayoutRequest {
                        text: LINE_CLAMP_TEXT,
                        font_size: px(14.),
                        runs: &runs,
                        options: TextLayoutOptions {
                            wrap_width: Some(*width),
                            text_align: TextAlign::Left,
                            ..Default::default()
                        },
                    });

                    layout.visual_lines.len() == 2
                        && layout.platform_layout.size().width <= *width + px(0.01)
                        && layout
                            .visual_lines
                            .iter()
                            .any(|line| line.advance > *width + px(0.01))
                })
                .expect("sample text should have a two-line width with hanging whitespace");
            let mut cx = headless();
            let mut view = OverflowParagraph::new(gpui::TruncateFrom::End);
            view.text = LINE_CLAMP_TEXT;
            view.max_lines = Some(2);
            view.width = f32::from(width);
            let painted = view.painted.clone();
            cx.open_window(size(px(900.), px(180.)), |_window, cx| cx.new(|_cx| view))
                .unwrap();
            cx.run_until_parked();

            let captured = painted.borrow();
            let (text, layout) = &captured[0];
            assert_eq!(text.as_ref(), LINE_CLAMP_TEXT);
            assert_paragraph_fits(text, layout, f32::from(width), 2);
        }

        #[test]
        fn block_paragraph_remeasures_nowrap_overflow_and_restores_original_text() {
            let mut cx = headless();
            let mut view = OverflowParagraph::new(gpui::TruncateFrom::Middle);
            view.remeasure = true;
            let painted = view.painted.clone();
            let measurements = view.measurements.clone();
            let window = cx
                .open_window(size(px(2200.), px(180.)), |_window, cx| cx.new(|_cx| view))
                .unwrap();
            cx.run_until_parked();

            {
                let measured = measurements.borrow();
                assert_eq!(measured.len(), 4);
                assert!(measured[0].0.len() < measured[1].0.len());
                assert_eq!(measured[2].0.as_ref(), OVERFLOW_TEXT);
                assert_eq!(measured[0].0, measured[3].0);

                for ((text, layout), width) in measured.iter().zip([80., 200., 2000., 80.]) {
                    assert_paragraph_fits(text, layout, width, 1);
                }
            }

            let initial = painted.borrow()[0].0.clone();

            for width in [90., 2000., 180.] {
                update_view(&mut cx, window, |view| view.width = width);
                let captured = painted.borrow();
                assert_paragraph_fits(&captured[0].0, &captured[0].1, width, 1);

                if width == 2000. {
                    assert_eq!(captured[0].0.as_ref(), OVERFLOW_TEXT);
                }
            }

            assert_eq!(painted.borrow()[0].0, initial);
        }

        #[test]
        fn block_ellipsis_preserves_span_metrics_and_only_places_display_ranges() {
            let mut cx = headless();
            let mut view = OverflowParagraph::new(gpui::TruncateFrom::Middle);
            view.styled = true;
            view.width = 260.;
            let painted = view.painted.clone();
            let window = cx
                .open_window(size(px(360.), px(180.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);

                    cx.new(|_cx| view)
                })
                .unwrap();
            cx.run_until_parked();

            for width in [260., 100., 260.] {
                update_view(&mut cx, window, |view| view.width = width);

                let captured = painted.borrow();
                let (text, layout) = &captured[0];
                assert_paragraph_fits(text, layout, width, 1);
                assert!(text.contains('…') && text.ends_with("end"));
                let spans = quads(&mut cx, window.into(), first_group_color());
                assert_eq!(spans.len(), 1);
                assert_close(
                    spans[0].size.width,
                    px((f32::from(layout.size.width) * SCALE_FACTOR).round() / SCALE_FACTOR),
                    "outer span covers the displayed prefix and suffix",
                );
                assert_close(
                    spans[0].size.height,
                    px((f32::from(layout.size.height) * SCALE_FACTOR).round() / SCALE_FACTOR),
                    "outer span uses the displayed line height",
                );

                if width == 100. {
                    assert!(!text.contains('W'));
                    assert!(quads(&mut cx, window.into(), second_group_color()).is_empty());
                    assert!(
                        cx.glyph_bounds(window.into(), second_group_color())
                            .unwrap()
                            .is_empty()
                    );
                    assert!(layout.size.height < px(46.));
                } else {
                    assert!(layout.size.height >= px(46.));
                    assert!(
                        layout
                            .layout
                            .paint_fragments
                            .iter()
                            .any(|fragment| fragment.font_size == px(30.))
                    );
                    assert!(
                        layout
                            .layout
                            .paint_fragments
                            .iter()
                            .any(|fragment| fragment.font_size == px(14.))
                    );
                    let backgrounds = quads(&mut cx, window.into(), second_group_color());
                    assert!(!backgrounds.is_empty());

                    for glyph in cx
                        .glyph_bounds(window.into(), second_group_color())
                        .unwrap()
                    {
                        assert!(
                            backgrounds
                                .iter()
                                .any(|bounds| bounds.contains(&logical_bounds(glyph).center()))
                        );
                    }
                }
            }
        }

        fn quads(
            cx: &mut HeadlessAppContext,
            window: gpui::AnyWindowHandle,
            color: Hsla,
        ) -> Vec<Bounds<Pixels>> {
            let mut bounds: Vec<_> = cx
                .solid_quad_bounds(window, color)
                .unwrap()
                .into_iter()
                .map(logical_bounds)
                .collect();
            bounds.sort_by(|left, right| {
                left.origin
                    .y
                    .partial_cmp(&right.origin.y)
                    .unwrap()
                    .then_with(|| left.origin.x.partial_cmp(&right.origin.x).unwrap())
            });

            bounds
        }

        struct NestedWrapping {
            width: f32,
            flat: bool,
        }

        const PREFIX: &str = "Before inter";
        const SPAN: &str = "national café e\u{301} words ";
        const INNER: &str = "can wrap across lines";
        const SUFFIX: &str = " after.";

        impl Render for NestedWrapping {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let mut paragraph = div()
                    .block()
                    .w(px(self.width))
                    .font_family("IBM Plex Sans")
                    .text_size(px(17.))
                    .line_height(px(23.))
                    .debug_selector(|| "paragraph".into());

                if self.flat {
                    let text = format!("{PREFIX}{SPAN}{INNER}{SUFFIX}");
                    paragraph = paragraph.child(StyledText::new(text).with_highlights([
                        (
                            PREFIX.len()..PREFIX.len() + SPAN.len(),
                            HighlightStyle {
                                color: Some(first_group_color()),
                                background_color: Some(first_group_color()),
                                ..Default::default()
                            },
                        ),
                        (
                            PREFIX.len() + SPAN.len()..PREFIX.len() + SPAN.len() + INNER.len(),
                            HighlightStyle {
                                color: Some(second_group_color()),
                                background_color: Some(second_group_color()),
                                font_weight: Some(GpuiFontWeight::SEMIBOLD),
                                ..Default::default()
                            },
                        ),
                    ]));
                } else {
                    paragraph = paragraph
                        .child(PREFIX)
                        .child(
                            div()
                                .inline()
                                .text_color(first_group_color())
                                .text_bg(first_group_color())
                                .child(SPAN)
                                .child(
                                    div()
                                        .inline()
                                        .font_weight(GpuiFontWeight::SEMIBOLD)
                                        .text_color(second_group_color())
                                        .text_bg(second_group_color())
                                        .child(INNER),
                                ),
                        )
                        .child(SUFFIX);
                }

                div().size_full().items_start().child(paragraph)
            }
        }

        #[test]
        fn nested_spans_wrap_like_flat_styled_text_without_boundary_breaks() {
            let mut cx = headless();
            let nested = cx
                .open_window(size(px(360.), px(300.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| NestedWrapping {
                        width: 130.,
                        flat: false,
                    })
                })
                .unwrap();
            let flat = cx
                .open_window(size(px(360.), px(300.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| NestedWrapping {
                        width: 130.,
                        flat: true,
                    })
                })
                .unwrap();

            for width in [130., 179., 240., 310.] {
                for window in [nested, flat] {
                    window
                        .update(&mut cx, |view, _, cx| {
                            view.width = width;
                            cx.notify();
                        })
                        .unwrap();
                }

                cx.run_until_parked();
                assert_eq!(
                    cx.debug_bounds(nested.into(), "paragraph").unwrap(),
                    cx.debug_bounds(flat.into(), "paragraph").unwrap()
                );

                for color in [first_group_color(), second_group_color()] {
                    assert_eq!(
                        quads(&mut cx, nested.into(), color),
                        quads(&mut cx, flat.into(), color),
                        "width {width}"
                    );
                }
            }
        }

        #[derive(Clone, Copy)]
        enum ParentContext {
            Block,
            Default,
            Flex,
            Grid,
        }

        struct ContextView {
            context: ParentContext,
        }

        impl Render for ContextView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let mut parent = div()
                    .w(px(220.))
                    .debug_selector(|| "parent".into())
                    .text_size(px(16.))
                    .line_height(px(24.));

                parent = match self.context {
                    ParentContext::Block => parent.block(),
                    ParentContext::Default => parent,
                    ParentContext::Flex => parent.flex(),
                    ParentContext::Grid => parent.grid().grid_cols(2),
                };

                div().size_full().items_start().child(
                    parent
                        .child(
                            div()
                                .inline()
                                .w(px(80.))
                                .h(px(31.))
                                .debug_selector(|| "context-inline".into())
                                .child("one two three four five six seven"),
                        )
                        .child(
                            div()
                                .inline_flex()
                                .w(px(20.))
                                .h(px(14.))
                                .debug_selector(|| "context-badge".into()),
                        ),
                )
            }
        }

        #[test]
        fn inline_sizing_depends_on_parent_context() {
            let mut cx = headless();

            for parent_context in [
                ParentContext::Block,
                ParentContext::Default,
                ParentContext::Flex,
                ParentContext::Grid,
            ] {
                let window = cx
                    .open_window(size(px(300.), px(240.)), |window, cx| {
                        window.set_scale_factor(SCALE_FACTOR);
                        cx.new(|_| ContextView {
                            context: parent_context,
                        })
                    })
                    .unwrap();
                cx.run_until_parked();

                let span = cx
                    .debug_bounds(window.into(), "context-inline")
                    .unwrap()
                    .unwrap();
                let badge = cx
                    .debug_bounds(window.into(), "context-badge")
                    .unwrap()
                    .unwrap();

                match parent_context {
                    ParentContext::Block | ParentContext::Default => {
                        assert!(span.size.width > px(80.));
                        assert!(span.size.height >= px(48.));
                        assert!(badge.origin.y > span.origin.y);
                    }

                    ParentContext::Flex | ParentContext::Grid => {
                        assert_close(span.size.width, px(80.), "independent inline width");
                        assert!((span.size.height - px(31.)).abs() < px(1.));
                        assert!(badge.origin.x >= span.right());
                    }
                }
            }
        }

        struct MixedFlow;

        impl Render for MixedFlow {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().items_start().child(
                    div()
                        .block()
                        .w(px(220.))
                        .text_size(px(16.))
                        .line_height(px(24.))
                        .debug_selector(|| "mixed".into())
                        .child("before ")
                        .child(
                            div()
                                .inline()
                                .bg(first_group_color())
                                .debug_selector(|| "split-span".into())
                                .child("first")
                                .child(
                                    div()
                                        .block()
                                        .h(px(30.))
                                        .debug_selector(|| "inner-block".into()),
                                )
                                .child("second"),
                        )
                        .child(" after")
                        .child(div().hidden().h(px(300.)).child("hidden"))
                        .child(div().inline().child(""))
                        .child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .left(px(4.))
                                .size(px(7.))
                                .debug_selector(|| "absolute".into()),
                        )
                        .child(
                            div()
                                .flex()
                                .h(px(12.))
                                .debug_selector(|| "following-block".into()),
                        ),
                )
            }
        }

        #[test]
        fn blocks_split_nested_spans_and_hidden_absolute_empty_children_add_no_rows() {
            let mut cx = headless();

            let window = cx
                .open_window(size(px(300.), px(240.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| MixedFlow)
                })
                .unwrap();
            cx.run_until_parked();

            let fragments = quads(&mut cx, window.into(), first_group_color());
            assert_eq!(fragments.len(), 2);

            let block = cx
                .debug_bounds(window.into(), "inner-block")
                .unwrap()
                .unwrap();
            let following = cx
                .debug_bounds(window.into(), "following-block")
                .unwrap()
                .unwrap();

            assert_close(
                block.origin.y,
                fragments[0].bottom(),
                "block follows first run",
            );
            assert_close(
                fragments[1].origin.y,
                block.bottom(),
                "span resumes after block",
            );
            assert_close(following.origin.y, fragments[1].bottom(), "no phantom rows");

            let paragraph = cx.debug_bounds(window.into(), "mixed").unwrap().unwrap();
            assert_close(
                paragraph.bottom(),
                following.bottom(),
                "wrapped height reaches parent",
            );
        }

        const ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><rect width="12" height="12" fill="#ff0000"/></svg>"##;

        struct AtomicFlow {
            width: f32,
        }

        impl Render for AtomicFlow {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().items_start().child(
                    div()
                        .block()
                        .w(px(self.width))
                        .text_size(px(16.))
                        .line_height(px(24.))
                        .child("Before these words ")
                        .child(
                            div()
                                .inline()
                                .bg(first_group_color())
                                .debug_selector(|| "box-span".into())
                                .child(
                                    div()
                                        .inline_flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .w(px(75.))
                                        .h(px(30.))
                                        .align_middle()
                                        .debug_selector(|| "badge".into())
                                        .child(
                                            div().size(px(10.)).debug_selector(|| "badge-a".into()),
                                        )
                                        .child(
                                            div().size(px(12.)).debug_selector(|| "badge-b".into()),
                                        ),
                                ),
                        )
                        .child(" after.")
                        .child(
                            div()
                                .inline()
                                .debug_selector(|| "icon-span".into())
                                .child(
                                    gpui::img(Arc::new(gpui::Image::from_bytes(
                                        gpui::ImageFormat::Svg,
                                        ICON.to_vec(),
                                    )))
                                    .inline()
                                    .size(px(12.))
                                    .debug_selector(|| "image".into()),
                                )
                                .child(
                                    gpui::svg()
                                        .data(ICON)
                                        .inline()
                                        .size(px(14.))
                                        .debug_selector(|| "svg".into()),
                                ),
                        ),
                )
            }
        }

        #[test]
        fn inline_flex_and_box_only_spans_move_as_atomic_content() {
            let mut cx = headless();

            let window = cx
                .open_window(size(px(340.), px(240.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| AtomicFlow { width: 150. })
                })
                .unwrap();
            let mut previous = None;

            for width in [150., 290.] {
                update_view(&mut cx, window, |view| view.width = width);

                let badge = cx.debug_bounds(window.into(), "badge").unwrap().unwrap();
                let span = cx.debug_bounds(window.into(), "box-span").unwrap().unwrap();

                let left = cx.debug_bounds(window.into(), "badge-a").unwrap().unwrap();
                let right = cx.debug_bounds(window.into(), "badge-b").unwrap().unwrap();

                let icon_span = cx
                    .debug_bounds(window.into(), "icon-span")
                    .unwrap()
                    .unwrap();
                let image = cx.debug_bounds(window.into(), "image").unwrap().unwrap();
                let svg = cx.debug_bounds(window.into(), "svg").unwrap().unwrap();

                assert_bounds_close(icon_span, image.union(&svg), "box-only span bounds");
                assert_close(image.size.width, px(12.), "inline image stays atomic");
                assert_close(svg.size.width, px(14.), "inline SVG stays atomic");
                assert_bounds_close(span, badge, "badge span bounds");

                let backgrounds = quads(&mut cx, window.into(), first_group_color());
                assert_eq!(backgrounds.len(), 1);
                assert_bounds_close(backgrounds[0], badge, "badge span background");

                assert!((right.origin.x - left.right() - px(5.)).abs() < px(1.));
                assert!((left.center().y - badge.center().y).abs() < px(1.));

                if let Some((old_badge, offset)) = previous {
                    assert!(badge.origin.y < old_badge);
                    assert_eq!(left.origin - badge.origin, offset);
                }

                previous = Some((badge.origin.y, left.origin - badge.origin));
            }
        }

        #[derive(gpui::IntoElement)]
        struct RenderedSpan {
            element: gpui::AnyElement,
            counts: Rc<[Cell<usize>; 4]>,
        }

        impl gpui::RenderOnce for RenderedSpan {
            fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
                self.counts[0].set(self.counts[0].get() + 1);

                LifecycleProbe {
                    element: self.element,
                    counts: self.counts,
                }
            }
        }

        struct LifecycleProbe {
            element: gpui::AnyElement,
            counts: Rc<[Cell<usize>; 4]>,
        }

        impl IntoElement for LifecycleProbe {
            type Element = Self;

            fn into_element(self) -> Self {
                self
            }
        }

        impl gpui::Element for LifecycleProbe {
            type RequestLayoutState = ();
            type PrepaintState = ();

            fn id(&self) -> Option<gpui::ElementId> {
                None
            }

            fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
                None
            }

            fn request_layout(
                &mut self,
                _: Option<&gpui::GlobalElementId>,
                _: Option<&gpui::InspectorElementId>,
                window: &mut Window,
                cx: &mut gpui::App,
            ) -> (gpui::LayoutId, ()) {
                self.counts[1].set(self.counts[1].get() + 1);
                (self.element.request_layout(window, cx), ())
            }

            fn prepaint(
                &mut self,
                _: Option<&gpui::GlobalElementId>,
                _: Option<&gpui::InspectorElementId>,
                _: Bounds<Pixels>,
                _: &mut (),
                window: &mut Window,
                cx: &mut gpui::App,
            ) {
                self.counts[2].set(self.counts[2].get() + 1);
                self.element.prepaint(window, cx);
            }

            fn paint(
                &mut self,
                _: Option<&gpui::GlobalElementId>,
                _: Option<&gpui::InspectorElementId>,
                _: Bounds<Pixels>,
                _: &mut (),
                _: &mut (),
                window: &mut Window,
                cx: &mut gpui::App,
            ) {
                self.counts[3].set(self.counts[3].get() + 1);
                self.element.paint(window, cx);
            }
        }

        struct InteractiveFlow {
            width: f32,
            large: bool,
            atomic: bool,
            clicks: Rc<Cell<usize>>,
            hovered: Rc<Cell<bool>>,
            counts: Rc<[Cell<usize>; 4]>,
            callback: Rc<RefCell<Vec<Bounds<Pixels>>>>,
            focus: gpui::FocusHandle,
            scroll: gpui::ScrollHandle,
        }

        impl Render for InteractiveFlow {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let hovered = self.hovered.clone();
                let clicks = self.clicks.clone();
                let focus = self.focus.clone();

                let callback = self.callback.clone();

                let span = div()
                    .id("interactive-span")
                    .inline()
                    .w(px(900.))
                    .h(px(900.))
                    .when(self.large, |span| {
                        span.text_size(px(24.))
                            .line_height(px(34.))
                            .text_color(second_group_color())
                    })
                    .when(self.atomic, |span| {
                        span.inline_flex().w(px(100.)).h(px(110.))
                    })
                    .on_hover(move |value, _, _| hovered.set(*value))
                    .tooltip(|_, cx| cx.new(|_| InlineTooltip).into())
                    .tooltip_show_delay(std::time::Duration::from_millis(10))
                    .track_focus(&self.focus)
                    .bg(first_group_color())
                    .debug_selector(|| "interactive-span".into())
                    .on_click(move |_, window, cx| {
                        clicks.set(clicks.get() + 1);
                        focus.focus(window, cx);
                    })
                    .child(gpui::text!(
                        "these words can wrap across several lines with a short ending"
                    ));

                div().size_full().items_start().child(
                    div()
                        .id("scroller")
                        .block()
                        .w(px(self.width))
                        .h(px(150.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(
                            div()
                                .block()
                                .w_full()
                                .text_size(px(17.))
                                .line_height(px(24.))
                                .child("Before ")
                                .child(RenderedSpan {
                                    element: span.into_any_element(),
                                    counts: self.counts.clone(),
                                })
                                .child(" after.")
                                .with_dynamic_prepaint_order(|_, _| [2, 1, 0].into_iter().collect())
                                .on_children_prepainted(move |bounds, _, _| {
                                    *callback.borrow_mut() = bounds
                                }),
                        )
                        .child(div().h(px(160.))),
                )
            }
        }

        fn click(
            cx: &mut HeadlessAppContext,
            window: gpui::AnyWindowHandle,
            position: Point<Pixels>,
        ) {
            cx.update_window(window, |_, window, cx| {
                window.simulate_mouse_move(position, cx);
                window.dispatch_event(
                    gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                        position,
                        button: gpui::MouseButton::Left,
                        click_count: 1,
                        ..Default::default()
                    }),
                    cx,
                );
                window.dispatch_event(
                    gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                        position,
                        button: gpui::MouseButton::Left,
                        click_count: 1,
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
            cx.run_until_parked();
        }

        #[test]
        fn fragment_interaction_reflows_scrolls_and_preserves_wrapped_element_lifecycle() {
            let mut cx = headless();
            let clicks = Rc::new(Cell::new(0));
            let hovered = Rc::new(Cell::new(false));

            let counts = Rc::new(std::array::from_fn(|_| Cell::new(0)));
            let callback = Rc::new(RefCell::new(Vec::new()));

            let scroll = gpui::ScrollHandle::new();
            let focus = cx.update(|cx| cx.focus_handle());

            let window = cx
                .open_window(size(px(350.), px(300.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| InteractiveFlow {
                        width: 190.,
                        large: false,
                        atomic: false,
                        clicks: clicks.clone(),
                        hovered: hovered.clone(),
                        counts: counts.clone(),
                        callback: callback.clone(),
                        focus: focus.clone(),
                        scroll: scroll.clone(),
                    })
                })
                .unwrap();
            cx.run_until_parked();

            let regions = quads(&mut cx, window.into(), first_group_color());
            assert!(regions.len() >= 3);

            let union = regions
                .iter()
                .copied()
                .reduce(|left, right| left.union(&right))
                .unwrap();
            assert!(union.size.width < px(900.) && union.size.height < px(900.));
            assert_eq!(callback.borrow().len(), 3);
            assert_eq!(callback.borrow()[1], union);

            click(&mut cx, window.into(), regions[0].center());
            click(&mut cx, window.into(), regions.last().unwrap().center());
            assert_eq!(clicks.get(), 2);
            assert!(hovered.get());
            assert!(
                cx.update_window(window.into(), |_, window, _| focus.is_focused(window))
                    .unwrap()
            );

            let gap = gpui::point(regions[0].origin.x / 2., regions[0].center().y);
            assert!(union.contains(&gap) && !regions.iter().any(|right| right.contains(&gap)));

            click(&mut cx, window.into(), gap);
            assert_eq!(
                clicks.get(),
                2,
                "the union's empty gap must not receive clicks"
            );

            assert!(!hovered.get());

            cx.update_window(window.into(), |_, window, cx| {
                window.simulate_mouse_move(regions[0].center(), cx)
            })
            .unwrap();
            cx.run_until_parked();
            cx.advance_clock(std::time::Duration::from_millis(20));
            cx.run_until_parked();
            assert!(
                cx.debug_bounds(window.into(), "inline-tooltip")
                    .unwrap()
                    .is_some()
            );

            cx.update_window(window.into(), |_, window, cx| {
                window.simulate_mouse_move(gap, cx)
            })
            .unwrap();
            cx.run_until_parked();
            cx.advance_clock(std::time::Duration::from_secs(1));
            cx.run_until_parked();
            assert!(
                cx.debug_bounds(window.into(), "inline-tooltip")
                    .unwrap()
                    .is_none()
            );

            window
                .update(&mut cx, |view, _, cx| {
                    view.width = 250.;
                    view.large = true;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let resized = quads(&mut cx, window.into(), first_group_color());
            assert_ne!(resized, regions);
            assert!(resized.iter().any(|right| right.size.height >= px(34.)));

            click(&mut cx, window.into(), resized[1].center());
            assert_eq!(clicks.get(), 3);

            scroll.set_offset(gpui::point(px(0.), px(-24.)));
            window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();

            let scrolled = quads(&mut cx, window.into(), first_group_color());
            assert_close(
                scrolled[1].origin.y,
                resized[1].origin.y - px(24.),
                "scroll updates fragment origin",
            );

            click(&mut cx, window.into(), scrolled[1].center());
            assert_eq!(clicks.get(), 4);

            scroll.set_offset(Point::default());
            window
                .update(&mut cx, |view, _, cx| {
                    view.atomic = true;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let atomic = cx
                .debug_bounds(window.into(), "interactive-span")
                .unwrap()
                .unwrap();
            assert_close(
                atomic.size.width,
                px(100.),
                "display change restores atomic width",
            );
            assert_close(
                atomic.size.height,
                px(110.),
                "display change restores atomic height",
            );

            let counts: Vec<_> = counts.iter().map(Cell::get).collect();
            assert!(counts[0] > 1);
            assert!(
                counts.iter().all(|count| *count == counts[0]),
                "each rendered wrapper runs each lifecycle once: {counts:?}"
            );
        }

        fn assert_bounds_close(actual: Bounds<Pixels>, expected: Bounds<Pixels>, context: &str) {
            assert_point_close(actual.origin, expected.origin, context);
            assert_close(actual.size.width, expected.size.width, context);
            assert_close(actual.size.height, expected.size.height, context);
        }

        struct InlineTooltip;

        impl Render for InlineTooltip {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size(px(20.))
                    .debug_selector(|| "inline-tooltip".into())
            }
        }

        struct MixedTypography;

        impl Render for MixedTypography {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().items_start().child(
                    div()
                        .block()
                        .w(px(170.))
                        .text_size(px(17.))
                        .line_height(px(24.))
                        .child("Before ")
                        .child(
                            div()
                                .inline()
                                .text_size(px(30.))
                                .line_height(px(46.))
                                .text_color(first_group_color())
                                .text_bg(first_group_color())
                                .underline()
                                .child("Tall letters wrap ")
                                .child(
                                    div()
                                        .inline()
                                        .text_size(px(14.))
                                        .line_height(px(20.))
                                        .text_color(second_group_color())
                                        .text_bg(second_group_color())
                                        .child("small tail"),
                                ),
                        )
                        .child(" after."),
                )
            }
        }

        #[test]
        fn mixed_font_sizes_colors_and_underlines_paint_in_their_wrapped_rows() {
            let mut cx = headless();

            let window = cx
                .open_window(size(px(320.), px(300.)), |window, cx| {
                    window.set_scale_factor(SCALE_FACTOR);
                    cx.new(|_| MixedTypography)
                })
                .unwrap();
            cx.run_until_parked();

            let mut glyph_heights = Vec::new();

            for color in [first_group_color(), second_group_color()] {
                let backgrounds = quads(&mut cx, window.into(), color);
                assert!(!backgrounds.is_empty());

                let underlines: Vec<_> = cx
                    .underline_bounds(window.into(), color)
                    .unwrap()
                    .into_iter()
                    .map(logical_bounds)
                    .collect();
                assert_eq!(underlines.len(), backgrounds.len());

                for underline in underlines {
                    assert!(
                        backgrounds.iter().any(|background| (underline.origin.x
                            - background.origin.x)
                            .abs()
                            < px(1.)
                            && underline.origin.y >= background.origin.y
                            && underline.bottom() <= background.bottom() + px(1.)),
                        "underline {underline:?} outside {backgrounds:?}"
                    );
                }

                let glyphs: Vec<_> = cx
                    .glyph_bounds(window.into(), color)
                    .unwrap()
                    .into_iter()
                    .map(logical_bounds)
                    .collect();
                assert!(!glyphs.is_empty());

                for glyph in &glyphs {
                    assert!(
                        backgrounds
                            .iter()
                            .any(|background| background.contains(&glyph.center())),
                        "glyph {glyph:?} outside {backgrounds:?}"
                    );
                }

                glyph_heights.push(
                    glyphs
                        .iter()
                        .map(|bounds| bounds.size.height)
                        .fold(px(0.), Pixels::max),
                );
            }

            assert!(quads(&mut cx, window.into(), first_group_color()).len() >= 2);
            assert!(
                glyph_heights[0] > glyph_heights[1] * 1.5,
                "glyph painting must use each shaped run's font size"
            );
        }
    }

    mod direction_layout {
        use super::*;

        struct DirectionFlexView {
            direction: gpui::Direction,
            cached_child: gpui::Entity<CachedDirectionChild>,
            cached_arabic: gpui::Entity<CachedArabicChild>,
        }

        struct CachedDirectionChild;
        struct CachedArabicChild {
            text: gpui::SharedString,
        }

        impl Render for CachedDirectionChild {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                div().flex().justify_start().size_full().child(
                    div()
                        .w(px(30.0))
                        .h(px(20.0))
                        .debug_selector(|| "cached-first".into()),
                )
            }
        }

        impl Render for CachedArabicChild {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                div().size_full().child(self.text.clone())
            }
        }

        impl Render for DirectionFlexView {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                let item = |selector: &'static str| {
                    div()
                        .w(px(30.0))
                        .h(px(20.0))
                        .debug_selector(move || selector.into())
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .direction(self.direction)
                            .flex()
                            .flex_row_reverse()
                            .justify_start()
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "logical-row".into())
                            .child(item("logical-first"))
                            .child(item("logical-second")),
                    )
                    .child(
                        div()
                            .direction(self.direction)
                            .flex()
                            .flex_row_reverse()
                            .justify_flex_start()
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "flex-row".into())
                            .child(item("flex-first"))
                            .child(item("flex-second")),
                    )
                    .child(
                        div()
                            .direction(self.direction)
                            .grid()
                            .grid_cols(2)
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "grid-row".into())
                            .child(item("grid-first"))
                            .child(item("grid-second")),
                    )
                    .child(
                        div()
                            .direction(self.direction)
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "cached-row".into())
                            .child(
                                self.cached_child
                                    .clone()
                                    .cached(gpui::StyleRefinement::default().size_full()),
                            ),
                    )
                    .child(
                        div()
                            .direction(self.direction)
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "deferred-row".into())
                            .child(gpui::deferred(
                                div().flex().justify_start().size_full().child(
                                    div()
                                        .w(px(30.0))
                                        .h(px(20.0))
                                        .debug_selector(|| "deferred-first".into()),
                                ),
                            )),
                    )
                    .child(
                        div()
                            .direction(gpui::Direction::Auto)
                            .flex()
                            .justify_start()
                            .w(px(200.0))
                            .h(px(20.0))
                            .debug_selector(|| "auto-cached-row".into())
                            .child(
                                div()
                                    .w(px(30.0))
                                    .h(px(20.0))
                                    .debug_selector(|| "auto-cached-item".into())
                                    .child(
                                        self.cached_arabic
                                            .clone()
                                            .cached(gpui::StyleRefinement::default().size_full()),
                                    ),
                            ),
                    )
            }
        }

        fn bounds(
            cx: &mut HeadlessAppContext,
            window: gpui::AnyWindowHandle,
            selector: &str,
        ) -> Bounds<Pixels> {
            cx.debug_bounds(window, selector)
                .unwrap()
                .unwrap_or_else(|| panic!("missing {selector}"))
        }

        #[test]
        fn direction_updates_logical_and_flex_relative_alignment() {
            let mut cx = HeadlessAppContext::new(test_system());
            let window = cx
                .open_window(size(px(320.0), px(160.0)), |_window, cx| {
                    let cached_child = cx.new(|_| CachedDirectionChild);
                    let cached_arabic = cx.new(|_| CachedArabicChild {
                        text: "مرحبا".into(),
                    });
                    cx.new(|_| DirectionFlexView {
                        direction: gpui::Direction::RightToLeft,
                        cached_child,
                        cached_arabic,
                    })
                })
                .unwrap();
            cx.run_until_parked();

            let any_window = window.into();
            let logical_row = bounds(&mut cx, any_window, "logical-row");
            let logical_first = bounds(&mut cx, any_window, "logical-first");
            let flex_row = bounds(&mut cx, any_window, "flex-row");
            let flex_first = bounds(&mut cx, any_window, "flex-first");
            let grid_row = bounds(&mut cx, any_window, "grid-row");
            let grid_first = bounds(&mut cx, any_window, "grid-first");
            let cached_row = bounds(&mut cx, any_window, "cached-row");
            let cached_first = bounds(&mut cx, any_window, "cached-first");
            let auto_cached_row = bounds(&mut cx, any_window, "auto-cached-row");
            let auto_cached_item = bounds(&mut cx, any_window, "auto-cached-item");
            let deferred_row = bounds(&mut cx, any_window, "deferred-row");
            let deferred_first = bounds(&mut cx, any_window, "deferred-first");
            assert!(logical_first.origin.x > logical_row.center().x);
            assert!(flex_first.right() < flex_row.center().x);
            assert!(grid_first.origin.x > grid_row.center().x);
            assert!(cached_first.origin.x > cached_row.center().x);
            assert!(auto_cached_item.origin.x > auto_cached_row.center().x);
            assert!(deferred_first.origin.x > deferred_row.center().x);

            window
                .update(&mut cx, |view, _, cx| {
                    view.direction = gpui::Direction::LeftToRight;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let logical_first = bounds(&mut cx, any_window, "logical-first");
            let flex_first = bounds(&mut cx, any_window, "flex-first");
            let grid_first = bounds(&mut cx, any_window, "grid-first");
            let cached_first = bounds(&mut cx, any_window, "cached-first");
            let auto_cached_item = bounds(&mut cx, any_window, "auto-cached-item");
            let deferred_first = bounds(&mut cx, any_window, "deferred-first");
            assert!(logical_first.right() < logical_row.center().x);
            assert!(flex_first.origin.x > flex_row.center().x);
            assert!(grid_first.right() < grid_row.center().x);
            assert!(cached_first.right() < cached_row.center().x);
            assert!(auto_cached_item.origin.x > auto_cached_row.center().x);
            assert!(deferred_first.right() < deferred_row.center().x);

            window
                .update(&mut cx, |view, _, cx| {
                    view.cached_arabic.update(cx, |child, cx| {
                        child.text = "English".into();
                        cx.notify();
                    });
                })
                .unwrap();
            cx.run_until_parked();

            let auto_cached_item = bounds(&mut cx, any_window, "auto-cached-item");
            assert!(auto_cached_item.right() < auto_cached_row.center().x);
        }
    }
}
