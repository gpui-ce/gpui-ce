use crate::{
    Bounds, FontId, GlyphId, InlineBoxRequest, Pixels, PlatformTextSystem, Point, SharedString,
    Size, StrikethroughStyle, TextLayoutRequest, TextRun, UnderlineStyle, VerticalAlign,
};
use collections::FxHashMap;
use palette::Hsla;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use smallvec::SmallVec;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// A laid-out and styled text document.
#[derive(Debug)]
pub struct LineLayout {
    /// The document's base font size.
    pub font_size: Pixels,
    /// The width of the widest visual line.
    pub width: Pixels,
    /// The document's ascent.
    pub ascent: Pixels,
    /// The document's descent.
    pub descent: Pixels,
    /// Visual lines in paint order. Unwrapped layouts contain one line.
    pub visual_lines: SmallVec<[VisualLine; 1]>,
    /// Style-specific paint runs referenced by `visual_lines`.
    pub paint_fragments: Vec<PaintFragment>,
    /// The length of the source text in UTF-8 bytes.
    pub len: usize,
    /// Backend-native interaction model.
    pub platform_layout: Arc<dyn PlatformTextLayout>,
}

/// Font metrics used to construct the vertical extents of an inline line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineTextMetrics {
    /// Distance above the text baseline.
    pub ascent: Pixels,
    /// Distance below the text baseline.
    pub descent: Pixels,
    /// Height of a lowercase x in the inline container's font.
    pub x_height: Pixels,
}

/// A row in an inline layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineVisualLine {
    /// The row origin relative to the inline layout.
    pub origin: Point<Pixels>,
    /// The row size before it is placed in its containing element.
    pub size: Size<Pixels>,
    /// The baseline relative to `origin.y`.
    pub baseline: Pixels,
}

/// The position assigned to an element embedded in text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PositionedInlineBox {
    /// The caller-provided box identifier.
    pub id: u64,
    /// Index of the visual line containing this box.
    pub line_index: usize,
    /// The box bounds relative to the complete inline layout.
    pub bounds: Bounds<Pixels>,
}

/// Text and element boxes laid out in one inline formatting context.
#[derive(Debug)]
pub struct InlineLayout {
    /// The shaped text shared with the ordinary GPUI paint pipeline.
    pub layout: Arc<LineLayout>,
    /// Per-row block geometry from the text backend.
    pub lines: Vec<InlineVisualLine>,
    /// Positioned element boxes.
    pub boxes: Vec<PositionedInlineBox>,
    /// Shared horizontal anchor used to align every visual line.
    pub alignment_offset: Pixels,
    /// Natural size of the complete inline layout.
    pub size: Size<Pixels>,
}

#[derive(Clone, Copy)]
struct InlineBoxPlacement {
    line_index: Option<usize>,
    vertical_align: VerticalAlign,
}

fn base_inline_line_bounds(metrics: InlineTextMetrics, line_height: Pixels) -> (Pixels, Pixels) {
    let half_leading = ((line_height - metrics.ascent - metrics.descent) / 2.).max(Pixels::ZERO);
    (
        -metrics.ascent - half_leading,
        metrics.descent + half_leading,
    )
}

fn expand_inline_line_for_box(
    top: &mut Pixels,
    bottom: &mut Pixels,
    top_box_height: &mut Pixels,
    bottom_box_height: &mut Pixels,
    height: Pixels,
    metrics: InlineTextMetrics,
    align: VerticalAlign,
) {
    match align {
        VerticalAlign::Baseline => *top = (*top).min(-height),
        VerticalAlign::Middle => {
            let middle = -metrics.x_height / 2.;
            *top = (*top).min(middle - height / 2.);
            *bottom = (*bottom).max(middle + height / 2.);
        }
        VerticalAlign::Top => *top_box_height = (*top_box_height).max(height),
        VerticalAlign::Bottom => *bottom_box_height = (*bottom_box_height).max(height),
    }
}

