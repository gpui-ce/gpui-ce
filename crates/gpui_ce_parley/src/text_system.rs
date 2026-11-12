use crate::FontStore;
use anyhow::{Context as _, Result, bail};
use fontique::{
    Attributes, Blob, Collection, CollectionOptions, QueryFamily, QueryFont, SourceCache,
};
use gpui::{
    Bounds, CaretAffinity, CaretMovement, CaretPosition, ColorGlyphKind, Font, FontId, FontMetrics,
    FontStyle as GpuiFontStyle, FontVariation, GlyphId, GlyphRasterizer, InlineBidiScope,
    InlineBoxRequest, InlineLayout, InlineLayoutRequest, InlineRangeGeometry, InlineTextStyle,
    InlineVisualLine, LineLayout, PaintFragment, PaintStyle, ParagraphDirection, Pixels,
    PlatformTextLayout, PlatformTextSystem, Point, PositionedInlineBox, PreparedRasterStyle,
    RasterStyleRequest, RasterizedGlyph, RenderGlyphParams, ResolvedDirection, ShapedGlyph, Size,
    TextAlign, TextBoundary as Boundary, TextDirection as Direction, TextLayoutOptions,
    TextLayoutRequest, TextMovement, TextRangeExt, TextRenderingMode, TextRun, TextSelectionKind,
    UnicodeBidi, VerticalAlign as GpuiVerticalAlign, VisualDirection, VisualLine,
    is_paragraph_separator, point, px, size,
};
use parking_lot::{Mutex, RwLock};
use parley::setting::Tag;
use parley::{
    Affinity, Alignment, AlignmentOptions, CHROMIUM_LINE_BREAK_OVERRIDE, Cluster, Cursor,
    FontContext, FontFamily, FontFamilyName, FontFeature, FontFeatures, FontStyle,
    FontVariation as ParleyFontVariation, FontWeight, FontWidth, GenericFamily, InlineBox,
    InlineBoxKind, Layout, LayoutContext, Line, LineHeight, PositionedLayoutItem, Selection,
    StyleProperty, VerticalAlign as ParleyVerticalAlign,
};
use skrifa::instance::NormalizedCoord;
use std::{
    borrow::Cow,
    cmp::Reverse,
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

use paragraphs::{ParagraphLayout, ParleyDocumentLayout, local_range, paragraph_ranges};

/// Creates an empty, unshared font context without discovering system fonts.
fn empty_font_context() -> FontContext {
    FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: SourceCache::default(),
    }
}

/// Registers font blobs after validating the entire batch.
/// Returns an error without registering any blobs if one contains no supported font faces.
fn register_font_blobs(context: &mut FontContext, fonts: &[Blob<u8>]) -> Result<()> {
    let mut validator = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });

    for blob in fonts {
        if validator.register_fonts(blob.clone(), None).is_empty() {
            bail!("font data did not contain a supported font face");
        }
    }

    for blob in fonts {
        context.collection.register_fonts(blob.clone(), None);
    }

    Ok(())
}

/// Returns sorted, deduplicated names of the available font families.
fn family_names(context: &mut FontContext) -> Vec<String> {
    let mut names = context
        .collection
        .family_names()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    names.sort_unstable();
    names.dedup();

    names
}

/// Returns the first matching face for the ordered families and requested attributes.
/// Returns `None` when Fontique cannot resolve a face.
fn resolve_face(
    context: &mut FontContext,
    families: &[FontFamilyName<'_>],
    font: &Font,
) -> Option<QueryFont> {
    let style = match font.style {
        GpuiFontStyle::Normal => FontStyle::Normal,
        GpuiFontStyle::Italic => FontStyle::Italic,
        GpuiFontStyle::Oblique => FontStyle::Oblique(None),
    };

    let mut query = context.collection.query(&mut context.source_cache);

    query.set_families(families.iter().map(|family| match family {
        FontFamilyName::Named(name) => QueryFamily::Named(name.as_ref()),
        FontFamilyName::Generic(generic) => QueryFamily::Generic(*generic),
    }));

    query.set_attributes(Attributes::new(
        FontWidth::from_percentage(font.width.0),
        style,
        FontWeight::new(font.weight.0),
    ));

    query.first_available_font()
}

struct ParleyState {
    font_context: FontContext,
    layout_context: LayoutContext<usize>,
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

type ParagraphCache = BoundedCache<ParagraphCacheKey, Layout<usize>, 512>;

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
    source_to_backend: Vec<usize>,
    backend_to_source: Vec<usize>,
}

impl SourceMap {
    fn identity(len: usize) -> Self {
        Self {
            source_to_backend: (0..=len).collect(),
            backend_to_source: (0..=len).collect(),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Reserved for composing preparation stages.")
    )]
    fn then(&self, next: &Self) -> Self {
        Self {
            source_to_backend: self
                .source_to_backend
                .iter()
                .map(|idx| next.backend_index(*idx))
                .collect(),
            backend_to_source: next
                .backend_to_source
                .iter()
                .map(|idx| self.source_index(*idx))
                .collect(),
        }
    }

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

    fn remap_runs(&self, text: &str, source_runs: &[TextRun]) -> (Vec<TextRun>, Vec<usize>) {
        let last_source_byte = self.source_to_backend.len().saturating_sub(2);
        let mut source_end = 0;
        let mut backend_end = 0;

        source_runs
            .iter()
            .enumerate()
            .filter_map(|(run_idx, run)| {
                source_end += run.len;
                let start = backend_end;

                // Clamp trailing controls to the final source byte, as for leading controls.
                backend_end = if run_idx + 1 == source_runs.len() {
                    text.len()
                } else {
                    text.ceil_char_boundary(
                        self.backend_to_source[..text.len()]
                            .partition_point(|idx| (*idx).min(last_source_byte) < source_end),
                    )
                };

                (backend_end > start).then(|| {
                    (
                        TextRun {
                            len: backend_end - start,
                            ..run.clone()
                        },
                        run_idx,
                    )
                })
            })
            .unzip()
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
        pixel_point: Point<Pixels>,
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
                .caret_bounds(self.map.backend_caret(endpoint), line_height)
                .map(|bounds| (bounds.origin - pixel_point).magnitude())
        };

        match (distance(start), distance(end)) {
            (Some(start_distance), Some(end_distance)) if end_distance < start_distance => end,
            (Some(_), Some(_)) => start,
            _ => mapped,
        }
    }
}

impl PlatformTextLayout for SourceMappedLayout {
    fn native_lines(&self) -> Option<&[InlineVisualLine]> {
        self.inner.native_lines()
    }

    fn visual_line_index(&self, caret: CaretPosition) -> usize {
        self.inner.visual_line_index(self.map.backend_caret(caret))
    }

    fn len(&self) -> usize {
        self.source_len
    }

    fn line_count(&self) -> usize {
        self.inner.line_count()
    }

    fn size(&self) -> Size<Pixels> {
        self.inner.size()
    }

