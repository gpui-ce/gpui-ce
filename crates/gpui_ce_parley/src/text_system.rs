use crate::{FontStore, SystemFonts, catalog::FontCatalog};
use anyhow::{Context as _, Result};
use gpui::{
    Bounds, CaretAffinity, CaretMovement, CaretPosition, ColorGlyphKind, Font, FontId, FontMetrics,
    FontStyle as GpuiFontStyle, FontVariation, GlyphId, GlyphRasterizer, InlineBidiScope,
    InlineBoxRequest, InlineLayout, InlineLayoutRequest, InlineRangeGeometry, InlineTextStyle,
    InlineVisualLine, LineLayout, PaintFragment, PaintStyle, ParagraphDirection, Pixels,
    PlatformTextLayout, PlatformTextSystem, Point, PositionedInlineBox, PreparedRasterStyle,
    RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, ResolvedDirection, ShapedGlyph, Size,
    TextAlign, TextBoundary as Boundary, TextDirection as Direction, TextLayoutOptions,
    TextLayoutRequest, TextMovement, TextRangeExt, TextRenderingMode, TextRun, TextSelectionKind,
    UnicodeBidi, VisualDirection, VisualLine, is_paragraph_separator, point, px, size,
};
use parking_lot::{Mutex, RwLock};
use parley::setting::Tag;
use parley::{
    Affinity, Alignment, AlignmentOptions, BoundingBox, BreakReason, CHROMIUM_LINE_BREAK_OVERRIDE,
    Cluster, Cursor, FontFamily, FontFamilyName, FontFeatures, FontStyle,
    FontVariation as ParleyFontVariation, FontWeight, FontWidth, GenericFamily, InlineBox,
    InlineBoxKind, Layout, LayoutContext, Line, LineHeight, PositionedLayoutItem, Selection,
    StyleProperty,
};
use skrifa::instance::NormalizedCoord;
use std::{
    borrow::Cow,
    cmp::Reverse,
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    iter::{once, repeat_n},
    ops::Range,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use unicode_segmentation::UnicodeSegmentation as _;

mod inline;
mod paragraphs;

use inline::parley_font_features;
use paragraphs::{ParagraphLayout, ParagraphRange, ParleyDocumentLayout};

struct ParleyState {
    catalog: FontCatalog,
    layout_context: LayoutContext<ParleyBrush>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ParleyBrush {
    source_run: usize,
    has_letter_spacing: bool,
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
    base_font: Option<Font>,
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
    text_align: TextAlign,
    alignment_width: Option<Pixels>,
}

type ParagraphResultCache = BoundedCache<ParagraphResultCacheKey, ParleyLayoutResult, 512>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceMap {
    source_to_backend: Arc<[usize]>,
    backend_to_source: Arc<[usize]>,
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
        CaretPosition {
            index: self.source_index(caret.index),
            affinity: caret.affinity,
        }
    }

    fn backend_caret(&self, caret: CaretPosition) -> CaretPosition {
        CaretPosition {
            index: self.backend_index(caret.index),
            affinity: caret.affinity,
        }
    }

    fn source_range(&self, range: Range<usize>) -> Range<usize> {
        self.source_index(range.start)..self.source_index(range.end)
    }

    fn backend_range(&self, range: Range<usize>) -> Range<usize> {
        self.backend_index(range.start)..self.backend_index(range.end)
    }

    fn remap_runs(
        &self,
        text: &str,
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
            let source_idx = self
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
}

struct PreparedBidiText {
    text: String,
    runs: Vec<TextRun>,
    run_sources: Vec<usize>,
    text_styles: Vec<InlineTextStyle>,
    inline_boxes: Vec<InlineBoxRequest>,
    map: SourceMap,
}

impl PreparedBidiText {
    fn new(
        text: &str,
        runs: &[TextRun],
        text_styles: &[InlineTextStyle],
        inline_boxes: &[InlineBoxRequest],
        direction: ParagraphDirection,
        unicode_bidi: UnicodeBidi,
        bidi_scopes: &[InlineBidiScope],
    ) -> Self {
        let root_direction = match direction {
            ParagraphDirection::Auto => None,
            ParagraphDirection::LeftToRight => Some(ResolvedDirection::LeftToRight),
            ParagraphDirection::RightToLeft => Some(ResolvedDirection::RightToLeft),
        };

        let mut controls = bidi_scopes
            .iter()
            .filter(|scope| text.contains_range(&scope.range))
            .filter_map(|scope| {
                BidiControls::for_scope(scope.range.clone(), scope.direction, scope.unicode_bidi)
            })
            .collect::<Vec<_>>();

        if let Some(root_direction) = root_direction
            && let Some(root) = BidiControls::for_scope(0..text.len(), root_direction, unicode_bidi)
        {
            controls.push(root);
        }

        controls.sort_by_key(|scope| (scope.start, Reverse(scope.end)));

        let mut prepared = String::new();
        let mut backend_to_source = vec![0];
        let mut source_to_backend = vec![0; text.len() + 1];

        let append_synthetic = |control: &str,
                                source_idx: usize,
                                prepared: &mut String,
                                backend_to_source: &mut Vec<usize>| {
            prepared.push_str(control);
            backend_to_source.extend(repeat_n(source_idx, control.len()));
        };

        if let Some(root_direction) = root_direction {
            let marker = match root_direction {
                ResolvedDirection::LeftToRight => "\u{200e}",
                ResolvedDirection::RightToLeft => "\u{200f}",
            };

            append_synthetic(marker, 0, &mut prepared, &mut backend_to_source);
        }

        for (source_idx, character) in text.char_indices().chain(once((text.len(), '\0'))) {
            for scope in controls.iter().rev().filter(|scope| {
                scope.start < scope.end && scope.end == source_idx && scope.end < text.len()
            }) {
                append_synthetic(
                    scope.close,
                    source_idx,
                    &mut prepared,
                    &mut backend_to_source,
                );
            }

            for scope in controls.iter().filter(|scope| scope.start == source_idx) {
                append_synthetic(
                    scope.open,
                    source_idx,
                    &mut prepared,
                    &mut backend_to_source,
                );
            }

            source_to_backend[source_idx] = prepared.len();

            if source_idx == text.len() {
                break;
            }

            for scope in controls
                .iter()
                .filter(|scope| scope.start == source_idx && scope.end == source_idx)
            {
                append_synthetic(
                    scope.close,
                    source_idx,
                    &mut prepared,
                    &mut backend_to_source,
                );
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
            append_synthetic(
                scope.close,
                text.len(),
                &mut prepared,
                &mut backend_to_source,
            );
        }

        let map = SourceMap {
            source_to_backend: source_to_backend.into(),
            backend_to_source: backend_to_source.into(),
        };

        let (runs, run_sources) = map.remap_runs(&prepared, runs, text.len());

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

        Self {
            text: prepared,
            runs,
            run_sources,
            text_styles,
            inline_boxes,
            map,
        }
    }
}

#[derive(Clone)]
struct BidiControls {
    start: usize,
    end: usize,
    open: &'static str,
    close: &'static str,
}

impl BidiControls {
    fn for_scope(
        range: Range<usize>,
        direction: ResolvedDirection,
        unicode_bidi: UnicodeBidi,
    ) -> Option<Self> {
        let (directional_embed, directional_isolate, directional_override, isolate_override) =
            match direction {
                ResolvedDirection::LeftToRight => {
                    ("\u{202a}", "\u{2066}", "\u{202d}", "\u{2066}\u{202d}")
                }
                ResolvedDirection::RightToLeft => {
                    ("\u{202b}", "\u{2067}", "\u{202e}", "\u{2067}\u{202e}")
                }
            };

        let (open, close) = match unicode_bidi {
            UnicodeBidi::Normal => return None,
            UnicodeBidi::Embed => (directional_embed, "\u{202c}"),
            UnicodeBidi::Isolate => (directional_isolate, "\u{2069}"),
            UnicodeBidi::BidiOverride => (directional_override, "\u{202c}"),
            UnicodeBidi::IsolateOverride => (isolate_override, "\u{202c}\u{2069}"),
            UnicodeBidi::Plaintext => ("\u{2068}", "\u{2069}"),
        };

        Some(Self {
            start: range.start,
            end: range.end,
            open,
            close,
        })
    }
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
    Normalized([u8; 4]),
}

impl From<RasterStyleRequest> for RasterStyleCacheKey {
    fn from(request: RasterStyleRequest) -> Self {
        let color = if request.requested_mode == gpui::GlyphRenderMode::Color {
            RasterStyleColorKey::Normalized(request.normalized_color().into())
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

struct ParleyLayoutParams<'a> {
    text: TextLayoutRequest<'a>,
    inline: Option<ParleyInlineLayoutParams<'a>>,
}

struct ParleyInlineLayoutParams<'a> {
    boxes: &'a [InlineBoxRequest],
    text_styles: &'a [InlineTextStyle],
    line_height: Pixels,
    base_font: &'a Font,
    bidi_scopes: &'a [InlineBidiScope],
}

#[derive(Clone)]
struct ParleyLayoutResult {
    layout: LineLayout,
    inline_lines: Vec<InlineVisualLine>,
    inline_boxes: Vec<PositionedInlineBox>,
    size: Size<Pixels>,
}

impl ParleyLayoutResult {
    fn inline_alignment_offset(&self, text_align: TextAlign) -> Pixels {
        let Some(line) = self.inline_lines.first() else {
            return Pixels::ZERO;
        };

        let direction = self.layout.visual_lines[0].direction;

        match text_align {
            TextAlign::Start if direction.is_rtl() => line.origin.x + line.size.width,
            TextAlign::Start | TextAlign::Left => Pixels::ZERO,
            TextAlign::Center => line.origin.x + line.size.width / 2.,
            TextAlign::End if direction.is_rtl() => Pixels::ZERO,
            TextAlign::End | TextAlign::Right => line.origin.x + line.size.width,
        }
    }

    fn apply_source_runs(&mut self, runs: &[TextRun], sources: &[usize]) {
        for fragment in &mut self.layout.paint_fragments {
            fragment.source_run = sources[fragment.source_run];
            fragment.style = PaintStyle::from(&runs[fragment.source_run]);
        }
    }
}

#[derive(Clone, Debug)]
struct ParleyLayout {
    native_geometry: bool,
    layout: Layout<ParleyBrush>,
    inline_lines: Vec<InlineVisualLine>,
    // Cluster and caret caches use backend text indices, including synthetic bidi controls.
    text: Arc<str>,
    source_map: SourceMap,
    caret_stops: OnceLock<ParleyCaretStops>,
    graphemes: OnceLock<Vec<std::ops::Range<usize>>>,
    cluster_geometry: OnceLock<Vec<ParleyClusterGeometry>>,
}

#[derive(Clone, Copy, Debug)]
struct ParleyCaretStop {
    caret: CaretPosition,
    block: f64,
    inline: f64,
}

trait IntoAffinity<T> {
    fn into_affinity(self) -> T;
}

impl IntoAffinity<Affinity> for CaretAffinity {
    fn into_affinity(self) -> Affinity {
        match self {
            Self::Downstream => Affinity::Downstream,
            Self::Upstream => Affinity::Upstream,
        }
    }
}

impl IntoAffinity<CaretAffinity> for Affinity {
    fn into_affinity(self) -> CaretAffinity {
        match self {
            Self::Downstream => CaretAffinity::Downstream,
            Self::Upstream => CaretAffinity::Upstream,
        }
    }
}

#[derive(Clone, Debug)]
struct ParleyCaretStops {
    stops: Vec<ParleyCaretStop>,
    indices: HashMap<CaretPosition, usize>,
}

#[derive(Clone, Debug)]
struct ParleyClusterGeometry {
    text_range: Range<usize>,
    bounds: Bounds<Pixels>,
    line_index: usize,
    visual_idx: usize,
}

impl ParleyLayout {
    fn new(
        layout: Layout<ParleyBrush>,
        text: Arc<str>,
        inline_lines: Vec<InlineVisualLine>,
        native_geometry: bool,
        source_map: SourceMap,
    ) -> Self {
        Self {
            native_geometry,
            layout,
            inline_lines,
            text,
            source_map,
            caret_stops: OnceLock::new(),
            graphemes: OnceLock::new(),
            cluster_geometry: OnceLock::new(),
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

    fn caret_position(cursor: Cursor) -> CaretPosition {
        let gpui_affinity = cursor.affinity().into_affinity();

        CaretPosition {
            index: cursor.index(),
            affinity: gpui_affinity,
        }
    }

    fn cursor_for_source(&self, caret: CaretPosition) -> Cursor {
        self.backend_cursor(self.source_map.backend_caret(caret))
    }

    fn source_caret(&self, cursor: Cursor) -> CaretPosition {
        self.source_map.source_caret(Self::caret_position(cursor))
    }

    fn backend_cursor(&self, caret: CaretPosition) -> Cursor {
        let parley_affinity = caret.affinity.into_affinity();

        Cursor::from_byte_index(&self.layout, caret.index, parley_affinity)
    }

    fn caret_line_edge(&self, caret: CaretPosition) -> Option<(usize, BoundingBox)> {
        let upstream = caret.affinity == CaretAffinity::Upstream || caret.index == self.text.len();
        let mut start = 0;
        let mut end = self.layout.len();

        while start < end {
            let line_idx = start + (end - start) / 2;
            let line = self.layout.get(line_idx)?;
            let range = line.text_range();

            if caret.index < range.start || (caret.index == range.start && upstream && line_idx > 0)
            {
                end = line_idx;

                continue;
            }

            if caret.index > range.end
                || (caret.index == range.end && !upstream && line_idx + 1 < self.layout.len())
            {
                start = line_idx + 1;

                continue;
            }

            // Only single-run LTR edges have an unambiguous position without resolving clusters.
            if line.len() != 1 || line.runs().next()?.is_rtl() {
                return None;
            }

            let metrics = line.metrics();
            let inline = if caret.index == range.start && !upstream {
                if let Some(previous) = line_idx.checked_sub(1).and_then(|idx| self.layout.get(idx))
                    && (previous.len() != 1 || previous.runs().next()?.is_rtl())
                {
                    return None;
                }

                metrics.offset
            } else if caret.index == range.end && line.break_reason() != BreakReason::Explicit {
                metrics.offset + metrics.advance
            } else {
                return None;
            };

            let top = metrics.block_min_coord.min(metrics.content_block_min_coord);
            let bottom = metrics.block_max_coord.max(metrics.content_block_max_coord);
            let block = ((f64::from(top) + f64::from(bottom)) / 2.) as f32;

            // Keep geometry-based lookup when the content midpoint lies outside the line box.
            if block < metrics.block_min_coord || block >= metrics.block_max_coord {
                return None;
            }

            return Some((
                line_idx,
                BoundingBox::new(
                    f64::from(inline),
                    f64::from(top),
                    f64::from(inline),
                    f64::from(bottom),
                ),
            ));
        }

        None
    }

    fn cursor_cluster(&self, cursor: Cursor) -> Option<(Cluster<'_, ParleyBrush>, bool)> {
        match cursor.visual_clusters(&self.layout) {
            [Some(left), Some(right)] if left.is_end_of_line() => {
                if !left.is_soft_line_break() {
                    return Some((right, false));
                }

                let attach_left = (left.is_rtl() && cursor.affinity() == Affinity::Downstream)
                    || (!left.is_rtl() && cursor.affinity() == Affinity::Upstream);

                Some(if attach_left {
                    (left, true)
                } else {
                    (right, false)
                })
            }
            [Some(left), None] if left.is_hard_line_break() => None,
            [Some(left), _right] => Some((left, true)),
            [_left, Some(right)] => Some((right, false)),
            _clusters => None,
        }
    }

    fn cursor_line_index(&self, cursor: Cursor) -> usize {
        let line = self
            .cursor_cluster(cursor)
            .map(|(cluster, _at_end)| cluster.line())
            .or_else(|| self.layout.get(self.layout.len().saturating_sub(1)));

        let Some(line) = line else {
            return 0;
        };

        let metrics = line.metrics();
        let top = metrics.block_min_coord.min(metrics.content_block_min_coord);
        let bottom = metrics.block_max_coord.max(metrics.content_block_max_coord);

        self.line_index_at_block(((f64::from(top) + f64::from(bottom)) / 2.) as f32)
    }

    fn cursor_geometry(&self, cursor: Cursor) -> BoundingBox {
        let Some((cluster, at_end)) = self.cursor_cluster(cursor) else {
            return cursor.geometry(&self.layout, 0.);
        };

        let line = cluster.line();
        let metrics = line.metrics();
        let inline = if at_end && cluster.is_end_of_line() {
            metrics.offset + metrics.advance
        } else if !at_end && cluster.is_start_of_line() {
            metrics.offset
        } else {
            cluster.visual_offset().unwrap_or_default()
                + if at_end { cluster.advance() } else { 0. }
        };

        BoundingBox::new(
            f64::from(inline),
            f64::from(metrics.block_min_coord.min(metrics.content_block_min_coord)),
            f64::from(inline),
            f64::from(metrics.block_max_coord.max(metrics.content_block_max_coord)),
        )
    }

    fn cursor_position(layout: &Layout<ParleyBrush>, cursor: Cursor) -> (f64, f64) {
        let geometry = cursor.geometry(layout, 0.0);

        (geometry.y0, geometry.x0)
    }

    fn visit_line_clusters(
        line: Line<'_, ParleyBrush>,
        mut visit: impl FnMut(Cluster<'_, ParleyBrush>, Range<f32>, Range<f32>),
    ) {
        let mut items = line.items();

        while let Some(item) = items.next() {
            let PositionedLayoutItem::GlyphRun(mut glyph_run) = item else {
                continue;
            };

            let run = *glyph_run.run();
            let metrics = run.font_metrics();
            let has_letter_spacing = glyph_run.style().brush.has_letter_spacing;
            let rtl = run.is_rtl();
            let mut clusters = run.visual_clusters();
            let mut inline = glyph_run.offset();

            let mut at_atom_start = true;
            let mut atom_style = glyph_run.style_index();
            let mut atom_has_glyphs = false;

            while let Some(cluster) = clusters.next() {
                let cluster_advance = cluster.advance();

                if (rtl && at_atom_start) || (!rtl && !cluster.is_ligature_continuation()) {
                    // Native glyph runs use the atom's first logical style for the whole ligature.
                    let (first, first_advance) = if rtl && cluster.is_ligature_continuation() {
                        let first = clusters
                            .clone()
                            .find(|cluster| !cluster.is_ligature_continuation())
                            .expect("Native runs contain complete ligatures");

                        (first, first.advance())
                    } else {
                        (cluster, cluster_advance)
                    };

                    atom_style = first.style_index();

                    // GPUI adds no word spacing or justification, so only letter spacing can
                    // give a glyphless atom a nonzero advance.
                    atom_has_glyphs = (!has_letter_spacing && first_advance != 0.)
                        || first.glyphs().next().is_some();
                }

                at_atom_start = !cluster.is_ligature_continuation();

                if !atom_has_glyphs {
                    continue;
                }

                if atom_style != glyph_run.style_index() {
                    let Some(PositionedLayoutItem::GlyphRun(next_run)) = items.next() else {
                        unreachable!("A native run's glyph fragments are contiguous");
                    };

                    debug_assert_eq!(next_run.run().index(), run.index());
                    debug_assert_eq!(next_run.style_index(), atom_style);

                    glyph_run = next_run;
                    inline = glyph_run.offset();
                }

                let advance = inline..inline + cluster_advance;

                let vertical =
                    glyph_run.baseline() - metrics.ascent..glyph_run.baseline() + metrics.descent;

                inline = advance.end;
                visit(cluster, advance, vertical);
            }
        }
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

            Self::visit_line_clusters(line, |cluster, advance, _vertical| {
                let range = cluster.text_range();
                let start = CaretPosition::attached_to_next_cluster(range.start);
                let end = CaretPosition::attached_to_previous_cluster(range.end);

                let (left, right) = if cluster.is_rtl() {
                    (end, start)
                } else {
                    (start, end)
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

        let cursor = self.backend_cursor(caret);

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
        let index = self.caret_stop_index(caret)?;
        let caret_stops = self.caret_stops.get()?;

        match direction {
            VisualDirection::Left => index
                .checked_sub(1)
                .and_then(|index| caret_stops.stops.get(index)),
            VisualDirection::Right => caret_stops.stops.get(index + 1),
        }
        .copied()
    }

    fn native_y_for_line(&self, line_index: usize) -> f32 {
        self.layout
            .get(line_index)
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

    fn native_point(&self, position: Point<Pixels>, line_height: Pixels) -> (f32, f32) {
        let y = if self.native_geometry {
            position.y.into()
        } else {
            self.native_y_for_line(Self::line_index_from_point(position, line_height))
        };

        (position.x.into(), y)
    }

    fn line_index_at_block(&self, block: f32) -> usize {
        if block < 0. {
            return 0;
        }

        let line_idx = self
            .inline_lines
            .partition_point(|line| f32::from(line.origin.y) <= block)
            .saturating_sub(1);

        self.layout
            .get(line_idx)
            .filter(|line| {
                let metrics = line.metrics();

                block >= metrics.block_min_coord && block < metrics.block_max_coord
            })
            .map(|_line| line_idx)
            .unwrap_or_else(|| self.layout.len().saturating_sub(1))
    }

    fn inline_cluster_geometry(&self) -> &[ParleyClusterGeometry] {
        self.cluster_geometry.get_or_init(|| {
            let mut geometries = Vec::new();

            for (line_index, line) in self.layout.lines().enumerate() {
                Self::visit_line_clusters(line, |cluster, advance, vertical| {
                    let left = px(advance.start);
                    let right = px(advance.end);

                    if right > left {
                        geometries.push(ParleyClusterGeometry {
                            text_range: cluster.text_range(),
                            bounds: Bounds::new(
                                point(left, px(vertical.start)),
                                size(right - left, px(vertical.end - vertical.start)),
                            ),
                            line_index,
                            visual_idx: geometries.len(),
                        });
                    }
                });
            }

            geometries.sort_unstable_by_key(|geometry| geometry.text_range.start);

            geometries
        })
    }

    fn append_inline_geometry(
        output: &mut Vec<InlineRangeGeometry>,
        geometry: &ParleyClusterGeometry,
    ) {
        if let Some(previous) = output.last_mut()
            && previous.visual_line_index == geometry.line_index
            && previous.bounds.origin.y == geometry.bounds.origin.y
            && previous.bounds.size.height == geometry.bounds.size.height
            && geometry.bounds.origin.x <= previous.bounds.right()
        {
            previous.bounds = previous.bounds.union(&geometry.bounds);

            return;
        }

        output.push(InlineRangeGeometry {
            bounds: geometry.bounds,
            visual_line_index: geometry.line_index,
        });
    }

    fn source_caret_for_point(
        &self,
        caret: CaretPosition,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> CaretPosition {
        let mapped = self.source_map.source_caret(caret);

        if self.text.len() == self.len() {
            return mapped;
        }

        let start = CaretPosition::attached_to_next_cluster(0);

        if self.len() == 0 {
            return start;
        }

        let end = CaretPosition::attached_to_previous_cluster(self.len());

        let mapped = match mapped.index {
            0 => start,
            idx if idx == self.len() => end,
            _ => return mapped,
        };

        // A synthetic bidi control can map a visual edge to the other source endpoint.
        let distance = |endpoint| {
            self.caret_bounds(endpoint, line_height)
                .map(|bounds| (bounds.origin - pixel_point).magnitude())
        };

        match (distance(start), distance(end)) {
            (Some(start_distance), Some(end_distance)) if end_distance < start_distance => end,
            (Some(_), Some(_)) => start,
            _ => mapped,
        }
    }

    fn logical_cluster(&self, caret: CaretPosition, before: bool) -> Option<Range<usize>> {
        let mut caret = self.source_map.backend_caret(caret);

        for _attempt in 0..=self.text.len() {
            let range = if self.native_geometry {
                let cursor = self.backend_cursor(caret);

                let cluster = if before {
                    cursor.upstream_cluster(&self.layout)
                } else {
                    cursor.downstream_cluster(&self.layout)
                };

                cluster?.text_range()
            } else if before {
                self.graphemes()
                    .iter()
                    .rfind(|range| range.start < caret.index)?
                    .clone()
            } else {
                self.graphemes()
                    .iter()
                    .find(|range| range.end > caret.index)?
                    .clone()
            };

            let source = self.source_map.source_range(range.clone());

            if !source.is_empty() {
                return Some(source);
            }

            caret.index = if before { range.start } else { range.end };
        }

        None
    }

    fn backend_caret_from_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        let line_idx = if self.native_geometry {
            self.line_index_at_block(pixel_point.y.into())
        } else {
            Self::line_index_from_point(pixel_point, line_height)
        };

        let (x, y) = self.native_point(pixel_point, line_height);
        let caret = Self::caret_position(Cursor::from_point(&self.layout, x, y));

        if self.text.is_empty() {
            return Err(caret);
        }

        let Some(line) = self.layout.get(line_idx) else {
            return Err(caret);
        };

        let metrics = line.metrics();
        let left = metrics.inline_min_coord + metrics.offset;
        let right = left + metrics.advance;

        if pixel_point.y >= Pixels::ZERO
            && (self.native_geometry || line_height > Pixels::ZERO)
            && (!self.native_geometry || pixel_point.y < px(self.layout.height()))
            && f32::from(pixel_point.x) >= left
            && f32::from(pixel_point.x) < right
        {
            Ok(caret)
        } else {
            Err(caret)
        }
    }
}

impl PlatformTextLayout for ParleyLayout {
    fn visual_line_index(&self, caret: CaretPosition) -> usize {
        let caret = self.source_map.backend_caret(caret);

        if let Some((line_idx, _geometry)) = self.caret_line_edge(caret) {
            return line_idx;
        }

        self.cursor_line_index(self.backend_cursor(caret))
    }

    fn native_lines(&self) -> Option<&[InlineVisualLine]> {
        self.native_geometry.then_some(self.inline_lines.as_slice())
    }

    fn len(&self) -> usize {
        self.source_map.source_to_backend.len() - 1
    }

    fn line_count(&self) -> usize {
        self.layout.len()
    }

    fn size(&self) -> Size<Pixels> {
        let width = if self.text.is_empty() && self.layout.inline_boxes().len() == 0 {
            Pixels::ZERO
        } else {
            px(self.layout.width())
        };

        size(width, px(self.layout.height()))
    }

    fn byte_index_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        let closest = self
            .backend_caret_from_point(pixel_point, line_height)
            .map_err(|caret| self.source_map.source_index(caret.index))?
            .index;

        let (x, y) = self.native_point(pixel_point, line_height);

        Cluster::from_point(&self.layout, x, y)
            .map(|(cluster, _side)| self.source_map.source_index(cluster.text_range().start))
            .ok_or_else(|| self.source_map.source_index(closest))
    }

    fn caret_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.backend_caret_from_point(pixel_point, line_height)
            .map(|caret| self.source_caret_for_point(caret, pixel_point, line_height))
            .map_err(|caret| self.source_caret_for_point(caret, pixel_point, line_height))
    }

    fn caret_bounds(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        let caret = self.source_map.backend_caret(caret);

        if caret.index > self.text.len() {
            return None;
        }

        if !self.native_geometry && self.text.is_empty() && self.layout.inline_boxes().len() == 0 {
            return Some(Bounds::new(
                point(self.inline_lines[0].origin.x, Pixels::ZERO),
                size(Pixels::ZERO, line_height),
            ));
        }

        let edge = self.caret_line_edge(caret);
        let geometry = edge
            .map(|(_line_idx, geometry)| geometry)
            .unwrap_or_else(|| self.cursor_geometry(self.backend_cursor(caret)));

        if self.native_geometry {
            let empty = self.text.is_empty() && self.layout.inline_boxes().len() == 0;

            let left = if empty {
                self.inline_lines[0].origin.x
            } else {
                px(geometry.x0 as f32)
            };

            let right = if empty { left } else { px(geometry.x1 as f32) };

            return Some(Bounds::from_corners(
                point(left, px(geometry.y0 as f32)),
                point(right, px(geometry.y1 as f32)),
            ));
        }

        let line_idx = edge.map_or_else(
            || self.line_index_at_block(((geometry.y0 + geometry.y1) / 2.) as f32),
            |(line_idx, _geometry)| line_idx,
        );

        Some(Bounds::from_corners(
            point(px(geometry.x0 as f32), line_height * line_idx),
            point(px(geometry.x1 as f32), line_height * (line_idx + 1)),
        ))
    }

    fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
        self.source_caret(self.cursor_for_source(caret))
    }

    fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let source_start = caret.index.min(self.len());
        let mut backend_caret = self.source_map.backend_caret(caret);

        for _attempt in 0..=self.text.len() {
            if self.native_geometry {
                let cursor = self.backend_cursor(backend_caret);

                let moved = match direction {
                    VisualDirection::Left => cursor.previous_visual(&self.layout),
                    VisualDirection::Right => cursor.next_visual(&self.layout),
                };

                if moved == cursor {
                    return None;
                }

                backend_caret = Self::caret_position(moved);
            } else {
                backend_caret = self.adjacent_caret_stop(backend_caret, direction)?.caret;
            }

            let source_caret = self.source_map.source_caret(backend_caret);

            if source_caret.index != source_start {
                return Some(source_caret);
            }
        }

        None
    }

    fn selection_bounds(
        &self,
        byte_range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        let byte_range = self.source_map.backend_range(byte_range);
        let anchor = Cursor::from_byte_index(&self.layout, byte_range.start, Affinity::Downstream);
        let focus = Cursor::from_byte_index(&self.layout, byte_range.end, Affinity::Upstream);
        let mut bounds = Vec::new();

        Selection::new(anchor, focus).geometry_with(&self.layout, |geometry, line_idx| {
            let (top, bottom) = if self.native_geometry {
                (px(geometry.y0 as f32), px(geometry.y1 as f32))
            } else {
                (line_height * line_idx, line_height * (line_idx + 1))
            };

            bounds.push(Bounds::from_corners(
                point(px(geometry.x0 as f32), top),
                point(px(geometry.x1 as f32), bottom),
            ));
        });

        bounds
    }

    fn inline_geometry(&self, range: Range<usize>) -> Option<Vec<InlineRangeGeometry>> {
        if range.is_empty() {
            return None;
        }

        Some(
            self.inline_geometry_for_ranges(std::slice::from_ref(&range))
                .pop()
                .unwrap(),
        )
    }

    fn inline_geometry_for_ranges(&self, ranges: &[Range<usize>]) -> Vec<Vec<InlineRangeGeometry>> {
        let mut output = vec![Vec::new(); ranges.len()];

        if ranges
            .iter()
            .all(|range| self.source_map.backend_range(range.clone()).is_empty())
        {
            return output;
        }

        let geometries = self.inline_cluster_geometry();

        for (range, output) in ranges.iter().zip(&mut output) {
            let range = self.source_map.backend_range(range.clone());

            if range.is_empty() {
                continue;
            }

            // Logical cluster ranges are disjoint, so both endpoints can use binary search.
            let start =
                geometries.partition_point(|geometry| geometry.text_range.end <= range.start);

            let end = geometries.partition_point(|geometry| geometry.text_range.start < range.end);

            let mut visual_order = geometries[start..end].iter().collect::<Vec<_>>();
            visual_order.sort_unstable_by_key(|geometry| geometry.visual_idx);

            for geometry in visual_order {
                Self::append_inline_geometry(output, geometry);
            }
        }

        output
    }

    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.logical_cluster(caret, true)
    }

    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.logical_cluster(caret, false)
    }

    fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        vertical_navigation_x: Option<Pixels>,
    ) -> CaretMovement {
        let cursor = self.cursor_for_source(caret);

        let moved = match (movement.direction, movement.boundary) {
            (Direction::Left | Direction::Right, Boundary::Cluster) => {
                let direction = if movement.direction == Direction::Left {
                    VisualDirection::Left
                } else {
                    VisualDirection::Right
                };

                return CaretMovement {
                    result: self
                        .adjacent_visual_caret(caret, direction)
                        .unwrap_or(caret),
                    vertical_navigation_x: None,
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

                let geometry = cursor.geometry(&self.layout, 0.);

                let line_idx = self.line_index_at_block(((geometry.y0 + geometry.y1) / 2.) as f32);

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
                        result: self.source_caret(moved.focus()),
                        vertical_navigation_x,
                    };
                };

                let x = vertical_navigation_x.map_or(geometry.x0 as f32, f32::from);
                let moved = Cursor::from_point(&self.layout, x, self.native_y_for_line(target_idx));

                return CaretMovement {
                    result: self.source_caret(moved),
                    vertical_navigation_x: Some(px(x)),
                };
            }
            _ => cursor,
        };

        CaretMovement {
            result: self.source_caret(moved),
            vertical_navigation_x: None,
        }
    }

    fn selection_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        let (x, y) = self.native_point(pixel_point, line_height);

        let selection = match kind {
            TextSelectionKind::Word => Selection::word_from_point(&self.layout, x, y),
            TextSelectionKind::VisualLine => Selection::line_from_point(&self.layout, x, y),
            TextSelectionKind::HardLine => Selection::hard_line_from_point(&self.layout, x, y),
        }
        .text_range();

        self.source_map.source_range(selection)
    }
}

impl ParleyState {
    fn new(system_fonts: SystemFonts) -> Self {
        Self {
            catalog: FontCatalog::new(system_fonts),
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
    fn font_properties<'a>(
        &'a self,
        font: &'a Font,
    ) -> Result<[StyleProperty<'a, ParleyBrush>; 5]> {
        let families = self
            .font_families(font)
            .chain(
                self.additional_fallbacks
                    .iter()
                    .map(|family| FontFamilyName::Named(Cow::Borrowed(family.as_str()))),
            )
            .collect::<Vec<_>>();

        let style = match font.style {
            GpuiFontStyle::Normal => FontStyle::Normal,
            GpuiFontStyle::Italic => FontStyle::Italic,
            GpuiFontStyle::Oblique => FontStyle::Oblique(None),
        };

        let features = parley_font_features(font)?;

        Ok([
            StyleProperty::FontFamily(FontFamily::List(Cow::Owned(families))),
            StyleProperty::FontWeight(FontWeight::new(font.weight.0)),
            StyleProperty::FontWidth(FontWidth::from_percentage(font.width.0)),
            StyleProperty::FontStyle(style),
            StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(features))),
        ])
    }

    /// Creates a text system which delegates only glyph rasterization to `rasterizer`.
    pub fn new(
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

    /// Adds fallback families after those supplied by each text style.
    pub fn with_fallback_families(
        mut self,
        families: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.additional_fallbacks = families.into_iter().map(Into::into).collect();
        self
    }

    fn face_families<'a>(&'a self, name: &'a str) -> impl Iterator<Item = FontFamilyName<'a>> {
        let is_system_font = name == ".SystemUIFont";
        let family = if is_system_font {
            self.system_font_fallback.as_str()
        } else {
            name
        };

        is_system_font
            .then_some(FontFamilyName::Generic(GenericFamily::SystemUi))
            .into_iter()
            .chain(once(FontFamilyName::Named(Cow::Borrowed(family))))
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
            .flat_map(|name| self.face_families(name))
    }

    fn resolve_canonical_font(&self, descriptor: &Font) -> Result<FontId> {
        let families = self
            .font_families(descriptor)
            .chain(
                self.additional_fallbacks
                    .iter()
                    .flat_map(|name| self.face_families(name)),
            )
            .collect::<Vec<_>>();

        let resolved = {
            let mut state = self.parley.lock();

            state
                .catalog
                .resolve_face(
                    &families,
                    descriptor.width,
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

    fn parley_layout(&self, params: ParleyLayoutParams<'_>) -> Result<ParleyLayoutResult> {
        let request = params.text;
        let (inline_boxes, text_styles, bidi_scopes) = params
            .inline
            .as_ref()
            .map_or((&[][..], &[][..], &[][..]), |inline| {
                (inline.boxes, inline.text_styles, inline.bidi_scopes)
            });

        // Parley resolves one base direction per layout; split hard paragraphs first.
        if !request.text.chars().any(is_paragraph_separator) {
            return self.parley_paragraph_layout(params);
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

        for source in ParagraphRange::split(text) {
            let first_run = run_ranges.partition_point(|range| range.end <= source.content.start);
            let (mut paragraph_run_sources, mut paragraph_runs): (Vec<_>, Vec<_>) = run_ranges
                .iter()
                .enumerate()
                .skip(first_run)
                .take_while(|(_idx, range)| range.start < source.content.end)
                .filter_map(|(idx, range)| {
                    let local = source.local_content_range(range)?;

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
                    let range = source.local_content_range(&style.range)?;

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

            let paragraph_bidi_scopes = bidi_scopes
                .iter()
                .filter_map(|scope| {
                    source
                        .local_content_range(&scope.range)
                        .map(|range| InlineBidiScope {
                            range,
                            direction: scope.direction,
                            unicode_bidi: scope.unicode_bidi,
                        })
                })
                .collect::<Vec<_>>();

            let mut result = self.parley_paragraph_layout(ParleyLayoutParams {
                text: TextLayoutRequest {
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
                inline: params
                    .inline
                    .as_ref()
                    .map(|inline| ParleyInlineLayoutParams {
                        boxes: &paragraph_boxes,
                        text_styles: &paragraph_styles,
                        bidi_scopes: &paragraph_bidi_scopes,
                        ..*inline
                    }),
            })?;

            for fragment in &mut result.layout.paint_fragments {
                fragment.source_run = paragraph_run_sources[fragment.source_run];
            }

            let first_line = visual_lines.len();
            let first_fragment = paint_fragments.len();
            let block_offset = document_size.height;

            for line in &mut result.layout.visual_lines {
                line.text_range.start += source.content.start;
                line.text_range.end += source.content.start;
                line.paint_fragment_range.start += first_fragment;
                line.paint_fragment_range.end += first_fragment;
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

        let platform_layout = ParleyDocumentLayout::new(
            paragraphs,
            text,
            document_size,
            params.inline.is_some().then(|| inline_lines.clone()),
        );

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
        params: ParleyLayoutParams<'_>,
    ) -> Result<ParleyLayoutResult> {
        let request = params.text;

        let (inline_boxes, text_styles, line_height, base_font, bidi_scopes) =
            match params.inline.as_ref() {
                Some(inline) => (
                    inline.boxes,
                    inline.text_styles,
                    Some(inline.line_height),
                    Some(inline.base_font),
                    inline.bidi_scopes,
                ),
                None => (&[][..], &[][..], None, None, &[][..]),
            };

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

        let base_font = base_font.or_else(|| runs.first().map(|run| &run.font));
        let source_text_len = text.len();
        let source_runs = runs;

        let prepared = PreparedBidiText::new(
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
            base_font: base_font.cloned(),
            runs: runs.iter().map(ParleyShapingRun::from).collect(),
            inline_boxes: inline_boxes.to_vec(),
            text_styles: text_styles.to_vec(),
            line_height,
        };

        let result_cache_key = ParagraphResultCacheKey {
            paragraph: cache_key,
            source_map: prepared.map.clone(),
            wrap,
            text_align,
            alignment_width,
        };

        if let Some(mut result) = self.paragraph_result_cache.lock().get(&result_cache_key) {
            result.apply_source_runs(source_runs, &prepared.run_sources);

            return Ok(result);
        }

        let mut layout = if let Some(layout) =
            self.paragraph_cache.lock().get(&result_cache_key.paragraph)
        {
            layout
        } else {
            let base_properties = base_font
                .map(|font| self.font_properties(font))
                .transpose()?;

            let font_properties = runs
                .iter()
                .map(|run| {
                    if base_font == Some(&run.font) {
                        return Ok(None);
                    }

                    self.font_properties(&run.font).map(Some)
                })
                .collect::<Result<Vec<_>>>()?;

            // Parley may merge nearly equal spacing, so use one flag for the paragraph.
            let has_letter_spacing = runs.iter().any(|run| {
                run.letter_spacing
                    .is_some_and(|spacing| spacing != Pixels::ZERO)
            });

            let mut state = self.parley.lock();

            let ParleyState {
                catalog,
                layout_context,
            } = &mut *state;

            let mut builder =
                layout_context.ranged_builder(catalog.context_mut(), text, 1.0, false);

            builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
            builder.push_default(StyleProperty::FontSize(f32::from(font_size)));

            builder.push_default(StyleProperty::Brush(ParleyBrush {
                source_run: 0,
                has_letter_spacing,
            }));

            for property in base_properties.into_iter().flatten() {
                builder.push_default(property);
            }

            if self.automatic_optical_sizing {
                builder.push_default(StyleProperty::FontVariations(
                    [ParleyFontVariation::new(
                        Tag::new(b"opsz"),
                        f32::from(font_size),
                    )]
                    .as_slice()
                    .into(),
                ));
            }

            if let Some(line_height) = line_height {
                builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(f32::from(
                    line_height,
                ))));
            }

            for (run_index, (run, properties)) in runs.iter().zip(font_properties).enumerate() {
                let range = run_ranges[run_index].clone();

                if range.is_empty() && !text.is_empty() {
                    continue;
                }

                for property in properties.into_iter().flatten() {
                    builder.push(property, range.clone());
                }

                builder.push(
                    StyleProperty::Brush(ParleyBrush {
                        source_run: run_index,
                        has_letter_spacing,
                    }),
                    range.clone(),
                );

                if let Some(letter_spacing) = run.letter_spacing {
                    builder.push(
                        StyleProperty::LetterSpacing(f32::from(letter_spacing)),
                        range,
                    );
                }
            }

            let mut push_style = |property: StyleProperty<'_, ParleyBrush>, range| {
                if text.is_empty() {
                    builder.push_default(property);
                } else {
                    builder.push(property, range);
                }
            };

            for style in text_styles {
                push_style(
                    StyleProperty::FontSize(f32::from(style.font_size)),
                    style.range.clone(),
                );

                if self.automatic_optical_sizing {
                    push_style(
                        StyleProperty::FontVariations(
                            [ParleyFontVariation::new(
                                Tag::new(b"opsz"),
                                f32::from(style.font_size),
                            )]
                            .as_slice()
                            .into(),
                        ),
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
                    baseline: None,
                    vertical_align: match inline_box.vertical_align {
                        gpui::VerticalAlign::Baseline => parley::VerticalAlign::BASELINE,
                        gpui::VerticalAlign::Middle => parley::VerticalAlign::MIDDLE,
                        gpui::VerticalAlign::Top => parley::VerticalAlign::TOP,
                        gpui::VerticalAlign::Bottom => parley::VerticalAlign::BOTTOM,
                    },
                });
            }

            let layout = builder.build(text);

            self.paragraph_cache
                .lock()
                .insert(result_cache_key.paragraph.clone(), layout.clone());

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
                    last_line_alignment: None,
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

        let mut width = px(0.0);
        let mut ascent = px(0.0);
        let mut descent = px(0.0);

        for (line_index, line) in layout.lines().enumerate() {
            let fragment_start = paint_fragments.len();
            let metrics = *line.metrics();

            let line_x =
                empty_alignment.unwrap_or_else(|| px(metrics.inline_min_coord + metrics.offset));

            let line_advance = if empty_alignment.is_some() {
                Pixels::ZERO
            } else {
                px(metrics.advance)
            };

            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    let PositionedLayoutItem::InlineBox(inline_box) = item else {
                        unreachable!();
                    };

                    positioned_inline_boxes.push(PositionedInlineBox {
                        id: inline_box.id,
                        line_index: line_index,
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
                    .map(|coord| NormalizedCoord::from_bits(coord.to_bits()))
                    .collect::<Vec<_>>();

                let shaping_variations = self
                    .automatic_optical_sizing
                    .then(|| [FontVariation::new(*b"opsz", run.font_size())]);

                let font_id = self.font_instances.write().intern(
                    run.font().data.clone(),
                    run.font().index,
                    &normalized_coords,
                    run.synthesis(),
                    shaping_variations.as_ref().map_or(&[], |settings| settings),
                )?;

                let baseline = glyph_run.baseline();
                let run_metrics = run.font_metrics();

                // Bidi preparation can add direction markers to an empty source string.
                if source_text_len == 0 {
                    continue;
                }

                let glyphs = {
                    let font_instances = self.font_instances.read();
                    let font = font_instances
                        .get(font_id)
                        .context("canonical font missing after interning")?;

                    let color_glyphs = font
                        .has_color_glyphs
                        .then(|| font.color_glyphs())
                        .transpose()?;

                    glyph_run
                        .positioned_glyphs()
                        .map(|glyph| {
                            let id = GlyphId(glyph.id);
                            ShapedGlyph {
                                id,
                                position: point(
                                    px(glyph.x) - line_x,
                                    px(glyph.y - metrics.baseline),
                                ),
                                is_emoji: color_glyphs.as_ref().is_some_and(|color_glyphs| {
                                    color_glyphs
                                        .available_kinds(id)
                                        .any(|kind| self.color_glyph_formats.contains(&kind))
                                }),
                            }
                        })
                        .collect()
                };

                let start = px(glyph_run.offset()) - line_x;
                let source_run = glyph_run.style().brush.source_run;

                paint_fragments.push(PaintFragment {
                    baseline_offset: px(baseline - metrics.baseline),
                    background_range: Some(
                        px(baseline - metrics.baseline - run_metrics.ascent)
                            ..px(baseline - metrics.baseline + run_metrics.descent),
                    ),
                    source_run,
                    font_id,
                    font_size: px(run.font_size()),
                    glyphs,
                    x_range: start..start + px(glyph_run.advance()),
                    style: PaintStyle::from(&source_runs[prepared.run_sources[source_run]]),
                    // Parley reports font metrics y-up (an underline below the
                    // baseline is negative); gpui adds these to `baseline_y` in
                    // y-down space, so flip them.
                    underline_offset: Some(px(-run_metrics.underline_offset)),
                    strikethrough_offset: Some(px(-run_metrics.strikethrough_offset)),
                });
            }

            let parley_text_range = line.text_range();

            let text_range = prepared.map.source_range(
                parley_text_range.start.min(text.len())..parley_text_range.end.min(text.len()),
            );

            visual_lines.push(VisualLine {
                text_range,
                paint_fragment_range: fragment_start..paint_fragments.len(),
                advance_width: line_advance,
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

            width = width.max(line_advance);
            ascent = ascent.max(px(
                (metrics.baseline - metrics.content_block_min_coord).max(0.)
            ));
            descent = descent.max(px(
                (metrics.content_block_max_coord - metrics.baseline).max(0.)
            ));
        }

        if visual_lines.is_empty() {
            anyhow::bail!("Parley produced no line");
        }

        let mut size = size(px(layout.width()), px(layout.height()));

        if empty_alignment.is_some() {
            size.width = Pixels::ZERO;
        }

        let platform_layout = ParleyLayout::new(
            layout,
            paragraph_text,
            inline_lines.clone(),
            params.inline.is_some(),
            prepared.map,
        );

        let line_layout = LineLayout {
            font_size,
            width,
            ascent,
            descent,
            visual_lines: visual_lines.into_iter().collect(),
            paint_fragments,
            len: source_text_len,
            platform_layout: Arc::new(platform_layout),
        };

        let mut result = ParleyLayoutResult {
            layout: line_layout,
            inline_lines,
            inline_boxes: positioned_inline_boxes,
            size,
        };

        // Cache backend run indices so each caller can restore its own authored run mapping.
        self.paragraph_result_cache
            .lock()
            .insert(result_cache_key, result.clone());

        result.apply_source_runs(source_runs, &prepared.run_sources);

        Ok(result)
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
            state.catalog.register_fonts(&blobs)?;

            let _previous_generation = self.font_generation.fetch_add(1, Ordering::Release);
            self.paragraph_cache.lock().clear();
            self.paragraph_result_cache.lock().clear();
        }

        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names = {
            let mut state = self.parley.lock();

            state.catalog.family_names()
        };
        names.push(".SystemUIFont".to_owned());
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

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.font_instances
            .read()
            .get(font_id)?
            .glyph_for_char(ch)
            .ok()?
    }

    fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
        let font_instances = self.font_instances.read();
        let font = font_instances
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
                    "glyph rasterization failed for FontId {:?}, data identity {data_identity}, face index {face_index}, variations {variations:?}",
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
        self.parley_layout(ParleyLayoutParams {
            text: request,
            inline: None,
        })
        .expect("Parley failed to lay out a validated GPUI document")
        .layout
    }

    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
        let result = self
            .parley_layout(ParleyLayoutParams {
                text: TextLayoutRequest {
                    text: request.text,
                    font_size: request.font_size,
                    runs: request.runs,
                    options: request.options,
                },
                inline: Some(ParleyInlineLayoutParams {
                    boxes: request.boxes,
                    text_styles: request.text_styles,
                    line_height: request.line_height,
                    base_font: request.base_font,
                    bidi_scopes: request.bidi_scopes,
                }),
            })
            .expect("Parley failed to lay out a validated GPUI inline document");

        let alignment_offset = result.inline_alignment_offset(request.options.text_align);

        InlineLayout {
            layout: std::sync::Arc::new(result.layout),
            alignment_offset,
            lines: result.inline_lines,
            boxes: result.inline_boxes,
            size: result.size,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext, CaretSelection, Context, FontFallbacks, FontFeatures as GpuiFontFeatures,
        FontStyle as GpuiFontStyle, FontSynthesis, FontWeight as GpuiFontWeight, GlyphRenderMode,
        HeadlessAppContext, HighlightStyle, Hsla, IntoElement, Point, RasterFace,
        RasterizedGlyphFormat, Render, StrikethroughStyle, Styled, StyledText, TestTextSystem,
        TextSystem, UnderlineStyle, VerticalAlign, Window, WindowHandle, WindowTextSystem, div,
        font, hsla, prelude::*,
    };
    use gpui_fonts::{
        IBM_PLEX, IBM_PLEX_SEMIBOLD, LILEX, NOTO_ARABIC, NOTO_COLOR_EMOJI, NOTO_HEBREW, NOTO_SANS,
        SOURCE_SERIF,
    };
    use parley::VerticalAlign as ParleyVerticalAlign;
    use skrifa::{
        FontRef, MetadataProvider as _,
        instance::{LocationRef, Size as FontSize},
    };
    use std::{
        cell::Cell,
        cell::RefCell,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[test]
    fn caret_affinity_round_trips_through_parley() {
        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
            let parley_affinity: Affinity = affinity.into_affinity();
            let gpui_affinity: CaretAffinity = parley_affinity.into_affinity();
            assert_eq!(gpui_affinity, affinity);
        }
    }

    #[test]
    fn line_lookup_preserves_native_boundaries_and_empty_rows() {
        let system = test_system();
        let text = "one two three four five six seven eight ".repeat(8);

        for line_height in [0., 12.] {
            let native =
                native_test_layout(&system, &text, IBM_PLEX.family, line_height, Some(60.));

            assert_cursor_geometry(&native);

            let last_idx = native.layout.len() - 1;
            assert!(native.layout.len() > 8);

            let samples = native.layout.lines().flat_map(|line| {
                let metrics = line.metrics();

                [
                    metrics.block_min_coord,
                    (metrics.block_min_coord + metrics.block_max_coord) / 2.,
                    metrics.block_max_coord,
                ]
            });

            for block in samples.chain([-1., native.layout.height(), f32::INFINITY, f32::NAN]) {
                let expected = native.layout.lines().position(|line| {
                    let metrics = line.metrics();

                    block >= metrics.block_min_coord && block < metrics.block_max_coord
                });
                let expected = if block < 0. {
                    0
                } else {
                    expected.unwrap_or(last_idx)
                };
                assert_eq!(native.line_index_at_block(block), expected);
            }
        }
    }

    fn assert_cursor_geometry(native: &ParleyLayout) {
        for idx in 0..=native.text.len() {
            for affinity in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
                let caret = CaretPosition {
                    index: idx,
                    affinity,
                };

                let cursor = native.backend_cursor(caret);
                let expected = cursor.geometry(&native.layout, 0.);
                let edge = native.caret_line_edge(caret);
                let actual = edge.map_or_else(
                    || native.cursor_geometry(cursor),
                    |(_line_idx, geometry)| geometry,
                );

                // Line advances and cursor offsets accumulate f32 spacing in different orders.
                for (actual, expected) in [
                    (actual.x0, expected.x0),
                    (actual.x1, expected.x1),
                    (actual.y0, expected.y0),
                    (actual.y1, expected.y1),
                ] {
                    assert!(
                        (actual - expected).abs() < 0.001 + expected.abs() * 0.00001,
                        "{caret:?}: {actual} != {expected}, text {:?}",
                        native.text
                    );
                }

                let expected_line =
                    native.line_index_at_block(((expected.y0 + expected.y1) / 2.) as f32);

                assert_eq!(
                    edge.map_or_else(
                        || native.cursor_line_index(cursor),
                        |(line_idx, _geometry)| line_idx
                    ),
                    expected_line,
                    "{caret:?}, text {:?}",
                    native.text
                );
            }
        }
    }

    fn native_test_layout(
        system: &ParleyTextSystem,
        text: &str,
        family: &str,
        line_height: f32,
        wrap_width: Option<f32>,
    ) -> ParleyLayout {
        let descriptor = font(family);
        let mut state = system.parley.lock();

        let ParleyState {
            catalog,
            layout_context,
        } = &mut *state;

        let mut builder = layout_context.ranged_builder(catalog.context_mut(), text, 1., false);

        for property in system.font_properties(&descriptor).unwrap() {
            builder.push_default(property);
        }

        builder.push_default(StyleProperty::FontSize(16.));
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height)));

        if line_height > 0. {
            builder.push(
                StyleProperty::LineHeight(LineHeight::Absolute(32.)),
                0..24.min(text.len()),
            );
        }

        let mut layout = builder.build(text);
        layout.break_all_lines(wrap_width);

        let inline_lines = layout
            .lines()
            .map(|line| InlineVisualLine {
                origin: point(Pixels::ZERO, px(line.metrics().block_min_coord)),
                ..Default::default()
            })
            .collect();

        ParleyLayout::new(
            layout,
            text.into(),
            inline_lines,
            true,
            identity_source_map(text.len()),
        )
    }

    #[test]
    fn cursor_geometry_preserves_long_unwrapped_lines() {
        let system = test_system();
        let text = "office ".repeat(256);

        for family in [IBM_PLEX.family, SOURCE_SERIF.family] {
            let native = native_test_layout(&system, &text, family, 24., None);
            assert_cursor_geometry(&native);
        }
    }

    fn identity_source_map(len: usize) -> SourceMap {
        let indices: Arc<[usize]> = (0..=len).collect();

        SourceMap {
            source_to_backend: indices.clone(),
            backend_to_source: indices,
        }
    }

    fn test_system() -> Arc<ParleyTextSystem> {
        let system = Arc::new(
            ParleyTextSystem::new(SystemFonts::Skip, IBM_PLEX.family, TestTextSystem)
                .with_fallback_families([
                    IBM_PLEX.family,
                    LILEX.family,
                    SOURCE_SERIF.family,
                    NOTO_COLOR_EMOJI.family,
                    NOTO_ARABIC.family,
                    NOTO_HEBREW.family,
                ]),
        );

        system
            .add_fonts(vec![
                Cow::Borrowed(IBM_PLEX.data),
                Cow::Borrowed(IBM_PLEX_SEMIBOLD.data),
                Cow::Borrowed(LILEX.data),
                Cow::Borrowed(SOURCE_SERIF.data),
                Cow::Borrowed(NOTO_SANS.data),
                Cow::Borrowed(NOTO_COLOR_EMOJI.data),
                Cow::Borrowed(NOTO_ARABIC.data),
                Cow::Borrowed(NOTO_HEBREW.data),
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

    fn inline_request<'a>(
        text: &'a str,
        runs: &'a [TextRun],
        base_font: &'a Font,
    ) -> InlineLayoutRequest<'a> {
        InlineLayoutRequest {
            text,
            runs,
            base_font,
            font_size: px(18.),
            line_height: px(24.),
            text_styles: &[],
            boxes: &[],
            bidi_scopes: &[],
            options: TextLayoutOptions {
                text_align: TextAlign::Left,
                ..Default::default()
            },
        }
    }

    fn layout_with_options(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        options: TextLayoutOptions,
    ) -> LineLayout {
        system.layout_text(TextLayoutRequest {
            text,
            font_size,
            runs,
            options,
        })
    }

    fn layout_line(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> LineLayout {
        layout_with_options(
            system,
            text,
            font_size,
            runs,
            TextLayoutOptions {
                text_align: TextAlign::Left,
                ..Default::default()
            },
        )
    }

    fn layout_wrapped(
        system: &ParleyTextSystem,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Pixels,
        line_clamp: Option<usize>,
    ) -> LineLayout {
        layout_with_options(
            system,
            text,
            font_size,
            runs,
            TextLayoutOptions {
                wrap_width: Some(wrap_width),
                line_clamp,
                text_align: TextAlign::Left,
                ..Default::default()
            },
        )
    }

    fn wrapped_sample(system: &ParleyTextSystem, text: &str, width: Pixels) -> LineLayout {
        layout_wrapped(
            system,
            text,
            px(18.),
            &[text_run(text, IBM_PLEX.family)],
            width,
            None,
        )
    }

    #[test]
    fn decoration_offsets_are_y_down_from_the_baseline() {
        let system = test_system();
        for family in [IBM_PLEX.family, SOURCE_SERIF.family, LILEX.family] {
            let text = "Learn how";
            let layout = layout_line(&system, text, px(16.0), &[text_run(text, family)]);
            let fragment = &layout.paint_fragments[0];
            let underline = fragment.underline_offset.unwrap();
            let strikethrough = fragment.strikethrough_offset.unwrap();

            // gpui paints at `baseline_y + offset` with y growing downward: the
            // underline sits below the baseline, within the descent, and the
            // strikethrough sits above it, within the ascent.
            assert!(underline > px(0.0), "{family}: underline={underline:?}");
            assert!(
                underline <= layout.descent,
                "{family}: underline={underline:?}, descent={:?}",
                layout.descent
            );
            assert!(
                strikethrough < px(0.0),
                "{family}: strikethrough={strikethrough:?}"
            );
            assert!(
                -strikethrough < layout.ascent,
                "{family}: strikethrough={strikethrough:?}, ascent={:?}",
                layout.ascent
            );
        }
    }

    #[test]
    fn empty_directional_text_without_runs_keeps_metrics_without_paint() {
        let system = test_system();
        {
            // Give Parley a default font for synthetic bidi markers without
            // depending on installed system fonts or an explicit text run.
            let mut state = system.parley.lock();
            let collection = &mut state.catalog.context_mut().collection;
            let family = collection.family_id(IBM_PLEX.family).unwrap();
            collection.set_generic_families(GenericFamily::SansSerif, std::iter::once(family));
        }

        for (direction, expected_direction) in [
            (ParagraphDirection::Auto, ResolvedDirection::LeftToRight),
            (
                ParagraphDirection::LeftToRight,
                ResolvedDirection::LeftToRight,
            ),
            (
                ParagraphDirection::RightToLeft,
                ResolvedDirection::RightToLeft,
            ),
        ] {
            // Exercise both a fresh layout and the cached result.
            for _ in 0..2 {
                let layout = system.layout_text(TextLayoutRequest {
                    text: "",
                    font_size: px(16.0),
                    runs: &[],
                    options: TextLayoutOptions {
                        direction,
                        ..Default::default()
                    },
                });

                assert_eq!(layout.len, 0);
                assert_eq!(layout.width, Pixels::ZERO);
                assert!(layout.ascent > Pixels::ZERO);
                assert!(layout.descent > Pixels::ZERO);
                assert_eq!(layout.visual_lines.len(), 1);
                assert_eq!(layout.visual_lines[0].text_range, 0..0);
                assert_eq!(layout.visual_lines[0].paint_fragment_range, 0..0);
                assert_eq!(layout.visual_lines[0].direction, expected_direction);
                assert!(layout.paint_fragments.is_empty());
            }
        }
    }

    #[test]
    fn paragraph_direction_preserves_logical_alignment_and_source_endpoint_carets() {
        let system = test_system();
        let line_height = px(24.0);

        let directions = [
            ParagraphDirection::LeftToRight,
            ParagraphDirection::RightToLeft,
        ];

        let endpoint_cases = directions.into_iter().flat_map(|direction| {
            [
                ("English 123", direction, TextAlign::Start, true),
                ("مرحبا", direction, TextAlign::Start, true),
                ("English", direction, TextAlign::Start, false),
            ]
        });

        let rtl_cases = [
            ("English 123!", TextAlign::Start),
            ("123 …", TextAlign::Start),
            ("", TextAlign::Start),
            ("English", TextAlign::End),
        ]
        .into_iter()
        .map(|(text, align)| (text, ParagraphDirection::RightToLeft, align, false));

        for (text, direction, align, endpoint_hits) in endpoint_cases.chain(rtl_cases) {
            let name = format!("{text:?}, {direction:?}, {align:?}");
            let right_aligned = match direction {
                ParagraphDirection::RightToLeft => align == TextAlign::Start,
                _ => align == TextAlign::End,
            };

            let layout = layout_with_options(
                &system,
                text,
                px(16.),
                &[text_run(text, IBM_PLEX.family)],
                TextLayoutOptions {
                    direction,
                    text_align: align,
                    alignment_width: Some(px(240.)),
                    ..Default::default()
                },
            );

            let native = &layout.platform_layout;

            let expected_direction = match direction {
                ParagraphDirection::RightToLeft => ResolvedDirection::RightToLeft,
                _ => ResolvedDirection::LeftToRight,
            };

            assert_eq!(layout.len, text.len(), "{name}");
            assert_eq!(native.len(), text.len(), "{name}");
            assert_eq!(
                layout.visual_lines[0].direction, expected_direction,
                "{name}"
            );

            let start = native
                .caret_bounds(CaretPosition::attached_to_next_cluster(0), line_height)
                .unwrap();

            assert_eq!(
                start.origin.x > px(100.0),
                right_aligned,
                "{name}: {start:?}"
            );

            if text.is_empty() {
                continue;
            }

            let selection = native.selection_bounds(0..text.len(), line_height);
            assert!(!selection.is_empty(), "{name}");

            if right_aligned {
                assert!(selection[0].origin.x > px(100.0), "{name}: {selection:?}");
            } else {
                assert!(selection[0].origin.x < px(1.0), "{name}: {selection:?}");
            }

            if !endpoint_hits {
                continue;
            }

            for expected in [
                CaretPosition::attached_to_next_cluster(0),
                CaretPosition::attached_to_previous_cluster(text.len()),
            ] {
                let bounds = native.caret_bounds(expected, line_height).unwrap();
                let hit = native
                    .caret_from_pixel_point(
                        bounds.origin + point(px(0.0), line_height / 2.0),
                        line_height,
                    )
                    .unwrap_or_else(|caret| caret);

                assert_eq!(hit, expected, "{name}");
            }
        }
    }

    #[test]
    fn bidi_scope_controls_preserve_source_byte_indices() {
        let system = test_system();
        let text = "A אב B";
        let hebrew = text.find('א').unwrap()..text.find('ב').unwrap() + 'ב'.len_utf8();
        let scopes = [InlineBidiScope {
            range: hebrew,
            direction: ResolvedDirection::RightToLeft,
            unicode_bidi: UnicodeBidi::IsolateOverride,
        }];

        for (text, scopes) in [(text, scopes.as_slice()), ("ab\u{202a}cd\u{202c}ef", &[])] {
            for spacing in [-1.5, 0., 1.5] {
                let runs = [TextRun {
                    letter_spacing: Some(px(spacing)),
                    ..text_run(text, IBM_PLEX.family)
                }];

                let layout = system.layout_inline(InlineLayoutRequest {
                    font_size: px(16.),
                    options: TextLayoutOptions {
                        alignment_width: Some(px(240.)),
                        ..Default::default()
                    },
                    bidi_scopes: scopes,
                    ..inline_request(text, &runs, &runs[0].font)
                });

                let native = &layout.layout.platform_layout;

                assert_eq!(layout.layout.len, text.len());
                assert_eq!(native.len(), text.len());
                assert_eq!(
                    native.logical_cluster_after(CaretPosition::attached_to_next_cluster(0)),
                    Some(0..1),
                );

                for (idx, _character) in text
                    .char_indices()
                    .chain(std::iter::once((text.len(), '\0')))
                {
                    let caret =
                        native.normalized_caret(CaretPosition::attached_to_next_cluster(idx));
                    assert!(caret.index <= text.len());
                }

                assert!(!native.selection_bounds(0..text.len(), px(24.)).is_empty());

                if !scopes.is_empty() {
                    continue;
                }

                let paint_width = layout
                    .layout
                    .paint_fragments
                    .iter()
                    .map(|fragment| fragment.x_range.end - fragment.x_range.start)
                    .sum::<Pixels>();
                let geometry_width = native
                    .inline_geometry(0..text.len())
                    .unwrap()
                    .iter()
                    .map(|geometry| geometry.bounds.size.width)
                    .sum::<Pixels>();

                assert!(
                    (geometry_width - paint_width).abs() < px(0.001),
                    "authored bidi controls must not add advance with letter spacing {spacing}",
                );
            }
        }
    }

    #[test]
    fn result_cache_keeps_source_maps_for_authored_directional_controls() {
        let system = test_system();
        let explicit = layout_with_options(
            &system,
            "a",
            px(16.),
            &[text_run("a", IBM_PLEX.family)],
            TextLayoutOptions {
                direction: ParagraphDirection::LeftToRight,
                text_align: TextAlign::Left,
                ..Default::default()
            },
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
            font_size: px(16.0),
            options: TextLayoutOptions {
                direction: ParagraphDirection::LeftToRight,
                ..Default::default()
            },
            bidi_scopes: &scopes,
            ..inline_request(text, &runs, &font(IBM_PLEX.family))
        });

        let positions = [0..1, 1..2, 2..3].map(|range| {
            layout
                .layout
                .platform_layout
                .inline_geometry(range)
                .unwrap()[0]
                .bounds
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
        for (line_index, line) in layout.lines.iter().enumerate() {
            assert!(
                line.origin.x + line.size.width <= width + epsilon,
                "line {line_index} extends past {width:?}: {line:?}"
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

    fn assert_document_structure(text: &str, layout: &LineLayout) {
        assert_eq!(layout.len, text.len());
        assert!(!layout.visual_lines.is_empty());
        assert_eq!(layout.visual_lines[0].text_range.start, 0);
        assert_eq!(
            layout.visual_lines.last().unwrap().text_range.end,
            text.len()
        );
        assert_eq!(layout.visual_lines[0].paint_fragment_range.start, 0);
        assert_eq!(
            layout.visual_lines.last().unwrap().paint_fragment_range.end,
            layout.paint_fragments.len()
        );

        for pair in layout.visual_lines.windows(2) {
            assert_eq!(pair[0].text_range.end, pair[1].text_range.start);
            assert_eq!(
                pair[0].paint_fragment_range.end,
                pair[1].paint_fragment_range.start
            );
        }

        for line in &layout.visual_lines {
            assert!(text.is_char_boundary(line.text_range.start));
            assert!(text.is_char_boundary(line.text_range.end));
            assert!(
                f32::from(line.advance_width).is_finite() && line.advance_width >= Pixels::ZERO
            );
            for fragment in &layout.paint_fragments[line.paint_fragment_range.clone()] {
                assert!(f32::from(fragment.x_range.start).is_finite());
                assert!(f32::from(fragment.x_range.end).is_finite());
                assert!(fragment.x_range.start <= fragment.x_range.end);
                assert!(fragment.x_range.start >= -layout.font_size * 2.0);
                assert!(fragment.x_range.end <= line.advance_width + layout.font_size * 2.0);
                for glyph in &*fragment.glyphs {
                    assert!(f32::from(glyph.position.x).is_finite());
                    assert!(f32::from(glyph.position.y).is_finite());
                }
            }
        }
    }

    fn assert_document_navigation(text: &str, layout: &LineLayout) {
        let line_height = px(24.0);
        let wrapped = shaped_text_layout(layout.clone(), px(120.0));
        let mut caret = wrapped
            .closest_caret_for_pixel_point(point(px(-100.0), line_height * 0.5), line_height)
            .unwrap_err();
        let mut seen = Vec::new();
        let max_steps = text.chars().count() * 4 + wrapped.line_count() * 4 + 8;
        for _ in 0..max_steps {
            assert!(text.is_char_boundary(caret.index));
            assert!(!seen.contains(&caret), "visual caret traversal cycled");
            seen.push(caret);
            let Some(next) = wrapped.adjacent_visual_caret(caret, VisualDirection::Right) else {
                break;
            };

            caret = next;
        }

        assert!(
            wrapped
                .adjacent_visual_caret(caret, VisualDirection::Right)
                .is_none()
        );
        for _ in 0..max_steps {
            let Some(previous) = wrapped.adjacent_visual_caret(caret, VisualDirection::Left) else {
                break;
            };

            caret = previous;
        }

        assert!(
            wrapped
                .adjacent_visual_caret(caret, VisualDirection::Left)
                .is_none(),
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
    fn inline_alignment_uses_parent_font_and_keeps_it_in_both_caches() {
        let system = test_system();

        // OS/2 versions before 2 have no sxHeight. Keep the fixture's other metrics.
        let mut bytes = IBM_PLEX.data.to_vec();

        let offset = FontRef::new(&bytes)
            .unwrap()
            .table_directory()
            .table_records()
            .iter()
            .find(|record| record.tag() == *b"OS/2")
            .unwrap()
            .offset() as usize;

        bytes[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes());

        assert!(
            FontRef::new(&bytes)
                .unwrap()
                .metrics(FontSize::unscaled(), LocationRef::default())
                .x_height
                .is_none()
        );

        let missing = ParleyTextSystem::new(SystemFonts::Skip, IBM_PLEX.family, TestTextSystem);

        missing
            .add_fonts(vec![Cow::Owned(bytes), Cow::Borrowed(SOURCE_SERIF.data)])
            .unwrap();

        let mut parent_font = font(SOURCE_SERIF.family);
        parent_font.features = GpuiFontFeatures(Arc::new(vec![("liga".into(), 0)]));

        let fonts = [
            parent_font,
            font(IBM_PLEX.family),
            font(SOURCE_SERIF.family),
        ];
        let boxes = [InlineBoxRequest {
            id: 1,
            index: 0,
            size: size(px(8.), px(8.)),
            vertical_align: VerticalAlign::Middle,
        }];

        for (system, text, include_run) in [
            (system.as_ref(), "office", true),
            (system.as_ref(), "", false),
            (system.as_ref(), "", true),
            (&missing, "", false),
        ] {
            let runs = [text_run(text, SOURCE_SERIF.family)];
            let expected = layout_line(system, text, px(18.), &runs);
            let expected_glyphs = expected
                .paint_fragments
                .iter()
                .flat_map(|fragment| fragment.glyphs.iter().map(|glyph| glyph.id))
                .collect::<Vec<_>>();

            if text == "office" {
                assert!(
                    expected_glyphs.len() < text.len(),
                    "The fixture must form a ligature"
                );
            }

            let request = InlineLayoutRequest {
                boxes: &boxes,
                line_height: px(32.),
                ..inline_request(text, if include_run { &runs } else { &[] }, &fonts[0])
            };

            for idx in [0, 1, 2, 0, 1, 2] {
                let base_font = &fonts[idx];
                let metrics = system.font_metrics(system.font_id(base_font).unwrap());
                let x_height = metrics.x_height(request.font_size);
                assert!(x_height > Pixels::ZERO && x_height < metrics.ascent(request.font_size));

                let layout = system.layout_inline(InlineLayoutRequest {
                    base_font,
                    ..request
                });
                let line = layout.lines[0];

                assert!(
                    (layout.boxes[0].bounds.center().y
                        - (line.origin.y + line.baseline - x_height / 2.))
                        .abs()
                        < px(0.001),
                    "parent font {} must control middle alignment for {text:?}",
                    base_font.family,
                );
                assert_eq!(layout.size, layout.layout.platform_layout.size());
                assert_eq!(
                    layout
                        .layout
                        .paint_fragments
                        .iter()
                        .flat_map(|fragment| fragment.glyphs.iter().map(|glyph| glyph.id))
                        .collect::<Vec<_>>(),
                    expected_glyphs,
                    "child runs must retain their own font features",
                );
            }
        }
    }

    #[test]
    fn native_cluster_geometry_matches_positioned_runs() {
        let system = test_system();

        for (text, family, style_range, letter_spacing, box_index) in [
            ("abcde", IBM_PLEX.family, 2..3, 0., None),
            ("office", SOURCE_SERIF.family, 2..3, 0., None),
            ("لا", NOTO_ARABIC.family, 2..4, 0., None),
            ("\u{301}a", IBM_PLEX.family, 0..2, 0., None),
            ("ab\ncd", IBM_PLEX.family, 2..3, 1.5, None),
            ("abcde", IBM_PLEX.family, 2..3, 1.5, Some(3)),
            ("A אב office B", IBM_PLEX.family, 4..6, 1.5, Some(7)),
            ("אב office", IBM_PLEX.family, 0..4, -1.5, None),
            ("", IBM_PLEX.family, 0..0, 0., None),
            ("", IBM_PLEX.family, 0..0, 0., Some(0)),
            ("\n\n", IBM_PLEX.family, 0..0, 0., None),
            ("office\n", IBM_PLEX.family, 0..0, 0., None),
            ("a\r\nb", IBM_PLEX.family, 0..0, 0., None),
            ("👩🏽‍💻 cafe\u{301}", IBM_PLEX.family, 0..0, 0., None),
            ("a \u{2067}אב\u{2069} b", IBM_PLEX.family, 0..0, 0., None),
        ] {
            let descriptor = font(family);
            let properties = system.font_properties(&descriptor).unwrap();
            let mut layout = {
                let mut state = system.parley.lock();

                let ParleyState {
                    catalog,
                    layout_context,
                } = &mut *state;

                let mut builder =
                    layout_context.ranged_builder(catalog.context_mut(), text, 1., false);

                for property in properties {
                    builder.push_default(property);
                }

                builder.push_default(StyleProperty::Brush(ParleyBrush {
                    source_run: 0,
                    has_letter_spacing: letter_spacing != 0.,
                }));
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
                builder.push_default(StyleProperty::FontSize(18.));
                builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(24.)));
                builder.push(
                    StyleProperty::LineHeight(LineHeight::Absolute(36.)),
                    style_range,
                );

                if let Some(index) = box_index {
                    builder.push_inline_box(InlineBox {
                        id: 0,
                        kind: InlineBoxKind::InFlow,
                        index,
                        width: 12.,
                        height: 10.,
                        baseline: None,
                        vertical_align: ParleyVerticalAlign::MIDDLE,
                    });
                }

                builder.build(text)
            };
            layout.break_all_lines(None);
            layout.align(Alignment::Left, AlignmentOptions::default());

            let mut expected_advance = 0.;
            let mut actual_advance = 0.;
            let mut actual_ranges = Vec::new();
            let mut ligature_continuations = 0;

            for line in layout.lines() {
                let native_regions = line
                    .items()
                    .filter_map(|item| {
                        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                            return None;
                        };
                        let metrics = glyph_run.run().font_metrics();
                        expected_advance += glyph_run.advance();

                        Some((
                            glyph_run.offset()..glyph_run.offset() + glyph_run.advance(),
                            glyph_run.baseline() - metrics.ascent
                                ..glyph_run.baseline() + metrics.descent,
                        ))
                    })
                    .collect::<Vec<_>>();

                ParleyLayout::visit_line_clusters(line, |cluster, advance, vertical| {
                    actual_advance += advance.end - advance.start;
                    actual_ranges.push(cluster.text_range());
                    ligature_continuations += usize::from(cluster.is_ligature_continuation());
                    assert!(
                        native_regions
                            .iter()
                            .any(|(native_inline, native_vertical)| {
                                advance.start >= native_inline.start - 0.001
                                    && advance.end <= native_inline.end + 0.001
                                    && vertical == *native_vertical
                            }),
                        "{text:?}: cluster geometry must stay within its native glyph run",
                    );
                });
            }

            let expected_ranges = text
                .grapheme_indices(true)
                .filter(|(_idx, grapheme)| !matches!(*grapheme, "\n" | "\r\n"))
                .map(|(idx, grapheme)| idx..idx + grapheme.len())
                .collect::<Vec<_>>();
            actual_ranges.sort_unstable_by_key(|range| range.start);
            assert_eq!(actual_ranges, expected_ranges, "{text:?}");
            assert!(
                (actual_advance - expected_advance).abs() < 0.001,
                "{text:?}: cluster advances must cover each native glyph run once",
            );

            if matches!(text, "office" | "لا") {
                assert!(
                    ligature_continuations > 0,
                    "{text:?}: fixture must form a ligature"
                );
            }

            let inline_lines = layout
                .lines()
                .map(|line| InlineVisualLine {
                    origin: point(Pixels::ZERO, px(line.metrics().block_min_coord)),
                    size: size(px(line.metrics().advance), px(line.metrics().line_height)),
                    baseline: px(line.metrics().baseline - line.metrics().block_min_coord),
                })
                .collect();

            let native = ParleyLayout::new(
                layout,
                text.into(),
                inline_lines,
                true,
                identity_source_map(text.len()),
            );

            assert_cursor_geometry(&native);

            let geometries = native.inline_cluster_geometry();

            for pair in geometries.windows(2) {
                assert!(
                    pair[0].text_range.end <= pair[1].text_range.start,
                    "{text:?}: logical cluster ranges must not overlap",
                );
            }

            let mut ranges = vec![
                text.len()..text.len() + 4,
                0..text.len(),
                1..text.len().saturating_sub(1),
                0..0,
                text.len()..0,
                0..usize::MAX,
                1..1,
                0..text.len(),
            ];

            ranges.extend((0..text.len()).map(|idx| idx..idx + 1));

            let actual = native.inline_geometry_for_ranges(&ranges);
            assert_eq!(actual.len(), ranges.len());

            let mut visual_order = geometries.iter().collect::<Vec<_>>();
            visual_order.sort_unstable_by_key(|geometry| geometry.visual_idx);

            for (range, actual) in ranges.iter().zip(actual) {
                let mut expected = Vec::new();

                for geometry in visual_order.iter().filter(|geometry| {
                    !range.is_empty()
                        && geometry.text_range.start < range.end
                        && range.start < geometry.text_range.end
                }) {
                    ParleyLayout::append_inline_geometry(&mut expected, geometry);
                }

                assert_eq!(actual, expected, "{text:?}, range {range:?}");
            }

            let mut wrapped = native.clone();
            wrapped.layout.break_all_lines(Some(40.));
            wrapped
                .layout
                .align(Alignment::Left, AlignmentOptions::default());

            wrapped.inline_lines = wrapped
                .layout
                .lines()
                .map(|line| InlineVisualLine {
                    origin: point(Pixels::ZERO, px(line.metrics().block_min_coord)),
                    ..Default::default()
                })
                .collect();

            for alignment in [Alignment::Left, Alignment::Center, Alignment::Right] {
                wrapped.layout.align(alignment, AlignmentOptions::default());
                assert_cursor_geometry(&wrapped);
            }
        }
    }

    #[test]
    fn inline_line_height_preserves_typographic_content_extents() {
        let system = test_system();
        let base_font = font(IBM_PLEX.family);
        let metrics = system.font_metrics(system.font_id(&base_font).unwrap());
        let runs = [text_run("x", IBM_PLEX.family)];

        let layout = system.layout_inline(InlineLayoutRequest {
            line_height: px(12.),
            options: TextLayoutOptions::default(),
            ..inline_request("x", &runs, &base_font)
        });

        assert_eq!(layout.lines[0].size.height, px(12.));
        assert_eq!(layout.size.height, px(12.));
        assert!((layout.layout.ascent - metrics.ascent(px(18.))).abs() < px(0.001));
        assert!((layout.layout.descent - metrics.descent(px(18.))).abs() < px(0.001));
        assert!(layout.layout.ascent + layout.layout.descent > layout.size.height);

        let geometry = layout.layout.platform_layout.inline_geometry(0..1).unwrap();

        assert!(
            (geometry[0].bounds.size.height - metrics.ascent(px(18.)) - metrics.descent(px(18.)))
                .abs()
                < px(0.001)
        );
    }

    #[test]
    fn inline_ranges_forward_font_metrics_and_keep_native_geometry() {
        let system = test_system();

        let text = "small\nBIG words\nend";
        let runs = [text_run(text, IBM_PLEX.family)];

        let text_styles = [InlineTextStyle {
            range: 6..15,
            font_size: px(30.),
            line_height: px(46.),
        }];

        let layout = system.layout_inline(InlineLayoutRequest {
            text_styles: &text_styles,
            font_size: px(14.),
            line_height: px(20.),
            options: TextLayoutOptions {
                wrap_width: Some(px(300.)),
                text_align: TextAlign::Left,
                ..Default::default()
            },
            ..inline_request(text, &runs, &font(IBM_PLEX.family))
        });

        let platform = &layout.layout.platform_layout;
        assert_eq!(layout.lines.len(), 3);

        assert!(
            layout.lines[1].size.height >= px(45.99),
            "{:?}",
            layout.lines
        );

        for font_size in [px(14.), px(30.)] {
            assert!(
                layout
                    .layout
                    .paint_fragments
                    .iter()
                    .any(|fragment| fragment.font_size == font_size),
                "missing {font_size:?} paint fragment",
            );
        }

        let native = platform.inline_geometry(16..19).unwrap();
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].visual_line_index, 2);
        assert!(native[0].bounds.origin.y >= px(65.99), "{native:?}");

        let selection = platform.selection_bounds(16..19, px(20.));

        assert_eq!(
            selection[0].origin.y, layout.lines[2].origin.y,
            "selection follows the native line position"
        );

        assert_eq!(platform.native_lines().unwrap(), layout.lines);

        for (line_idx, source_idx) in [(0, 0), (1, 6), (2, 16)] {
            let caret = CaretPosition::attached_to_next_cluster(source_idx);
            let bounds = platform.caret_bounds(caret, px(1.)).unwrap();
            let line = layout.lines[line_idx];
            let position = point(
                bounds.origin.x + px(0.1),
                line.origin.y + line.size.height / 2.,
            );

            assert_eq!(platform.visual_line_index(caret), line_idx);
            assert_eq!(bounds.origin.y, line.origin.y);
            assert_eq!(
                platform
                    .caret_from_pixel_point(position, px(1.))
                    .unwrap()
                    .index,
                source_idx
            );
            assert_eq!(
                platform
                    .byte_index_from_pixel_point(position, px(1.))
                    .unwrap(),
                source_idx
            );
        }

        let moved = platform.caret_movement(
            CaretPosition::attached_to_next_cluster(6),
            Direction::Down.with_boundary(Boundary::VisualLine),
            None,
        );

        assert_eq!(platform.visual_line_index(moved.result), 2);
        assert_eq!(moved.result.index, 16);

        let third = layout.lines[2];

        assert_eq!(
            platform.selection_from_pixel_point(
                point(
                    third.origin.x + px(1.),
                    third.origin.y + third.size.height / 2.
                ),
                px(1.),
                TextSelectionKind::Word,
            ),
            16..19,
        );

        let raw = layout_line(&system, text, px(14.), &runs);
        assert!(raw.platform_layout.native_lines().is_none());

        assert_eq!(
            raw.platform_layout
                .caret_bounds(CaretPosition::attached_to_next_cluster(16), px(20.))
                .unwrap()
                .origin
                .y,
            px(40.),
            "raw editor documents retain caller-sized rows",
        );

        assert!(
            platform.inline_geometry(5..6).unwrap().is_empty(),
            "newlines add no span hit region"
        );

        assert!(platform.inline_geometry(6..6).is_none());
        let batched = platform.inline_geometry_for_ranges(&[6..6, 5..6, 16..19]);
        assert_eq!(batched, vec![vec![], vec![], native]);
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

        let base_font = font(IBM_PLEX.family);
        let x_height = system
            .font_metrics(system.font_id(&base_font).unwrap())
            .x_height(px(18.));

        let request = InlineLayoutRequest {
            boxes: &boxes,
            options: TextLayoutOptions {
                wrap_width: Some(px(160.0)),
                text_align: TextAlign::Center,
                ..Default::default()
            },
            ..inline_request(text, &runs, &base_font)
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

        for color in [first_color, second_color] {
            assert!(
                layout
                    .layout
                    .paint_fragments
                    .iter()
                    .any(|fragment| fragment.style.color == color),
                "missing {color:?} paint fragment",
            );
        }

        for inline_box in &layout.boxes {
            let line = layout.lines[inline_box.line_index];
            let bounds = positioned_box_bounds(&layout, inline_box);

            let offset = match inline_box.id {
                7 => bounds.bottom() - (line.origin.y + line.baseline),
                9 => bounds.center().y - (line.origin.y + line.baseline - x_height / 2.),
                11 => bounds.origin.y - line.origin.y,
                13 => bounds.bottom() - (line.origin.y + line.size.height),
                _ => unreachable!(),
            };

            assert!(
                offset.abs() < px(0.01),
                "inline box {} is misaligned by {offset:?}",
                inline_box.id,
            );
        }

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
            boxes: &boxes,
            options: TextLayoutOptions {
                wrap_width: Some(px(24.0)),
                text_align: TextAlign::Left,
                ..Default::default()
            },
            ..inline_request("", &[], &font(IBM_PLEX.family))
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

    fn paragraph_samples() -> [String; 4] {
        let sample =
            "שלום עולם\nمرحبا بالعالم\nabc אבג def\nx (مرحبا) y\nEnglish ثم عربي ثم English";

        [
            sample.to_string(),
            format!("Latin first\n{sample}\n\n"),
            "אבג\u{2028}abc אבג def\n123 (45)\n\u{2067}אבג\u{2069} abc\nمرحبا".to_string(),
            "שלום עולם\nabc אבג def\nx (مرحبا) y".to_string(),
        ]
    }

    #[test]
    fn paragraph_paint_and_carets_match_independent_layouts() {
        let system = test_system();
        for text in paragraph_samples() {
            for width in [px(500.), px(90.)] {
                let document = wrapped_sample(&system, &text, width);

                if width == px(500.) && !text.contains('\u{2028}') {
                    let shaped = shaped_text_layout(document.clone(), width);

                    for (left, right) in [("abc", "def"), ("x", "y")] {
                        let Some((left_idx, right_idx)) = text.find(left).zip(text.find(right))
                        else {
                            continue;
                        };

                        let left_position = shaped
                            .visual_position_for_byte_index(left_idx, px(24.))
                            .unwrap();
                        let right_position = shaped
                            .visual_position_for_byte_index(right_idx, px(24.))
                            .unwrap();
                        assert_eq!(left_position.y, right_position.y);
                        assert!(
                            left_position.x < right_position.x,
                            "{left} must precede {right}"
                        );
                    }
                }

                let mut first_line = 0;

                for source in ParagraphRange::split(&text) {
                    let content = &text[source.content.clone()];
                    let independent = wrapped_sample(&system, content, width);

                    for (idx, expected) in independent.visual_lines.iter().enumerate() {
                        let actual = &document.visual_lines[first_line + idx];
                        assert_eq!(actual.advance_width, expected.advance_width);
                        assert_eq!(
                            document.paint_fragments[actual.paint_fragment_range.clone()],
                            independent.paint_fragments[expected.paint_fragment_range.clone()],
                            "{content:?} at {width:?}"
                        );
                    }

                    for (idx, _character) in content
                        .char_indices()
                        .chain(std::iter::once((content.len(), '\0')))
                    {
                        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
                            let local = CaretPosition {
                                index: idx,
                                affinity,
                            };
                            let global = CaretPosition {
                                index: source.content.start + idx,
                                affinity,
                            };
                            let mut expected = independent
                                .platform_layout
                                .caret_bounds(local, px(24.))
                                .unwrap();
                            expected.origin.y += px(24.) * first_line;

                            assert_eq!(
                                document.platform_layout.caret_bounds(global, px(24.)),
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

    fn assert_paragraph_break_interaction(system: &ParleyTextSystem) {
        let text = "שלום עולם\r\n\nabc אבג def\u{2029}مرحبا بالعالم\n";
        let layout = wrapped_sample(system, text, px(100.));
        let native = &layout.platform_layout;
        let line_height = px(24.);
        let geometry = |caret| native.caret_bounds(caret, line_height).unwrap();
        let mut caret = native
            .caret_from_pixel_point(point(px(-100.), px(12.)), line_height)
            .unwrap_err();
        let mut steps = 0;

        while let Some(next) = native.adjacent_visual_caret(caret, VisualDirection::Right) {
            let previous = native
                .adjacent_visual_caret(next, VisualDirection::Left)
                .unwrap_or_else(|| panic!("cannot reverse {caret:?} -> {next:?}"));
            assert_eq!(
                geometry(previous),
                geometry(caret),
                "visual movement must reverse {caret:?} -> {next:?} -> {previous:?}"
            );

            let bounds = geometry(next);
            let hit = native
                .caret_from_pixel_point(
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

        for source in ParagraphRange::split(text) {
            if source.separator.is_empty() {
                continue;
            }

            let before = CaretPosition::attached_to_next_cluster(source.separator.start);
            let after = CaretPosition::attached_to_next_cluster(source.separator.end);
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
                    .selection_bounds(source.separator.clone(), line_height)
                    .is_empty()
            );
            assert!(
                native
                    .inline_geometry(source.separator.clone())
                    .unwrap()
                    .is_empty()
            );

            let position = geometry(before).origin + point(px(0.), line_height / 2.);
            let selected = native.selection_from_pixel_point(
                position,
                line_height,
                TextSelectionKind::HardLine,
            );
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
                    native.caret_movement(before, movement, None).result.index,
                    expected
                );
            }
        }

        let crlf = text.find('\r').unwrap();
        let inside = CaretPosition::attached_to_next_cluster(crlf + 1);
        assert_eq!(native.normalized_caret(inside).index, crlf);

        let start = CaretPosition::default();
        let down = native.caret_movement(
            start,
            Direction::Down.with_boundary(Boundary::VisualLine),
            None,
        );
        assert_eq!(
            geometry(down.result).origin.y,
            geometry(start).origin.y + line_height
        );
        assert_eq!(down.vertical_navigation_x, Some(geometry(start).origin.x));

        let mut vertical_caret = start;
        let mut vertical_navigation_x = None;

        let down_lines = (1..native.line_count()).map(|idx| (Direction::Down, idx));

        let up_lines = (0..native.line_count() - 1)
            .rev()
            .map(|idx| (Direction::Up, idx));

        for (direction, line_idx) in down_lines.chain(up_lines) {
            let moved = native.caret_movement(
                vertical_caret,
                direction.with_boundary(Boundary::VisualLine),
                vertical_navigation_x,
            );

            vertical_caret = moved.result;
            vertical_navigation_x = moved.vertical_navigation_x;
            assert_eq!(geometry(vertical_caret).origin.y, line_height * line_idx);
            assert_eq!(vertical_navigation_x, Some(geometry(start).origin.x));
        }

        let end = CaretPosition::attached_to_previous_cluster(text.len());

        for (origin, direction, expected) in
            [(end, Direction::Start, start), (start, Direction::End, end)]
        {
            let moved =
                native.caret_movement(origin, direction.with_boundary(Boundary::Document), None);

            assert_eq!(moved.result, expected, "{direction:?}");
        }

        for (direction, edge_x) in [
            (Direction::Left, px(-100.)),
            (Direction::Right, px(10_000.)),
        ] {
            let empty_row = native
                .caret_from_pixel_point(point(edge_x, line_height * 1.5), line_height)
                .unwrap_or_else(|caret| caret);

            let word = native
                .caret_movement(empty_row, direction.with_boundary(Boundary::Word), None)
                .result;

            assert_ne!(geometry(word).origin.y, geometry(empty_row).origin.y);
        }

        let selected = native.selection_bounds(0..text.len(), line_height);
        assert!(
            selected
                .windows(2)
                .all(|pair| pair[0].origin.y <= pair[1].origin.y)
        );

        let mixed = "abc אבג\nאבג abc\n";
        let layout = layout_line(system, mixed, px(18.), &[text_run(mixed, "IBM Plex Sans")]);

        for (line_index, source) in ParagraphRange::split(mixed).into_iter().take(2).enumerate() {
            let selection = layout
                .platform_layout
                .selection_bounds(source.separator, line_height);
            let bounds = selection[0];
            assert_eq!(bounds.origin.y, line_height * line_index);

            if line_index == 0 {
                assert!(bounds.origin.x >= layout.visual_lines[line_index].advance_width);
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
                text_styles: &styles,
                boxes: &boxes,
                line_height: px(28.),
                options: TextLayoutOptions {
                    wrap_width: Some(px(260.)),
                    text_align,
                    ..Default::default()
                },
                ..inline_request(text, &runs, &font(IBM_PLEX.family))
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
                .caret_bounds(
                    CaretPosition::attached_to_next_cluster(text.len() - 2),
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
                .inline_geometry(split..text.len())
                .unwrap();
            assert!(regions.iter().all(|geometry| {
                let line = inline.lines[geometry.visual_line_index];

                geometry.bounds.bottom() > line.origin.y
                    && geometry.bounds.origin.y < line.origin.y + line.size.height
            }));
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
                .parley_paragraph_layout(ParleyLayoutParams {
                    text: TextLayoutRequest {
                        text,
                        font_size: px(18.),
                        runs: &runs,
                        options: TextLayoutOptions {
                            wrap_width: Some(px(80.)),
                            line_clamp: max_lines,
                            ..TextLayoutOptions::default()
                        },
                    },
                    inline: None,
                })
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
        }
    }

    struct DocumentCase {
        name: &'static str,
        text: Cow<'static, str>,
        wrap_width: Option<Pixels>,
        line_clamp: Option<usize>,
        minimum_lines: usize,
        exact_line_count: bool,
    }

    fn document_cases() -> Vec<DocumentCase> {
        let mut cases = [
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
        ]
        .into_iter()
        .map(
            |(name, text, width, line_clamp, minimum_lines)| DocumentCase {
                name,
                text: Cow::Borrowed(text),
                wrap_width: Some(width),
                line_clamp,
                minimum_lines,
                exact_line_count: line_clamp.is_some(),
            },
        )
        .collect::<Vec<_>>();

        cases.push(DocumentCase {
            name: "unwrapped empty",
            text: Cow::Borrowed(""),
            wrap_width: None,
            line_clamp: None,
            minimum_lines: 1,
            exact_line_count: false,
        });

        for text in paragraph_samples() {
            for width in [px(500.), px(90.)] {
                cases.push(DocumentCase {
                    name: "paragraph composition",
                    text: Cow::Owned(text.clone()),
                    wrap_width: Some(width),
                    line_clamp: None,
                    minimum_lines: 1,
                    exact_line_count: false,
                });
            }
        }

        for line_clamp in [None, Some(0), Some(1), Some(3), Some(8)] {
            cases.push(DocumentCase {
                name: "Latin composition clamp",
                text: Cow::Borrowed("one two three four\nfive six seven eight\nnine ten\n"),
                wrap_width: Some(px(80.)),
                line_clamp,
                minimum_lines: 1,
                exact_line_count: false,
            });
        }

        cases
    }

    fn layout_document_case(system: &ParleyTextSystem, case: &DocumentCase) -> LineLayout {
        let runs = (!case.text.is_empty())
            .then(|| text_run(&case.text, IBM_PLEX.family))
            .into_iter()
            .collect::<Vec<_>>();

        layout_with_options(
            system,
            &case.text,
            px(18.),
            &runs,
            TextLayoutOptions {
                wrap_width: case.wrap_width,
                line_clamp: case.line_clamp,
                text_align: TextAlign::Left,
                ..Default::default()
            },
        )
    }

    #[test]
    fn document_structure_preserves_source_ranges_fragments_and_line_counts() {
        let system = test_system();

        for case in document_cases() {
            let layout = layout_document_case(&system, &case);
            assert_document_structure(&case.text, &layout);

            if case.exact_line_count {
                assert_eq!(
                    layout.visual_lines.len(),
                    case.minimum_lines,
                    "{}",
                    case.name
                );
            } else {
                assert!(
                    layout.visual_lines.len() >= case.minimum_lines,
                    "{}: {:?}",
                    case.name,
                    case.text
                );
            }

            if case.text.is_empty() {
                assert_eq!(layout.visual_lines[0].text_range, 0..0);
                assert_eq!(layout.width, Pixels::ZERO);
                assert!(layout.paint_fragments.is_empty());
            }
        }
    }

    #[test]
    fn document_navigation_preserves_source_boundaries_across_scripts_and_clamps() {
        let system = test_system();

        for case in document_cases() {
            let layout = layout_document_case(&system, &case);
            assert_document_navigation(&case.text, &layout);
        }

        assert_paragraph_break_interaction(&system);
        assert_native_cluster_interaction(&system);
    }

    #[test]
    fn wrapping_does_not_move_breaks_forward_as_width_shrinks() {
        let system = test_system();
        let cases = [
            (
                "code punctuation",
                "Lilex regular: fn main() { println!(\"hello\"); }",
                LILEX.family,
            ),
            (
                "prose punctuation",
                "One sentence with punctuation, followed by another.",
                IBM_PLEX.family,
            ),
            (
                "nested delimiters",
                "call(value, other_value) } trailing",
                IBM_PLEX.family,
            ),
            (
                "mixed scripts",
                "English العربية 日本語 punctuation.",
                IBM_PLEX.family,
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
    fn paint_changes_preserve_geometry_cache_identity_and_authored_run_boundaries() {
        let system = test_system();
        let text = "office café العربية";
        let first_end = "office ".len();
        let second_end = first_end + "café ".len();
        let mut first_font = font(SOURCE_SERIF.family);
        first_font.features = GpuiFontFeatures::disable_ligatures();
        first_font.fallbacks = Some(FontFallbacks::from_fonts(vec![
            IBM_PLEX.family.into(),
            NOTO_COLOR_EMOJI.family.into(),
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
                font: font(IBM_PLEX.family).bold(),
                color: hsla(0.35, 0.7, 0.35, 1.0),
                ..Default::default()
            },
            TextRun {
                len: text.len() - second_end,
                font: font(IBM_PLEX.family),
                color: hsla(0.6, 0.8, 0.45, 1.0),
                ..Default::default()
            },
        ];
        let mut left = text_run("left ", NOTO_SANS.family);
        left.font.width = gpui::FontWidth::CONDENSED;
        let right = text_run("right", IBM_PLEX.family);

        for (name, text, original_runs, font_size) in [
            ("styled document", text, base_runs, px(20.)),
            (
                "identical paint boundaries",
                "left right",
                vec![left, right],
                px(18.),
            ),
            (
                "matching parent font",
                "left right",
                vec![
                    TextRun {
                        letter_spacing: Some(px(0.4)),
                        ..text_run("left ", IBM_PLEX.family)
                    },
                    text_run("right", IBM_PLEX.family),
                ],
                px(18.),
            ),
        ] {
            let mut decorated_runs = original_runs.clone();

            for (idx, run) in decorated_runs.iter_mut().enumerate() {
                run.color = hsla(idx as f32 * 0.3, 0.8, 0.4, 1.0);
            }

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

            let original = layout_line(&system, text, font_size, &original_runs);
            let decorated = layout_line(&system, text, font_size, &decorated_runs);

            if name == "matching parent font" {
                let unspaced =
                    layout_line(&system, text, font_size, &[text_run(text, IBM_PLEX.family)]);
                let expected_spacing = px(0.4) * "left ".chars().count();
                assert!((original.width - unspaced.width - expected_spacing).abs() < px(0.001));
            }

            let geometry = |layout: &LineLayout| {
                layout
                    .paint_fragments
                    .iter()
                    .flat_map(|fragment| {
                        fragment.glyphs.iter().map(move |glyph| {
                            (
                                fragment.source_run,
                                fragment.font_id,
                                glyph.id,
                                glyph.position,
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(original.width, decorated.width, "{name}");
            assert_eq!(geometry(&original), geometry(&decorated), "{name}");
            assert!(
                Arc::ptr_eq(&original.platform_layout, &decorated.platform_layout),
                "{name}"
            );

            for (idx, run) in decorated_runs.iter().enumerate() {
                let fragments = decorated
                    .paint_fragments
                    .iter()
                    .filter(|fragment| fragment.source_run == idx)
                    .collect::<Vec<_>>();
                assert!(!fragments.is_empty(), "{name}: missing authored run {idx}");
                assert!(
                    fragments
                        .iter()
                        .all(|fragment| fragment.style == PaintStyle::from(run)),
                    "{name}: lost paint style for run {idx}"
                );
            }
        }
    }

    #[test]
    fn cached_paint_tracks_authored_runs_after_empty_runs_are_removed() {
        let lengths = [&[0, 1][..], &[1][..], &[1, 0][..]];

        for order in [[0, 1, 2, 0], [1, 0, 2, 1]] {
            let system = test_system();
            let mut geometry = None;

            for case_idx in order {
                let runs = lengths[case_idx]
                    .iter()
                    .enumerate()
                    .map(|(idx, len)| TextRun {
                        len: *len,
                        color: hsla(case_idx as f32 * 0.3 + idx as f32 * 0.1, 0.8, 0.4, 1.),
                        ..text_run("a", IBM_PLEX.family)
                    })
                    .collect::<Vec<_>>();

                let layout = layout_line(&system, "a", px(18.), &runs);
                let source_run_idx = lengths[case_idx].iter().position(|len| *len != 0).unwrap();
                assert!(!layout.paint_fragments.is_empty());

                for fragment in &layout.paint_fragments {
                    assert_eq!(fragment.source_run, source_run_idx, "case {case_idx}");

                    assert_eq!(
                        fragment.style,
                        PaintStyle::from(&runs[source_run_idx]),
                        "case {case_idx}",
                    );
                }

                let cached = geometry.get_or_insert_with(|| layout.platform_layout.clone());

                assert!(
                    Arc::ptr_eq(cached, &layout.platform_layout),
                    "case {case_idx} must reuse cached geometry",
                );
            }
        }
    }

    #[test]
    fn paragraph_caches_preserve_shaping_across_widths_and_edits() {
        let system = test_system();
        let text = "one two three four five";
        let runs = [text_run(text, NOTO_SANS.family)];
        let normal = layout_line(&system, text, px(18.0), &runs);
        let mut condensed_runs = runs.clone();
        condensed_runs[0].font.width = gpui::FontWidth::CONDENSED;
        let condensed = layout_line(&system, text, px(18.0), &condensed_runs);
        let wrap_width = (normal.width + condensed.width) / 2.0;

        assert!(condensed.width < normal.width);
        assert_eq!(
            layout_wrapped(&system, text, px(18.0), &runs, wrap_width, None)
                .visual_lines
                .len(),
            2
        );
        assert_eq!(
            layout_wrapped(&system, text, px(18.0), &condensed_runs, wrap_width, None)
                .visual_lines
                .len(),
            1
        );
        assert_eq!(system.paragraph_cache.lock().entries.len(), 2);
        assert_eq!(system.paragraph_result_cache.lock().entries.len(), 4);

        let restored = layout_line(&system, text, px(18.0), &runs);
        assert!(Arc::ptr_eq(
            &normal.platform_layout,
            &restored.platform_layout
        ));

        let first = "stable paragraph\nfirst ending";
        let second = "stable paragraph\nsecond ending";
        wrapped_sample(&system, first, px(120.0));
        let cache_entries = system.paragraph_cache.lock().entries.len();
        wrapped_sample(&system, second, px(120.0));

        assert_eq!(
            system.paragraph_cache.lock().entries.len(),
            cache_entries + 1,
            "the unchanged first paragraph should keep its cached shaping"
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

        let system = ParleyTextSystem::new(
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
            ParleyTextSystem::new(SystemFonts::Skip, IBM_PLEX.family, TestTextSystem)
                .with_fallback_families([IBM_PLEX.family]),
        );

        backend
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX.data)])
            .unwrap();
        let text_system = Arc::new(TextSystem::new(backend.clone()));
        let window_text_system = WindowTextSystem::new(text_system);
        let text = "registered later";
        let run = text_run(text, SOURCE_SERIF.family);
        let before = window_text_system
            .shape_text(text, px(18.0), std::slice::from_ref(&run), None, None)
            .unwrap();
        let fallback_id = before.paint_fragments[0].font_id;

        backend
            .add_fonts(vec![
                Cow::Borrowed(SOURCE_SERIF.data),
                Cow::Borrowed(NOTO_SANS.data),
            ])
            .unwrap();
        let after = window_text_system
            .shape_text(text, px(18.0), &[run], None, None)
            .unwrap();
        assert_ne!(fallback_id, after.paint_fragments[0].font_id);
        let source_serif_regular = backend.font_id(&font("Source Serif 4")).unwrap();
        assert_eq!(after.paint_fragments[0].font_id, source_serif_regular);

        backend
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX_SEMIBOLD.data)])
            .unwrap();
        let regular = backend.font_id(&font(IBM_PLEX.family)).unwrap();
        let bold = backend.font_id(&font(IBM_PLEX.family).bold()).unwrap();
        assert_ne!(regular, bold);

        let mut variable = font(SOURCE_SERIF.family);
        variable.weight = GpuiFontWeight(725.0);
        variable.style = GpuiFontStyle::Oblique;
        let variable_id = backend.font_id(&variable).unwrap();
        let source_serif_regular = backend.font_id(&font(SOURCE_SERIF.family)).unwrap();
        assert_ne!(source_serif_regular, variable_id);

        let mut run = text_run("H", NOTO_SANS.family);
        run.font.weight = GpuiFontWeight(725.0);
        let normal_id = backend.font_id(&run.font).unwrap();
        let normal = window_text_system.layout_line("H", px(32.0), &[run.clone()]);
        run.font.width = gpui::FontWidth(82.5);
        let condensed_id = backend.font_id(&run.font).unwrap();
        let condensed = window_text_system.layout_line("H", px(32.0), &[run.clone()]);

        assert_eq!(condensed.paint_fragments[0].font_id, condensed_id);
        assert_ne!(normal_id, condensed_id);
        assert!(condensed.width < normal.width);

        let glyph_id = condensed.paint_fragments[0].glyphs[0].id;
        let advance = backend.advance(condensed_id, glyph_id).unwrap().width;
        let units_per_em = backend.font_metrics(condensed_id).units_per_em as f32;
        assert!((condensed.width - px(advance / units_per_em * 32.0)).abs() < px(0.01));

        run.font.width = gpui::FontWidth::EXPANDED;
        assert_eq!(backend.font_id(&run.font).unwrap(), normal_id);

        let mut static_font = font(IBM_PLEX.family);
        static_font.width = gpui::FontWidth::CONDENSED;
        assert_eq!(backend.font_id(&static_font).unwrap(), regular);
    }

    fn assert_native_cluster_interaction(system: &ParleyTextSystem) {
        for text in ["👩🏽‍💻", "👨‍👩‍👧‍👦", "🇬🇧", "ก้"] {
            let layout = layout_line(system, text, px(22.0), &[text_run(text, IBM_PLEX.family)]);
            let wrapped = shaped_text_layout(layout, px(500.0));
            let native = &wrapped.platform_layout;

            assert_eq!(
                wrapped.logical_cluster_after(CaretPosition::default()),
                Some(0..text.len()),
                "{text:?} must be one editable cluster"
            );

            let end = native
                .adjacent_visual_caret(CaretPosition::default(), VisualDirection::Right)
                .unwrap();

            assert_eq!(
                end.index,
                text.len(),
                "{text:?} must have no internal caret"
            );

            assert_eq!(
                native
                    .adjacent_visual_caret(end, VisualDirection::Left)
                    .unwrap()
                    .index,
                0,
                "{text:?} must be one visual caret step"
            );
        }

        let single_line_text = "abcd";
        let single_line = shaped_text_layout(
            layout_line(
                system,
                single_line_text,
                px(20.0),
                &[text_run(single_line_text, IBM_PLEX.family)],
            ),
            px(500.0),
        );
        let middle = CaretPosition::attached_to_next_cluster(2);

        for (direction, expected) in [
            (Direction::Up, 0),
            (Direction::Down, single_line_text.len()),
        ] {
            let moved = single_line.caret_movement(
                middle,
                direction.with_boundary(Boundary::VisualLine),
                None,
            );

            assert_eq!(moved.result.index, expected, "{direction:?}");
        }

        let text = "abc אבג العربية xyz";
        let layout = shaped_text_layout(
            layout_wrapped(
                system,
                text,
                px(20.0),
                &[text_run(text, IBM_PLEX.family)],
                px(90.0),
                None,
            ),
            px(90.0),
        );
        let line_height = px(26.0);
        let start = layout
            .closest_caret_for_pixel_point(point(px(-10.0), px(10.0)), line_height)
            .unwrap_err();
        let end = layout
            .closest_caret_for_pixel_point(point(px(10_000.0), px(10.0)), line_height)
            .unwrap_err();
        let selection = CaretSelection {
            anchor: end,
            caret: start,
        };

        for (direction, expected) in [(Direction::Left, start), (Direction::Right, end)] {
            let collapsed = layout.selection_movement(
                selection,
                direction.with_boundary(Boundary::Cluster),
                false,
                None,
                line_height,
            );

            assert!(collapsed.result.is_empty());
            assert_eq!(collapsed.result.caret, expected, "{direction:?}");
        }

        let word = layout.selection_movement(
            start.into(),
            Direction::Right.with_boundary(Boundary::Word),
            true,
            None,
            line_height,
        );
        assert_eq!(word.result.anchor, start);
        assert_ne!(word.result.caret, start);
        let down = layout.selection_movement(
            word.result.caret.into(),
            Direction::Down.with_boundary(Boundary::VisualLine),
            false,
            None,
            line_height,
        );
        assert!(down.vertical_navigation_x.is_some());
        let maintained_x = layout
            .selection_movement(
                down.result,
                Direction::Down.with_boundary(Boundary::VisualLine),
                false,
                down.vertical_navigation_x,
                line_height,
            )
            .vertical_navigation_x;
        assert_eq!(maintained_x, down.vertical_navigation_x);
        let selection = layout.selection_from_pixel_point(
            point(px(12.0), px(10.0)),
            line_height,
            TextSelectionKind::Word,
        );
        assert!(!selection.is_empty());
        assert!(text.is_char_boundary(selection.start));
        assert!(text.is_char_boundary(selection.end));
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
    fn automatic_optical_sizing_tracks_logical_default_and_inline_sizes() {
        let system = ParleyTextSystem::new(SystemFonts::Skip, SOURCE_SERIF.family, TestTextSystem)
            .with_automatic_optical_sizing();

        system
            .add_fonts(vec![Cow::Borrowed(SOURCE_SERIF.data)])
            .unwrap();

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
            text_styles: &text_styles,
            font_size: px(12.0),
            line_height: px(16.0),
            ..inline_request(text, &runs, &font(SOURCE_SERIF.family))
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
        let system = ParleyTextSystem::new(
            SystemFonts::Skip,
            SOURCE_SERIF.family,
            RecordingRasterizer { seen: seen.clone() },
        )
        .with_automatic_optical_sizing();

        system
            .add_fonts(vec![Cow::Borrowed(SOURCE_SERIF.data)])
            .unwrap();

        let text = "A";
        let layout = layout_line(
            &system,
            text,
            px(22.0),
            &[TextRun {
                len: text.len(),
                font: font(SOURCE_SERIF.family).bold().italic(),
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

        let instances = system.font_instances.read();
        let selected = instances.get(font_id).unwrap();

        assert_eq!(seen.source_id, selected.data.id());
        assert_eq!(seen.normalized_coords, selected.normalized_coords);
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
        source_id: u64,
        normalized_coords: Vec<NormalizedCoord>,
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
                source_id: face.source_id,
                normalized_coords: face.normalized_coords.to_vec(),
                face_index: face.face_index,
                data_matches: face.data() == SOURCE_SERIF.data,
                variations: face.variations.to_vec(),
                synthesis: face.synthesis,
                has_color_glyphs: face.has_color_glyphs,
            });

            Ok(RasterizedGlyph::empty(RasterizedGlyphFormat::AlphaMask))
        }
    }

    mod inline_reflow;

    mod direction_layout {
        use super::*;

        struct DirectionFlexView {
            direction: gpui::LayoutDirection,
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

                let row = |selector: &'static str| {
                    div()
                        .direction(self.direction)
                        .w(px(200.0))
                        .h(px(20.0))
                        .debug_selector(move || selector.into())
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        row("logical-row")
                            .flex()
                            .flex_row_reverse()
                            .justify_start()
                            .child(item("logical-first"))
                            .child(item("logical-second")),
                    )
                    .child(
                        row("flex-row")
                            .flex()
                            .flex_row_reverse()
                            .justify_flex_start()
                            .child(item("flex-first"))
                            .child(item("flex-second")),
                    )
                    .child(
                        row("grid-row")
                            .grid()
                            .grid_cols(2)
                            .child(item("grid-first"))
                            .child(item("grid-second")),
                    )
                    .child(
                        row("cached-row").child(
                            self.cached_child
                                .clone()
                                .cached(gpui::StyleRefinement::default().size_full()),
                        ),
                    )
                    .child(
                        row("deferred-row").child(gpui::deferred(
                            div()
                                .flex()
                                .justify_start()
                                .size_full()
                                .child(item("deferred-first")),
                        )),
                    )
                    .child(
                        row("auto-cached-row")
                            .direction(gpui::LayoutDirection::Auto)
                            .flex()
                            .justify_start()
                            .child(
                                item("auto-cached-item").child(
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
                        direction: gpui::LayoutDirection::RightToLeft,
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
                    view.direction = gpui::LayoutDirection::LeftToRight;
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