fn aligned_inline_box_y(
    line: InlineVisualLine,
    metrics: InlineTextMetrics,
    height: Pixels,
    align: VerticalAlign,
) -> Pixels {
    match align {
        VerticalAlign::Baseline => line.baseline - height,
        VerticalAlign::Middle => line.baseline - metrics.x_height / 2. - height / 2.,
        VerticalAlign::Top => Pixels::ZERO,
        VerticalAlign::Bottom => line.size.height - height,
    }
}

/// Applies CSS-like vertical alignment to boxes after a text backend assigns them to lines.
pub fn align_inline_boxes(
    lines: &mut [InlineVisualLine],
    boxes: &mut [PositionedInlineBox],
    size: &mut Size<Pixels>,
    requests: &[InlineBoxRequest],
    line_metrics: &[InlineTextMetrics],
    fallback_metrics: InlineTextMetrics,
    line_height: Pixels,
) {
    let box_placements = boxes
        .iter()
        .map(|inline_box| {
            let vertical_align = requests
                .iter()
                .find(|request| request.id == inline_box.id)
                .map_or(VerticalAlign::Baseline, |request| request.vertical_align);
            InlineBoxPlacement {
                line_index: (inline_box.line_index < lines.len()).then_some(inline_box.line_index),
                vertical_align,
            }
        })
        .collect::<Vec<_>>();
    let mut line_y = Pixels::ZERO;

    for (line_index, line) in lines.iter_mut().enumerate() {
        let metrics = line_metrics
            .get(line_index)
            .copied()
            .unwrap_or(fallback_metrics);
        let (mut top, mut bottom) = base_inline_line_bounds(metrics, line_height);
        let mut top_box_height = Pixels::ZERO;
        let mut bottom_box_height = Pixels::ZERO;

        for (inline_box, placement) in boxes.iter().zip(&box_placements) {
            if placement.line_index != Some(line_index) {
                continue;
            }
            expand_inline_line_for_box(
                &mut top,
                &mut bottom,
                &mut top_box_height,
                &mut bottom_box_height,
                inline_box.bounds.size.height,
                metrics,
                placement.vertical_align,
            );
        }

        bottom = bottom.max(top + top_box_height);
        top = top.min(bottom - bottom_box_height);
        line.origin.y = line_y;
        line.size.height = bottom - top;
        line.baseline = -top;

        for (inline_box, placement) in boxes.iter_mut().zip(&box_placements) {
            if placement.line_index != Some(line_index) {
                continue;
            }
            inline_box.bounds.origin.y = line_y
                + aligned_inline_box_y(
                    *line,
                    metrics,
                    inline_box.bounds.size.height,
                    placement.vertical_align,
                );
        }

        line_y += line.size.height;
    }

    size.height = line_y;
}