    fn byte_index_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.inner
            .byte_index_from_pixel_point(pixel_point, line_height)
            .map(|idx| self.map.source_index(idx))
            .map_err(|idx| self.map.source_index(idx))
    }

    fn caret_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.inner
            .caret_from_pixel_point(pixel_point, line_height)
            .map(|caret| self.source_caret_for_point(caret, pixel_point, line_height))
            .map_err(|caret| self.source_caret_for_point(caret, pixel_point, line_height))
    }

    fn caret_bounds(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        self.inner
            .caret_bounds(self.map.backend_caret(caret), line_height)
    }

    fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
        self.map
            .source_caret(self.inner.normalized_caret(self.map.backend_caret(caret)))
    }

    fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        let source_start = caret.index.min(self.source_len);
        let mut backend_caret = self.map.backend_caret(caret);

        for _attempt in 0..=self.inner.len() {
            backend_caret = self.inner.adjacent_visual_caret(backend_caret, direction)?;
            let source_caret = self.map.source_caret(backend_caret);

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
        self.inner
            .selection_bounds(self.map.backend_range(byte_range), line_height)
    }

    fn inline_geometry(&self, range: Range<usize>) -> Option<Vec<InlineRangeGeometry>> {
        if range.is_empty() {
            return None;
        }

        Some(
            self.inner
                .inline_geometry(self.map.backend_range(range))
                .unwrap_or_default(),
        )
    }

    fn inline_geometry_for_ranges(&self, ranges: &[Range<usize>]) -> Vec<Vec<InlineRangeGeometry>> {
        let ranges = ranges
            .iter()
            .cloned()
            .map(|range| {
                if range.is_empty() {
                    0..0
                } else {
                    self.map.backend_range(range)
                }
            })
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
        vertical_navigation_x: Option<Pixels>,
    ) -> CaretMovement {
        let direction = match (movement.direction, movement.boundary) {
            (Direction::Left, Boundary::Cluster) => Some(VisualDirection::Left),
            (Direction::Right, Boundary::Cluster) => Some(VisualDirection::Right),
            _ => None,
        };

        if let Some(direction) = direction {
            return CaretMovement {
                result: self
                    .adjacent_visual_caret(caret, direction)
                    .unwrap_or(caret),
                vertical_navigation_x: None,
            };
        }

        let CaretMovement {
            result: caret,
            vertical_navigation_x,
        } = self.inner.caret_movement(
            self.map.backend_caret(caret),
            movement,
            vertical_navigation_x,
        );

        CaretMovement {
            result: self.map.source_caret(caret),
            vertical_navigation_x,
        }
    }

    fn selection_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        self.map.source_range(
            self.inner
                .selection_from_pixel_point(pixel_point, line_height, kind),
        )
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
    fn remap(
        text: String,
        map: SourceMap,
        source_runs: &[TextRun],
        text_styles: &[InlineTextStyle],
        inline_boxes: &[InlineBoxRequest],
    ) -> Self {
        let (runs, run_sources) = map.remap_runs(&text, source_runs);

        Self {
            text,
            runs,
            run_sources,
            text_styles: text_styles
                .iter()
                .map(|style| InlineTextStyle {
                    range: map.backend_range(style.range.clone()),
                    ..style.clone()
                })
                .collect(),
            inline_boxes: inline_boxes
                .iter()
                .map(|inline_box| InlineBoxRequest {
                    index: map.backend_index(inline_box.index),
                    ..*inline_box
                })
                .collect(),
            map,
        }
    }
}

fn controls_for_scope(
    direction: ResolvedDirection,
    unicode_bidi: UnicodeBidi,
) -> Option<(&'static str, &'static str)> {
    let (ltr, rtl, close) = match unicode_bidi {
        UnicodeBidi::Normal => return None,
        UnicodeBidi::Embed => ("\u{202a}", "\u{202b}", "\u{202c}"),
        UnicodeBidi::Isolate => ("\u{2066}", "\u{2067}", "\u{2069}"),
        UnicodeBidi::BidiOverride => ("\u{202d}", "\u{202e}", "\u{202c}"),

        UnicodeBidi::IsolateOverride => {
            ("\u{2066}\u{202d}", "\u{2067}\u{202e}", "\u{202c}\u{2069}")
        }

        UnicodeBidi::Plaintext => ("\u{2068}", "\u{2068}", "\u{2069}"),
    };

    Some((if direction.is_rtl() { rtl } else { ltr }, close))
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

    let root_scope = root_direction.map(|direction| InlineBidiScope {
        range: 0..text.len(),
        direction,
        unicode_bidi,
    });

    // Opens nest outermost first. Nonempty closes precede opens, and empty closes follow.
    let mut insertions = bidi_scopes
        .iter()
        .filter(|scope| text.contains_range(&scope.range))
        .chain(root_scope.iter())
        .enumerate()
        .filter_map(|(scope_idx, scope)| {
            controls_for_scope(scope.direction, scope.unicode_bidi)
                .map(|controls| (scope_idx, &scope.range, controls))
        })
        .flat_map(|(scope_idx, range, (open, close))| {
            let closing_phase = if range.end == text.len() || range.is_empty() {
                2
            } else {
                0
            };

            let closing_order = if range.is_empty() && range.end < text.len() {
                scope_idx
            } else {
                usize::MAX - scope_idx
            };

            [
                (range.start, 1, Reverse(range.end), scope_idx, open),
                (
                    range.end,
                    closing_phase,
                    Reverse(range.start),
                    closing_order,
                    close,
                ),
            ]
        })
        .collect::<Vec<_>>();

    insertions.sort_by_key(|insertion| (insertion.0, insertion.1, insertion.2, insertion.3));

    let mut prepared = match root_direction {
        Some(ResolvedDirection::LeftToRight) => "\u{200e}",
        Some(ResolvedDirection::RightToLeft) => "\u{200f}",
        None => "",
    }
    .to_owned();

    let mut map = SourceMap::identity(0);
    map.source_to_backend[0] = prepared.len();
    map.backend_to_source.resize(prepared.len() + 1, 0);

    let mut source_start = 0;

    for (source_idx, phase, _boundary, _order, control) in insertions
        .into_iter()
        .chain(std::iter::once((text.len(), 2, Reverse(0), 0, "")))
    {
        let backend_start = prepared.len();
        prepared.push_str(&text[source_start..source_idx]);

        map.source_to_backend
            .extend(backend_start + 1..=prepared.len());

        map.backend_to_source.extend(source_start + 1..=source_idx);
        prepared.push_str(control);

        map.backend_to_source
            .extend(std::iter::repeat_n(source_idx, control.len()));

        if phase <= 1 {
            map.source_to_backend[source_idx] = prepared.len();
        }

        source_start = source_idx;
    }

    PreparedBidiText::remap(prepared, map, runs, text_styles, inline_boxes)
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
    fn apply_source_runs(&mut self, runs: &[TextRun], sources: &[usize]) {
        for fragment in &mut self.layout.paint_fragments {
            fragment.source_run = sources[fragment.source_run];
            fragment.style = PaintStyle::from(&runs[fragment.source_run]);
        }
    }
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
    native_geometry: bool,
    layout: Layout<usize>,
    inline_lines: Vec<InlineVisualLine>,
    // Cluster and caret caches use backend text indices, including synthetic bidi controls.
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
}

impl ParleyLayout {
    fn new(
        layout: Layout<usize>,
        text: Arc<str>,
        inline_lines: Vec<InlineVisualLine>,
        native_geometry: bool,
    ) -> Self {
        Self {
            native_geometry,
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

    fn caret_position(cursor: Cursor) -> CaretPosition {
        let gpui_affinity = cursor.affinity().into_affinity();

        CaretPosition {
            index: cursor.index(),
            affinity: gpui_affinity,
        }
    }

    fn cursor(&self, caret: CaretPosition) -> Cursor {
        let parley_affinity = caret.affinity.into_affinity();

        Cursor::from_byte_index(&self.layout, caret.index, parley_affinity)
    }

    fn cursor_position(layout: &Layout<usize>, cursor: Cursor) -> (f64, f64) {
        let geometry = cursor.geometry(layout, 0.0);

        (geometry.y0, geometry.x0)
    }

    fn visit_line_clusters(
        line: Line<'_, usize>,
        mut visit: impl FnMut(Cluster<'_, usize>, Range<f32>, Range<f32>),
    ) {
        let mut previous_run = None;

        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };

            let run = glyph_run.run();

            if previous_run == Some(run.index()) {
                continue;
            }

            previous_run = Some(run.index());
            let metrics = run.font_metrics();
            let mut inline = glyph_run.offset();
            let mut clusters = run.visual_clusters();
            let mut atom_has_glyphs = false;
            let mut at_atom_start = true;

            // GPUI requests baseline alignment for all text, so paint fragments share a baseline.
            let vertical =
                glyph_run.baseline() - metrics.ascent..glyph_run.baseline() + metrics.descent;

            while let Some(cluster) = clusters.next() {
                if (run.is_rtl() && at_atom_start)
                    || (!run.is_rtl() && !cluster.is_ligature_continuation())
                {
                    let first = if cluster.is_ligature_continuation() {
                        clusters
                            .clone()
                            .find(|cluster| !cluster.is_ligature_continuation())
                            .expect("Native runs contain complete ligatures")
                    } else {
                        cluster
                    };

                    atom_has_glyphs = first.glyphs().next().is_some();
                }

                at_atom_start = !cluster.is_ligature_continuation();

                if atom_has_glyphs {
                    let advance = inline..inline + cluster.advance();
                    inline = advance.end;
                    visit(cluster, advance, vertical.clone());
                }
            }
        }
    }

    fn collect_caret_stops(
        layout: &Layout<usize>,
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

    fn inline_cluster_geometry(&self) -> Vec<ParleyClusterGeometry> {
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
                    });
                }
            });
        }

        geometries
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

    fn logical_cluster(&self, caret: CaretPosition, before: bool) -> Option<Range<usize>> {
        if self.native_geometry {
            let cursor = self.cursor(caret);

            let cluster = if before {
                cursor.upstream_cluster(&self.layout)
            } else {
                cursor.downstream_cluster(&self.layout)
            };

            return Some(cluster?.text_range());
        }

        if before {
            self.graphemes()
                .iter()
                .rfind(|range| range.start < caret.index)
                .cloned()
        } else {
            self.graphemes()
                .iter()
                .find(|range| range.end > caret.index)
                .cloned()
        }
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
        let geometry = self.cursor(caret).geometry(&self.layout, 0.);

        // Parley's private Cursor::line helper uses the midpoint of this native rectangle.
        self.line_index_at_block(((geometry.y0 + geometry.y1) / 2.) as f32)
    }

    fn native_lines(&self) -> Option<&[InlineVisualLine]> {
        self.native_geometry.then_some(self.inline_lines.as_slice())
    }

    fn len(&self) -> usize {
        self.text.len()
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
            .map_err(|caret| caret.index)?
            .index;

        let (x, y) = self.native_point(pixel_point, line_height);

        Cluster::from_point(&self.layout, x, y)
            .map(|(cluster, _side)| cluster.text_range().start)
            .ok_or(closest)
    }

    fn caret_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.backend_caret_from_point(pixel_point, line_height)
    }

    fn caret_bounds(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>> {
        if caret.index > self.text.len() {
            return None;
        }

        if !self.native_geometry && self.text.is_empty() && self.layout.inline_boxes().len() == 0 {
            return Some(Bounds::new(
                point(self.inline_lines[0].origin.x, Pixels::ZERO),
                size(Pixels::ZERO, line_height),
            ));
        }

        let cursor = self.cursor(caret);
        let geometry = cursor.geometry(&self.layout, 0.0);

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

        let line_idx = self.line_index_at_block(((geometry.y0 + geometry.y1) / 2.) as f32);

        Some(Bounds::from_corners(
            point(px(geometry.x0 as f32), line_height * line_idx),
            point(px(geometry.x1 as f32), line_height * (line_idx + 1)),
        ))
    }

    fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
        Self::caret_position(self.cursor(caret))
    }

    fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        if self.text.is_empty() && self.layout.inline_boxes().len() == 0 {
            return None;
        }

        if !self.native_geometry {
            return self
                .adjacent_caret_stop(caret, direction)
                .map(|stop| stop.caret);
        }

        let cursor = self.cursor(caret);

        let moved = match direction {
            VisualDirection::Left => cursor.previous_visual(&self.layout),
            VisualDirection::Right => cursor.next_visual(&self.layout),
        };

        (moved != cursor).then(|| Self::caret_position(moved))
    }

    fn selection_bounds(
        &self,
        byte_range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
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

    fn inline_geometry(&self, range: std::ops::Range<usize>) -> Option<Vec<InlineRangeGeometry>> {
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
        let cursor = self.cursor(caret);

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
                        result: Self::caret_position(moved.focus()),
                        vertical_navigation_x,
                    };
                };

                let x = vertical_navigation_x.map_or(geometry.x0 as f32, f32::from);
                let moved = Cursor::from_point(&self.layout, x, self.native_y_for_line(target_idx));

                return CaretMovement {
                    result: Self::caret_position(moved),
                    vertical_navigation_x: Some(px(x)),
                };
            }
            _ => cursor,
        };

        CaretMovement {
            result: Self::caret_position(moved),
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

        selection
    }
}

impl ParleyState {
    fn new() -> Self {
        Self {
            font_context: empty_font_context(),
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
    system_fallback_family: Option<String>,
    fallback_families: Vec<String>,
}

impl ParleyTextSystem {
    fn font_properties<'a>(&'a self, font: &'a Font) -> Result<[StyleProperty<'a, usize>; 5]> {
        let families = self
            .font_families(font)
            .chain(
                self.fallback_families
                    .iter()
                    .map(|family| FontFamilyName::Named(Cow::Borrowed(family.as_str()))),
            )
            .collect::<Vec<_>>();

        let style = match font.style {
            GpuiFontStyle::Normal => FontStyle::Normal,
            GpuiFontStyle::Italic => FontStyle::Italic,
            GpuiFontStyle::Oblique => FontStyle::Oblique(None),
        };

        let features = font
            .features
            .tag_value_list()
            .iter()
            .map(|(tag, value)| {
                let tag = Tag::parse(tag)
                    .with_context(|| format!("invalid OpenType feature tag '{tag}'"))?;
                let value = (*value)
                    .try_into()
                    .context("OpenType feature value is larger than u16")?;

                Ok(FontFeature::new(tag, value))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok([
            StyleProperty::FontFamily(FontFamily::List(Cow::Owned(families))),
            StyleProperty::FontWeight(FontWeight::new(font.weight.0)),
            StyleProperty::FontWidth(FontWidth::from_percentage(font.width.0)),
            StyleProperty::FontStyle(style),
            StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(features))),
        ])
    }

    /// Creates a text system with an empty font catalog and the supplied glyph rasterizer.
    pub fn new(rasterizer: impl GlyphRasterizer + 'static) -> Self {
        let parley = ParleyState::new();

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
            system_fallback_family: None,
            fallback_families: Vec::new(),
        }
    }

    /// Loads operating-system fonts and returns the text system.
    /// Fonts registered with `.add_fonts(...)` and their IDs are preserved across repeated calls.
    #[cfg(feature = "system-fonts")]
    pub fn with_system_fonts(mut self) -> Self {
        self.parley
            .get_mut()
            .font_context
            .collection
            .load_system_fonts();

        self.fonts_changed();

        self
    }

    /// Enables explicit optical sizing from each shaped run's logical font size.
    pub fn with_automatic_optical_sizing(mut self) -> Self {
        self.automatic_optical_sizing = true;

        self
    }

    /// Sets the fallback family for `.SystemUIFont` when the system UI family is unavailable.
    pub fn with_system_fallback_family(mut self, family: impl Into<String>) -> Self {
        self.system_fallback_family = Some(family.into());

        self
    }

    /// Sets ordered fallback families after those supplied by each text style.
    pub fn with_fallback_families(
        mut self,
        families: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.fallback_families = families.into_iter().map(Into::into).collect();

        self
    }

    fn fonts_changed(&self) {
        let _previous_generation = self.font_generation.fetch_add(1, Ordering::Release);
        self.paragraph_cache.lock().clear();
        self.paragraph_result_cache.lock().clear();
    }

    fn face_families<'a>(&'a self, name: &'a str) -> impl Iterator<Item = FontFamilyName<'a>> {
        let is_system_font = name == ".SystemUIFont";

        let family = if is_system_font {
            self.system_fallback_family.as_deref()
        } else {
            Some(name)
        };

        is_system_font
            .then_some(FontFamilyName::Generic(GenericFamily::SystemUi))
            .into_iter()
            .chain(family.map(|family| FontFamilyName::Named(Cow::Borrowed(family))))
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
                self.fallback_families
                    .iter()
                    .flat_map(|name| self.face_families(name)),
            )
            .collect::<Vec<_>>();