/// Backend-owned text interaction for a laid-out string.
///
/// Byte positions are UTF-8 boundaries. Geometry is in GPUI layout coordinates, using the
/// caller-provided line height. Implementations must preserve visual order and caret affinity.
pub trait PlatformTextLayout: Send + Sync + std::fmt::Debug {
    /// Length of the source text in UTF-8 bytes.
    fn len(&self) -> usize;
    /// Number of visual lines.
    fn line_count(&self) -> usize;
    /// Natural layout size reported by the backend.
    fn size(&self) -> Size<Pixels>;
    /// Returns the source index of the cluster under a point.
    fn index_from_point(&self, point: Point<Pixels>, line_height: Pixels) -> Result<usize, usize>;
    /// Returns the closest caret for a point. Points outside a visual row return `Err`.
    fn caret_from_point(
        &self,
        point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition>;
    /// Returns the caret rectangle.
    fn caret_geometry(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>>;
    /// Snaps a caret to a native cluster boundary.
    fn refresh_caret(&self, caret: CaretPosition) -> CaretPosition;
    /// Moves one caret stop in visual order.
    fn move_visual(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition>;
    /// Returns selection rectangles in visual order.
    fn selection_geometry(&self, range: Range<usize>, line_height: Pixels) -> Vec<Bounds<Pixels>>;
    /// Returns the atomic logical cluster before the caret.
    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>>;
    /// Returns the atomic logical cluster after the caret.
    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>>;
    /// Moves a caret using backend-native text semantics.
    fn move_caret(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> (CaretPosition, Option<Pixels>);
    /// Returns the word or line selected at a point.
    fn selection_from_point(
        &self,
        point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize>;
}

/// A direction in visual, rather than logical, text order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualDirection {
    /// Move toward the visual left.
    Left,
    /// Move toward the visual right.
    Right,
}

/// A semantic caret movement handled by the native text layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextMovement {
    /// Move to the preceding visual caret stop.
    VisualLeft,
    /// Move to the following visual caret stop.
    VisualRight,
    /// Move to the preceding word in visual order.
    VisualWordLeft,
    /// Move to the following word in visual order.
    VisualWordRight,
    /// Move to the visual row above.
    VisualUp,
    /// Move to the visual row below.
    VisualDown,
    /// Move to the start of the current visual row.
    VisualLineStart,
    /// Move to the end of the current visual row.
    VisualLineEnd,
    /// Move to the start of the current hard line.
    HardLineStart,
    /// Move to the end of the current hard line.
    HardLineEnd,
}

/// A semantic selection derived from a point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSelectionKind {
    /// Select a word.
    Word,
    /// Select a soft-wrapped visual row.
    VisualLine,
    /// Select a line delimited by a hard break.
    HardLine,
}

/// A row produced by shaping and line breaking.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualLine {
    /// The logical UTF-8 byte range assigned to this row.
    pub text_range: Range<usize>,
    /// The range of paintable fragments in this row, in visual order.
    pub fragment_range: Range<usize>,
    /// The row's advance before alignment.
    pub advance: Pixels,
}

/// GPUI paint properties carried through Parley's brush.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintStyle {
    /// Glyph foreground color.
    pub color: Hsla,
    /// Optional background color.
    pub background_color: Option<Hsla>,
    /// Optional underline style.
    pub underline: Option<UnderlineStyle>,
    /// Optional strikethrough style.
    pub strikethrough: Option<StrikethroughStyle>,
}

/// A positioned, style-specific glyph run painted on one visual line.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintFragment {
    /// Canonical font used by the glyphs.
    pub font_id: FontId,
    /// Positioned glyphs local to the visual line.
    pub glyphs: Vec<ShapedGlyph>,
    /// Horizontal bounds of this fragment.
    pub x_range: Range<Pixels>,
    /// Paint properties shared by every glyph in the fragment.
    pub style: PaintStyle,
    /// Resolved underline offset from the baseline.
    pub underline_offset: Option<Pixels>,
    /// Resolved strikethrough offset from the baseline.
    pub strikethrough_offset: Option<Pixels>,
}

/// A single glyph, ready to paint.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// The ID for this glyph, as determined by the text system.
    pub id: GlyphId,

    /// The position of this glyph in its containing line.
    pub position: Point<Pixels>,

    /// Whether this glyph is an emoji
    pub is_emoji: bool,
}

/// Determines which logical neighbor owns a caret at a text boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CaretAffinity {
    /// The caret attaches to the logically following cluster.
    #[default]
    Downstream,
    /// The caret attaches to the logically preceding cluster.
    Upstream,
}

/// A byte position together with the information needed to place it visually.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaretPosition {
    /// The logical UTF-8 byte index.
    pub index: usize,
    /// The logical neighbor that owns this position.
    pub affinity: CaretAffinity,
}

impl CaretPosition {
    /// Creates a caret at a byte index with the given affinity.
    pub fn new(index: usize, affinity: CaretAffinity) -> Self {
        Self { index, affinity }
    }
}

/// An affinity-aware text selection.
///
/// The anchor stays fixed while the focus is the active caret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaretSelection {
    /// The fixed end of the selection.
    pub anchor: CaretPosition,
    /// The active end of the selection.
    pub focus: CaretPosition,
}

impl From<usize> for CaretSelection {
    fn from(index: usize) -> Self {
        Self::collapsed(CaretPosition::new(index, CaretAffinity::Downstream))
    }
}

/// Creates a downstream-affinity selection from `(focus, anchor)` byte indices.
impl From<(usize, usize)> for CaretSelection {
    fn from((focus, anchor): (usize, usize)) -> Self {
        Self::from_focus_anchor(
            CaretPosition::new(focus, CaretAffinity::Downstream),
            CaretPosition::new(anchor, CaretAffinity::Downstream),
        )
    }
}

/// Creates a downstream-affinity selection whose focus is `start` and anchor is `end`.
impl From<Range<usize>> for CaretSelection {
    fn from(range: Range<usize>) -> Self {
        Self::from((range.start, range.end))
    }
}

impl CaretSelection {
    /// Creates a selection from its fixed anchor and active focus.
    pub fn new(anchor: CaretPosition, focus: CaretPosition) -> Self {
        Self { anchor, focus }
    }

    /// Creates a selection from its active focus and fixed anchor.
    pub fn from_focus_anchor(focus: CaretPosition, anchor: CaretPosition) -> Self {
        Self { anchor, focus }
    }

    /// Creates an empty selection at `caret`.
    pub fn collapsed(caret: CaretPosition) -> Self {
        Self::new(caret, caret)
    }

    /// Returns whether the selection is empty.
    pub fn is_empty(&self) -> bool {
        self.anchor.index == self.focus.index
    }

    /// Returns the selected UTF-8 byte range in logical order.
    pub fn byte_range(self) -> Range<usize> {
        self.anchor.index.min(self.focus.index)..self.anchor.index.max(self.focus.index)
    }

    /// Moves the active end while preserving the anchor.
    pub fn with_focus(self, focus: CaretPosition) -> Self {
        Self { focus, ..self }
    }
}

/// The result of moving or extending a selection through a laid-out document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaretSelectionMove {
    /// The selection after movement.
    pub selection: CaretSelection,
    /// The horizontal position retained by successive vertical movements.
    pub preferred_x: Option<Pixels>,
}

/// A document layout with its optional wrapping constraint.
#[derive(Debug)]
pub struct WrappedLineLayout {
    /// The laid out document.
    pub layout: Arc<LineLayout>,

    /// The width constraint, when wrapping is enabled.
    pub wrap_width: Option<Pixels>,
}