        let resolved = {
            let mut state = self.parley.lock();

            resolve_face(&mut state.font_context, &families, descriptor)
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

        // Parley 0.11 resolves one base direction per layout, including across newlines.
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
            let paragraph_bidi_scopes = bidi_scopes
                .iter()
                .filter_map(|scope| {
                    local_range(&scope.range, &source.content).map(|range| InlineBidiScope {
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
        let base_font = base_font.or_else(|| runs.first().map(|run| &run.font));
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
            paragraph: cache_key.clone(),
            source_map: prepared.map.clone(),
            wrap,
            text_align,
            alignment_width,
        };

        if let Some(mut result) = self.paragraph_result_cache.lock().get(&result_cache_key) {
            result.apply_source_runs(source_runs, &prepared.run_sources);

            return Ok(result);
        }

        let mut layout = if let Some(layout) = self.paragraph_cache.lock().get(&cache_key) {
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

            let mut state = self.parley.lock();
            let ParleyState {
                font_context,
                layout_context,
            } = &mut *state;

            let mut builder = layout_context.ranged_builder(font_context, text, 1.0, false);
            builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
            builder.push_default(StyleProperty::FontSize(f32::from(font_size)));

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

                builder.push(StyleProperty::Brush(run_index), range.clone());

                if let Some(letter_spacing) = run.letter_spacing {
                    builder.push(
                        StyleProperty::LetterSpacing(f32::from(letter_spacing)),
                        range,
                    );
                }
            }

            let mut push_style = |property: StyleProperty<'_, usize>, range| {
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
                        GpuiVerticalAlign::Baseline => ParleyVerticalAlign::BASELINE,
                        GpuiVerticalAlign::Middle => ParleyVerticalAlign::MIDDLE,
                        GpuiVerticalAlign::Top => ParleyVerticalAlign::TOP,
                        GpuiVerticalAlign::Bottom => ParleyVerticalAlign::BOTTOM,
                    },
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
                let run_metrics = glyph_run.run().font_metrics();

                // Bidi preparation can add direction markers to an empty source string.
                if source_text_len == 0 {
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
                            let id = GlyphId(glyph.id);
                            ShapedGlyph {
                                id,
                                position: point(
                                    px(glyph.x) - line_x,
                                    px(glyph.y - metrics.baseline),
                                ),
                                is_emoji: color_glyphs
                                    .available_kinds(id)
                                    .any(|kind| self.color_glyph_formats.contains(&kind)),
                            }
                        })
                        .collect()
                };

                let start = px(glyph_run.offset()) - line_x;
                let source_run = glyph_run.style().brush;
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
                    style: PaintStyle::default(),
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
        );
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

        let mut result = ParleyLayoutResult {
            layout: line_layout,
            inline_lines,
            inline_boxes: positioned_inline_boxes,
            size,
        };

        // Keep prepared run indices in the cache, independent of authored empty runs.
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
            .map(|bytes| Blob::from(bytes.into_owned()))
            .collect::<Vec<_>>();

        {
            let mut state = self.parley.lock();
            register_font_blobs(&mut state.font_context, &blobs)?;

            self.fonts_changed();
        }

        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut names = {
            let mut state = self.parley.lock();

            family_names(&mut state.font_context)
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
        FontStyle as GpuiFontStyle, FontSynthesis, FontWeight as GpuiFontWeight,
        FontWidth as GpuiFontWidth, GlyphRenderMode, HeadlessAppContext, HighlightStyle, Hsla,
        IntoElement, Point, RasterFace, RasterizedGlyphFormat, Render, StrikethroughStyle, Styled,
        StyledText, TestTextSystem, TextSystem, UnderlineStyle, VerticalAlign, Window,
        WindowHandle, WindowTextSystem, div, font, hsla, prelude::*,
    };
    use gpui_fonts::{
        FontFixture, IBM_PLEX, IBM_PLEX_SEMIBOLD, IBM_PLEX_SEMIBOLD_ITALIC, LILEX, NOTO_ARABIC,
        NOTO_COLOR_EMOJI, NOTO_HEBREW, NOTO_SANS, SOURCE_SERIF, WIDTH_CONDENSED, WIDTH_REGULAR,
    };
    use parley::RangedBuilder;
    use skrifa::{
        FontRef, MetadataProvider,
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

    fn register_bytes(context: &mut FontContext, fonts: &[&[u8]]) -> Result<()> {
        let blobs = fonts
            .iter()
            .map(|bytes| Blob::from(bytes.to_vec()))
            .collect::<Vec<_>>();

        register_font_blobs(context, &blobs)
    }

    #[test]
    fn registered_fonts_are_enumerated_and_resolved() {
        let mut context = empty_font_context();

        register_bytes(
            &mut context,
            &[
                IBM_PLEX.data,
                IBM_PLEX_SEMIBOLD_ITALIC.data,
                LILEX.data,
                WIDTH_REGULAR.data,
                WIDTH_CONDENSED.data,
            ],
        )
        .unwrap();

        assert_eq!(
            family_names(&mut context),
            [WIDTH_REGULAR.family, IBM_PLEX.family, LILEX.family]
        );

        for (fixture, descriptor) in [
            (IBM_PLEX, font(IBM_PLEX.family)),
            (
                IBM_PLEX_SEMIBOLD_ITALIC,
                Font {
                    weight: GpuiFontWeight::SEMIBOLD,
                    ..font(IBM_PLEX.family).italic()
                },
            ),
            (WIDTH_REGULAR, font(WIDTH_REGULAR.family)),
            (
                WIDTH_CONDENSED,
                Font {
                    width: GpuiFontWidth::CONDENSED,
                    ..font(WIDTH_REGULAR.family)
                },
            ),
        ] {
            let families = [FontFamilyName::Named(fixture.family.into())];
            let resolved = resolve_face(&mut context, &families, &descriptor).unwrap();

            assert_eq!(resolved.blob.as_ref(), fixture.data, "{descriptor:?}");
            assert_eq!(resolved.index, 0, "{descriptor:?}");
        }
    }

    #[test]
    fn font_registration_is_atomic() {
        let mut context = empty_font_context();
        register_bytes(&mut context, &[LILEX.data]).unwrap();
        let families_before = family_names(&mut context);

        assert!(register_bytes(&mut context, &[IBM_PLEX.data, b"not a font"]).is_err());
        assert_eq!(family_names(&mut context), families_before);

        let families = [FontFamilyName::Named(IBM_PLEX.family.into())];

        assert!(resolve_face(&mut context, &families, &font(IBM_PLEX.family)).is_none());
    }

    #[test]
    fn caret_affinity_round_trips_through_parley() {
        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
            let parley_affinity: Affinity = affinity.into_affinity();
            let gpui_affinity: CaretAffinity = parley_affinity.into_affinity();
            assert_eq!(gpui_affinity, affinity);
        }
    }

    fn test_system() -> Arc<ParleyTextSystem> {
        let system = test_system_with_fonts(&[
            IBM_PLEX,
            IBM_PLEX_SEMIBOLD,
            LILEX,
            SOURCE_SERIF,
            NOTO_SANS,
            NOTO_COLOR_EMOJI,
            NOTO_ARABIC,
            NOTO_HEBREW,
        ])
        .with_fallback_families([
            IBM_PLEX.family,
            LILEX.family,
            SOURCE_SERIF.family,
            NOTO_COLOR_EMOJI.family,
            NOTO_ARABIC.family,
            NOTO_HEBREW.family,
        ]);

        Arc::new(system)
    }

    fn test_system_with_fonts(fonts: &[FontFixture]) -> ParleyTextSystem {
        let system = ParleyTextSystem::new(TestTextSystem)
            .with_fallback_families(fonts.first().map(|fixture| fixture.family));

        system
            .add_fonts(
                fonts
                    .iter()
                    .map(|fixture| Cow::Borrowed(fixture.data))
                    .collect(),
            )
            .unwrap();

        system
    }

    fn set_generic_family(system: &ParleyTextSystem, generic: GenericFamily, family: &str) {
        let mut state = system.parley.lock();
        let collection = &mut state.font_context.collection;
        let family = collection.family_id(family).unwrap();
        collection.set_generic_families(generic, std::iter::once(family));
    }