impl WrappedLineLayout {
    /// The length of the underlying text, in utf8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.platform_layout.len()
    }

    /// The document width, capped by the wrapping constraint.
    pub fn width(&self) -> Pixels {
        self.wrap_width
            .unwrap_or(Pixels::MAX)
            .min(self.layout.width)
    }

    /// The size of the whole wrapped text for the given line height.
    pub fn size(&self, line_height: Pixels) -> Size<Pixels> {
        Size {
            width: self.width(),
            height: line_height * self.line_count(),
        }
    }

    /// Returns the number of visual lines in this layout.
    pub fn line_count(&self) -> usize {
        self.layout.platform_layout.line_count()
    }

    /// The ascent of a line in this layout
    pub fn ascent(&self) -> Pixels {
        self.layout.ascent
    }

    /// The descent of a line in this layout
    pub fn descent(&self) -> Pixels {
        self.layout.descent
    }

    /// Returns the visual lines in paint order.
    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.layout.visual_lines
    }

    /// Returns the fragments belonging to a visual line.
    pub fn fragments_for_line(&self, line: &VisualLine) -> &[PaintFragment] {
        &self.layout.paint_fragments[line.fragment_range.clone()]
    }

    /// The font size of this layout
    pub fn font_size(&self) -> Pixels {
        self.layout.font_size
    }

    /// The index corresponding to a given position in this layout for the given line height.
    ///
    /// The backend returns the logical start of the visual cluster under the point.
    /// Whitespace is hit like any other cluster. Positions outside the line return the boundary at
    /// that visual edge in `Err`.
    ///
    /// See also [`Self::closest_index_for_position`].
    pub fn index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.layout
            .platform_layout
            .index_from_point(position, line_height)
    }

    /// The closest index to a given position in this layout for the given line height.
    ///
    /// Closest means the character boundary closest to the given position.
    /// The backend only returns cluster boundaries. For right-to-left clusters, the
    /// visual left edge maps to the logical end and the visual right edge maps to the logical
    /// start. Zero-width clusters share a stop with an adjacent cluster and use visual order to
    /// break ties.
    ///
    pub fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.closest_caret_for_position(position, line_height)
            .map(|caret| caret.index)
            .map_err(|caret| caret.index)
    }

    /// Returns the closest backend-native caret for a point.
    ///
    /// Positions outside a visual line return the caret at that edge in `Err`, matching
    /// [`Self::closest_index_for_position`].
    pub fn closest_caret_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.layout
            .platform_layout
            .caret_from_point(position, line_height)
    }

    /// Returns the pixel position for the given byte index.
    ///
    /// The backend maps cluster boundaries to direction-aware visual edges. An index
    /// inside an atomic cluster snaps to its logical start. On a shared wrap boundary, the cluster
    /// starting at the index owns the position, so the caret moves to the following visual line.
    pub fn position_for_index(&self, index: usize, line_height: Pixels) -> Option<Point<Pixels>> {
        if index > self.len() {
            return None;
        }

        self.layout
            .platform_layout
            .caret_geometry(
                CaretPosition::new(index, CaretAffinity::Downstream),
                line_height,
            )
            .map(|bounds| bounds.origin)
    }

    /// Returns the pixel position for an affinity-aware caret.
    pub fn position_for_caret(
        &self,
        caret: CaretPosition,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        self.layout
            .platform_layout
            .caret_geometry(caret, line_height)
            .map(|bounds| bounds.origin)
    }

    /// Snaps a byte position to a valid cluster boundary while preserving affinity when possible.
    pub fn refresh_caret(&self, caret: CaretPosition) -> CaretPosition {
        self.layout.platform_layout.refresh_caret(caret)
    }

    /// Returns the previous caret stop in visual order.
    pub fn previous_visual_caret(&self, caret: CaretPosition) -> Option<CaretPosition> {
        self.layout
            .platform_layout
            .move_visual(caret, VisualDirection::Left)
    }

    /// Returns the next caret stop in visual order.
    pub fn next_visual_caret(&self, caret: CaretPosition) -> Option<CaretPosition> {
        self.layout
            .platform_layout
            .move_visual(caret, VisualDirection::Right)
    }

    /// Returns the logical cluster immediately before the caret.
    pub fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.layout.platform_layout.logical_cluster_before(caret)
    }

    /// Returns the logical cluster immediately after the caret.
    pub fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.layout.platform_layout.logical_cluster_after(caret)
    }

    /// Moves a caret using the backend's visual and line-breaking model.
    pub fn move_caret(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        preferred_x: Option<Pixels>,
    ) -> (CaretPosition, Option<Pixels>) {
        self.layout
            .platform_layout
            .move_caret(caret, movement, preferred_x)
    }

    /// Moves or extends an affinity-aware selection using visual text order.
    ///
    /// Horizontal movement without extension collapses a non-empty selection toward the requested
    /// visual edge. Other movement starts at the focus. Extending keeps the anchor fixed.
    pub fn move_selection(
        &self,
        selection: CaretSelection,
        movement: TextMovement,
        extend: bool,
        preferred_x: Option<Pixels>,
        line_height: Pixels,
    ) -> CaretSelectionMove {
        let forward = matches!(
            movement,
            TextMovement::VisualRight | TextMovement::VisualWordRight
        );
        let horizontal = forward
            || matches!(
                movement,
                TextMovement::VisualLeft | TextMovement::VisualWordLeft
            );

        if !extend && !selection.is_empty() && horizontal {
            let focus_position = self.position_for_caret(selection.focus, line_height);
            let anchor_position = self.position_for_caret(selection.anchor, line_height);
            let (visual_start, visual_end) = focus_position
                .zip(anchor_position)
                .map(|(focus_position, anchor_position)| {
                    if (focus_position.y, focus_position.x)
                        <= (anchor_position.y, anchor_position.x)
                    {
                        (selection.focus, selection.anchor)
                    } else {
                        (selection.anchor, selection.focus)
                    }
                })
                .unwrap_or_else(|| {
                    if selection.focus.index <= selection.anchor.index {
                        (selection.focus, selection.anchor)
                    } else {
                        (selection.anchor, selection.focus)
                    }
                });
            let caret = if forward { visual_end } else { visual_start };
            return CaretSelectionMove {
                selection: CaretSelection::collapsed(caret),
                preferred_x: None,
            };
        }

        let (focus, preferred_x) = self.move_caret(selection.focus, movement, preferred_x);
        CaretSelectionMove {
            selection: if extend {
                selection.with_focus(focus)
            } else {
                CaretSelection::collapsed(focus)
            },
            preferred_x,
        }
    }

    /// Selects the word or line at a point.
    pub fn selection_from_point(
        &self,
        point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        self.layout
            .platform_layout
            .selection_from_point(point, line_height, kind)
    }

    /// Returns rectangles covering all selected clusters in visual order.
    pub fn selection_bounds(
        &self,
        range: Range<usize>,
        line_height: Pixels,
    ) -> SmallVec<[Bounds<Pixels>; 4]> {
        let mut result = SmallVec::new();
        if range.is_empty() {
            return result;
        }

        result.extend(
            self.layout
                .platform_layout
                .selection_geometry(range, line_height),
        );
        result
    }
}