    #[test]
    fn font_catalog_starts_empty_and_resolves_independent_fallback_families() {
        let system = ParleyTextSystem::new(TestTextSystem);
        assert_eq!(system.all_font_names(), [".SystemUIFont"]);
        assert!(system.font_id(&font(IBM_PLEX.family)).is_err());
        assert!(system.font_id(&font(".SystemUIFont")).is_err());

        assert_eq!(
            system.face_families(".SystemUIFont").collect::<Vec<_>>(),
            [FontFamilyName::Generic(GenericFamily::SystemUi)]
        );

        system
            .add_fonts(vec![Cow::Borrowed(IBM_PLEX.data)])
            .unwrap();

        let registered = system.font_id(&font(IBM_PLEX.family)).unwrap();
        assert!(system.font_id(&font(".SystemUIFont")).is_err());
        let system = system.with_system_fallback_family(IBM_PLEX.family);
        assert_eq!(system.font_id(&font(".SystemUIFont")).unwrap(), registered);
        assert!(system.font_id(&font("Missing family")).is_err());
        set_generic_family(&system, GenericFamily::SystemUi, IBM_PLEX.family);
        assert_eq!(system.font_id(&font(".SystemUIFont")).unwrap(), registered);

        for families in [
            [IBM_PLEX.family, LILEX.family],
            [LILEX.family, IBM_PLEX.family],
        ] {
            let system = test_system_with_fonts(&[IBM_PLEX, LILEX])
                .with_system_fallback_family(families[1])
                .with_fallback_families(families);

            let primary = system.font_id(&font(families[0])).unwrap();
            let secondary = system.font_id(&font(families[1])).unwrap();
            assert_ne!(primary, secondary);
            let mut authored = font("Missing family");
            authored.fallbacks = Some(FontFallbacks::from_fonts(vec![families[1].to_owned()]));

            for (generic, descriptor, expected) in [
                (None, font(".SystemUIFont"), secondary),
                (None, font("Missing family"), primary),
                (None, font(families[0]), primary),
                (None, authored, secondary),
                (Some(families[0]), font(".SystemUIFont"), primary),
            ] {
                if let Some(family) = generic {
                    set_generic_family(&system, GenericFamily::SystemUi, family);
                }

                assert_eq!(
                    system.font_id(&descriptor).unwrap(),
                    expected,
                    "{descriptor:?}"
                );

                let text = if generic.is_some() {
                    "system UI"
                } else {
                    "abc"
                };

                let run = TextRun {
                    len: text.len(),
                    font: descriptor,
                    ..Default::default()
                };

                let layout = layout_line(&system, text, px(18.), &[run]);
                assert_eq!(layout.paint_fragments[0].font_id, expected, "{families:?}");
            }
        }
    }

    #[cfg(feature = "system-fonts")]
    #[test]
    fn loading_system_fonts_preserves_registered_faces_and_invalidates_layouts() {
        let mut system = test_system_with_fonts(&[IBM_PLEX]);
        let font_id = system.font_id(&font(IBM_PLEX.family)).unwrap();
        let text = "registered before system fonts";
        let runs = [text_run(text, IBM_PLEX.family)];

        for _attempt in 0..2 {
            let before = layout_line(&system, text, px(18.), &runs);
            let generation = system.font_generation();
            assert!(!system.paragraph_cache.lock().entries.is_empty());
            assert!(!system.paragraph_result_cache.lock().entries.is_empty());

            system = system.with_system_fonts();

            assert_eq!(system.font_generation(), generation + 1);
            assert!(system.paragraph_cache.lock().entries.is_empty());
            assert!(system.paragraph_result_cache.lock().entries.is_empty());
            assert_eq!(system.font_id(&font(IBM_PLEX.family)).unwrap(), font_id);
            assert!(
                system
                    .all_font_names()
                    .iter()
                    .any(|name| name == IBM_PLEX.family)
            );

            let after = layout_line(&system, text, px(18.), &runs);
            assert_eq!(after.paint_fragments[0].font_id, font_id);
            assert_eq!(before.width, after.width);
            assert_eq!(before.ascent, after.ascent);
            assert_eq!(before.descent, after.descent);

            let fonts = system.font_instances.read();
            assert_eq!(fonts.get(font_id).unwrap().data.as_ref(), IBM_PLEX.data);
        }
    }

    fn text_run(text: &str, family: &str) -> TextRun {
        TextRun {
            len: text.len(),
            font: font(family),
            ..Default::default()
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

        // Give synthetic bidi markers a default font without relying on installed fonts.
        set_generic_family(&system, GenericFamily::SansSerif, IBM_PLEX.family);

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

        for (name, text, direction, align, right_aligned, endpoint_hits) in [
            (
                "RTL Latin",
                "English 123!",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                false,
            ),
            (
                "RTL neutral",
                "123 …",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                false,
            ),
            (
                "RTL empty",
                "",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                false,
            ),
            (
                "opposing RTL",
                "English 123",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                true,
            ),
            (
                "opposing LTR",
                "مرحبا",
                ParagraphDirection::LeftToRight,
                TextAlign::Start,
                false,
                true,
            ),
            (
                "matching LTR",
                "English 123",
                ParagraphDirection::LeftToRight,
                TextAlign::Start,
                false,
                true,
            ),
            (
                "matching RTL",
                "مرحبا",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                true,
            ),
            (
                "RTL start",
                "English",
                ParagraphDirection::RightToLeft,
                TextAlign::Start,
                true,
                false,
            ),
            (
                "RTL end",
                "English",
                ParagraphDirection::RightToLeft,
                TextAlign::End,
                false,
                false,
            ),
            (
                "LTR start",
                "English",
                ParagraphDirection::LeftToRight,
                TextAlign::Start,
                false,
                false,
            ),
        ] {
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
    fn source_maps_compose_multibyte_removal_and_inserted_controls() {
        let source = "é  中";
        let intermediate = "é 中";

        let first = SourceMap {
            source_to_backend: vec![0, 1, 2, 3, 3, 4, 5, 6],
            backend_to_source: vec![0, 1, 2, 4, 5, 6, 7],
        };

        let next = prepare_bidi_text(
            intermediate,
            &[text_run(intermediate, IBM_PLEX.family)],
            &[],
            &[],
            ParagraphDirection::LeftToRight,
            UnicodeBidi::Isolate,
            &[],
        );

        let composed = first.then(&next.map);

        assert_eq!(composed.source_to_backend, [6, 7, 8, 9, 9, 10, 11, 12]);

        assert_eq!(
            composed.backend_to_source,
            [0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 5, 6, 7, 7, 7, 7]
        );

        for map in [SourceMap::identity(0), composed.clone()] {
            let source_identity = SourceMap::identity(map.source_to_backend.len() - 1);
            let backend_identity = SourceMap::identity(map.backend_to_source.len() - 1);

            assert_eq!(source_identity.then(&map), map);
            assert_eq!(map.then(&backend_identity), map);
        }

        assert_eq!(composed.backend_index(usize::MAX), 12);
        assert_eq!(composed.source_index(usize::MAX), source.len());

        let runs = ["é", " ", " ", "中"].map(|text| text_run(text, IBM_PLEX.family));

        let mut styles = [InlineTextStyle {
            range: 4..source.len(),
            font_size: px(20.),
            line_height: px(28.),
        }];

        let mut boxes = [InlineBoxRequest {
            id: 42,
            index: 4,
            size: size(px(12.), px(16.)),
            vertical_align: VerticalAlign::Baseline,
        }];

        let prepared = PreparedBidiText::remap(next.text, composed, &runs, &styles, &boxes);

        assert_eq!(prepared.run_sources, [0, 1, 3]);

        assert_eq!(
            prepared.runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            [8, 1, 6]
        );

        styles[0].range = 9..12;
        boxes[0].index = 9;
        assert_eq!(prepared.text_styles, styles);
        assert_eq!(prepared.inline_boxes, boxes);
    }

    #[test]
    fn bidi_scope_overrides_reverse_visual_order_and_preserve_source_indices() {
        let system = test_system();

        for (text, range, alignment_width) in
            [("A אב B", 2..6, Some(px(240.0))), ("abc", 0..3, None)]
        {
            let runs = [text_run(text, IBM_PLEX.family)];

            let scopes = [InlineBidiScope {
                range: range.clone(),
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
                base_font: &font(IBM_PLEX.family),
                options: TextLayoutOptions {
                    alignment_width,
                    direction: ParagraphDirection::LeftToRight,
                    ..Default::default()
                },
                bidi_scopes: &scopes,
            });

            let native = &layout.layout.platform_layout;
            assert_eq!(layout.layout.len, text.len(), "{text:?}");
            assert_eq!(native.len(), text.len(), "{text:?}");

            assert_eq!(
                native.logical_cluster_after(CaretPosition::attached_to_next_cluster(0)),
                Some(0..1),
                "{text:?}"
            );

            for (idx, _character) in text
                .char_indices()
                .chain(std::iter::once((text.len(), '\0')))
            {
                let caret = native.normalized_caret(CaretPosition::attached_to_next_cluster(idx));
                assert!(text.is_char_boundary(caret.index), "{text:?}: {caret:?}");
            }

            assert!(
                !native.selection_bounds(0..text.len(), px(24.0)).is_empty(),
                "{text:?}"
            );

            let positions = text[range.clone()]
                .char_indices()
                .map(|(idx, character)| {
                    let start = range.start + idx;

                    native
                        .inline_geometry(start..start + character.len_utf8())
                        .unwrap()[0]
                        .bounds
                        .origin
                        .x
                })
                .collect::<Vec<_>>();

            assert!(
                positions.windows(2).all(|pair| pair[0] > pair[1]),
                "{text:?}: {positions:?}"
            );
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

    fn native_layout(
        system: &ParleyTextSystem,
        text: &str,
        family: &str,
        wrap_width: Option<f32>,
        configure: impl FnOnce(&mut RangedBuilder<'_, usize>),
    ) -> ParleyLayout {
        let mut state = system.parley.lock();

        let ParleyState {
            font_context,
            layout_context,
        } = &mut *state;

        let mut builder = layout_context.ranged_builder(font_context, text, 1., false);

        for property in system.font_properties(&font(family)).unwrap() {
            builder.push_default(property);
        }

        builder.push_default(StyleProperty::FontSize(18.));
        builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(24.)));
        configure(&mut builder);
        let mut layout = builder.build(text);
        layout.break_all_lines(wrap_width);
        layout.align(Alignment::Left, AlignmentOptions::default());

        let inline_lines = layout
            .lines()
            .map(|line| InlineVisualLine {
                origin: point(Pixels::ZERO, px(line.metrics().block_min_coord)),
                size: size(px(line.metrics().advance), px(line.metrics().line_height)),
                baseline: px(line.metrics().baseline - line.metrics().block_min_coord),
            })
            .collect();

        ParleyLayout::new(layout, text.into(), inline_lines, true)
    }

    #[test]
    fn line_lookup_preserves_native_boundaries_and_empty_rows() {
        let system = test_system();
        let text = "one two three four five six seven eight ".repeat(8);

        for line_height in [0., 12.] {
            let native = native_layout(&system, &text, IBM_PLEX.family, Some(60.), |builder| {
                builder.push_default(StyleProperty::FontSize(16.));
                builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(line_height)));

                if line_height > 0. {
                    builder.push(StyleProperty::LineHeight(LineHeight::Absolute(32.)), 0..24);
                }
            });

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

        let missing =
            ParleyTextSystem::new(TestTextSystem).with_fallback_families([IBM_PLEX.family]);

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

            let expected_layouts = fonts
                .iter()
                .map(|base_font| {
                    system.paragraph_cache.lock().clear();
                    system.paragraph_result_cache.lock().clear();

                    system.layout_inline(InlineLayoutRequest {
                        base_font,
                        ..request
                    })
                })
                .collect::<Vec<_>>();
            system.paragraph_cache.lock().clear();
            system.paragraph_result_cache.lock().clear();

            for idx in [0, 1, 2, 0, 1, 2] {
                let base_font = &fonts[idx];
                let metrics = system.font_metrics(system.font_id(base_font).unwrap());
                let x_height = metrics.x_height(request.font_size);
                assert!(x_height > Pixels::ZERO && x_height <= metrics.ascent(request.font_size));

                let layout = system.layout_inline(InlineLayoutRequest {
                    base_font,
                    ..request
                });
                assert_eq!(layout.boxes, expected_layouts[idx].boxes);
                assert_eq!(layout.lines, expected_layouts[idx].lines);
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

    #[track_caller]
    fn assert_metric_close(actual: Pixels, expected: Pixels) {
        assert!(
            (actual - expected).abs() < px(0.001),
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn inline_content_extents_follow_fonts_independently_of_line_height() {
        let system = test_system();
        let parent = font(IBM_PLEX.family);

        for (pieces, font_size, line_height, style_range, style_height) in [
            (&["office ", "cafe\u{301} ", "x"][..], 18., 24., 14..15, 40.),
            (&["x"][..], 18., 12., 0..0, 12.),
        ] {
            let text = pieces.concat();

            let runs = pieces
                .iter()
                .map(|piece| text_run(piece, IBM_PLEX.family))
                .collect::<Vec<_>>();

            let ranges = run_ranges(&runs);

            let short_row = style_range.is_empty();

            let styles = [InlineTextStyle {
                range: style_range,
                font_size: px(30.),
                line_height: px(style_height),
            }];

            let styles = if short_row { &[][..] } else { &styles };

            let layout = system.layout_inline(InlineLayoutRequest {
                font_size: px(font_size),
                line_height: px(line_height),
                text_styles: styles,
                ..inline_request(&text, &runs, &parent)
            });

            for (visual, line) in layout.layout.visual_lines.iter().zip(&layout.lines) {
                for fragment in &layout.layout.paint_fragments[visual.paint_fragment_range.clone()]
                {
                    let metrics = system.font_metrics(fragment.font_id);
                    let content = fragment.background_range.as_ref().unwrap();
                    let expected_size =
                        [px(font_size), px(font_size), px(30.)][fragment.source_run];

                    assert_eq!(fragment.font_size, expected_size, "{text:?}");

                    assert_metric_close(
                        content.start,
                        fragment.baseline_offset - metrics.ascent(fragment.font_size),
                    );

                    assert_metric_close(
                        content.end,
                        fragment.baseline_offset + metrics.descent(fragment.font_size),
                    );

                    assert_eq!(fragment.glyphs[0].position.y, fragment.baseline_offset);

                    let geometry = layout
                        .layout
                        .platform_layout
                        .inline_geometry(ranges[fragment.source_run].clone())
                        .unwrap();

                    let bounds = geometry[0].bounds;

                    assert_metric_close(
                        bounds.origin.y,
                        line.origin.y + line.baseline + content.start,
                    );

                    assert_metric_close(bounds.size.height, content.end - content.start);
                }
            }

            let expected_height = px(if short_row { line_height } else { style_height });
            assert_eq!(layout.lines[0].size.height, expected_height);
            assert_eq!(layout.size.height, expected_height);

            if short_row {
                let metrics = system.font_metrics(system.font_id(&parent).unwrap());
                assert_metric_close(layout.layout.ascent, metrics.ascent(px(font_size)));
                assert_metric_close(layout.layout.descent, metrics.descent(px(font_size)));
                assert!(layout.layout.ascent + layout.layout.descent > layout.size.height);
            }
        }
    }

    #[test]
    fn empty_inline_paragraphs_use_parent_struts_for_carets() {
        let system = test_system();

        let cases = [IBM_PLEX.family, SOURCE_SERIF.family, LILEX.family]
            .into_iter()
            .flat_map(|family| ["", "\n", "\r\n\n"].map(|text| (family, text)))
            .flat_map(|case| {
                [ParagraphDirection::Auto, ParagraphDirection::RightToLeft]
                    .map(move |direction| (case, direction))
            });

        for ((family, text), direction) in cases {
            let parent = font(family);

            let inline = system.layout_inline(InlineLayoutRequest {
                options: TextLayoutOptions {
                    direction,
                    alignment_width: Some(px(180.)),
                    text_align: TextAlign::Center,
                    ..Default::default()
                },
                ..inline_request(text, &[], &parent)
            });

            let platform = &inline.layout.platform_layout;
            assert_eq!(platform.native_lines().unwrap(), inline.lines);
            assert!(inline.layout.paint_fragments.is_empty());
            assert!(inline.boxes.is_empty());
            assert_eq!(inline.size.height, px(24.) * inline.lines.len());

            for (line_idx, line) in inline.lines.iter().enumerate() {
                let caret = CaretPosition::attached_to_next_cluster(
                    inline.layout.visual_lines[line_idx].text_range.start,
                );

                let bounds = platform.caret_bounds(caret, px(1.)).unwrap();
                assert_eq!(platform.visual_line_index(caret), line_idx);
                assert!(bounds.bottom() > line.origin.y);
                assert!(bounds.origin.y < line.origin.y + line.size.height);
                assert_eq!(bounds.origin.x, line.origin.x);
                assert_eq!(bounds, platform.caret_bounds(caret, px(80.)).unwrap());
                assert!(line.baseline > Pixels::ZERO);
            }
        }
    }

    #[test]
    fn inline_interaction_uses_native_rows_and_wrap_affinities() {
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

        let native = platform.inline_geometry(16..19).unwrap();
        assert_eq!(native.len(), 1);
        assert_eq!(native[0].visual_line_index, 2);
        assert!(native[0].bounds.origin.y >= px(65.99), "{native:?}");

        let selection = platform.selection_bounds(16..19, px(20.));

        assert_eq!(
            selection[0].origin.y, layout.lines[2].origin.y,
            "selection follows the native line position"
        );

        let wrapped_text = "one two three four five six seven eight";

        let wrapped_layouts = [IBM_PLEX.family, SOURCE_SERIF.family, LILEX.family].map(|family| {
            let runs = [text_run(wrapped_text, family)];

            system.layout_inline(InlineLayoutRequest {
                line_height: px(32.),
                options: TextLayoutOptions {
                    wrap_width: Some(px(75.)),
                    unicode_bidi: UnicodeBidi::Isolate,
                    text_align: TextAlign::Left,
                    ..Default::default()
                },
                ..inline_request(wrapped_text, &runs, &font(family))
            })
        });

        let cases = std::iter::once((text, &layout))
            .chain(wrapped_layouts.iter().map(|inline| (wrapped_text, inline)));

        for (text, inline) in cases {
            let platform = &inline.layout.platform_layout;
            assert!(inline.lines.len() > 1);
            assert_eq!(platform.native_lines().unwrap(), inline.lines);

            for (line_idx, visual) in inline.layout.visual_lines.iter().enumerate() {
                let caret = CaretPosition::attached_to_next_cluster(visual.text_range.start);
                let bounds = platform.caret_bounds(caret, px(1.)).unwrap();
                let line = inline.lines[line_idx];

                let position = point(
                    bounds.origin.x + px(0.1),
                    line.origin.y + line.size.height / 2.,
                );

                assert_eq!(platform.visual_line_index(caret), line_idx, "{text:?}");
                assert_eq!(bounds.origin.y, line.origin.y, "{text:?}");
                assert_eq!(bounds.size.height, line.size.height, "{text:?}");
                let hit = platform.caret_from_pixel_point(position, px(1.)).unwrap();

                let byte_idx = platform
                    .byte_index_from_pixel_point(position, px(1.))
                    .unwrap();
                assert_eq!(hit.index, caret.index, "{text:?}");
                assert_eq!(byte_idx, caret.index, "{text:?}");

                if line_idx == 0 || text[..caret.index].ends_with(is_paragraph_separator) {
                    continue;
                }

                let upstream = CaretPosition::attached_to_previous_cluster(caret.index);
                let bounds = platform.caret_bounds(upstream, px(1.)).unwrap();
                let previous = inline.lines[line_idx - 1];
                assert_eq!(
                    platform.visual_line_index(upstream),
                    line_idx - 1,
                    "{text:?}"
                );
                assert_eq!(bounds.origin.y, previous.origin.y, "{text:?}");
                assert_eq!(bounds.size.height, previous.size.height, "{text:?}");
            }
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
                color: first_color,
                ..text_run(&text[..split], IBM_PLEX.family)
            },
            TextRun {
                color: second_color,
                ..text_run(&text[split..], SOURCE_SERIF.family)
            },
        ];

        let boxes = [
            (7, "alpha ".len(), 28., 32., VerticalAlign::Baseline),
            (9, split, 18., 14., VerticalAlign::Middle),
            (11, "alpha beta gamma ".len(), 20., 18., VerticalAlign::Top),
            (
                13,
                "alpha beta gamma ".len(),
                16.,
                28.,
                VerticalAlign::Bottom,
            ),
        ]
        .map(
            |(id, index, width, height, vertical_align)| InlineBoxRequest {
                id,
                index,
                size: size(px(width), px(height)),
                vertical_align,
            },
        );

        let base_font = font(IBM_PLEX.family);
        let metrics = system.font_metrics(system.font_id(&base_font).unwrap());

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

        for (request, inline_box) in boxes.iter().zip(&layout.boxes) {
            let bounds = positioned_box_bounds(&layout, inline_box);
            let line = layout.lines[inline_box.line_index];

            let expected_top = line.origin.y
                + match request.vertical_align {
                    VerticalAlign::Baseline => line.baseline - bounds.size.height,
                    VerticalAlign::Middle => {
                        line.baseline - metrics.x_height(px(18.)) / 2. - bounds.size.height / 2.
                    }
                    VerticalAlign::Top => Pixels::ZERO,
                    VerticalAlign::Bottom => line.size.height - bounds.size.height,
                };

            assert!(
                (bounds.origin.y - expected_top).abs() < px(0.01),
                "box {}: {bounds:?}, expected top {expected_top:?}",
                request.id
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
                let document = layout_wrapped(
                    &system,
                    &text,
                    px(18.),
                    &[text_run(&text, "IBM Plex Sans")],
                    width,
                    None,
                );

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
        let layout = layout_wrapped(
            system,
            text,
            px(18.),
            &[text_run(text, "IBM Plex Sans")],
            px(100.),
            None,
        );
        let native = &layout.platform_layout;
        assert!(native.native_lines().is_none());
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

        for source in paragraph_ranges(text) {
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

        for line_index in 1..native.line_count() {
            let moved = native.caret_movement(
                vertical_caret,
                Direction::Down.with_boundary(Boundary::VisualLine),
                vertical_navigation_x,
            );
            vertical_caret = moved.result;
            vertical_navigation_x = moved.vertical_navigation_x;

            assert_eq!(geometry(vertical_caret).origin.y, line_height * line_index);
            assert_eq!(vertical_navigation_x, Some(geometry(start).origin.x));
        }

        for line_index in (0..native.line_count() - 1).rev() {
            let moved = native.caret_movement(
                vertical_caret,
                Direction::Up.with_boundary(Boundary::VisualLine),
                vertical_navigation_x,
            );
            vertical_caret = moved.result;
            vertical_navigation_x = moved.vertical_navigation_x;

            assert_eq!(geometry(vertical_caret).origin.y, line_height * line_index);
        }

        let document_start = native.caret_movement(
            CaretPosition::attached_to_previous_cluster(text.len()),
            Direction::Start.with_boundary(Boundary::Document),
            None,
        );
        assert_eq!(
            document_start.result,
            CaretPosition::attached_to_next_cluster(0)
        );

        let document_end = native.caret_movement(
            CaretPosition::attached_to_next_cluster(0),
            Direction::End.with_boundary(Boundary::Document),
            None,
        );
        assert_eq!(
            document_end.result,
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
                .caret_from_pixel_point(point(edge_x, line_height * 1.5), line_height)
                .unwrap_or_else(|caret| caret);
            let word = native.caret_movement(empty_row, movement, None).result;
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

        for (line_index, source) in paragraph_ranges(mixed).into_iter().take(2).enumerate() {
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

    fn paint_geometry(layout: &LineLayout, runs: &[TextRun]) -> Vec<(Range<usize>, PaintFragment)> {
        let ranges = run_ranges(runs);
        let mut painted = vec![false; runs.len()];

        let geometry = layout
            .paint_fragments
            .iter()
            .map(|fragment| {
                let run_idx = fragment.source_run;

                assert_eq!(
                    fragment.style,
                    PaintStyle::from(&runs[run_idx]),
                    "paint for run {run_idx}"
                );

                painted[run_idx] = true;

                (
                    ranges[run_idx].clone(),
                    PaintFragment {
                        source_run: 0,
                        style: PaintStyle::default(),
                        ..fragment.clone()
                    },
                )
            })
            .collect();

        for (run_idx, run) in runs.iter().enumerate() {
            assert_eq!(painted[run_idx], run.len > 0, "authored run {run_idx}");
        }

        geometry
    }

    #[test]
    fn paint_changes_preserve_geometry_cache_identity_and_authored_run_boundaries() {
        let system = test_system();
        let text = "office café العربية";

        let mut base_runs =
            ["office ", "café ", "العربية"].map(|text| text_run(text, IBM_PLEX.family));

        base_runs[0].font = font(SOURCE_SERIF.family);
        base_runs[0].font.features = GpuiFontFeatures::disable_ligatures();

        base_runs[0].font.fallbacks = Some(FontFallbacks::from_fonts(vec![
            IBM_PLEX.family.into(),
            NOTO_COLOR_EMOJI.family.into(),
        ]));

        base_runs[0].letter_spacing = Some(px(0.4));
        base_runs[1].font = base_runs[1].font.clone().bold();

        let mut left = text_run("left ", NOTO_SANS.family);
        left.font.width = gpui::FontWidth::CONDENSED;
        let right = text_run("right", IBM_PLEX.family);

        for ((name, text, original_runs, font_size), native) in [
            ("styled document", text, base_runs.to_vec(), px(20.)),
            (
                "native ligatures and combining marks",
                "office cafe\u{301} x",
                ["office ", "cafe\u{301} ", "x"]
                    .map(|text| text_run(text, IBM_PLEX.family))
                    .to_vec(),
                px(18.),
            ),
            (
                "multibyte bidi controls",
                "café  \tאב 🦀",
                vec![
                    text_run("café  \t", IBM_PLEX.family),
                    text_run("אב 🦀", IBM_PLEX.family),
                ],
                px(18.),
            ),
            (
                "identical paint boundaries",
                "left right",
                vec![left, right],
                px(18.),
            ),
        ]
        .into_iter()
        .flat_map(|case| [false, true].map(move |native| (case.clone(), native)))
        {
            let mut decorated_runs = original_runs.clone();

            for (idx, run) in decorated_runs.iter_mut().enumerate() {
                run.color = hsla(idx as f32 * 0.3 + 0.1, 0.8, 0.4, 1.0);
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

            let empty = text_run("", SOURCE_SERIF.family);

            let padded_runs = decorated_runs
                .iter()
                .flat_map(|run| [empty.clone(), empty.clone(), run.clone()])
                .chain(std::iter::once(empty.clone()))
                .collect::<Vec<_>>();

            let styles = [InlineTextStyle {
                range: run_ranges(&original_runs).last().unwrap().clone(),
                font_size: px(30.),
                line_height: px(40.),
            }];

            let variants = [padded_runs, original_runs, decorated_runs];

            let shaped_paragraphs = system.paragraph_cache.lock().entries.len();
            let cached_results = system.paragraph_result_cache.lock().entries.len();

            for (width_idx, wrap_width) in [None, Some(px(75.))].into_iter().enumerate() {
                let options = TextLayoutOptions {
                    direction: ParagraphDirection::LeftToRight,
                    unicode_bidi: UnicodeBidi::Isolate,
                    text_align: TextAlign::Left,
                    wrap_width,
                    ..Default::default()
                };

                // Change authored boundaries when the new width reuses shaping.
                let shape = |runs: &[TextRun]| {
                    if native {
                        return system
                            .layout_inline(InlineLayoutRequest {
                                font_size,
                                options,
                                text_styles: &styles,
                                ..inline_request(text, runs, &font(IBM_PLEX.family))
                            })
                            .layout;
                    }

                    layout_with_options(&system, text, font_size, runs, options).into()
                };

                let original = shape(&variants[width_idx]);

                let expected_geometry = paint_geometry(&original, &variants[width_idx]);

                for runs in variants.iter().cycle().take(6) {
                    let layout = shape(runs);

                    assert_eq!(layout.width, original.width, "{name}");
                    assert_eq!(paint_geometry(&layout, runs), expected_geometry, "{name}");

                    assert!(
                        Arc::ptr_eq(&layout.platform_layout, &original.platform_layout),
                        "{name}"
                    );
                }

                assert_eq!(
                    system.paragraph_cache.lock().entries.len(),
                    shaped_paragraphs + 1
                );

                assert_eq!(
                    system.paragraph_result_cache.lock().entries.len(),
                    cached_results + width_idx + 1
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

        let system = ParleyTextSystem::new(PolicyRasterizer {
            prepare_calls: prepare_calls.clone(),
            recommended_calls: recommended_calls.clone(),
        })
        .with_fallback_families(["IBM Plex Sans"]);

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
        let backend = Arc::new(test_system_with_fonts(&[IBM_PLEX]));
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
            assert_eq!(
                wrapped.logical_cluster_after(CaretPosition::default()),
                Some(0..text.len()),
                "{text:?} must be one editable cluster"
            );
            let end = wrapped
                .platform_layout
                .adjacent_visual_caret(CaretPosition::default(), VisualDirection::Right)
                .unwrap();
            assert_eq!(
                end.index,
                text.len(),
                "{text:?} must have no internal caret"
            );
            assert_eq!(
                wrapped
                    .platform_layout
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
        assert_eq!(
            single_line
                .caret_movement(
                    middle,
                    Direction::Up.with_boundary(Boundary::VisualLine),
                    None,
                )
                .result
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
                .result
                .index,
            single_line_text.len()
        );

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
        let collapsed_left = layout.selection_movement(
            selection,
            Direction::Left.with_boundary(Boundary::Cluster),
            false,
            None,
            line_height,
        );
        assert!(collapsed_left.result.is_empty());
        assert_eq!(collapsed_left.result.caret, start);
        let collapsed_right = layout.selection_movement(
            selection,
            Direction::Right.with_boundary(Boundary::Cluster),
            false,
            None,
            line_height,
        );
        assert_eq!(collapsed_right.result.caret, end);

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
    fn bundled_color_artwork_is_classified_per_glyph() {
        let system = test_system();

        for (text, expected_flags) in [("👩🏽‍💻", &[true][..]), ("1 1", &[true, false, true][..])]
        {
            let layout = layout_line(
                &system,
                text,
                px(24.0),
                &[text_run(text, NOTO_COLOR_EMOJI.family)],
            );

            let color_flags = layout
                .paint_fragments
                .iter()
                .flat_map(|fragment| fragment.glyphs.iter().map(|glyph| glyph.is_emoji))
                .collect::<Vec<_>>();

            assert_eq!(color_flags, expected_flags, "{text:?}");
        }
    }

    #[test]
    fn automatic_optical_sizing_tracks_logical_default_and_inline_sizes() {
        let system = test_system_with_fonts(&[SOURCE_SERIF]).with_automatic_optical_sizing();

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
            ..inline_request(text, &runs, &font(IBM_PLEX.family))
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
        let system = ParleyTextSystem::new(RecordingRasterizer { seen: seen.clone() })
            .with_fallback_families([SOURCE_SERIF.family])
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
                            .direction(gpui::LayoutDirection::Auto)
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