impl std::ops::Deref for WrappedLineLayout {
    type Target = LineLayout;

    fn deref(&self) -> &Self::Target {
        &self.layout
    }
}

pub(crate) struct LineLayoutCache {
    previous_frame: Mutex<FrameCache>,
    current_frame: RwLock<FrameCache>,
    platform_text_system: Arc<dyn PlatformTextSystem>,
    font_generation: AtomicU64,
}

#[derive(Default)]
struct FrameCache {
    lines: FxHashMap<Arc<CacheKey>, Arc<LineLayout>>,
    wrapped_lines: FxHashMap<Arc<CacheKey>, Arc<WrappedLineLayout>>,
    used_lines: Vec<Arc<CacheKey>>,
    used_wrapped_lines: Vec<Arc<CacheKey>>,
}

#[derive(Clone, Default)]
pub(crate) struct LineLayoutIndex {
    lines_index: usize,
    wrapped_lines_index: usize,
}

impl LineLayoutCache {
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        let font_generation = platform_text_system.font_generation();
        Self {
            previous_frame: Mutex::default(),
            current_frame: RwLock::default(),
            platform_text_system,
            font_generation: AtomicU64::new(font_generation),
        }
    }

    pub fn layout_index(&self) -> LineLayoutIndex {
        let frame = self.current_frame.read();
        LineLayoutIndex {
            lines_index: frame.used_lines.len(),
            wrapped_lines_index: frame.used_wrapped_lines.len(),
        }
    }

    /// Invalidates every cached layout after backend shaping state changes.
    pub fn clear(&self) {
        *self.previous_frame.lock() = FrameCache::default();
        *self.current_frame.write() = FrameCache::default();
    }

    fn sync_font_generation(&self) {
        let generation = self.platform_text_system.font_generation();
        if self.font_generation.load(Ordering::Acquire) != generation {
            self.clear();
            self.font_generation.store(generation, Ordering::Release);
        }
    }

    pub fn reuse_layouts(&self, range: Range<LineLayoutIndex>) {
        let mut previous_frame = &mut *self.previous_frame.lock();
        let mut current_frame = &mut *self.current_frame.write();

        for key in &previous_frame.used_lines[range.start.lines_index..range.end.lines_index] {
            if let Some((key, line)) = previous_frame.lines.remove_entry(key) {
                current_frame.lines.insert(key, line);
            }
            current_frame.used_lines.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines
            [range.start.wrapped_lines_index..range.end.wrapped_lines_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines.remove_entry(key) {
                current_frame.wrapped_lines.insert(key, line);
            }
            current_frame.used_wrapped_lines.push(key.clone());
        }
    }

    pub fn truncate_layouts(&self, index: LineLayoutIndex) {
        let mut current_frame = &mut *self.current_frame.write();
        current_frame.used_lines.truncate(index.lines_index);
        current_frame
            .used_wrapped_lines
            .truncate(index.wrapped_lines_index);
    }

    pub fn finish_frame(&self) {
        let mut prev_frame = self.previous_frame.lock();
        let mut curr_frame = self.current_frame.write();
        std::mem::swap(&mut *prev_frame, &mut *curr_frame);
        curr_frame.lines.clear();
        curr_frame.wrapped_lines.clear();
        curr_frame.used_lines.clear();
        curr_frame.used_wrapped_lines.clear();
    }

    pub fn layout_wrapped_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        max_lines: Option<usize>,
    ) -> Arc<WrappedLineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.sync_font_generation();
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width,
            max_lines,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.wrapped_lines.get(key) {
            return layout.clone();
        }

        let previous_frame_entry = self.previous_frame.lock().wrapped_lines.remove_entry(key);
        if let Some((key, layout)) = previous_frame_entry {
            let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);
            layout
        } else {
            drop(current_frame);
            let text = SharedString::from(text);
            let document_layout =
                if wrap_width.is_some() || max_lines.is_some() || text.contains('\n') {
                    Arc::new(self.platform_text_system.layout_text(TextLayoutRequest {
                        text: &text,
                        font_size,
                        runs,
                        wrap_width,
                        line_clamp: max_lines,
                    }))
                } else {
                    self.layout_line::<&SharedString>(&text, font_size, runs)
                };
            let layout = Arc::new(WrappedLineLayout {
                layout: document_layout,
                wrap_width,
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width,
                max_lines,
            });

            let mut current_frame = self.current_frame.write();
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);

            layout
        }
    }

    pub fn layout_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[TextRun],
    ) -> Arc<LineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.sync_font_generation();
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width: None,
            max_lines: None,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.lines.get(key) {
            return layout.clone();
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
        if let Some((key, layout)) = self.previous_frame.lock().lines.remove_entry(key) {
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        } else {
            let text = SharedString::from(text);
            let layout = self.platform_text_system.layout_text(TextLayoutRequest {
                text: &text,
                font_size,
                runs,
                wrap_width: None,
                line_clamp: None,
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width: None,
                max_lines: None,
            });
            let layout = Arc::new(layout);
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        }
    }
}

trait AsCacheKeyRef {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_>;
}

#[derive(Clone, Debug, Eq)]
struct CacheKey {
    text: SharedString,
    font_size: Pixels,
    runs: SmallVec<[TextRun; 1]>,
    wrap_width: Option<Pixels>,
    max_lines: Option<usize>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: Pixels,
    runs: &'a [TextRun],
    wrap_width: Option<Pixels>,
    max_lines: Option<usize>,
}

impl PartialEq for dyn AsCacheKeyRef + '_ {
    fn eq(&self, other: &dyn AsCacheKeyRef) -> bool {
        self.as_cache_key_ref() == other.as_cache_key_ref()
    }
}

impl Eq for dyn AsCacheKeyRef + '_ {}

impl Hash for dyn AsCacheKeyRef + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state)
    }
}

impl AsCacheKeyRef for CacheKey {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        CacheKeyRef {
            text: &self.text,
            font_size: self.font_size,
            runs: self.runs.as_slice(),
            wrap_width: self.wrap_width,
            max_lines: self.max_lines,
        }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_cache_key_ref().eq(&other.as_cache_key_ref())
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state);
    }
}

impl<'a> Borrow<dyn AsCacheKeyRef + 'a> for Arc<CacheKey> {
    fn borrow(&self) -> &(dyn AsCacheKeyRef + 'a) {
        self.as_ref() as &dyn AsCacheKeyRef
    }
}

impl AsCacheKeyRef for CacheKeyRef<'_> {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        *self
    }
}
