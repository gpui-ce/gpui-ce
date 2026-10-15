use crate::{
    Bounds, FontId, GlyphId, InlineBidiScope, InlineBoxRequest, InlineLayoutRequest,
    InlineTextStyle, Pixels, PlatformTextSystem, Point, SharedString, Size, StrikethroughStyle,
    TextLayoutOptions, TextLayoutRequest, TextRun, UnderlineStyle, VerticalAlign,
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

/// The shaped and laid-out form of a string of text, ready to measure and paint.
///
/// `LineLayout` is what the text backend returns for one piece of text. Most
/// code gets it indirectly through [`ShapedLine`](crate::ShapedLine) or
/// [`ShapedText`](crate::ShapedText), and reads it through the methods on
/// [`ShapedTextLayout`]. Use the public fields when you need the raw geometry,
/// such as the width of the widest visual line.
#[derive(Clone, Debug)]
pub struct LineLayout {
    /// The base font size the text was shaped at, in logical pixels.
    pub font_size: Pixels,
    /// The width of the widest visual line, in logical pixels.
    pub width: Pixels,
    /// The distance from the baseline up to the top of the tallest glyphs.
    pub ascent: Pixels,
    /// The distance from the baseline down to the bottom of the lowest glyphs.
    pub descent: Pixels,
    /// The visual lines in top-to-bottom order. Without soft wraps or hard breaks, there is one line.
    pub visual_lines: SmallVec<[VisualLine; 1]>,
    /// The runs of glyphs that share one style, referenced by each visual line.
    pub paint_fragments: Vec<PaintFragment>,
    /// The length of the source text in UTF-8 bytes.
    pub len: usize,
    /// The backend's interaction model, used for caret movement and hit testing.
    pub platform_layout: Arc<dyn PlatformTextLayout>,
}

/// Font metrics that set the vertical extent of an inline line.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InlineTextMetrics {
    /// The distance from the baseline up to the top of the tallest glyphs.
    pub ascent: Pixels,
    /// The distance from the baseline down to the bottom of the lowest glyphs.
    pub descent: Pixels,
    /// The height of a lowercase `x` in the container's font.
    pub x_height: Pixels,
}

/// One row of an inline layout, positioned relative to the layout's origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineVisualLine {
    /// The top-left corner of the row, relative to the inline layout.
    pub origin: Point<Pixels>,
    /// The size of the row, including the room that atomic boxes need.
    pub size: Size<Pixels>,
    /// The baseline of the row, measured down from `origin.y`.
    pub baseline: Pixels,
}

/// An atomic box from an inline layout, with the position the layout gave it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PositionedInlineBox {
    /// The identifier from the matching [`InlineBoxRequest::id`].
    pub id: u64,
    /// The index of the visual line that contains this box.
    pub line_index: usize,
    /// The bounds of the box, relative to the complete inline layout.
    pub bounds: Bounds<Pixels>,
}

/// The area that a text range covers on one visual line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineRangeGeometry {
    /// The bounds of the range on this line, relative to the complete layout.
    pub bounds: Bounds<Pixels>,
    /// The index of the visual line that contains `bounds`.
    pub visual_line_index: usize,
}

/// Text and atomic boxes laid out together, following inline paragraph flow.
///
/// Use [`WindowTextSystem::layout_inline`](crate::WindowTextSystem::layout_inline)
/// to build one. Paint it with
/// [`InlineLayout::paint_background`] and [`InlineLayout::paint`]. Place each box from
/// [`InlineLayout::boxes`] in the surrounding element yourself; GPUI paints the text but does
/// not render the box contents.
#[derive(Clone, Debug)]
pub struct InlineLayout {
    /// The shaped text, which the ordinary paint pipeline draws.
    pub layout: Arc<LineLayout>,
    /// The position and height of each visual line.
    pub lines: Vec<InlineVisualLine>,
    /// The position of each atomic box.
    pub boxes: Vec<PositionedInlineBox>,
    /// The horizontal offset that aligns every visual line in the layout.
    pub alignment_offset: Pixels,
    /// The natural size of the complete layout.
    pub size: Size<Pixels>,
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
    text_bounds: &[(Pixels, Pixels)],
    fallback_metrics: InlineTextMetrics,
    line_height: Pixels,
) {
    let request_alignments = requests
        .iter()
        .map(|request| (request.id, request.vertical_align))
        .collect::<FxHashMap<_, _>>();
    let mut boxes_by_line: FxHashMap<_, Vec<_>> = FxHashMap::default();

    for inline_box in boxes
        .iter_mut()
        .filter(|inline_box| inline_box.line_index < lines.len())
    {
        let align = request_alignments
            .get(&inline_box.id)
            .copied()
            .unwrap_or(VerticalAlign::Baseline);

        boxes_by_line
            .entry(inline_box.line_index)
            .or_default()
            .push((inline_box, align));
    }

    let mut line_y = Pixels::ZERO;

    for (line_index, line) in lines.iter_mut().enumerate() {
        let line_boxes = boxes_by_line.remove(&line_index).unwrap_or_default();
        let metrics = line_metrics
            .get(line_index)
            .copied()
            .unwrap_or(fallback_metrics);
        let (mut top, mut bottom) = text_bounds
            .get(line_index)
            .copied()
            .unwrap_or_else(|| base_inline_line_bounds(metrics, line_height));
        let mut top_box_height = Pixels::ZERO;
        let mut bottom_box_height = Pixels::ZERO;

        for (inline_box, align) in &line_boxes {
            expand_inline_line_for_box(
                &mut top,
                &mut bottom,
                &mut top_box_height,
                &mut bottom_box_height,
                inline_box.bounds.size.height,
                metrics,
                *align,
            );
        }

        bottom = bottom.max(top + top_box_height);
        top = top.min(bottom - bottom_box_height);
        line.origin.y = line_y;
        line.size.height = bottom - top;
        line.baseline = -top;

        for (inline_box, align) in line_boxes {
            inline_box.bounds.origin.y =
                line_y + aligned_inline_box_y(*line, metrics, inline_box.bounds.size.height, align);
        }

        line_y += line.size.height;
    }

    size.height = line_y;
}

/// The text interaction model that a backend provides for one laid-out string.
///
/// GPUI calls these methods for hit testing, caret movement, and selection. The
/// backend does this work because only it knows the glyph clusters, bidi
/// runs, and line breaks. Application code does not implement or call this trait
/// directly. Use the methods on [`ShapedTextLayout`] instead.
///
/// Byte positions are UTF-8 boundaries. Geometry is in GPUI layout coordinates,
/// using the caller-provided line height. Implementations must preserve visual
/// order and caret affinity.
///
/// A cluster is one backend-defined caret step. It may contain several Unicode
/// scalar values, such as a combining sequence or an emoji ZWJ sequence. Logical
/// clusters are independent of visual direction and soft-wrapped rows.
pub trait PlatformTextLayout: Send + Sync + std::fmt::Debug {
    /// Returns the length of the source text in UTF-8 bytes.
    fn len(&self) -> usize;

    /// Returns the number of visual lines, after wrapping and clamping.
    fn line_count(&self) -> usize;

    /// Returns the natural size of the layout that the backend reports.
    fn size(&self) -> Size<Pixels>;

    /// Returns the UTF-8 byte index of the cluster under a point in GPUI layout coordinates.
    ///
    /// Returns `Ok` when the point is inside a visual row. Otherwise it returns
    /// `Err` with the closest index.
    fn byte_index_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize>;

    /// Returns the closest caret for a point in GPUI layout coordinates.
    /// Points outside a visual row return `Err`.
    fn caret_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition>;

    /// Returns the caret bounds in GPUI layout coordinates.
    fn caret_bounds(&self, caret: CaretPosition, line_height: Pixels) -> Option<Bounds<Pixels>>;

    /// Snaps a caret to a native cluster boundary.
    fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition;

    /// Returns the adjacent caret stop in visual order.
    fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition>;

    /// Returns bounds for a UTF-8 byte range in visual order.
    fn selection_bounds(
        &self,
        byte_range: Range<usize>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>>;

    /// Native range rectangles and their visual line indices, without selection-only extensions.
    ///
    /// Returns `None` for an empty input range. A nonempty range can produce an empty vector when
    /// it has no drawable geometry. Backends supporting inline flow should preserve actual
    /// vertical metrics here.
    fn inline_geometry(&self, range: Range<usize>) -> Option<Vec<InlineRangeGeometry>> {
        if range.is_empty() {
            return None;
        }

        let line_height = self.size().height / self.line_count().max(1) as f32;

        Some(
            self.selection_bounds(range, line_height)
                .into_iter()
                .map(|bounds| InlineRangeGeometry {
                    visual_line_index: (bounds.origin.y / line_height) as usize,
                    bounds,
                })
                .collect(),
        )
    }

    /// Returns native geometry for several ranges in input order.
    fn inline_geometry_for_ranges(&self, ranges: &[Range<usize>]) -> Vec<Vec<InlineRangeGeometry>> {
        ranges
            .iter()
            .map(|range| self.inline_geometry(range.clone()).unwrap_or_default())
            .collect()
    }

    /// Returns the atomic logical cluster before the caret.
    fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>>;

    /// Returns the atomic logical cluster after the caret.
    fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>>;

    /// Returns the caret and retained horizontal coordinate after applying a movement.
    fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        vertical_navigation_x: Option<Pixels>,
    ) -> CaretMovement;

    /// Returns the word or line selected at a point in GPUI layout coordinates.
    fn selection_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize>;
}

/// A direction in visual order, which is left and right on screen, regardless of text direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualDirection {
    /// Toward the left edge of the screen.
    Left,
    /// Toward the right edge of the screen.
    Right,
}

/// The direction of a semantic text movement, such as an arrow key or a Home key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextDirection {
    /// Move toward the visual left.
    Left,
    /// Move toward the visual right.
    Right,
    /// Move to the visual row above.
    Up,
    /// Move to the visual row below.
    Down,
    /// Move to the start of a line or document boundary.
    Start,
    /// Move to the end of a line or document boundary.
    End,
}

/// The unit that a semantic text movement stops at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextBoundary {
    /// One backend-defined caret step, such as one grapheme cluster.
    ///
    /// The backend decides what a cluster is. [`PlatformTextLayout::normalized_caret`]
    /// and [`PlatformTextLayout::adjacent_visual_caret`] find visual stops, and
    /// [`PlatformTextLayout::logical_cluster_before`] and
    /// [`PlatformTextLayout::logical_cluster_after`] return logical ranges.
    /// Layouts from the Parley backend, which the platforms use, step by grapheme.
    Cluster,
    /// A word boundary.
    Word,
    /// One visual row produced by line breaking, including soft wrapping.
    VisualLine,
    /// A line delimited by a hard break.
    HardLine,
    /// The complete text document.
    Document,
}

/// A caret movement described by a direction and a unit, such as "to the end of the word".
///
/// `direction` chooses where to move, and `boundary` chooses the unit. Consecutive vertical
/// movements reuse the returned preferred horizontal coordinate, so moving up or down keeps
/// the column you started in. Build one with [`TextDirection::with_boundary`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextMovement {
    /// The direction in which to move.
    pub direction: TextDirection,
    /// The boundary at which to stop.
    pub boundary: TextBoundary,
}

impl TextDirection {
    /// Returns a [`TextMovement`] in this direction that stops at `boundary`.
    pub const fn with_boundary(self, boundary: TextBoundary) -> TextMovement {
        TextMovement {
            direction: self,
            boundary,
        }
    }
}

/// The unit selected by a double or triple click at a point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextSelectionKind {
    /// The word under the point.
    Word,
    /// The soft-wrapped visual row under the point.
    VisualLine,
    /// The line between hard breaks under the point.
    HardLine,
}

/// One row of text produced by shaping and line breaking.
///
/// Soft wrapping can split one logical line across several rows. Bidirectional
/// ordering changes the visual order of glyphs but not the logical UTF-8 byte ranges.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualLine {
    /// The logical UTF-8 byte range assigned to this row.
    pub text_range: Range<usize>,
    /// Indices into [`LineLayout::paint_fragments`] for the positioned glyph runs
    /// painted on this row, ordered by visual position.
    pub paint_fragment_range: Range<usize>,
    /// Horizontal distance consumed by the row's shaped content before alignment,
    /// measured to the backend's final text pen position. Inline layouts include
    /// advances from embedded boxes.
    pub advance_width: Pixels,
    /// Horizontal offset assigned by backend layout.
    pub offset: Pixels,
    /// Base direction used to order and align this visual line.
    pub direction: crate::ResolvedDirection,
}

/// The paint properties of one run of text: color, background, underline, and strikethrough.
///
/// These come from the [`TextRun`] that covers the glyphs. They are kept apart from
/// the shaping, so changing them does not reshape the text.
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

/// A run of glyphs that share one font and one style, positioned on a visual line.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintFragment {
    /// Index of the input text run that supplies paint properties.
    pub source_run: usize,
    /// Canonical font used by the glyphs.
    pub font_id: FontId,
    /// Resolved size of this shaped run.
    pub font_size: Pixels,
    /// Positioned glyphs local to the visual line.
    pub glyphs: Arc<[ShapedGlyph]>,
    /// Horizontal bounds of this fragment.
    pub x_range: Range<Pixels>,
    /// Paint properties shared by every glyph in the fragment.
    pub style: PaintStyle,
    /// Resolved underline offset from the baseline.
    pub underline_offset: Option<Pixels>,
    /// Resolved strikethrough offset from the baseline.
    pub strikethrough_offset: Option<Pixels>,
}

/// One glyph in a shaped line, ready to paint.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// The glyph's ID in the font, as chosen by the text system.
    pub id: GlyphId,

    /// The glyph origin, relative to the start of its visual line.
    pub position: Point<Pixels>,

    /// Whether the glyph is a color emoji, which paints through the emoji path.
    pub is_emoji: bool,
}

/// Determines which logical neighbor owns a caret at a shared text boundary.
///
/// At a soft wrap, downstream places the caret at the next row's start, and upstream places it at
/// the previous row's end. At a bidirectional boundary, affinities can place the same byte index at
/// different horizontal positions. They refer to logical order, not visual left and right.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum CaretAffinity {
    /// The caret attaches to the logically following cluster.
    #[default]
    Downstream,
    /// The caret attaches to the logically preceding cluster.
    Upstream,
}

/// Hard line breaks that place an inserted-text caret downstream.
static HARD_LINE_BREAK_CHARACTERS: phf::Set<char> = phf::phf_set! {
    '\n',
    '\r',
    '\u{001c}',
    '\u{001d}',
    '\u{001e}',
    '\u{0085}',
    '\u{2028}',
    '\u{2029}',
};

/// Returns whether `character` separates Unicode text paragraphs.
///
/// Line separator (`U+2028`) is a hard line break within the current paragraph. Consumers are
/// responsible for treating a CRLF pair as one paragraph separator.
pub fn is_paragraph_separator(character: char) -> bool {
    character != '\u{2028}' && HARD_LINE_BREAK_CHARACTERS.contains(&character)
}

impl CaretAffinity {
    /// Returns the affinity for a caret placed right after `text` was inserted.
    ///
    /// Returns [`CaretAffinity::Downstream`] when `text` is empty or ends with a hard line break,
    /// so the caret starts the next line. Otherwise it returns [`CaretAffinity::Upstream`].
    pub fn for_inserted_text(text: &str) -> Self {
        Self::from(text)
    }
}

impl From<&str> for CaretAffinity {
    fn from(text: &str) -> Self {
        let Some(last_character) = text.chars().next_back() else {
            return Self::Downstream;
        };

        if HARD_LINE_BREAK_CHARACTERS.contains(&last_character) {
            return Self::Downstream;
        }

        Self::Upstream
    }
}

/// A position between characters, given as a UTF-8 byte index plus the side it attaches to.
///
/// The index alone is ambiguous at a soft wrap or a bidi boundary, because the
/// same byte can sit at the end of one row or the start of the next. The
/// affinity decides which.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct CaretPosition {
    /// The logical UTF-8 byte index.
    pub index: usize,
    /// The logical neighbor that owns this position.
    pub affinity: CaretAffinity,
}

impl From<(usize, CaretAffinity)> for CaretPosition {
    fn from((index, affinity): (usize, CaretAffinity)) -> Self {
        Self { index, affinity }
    }
}

impl CaretPosition {
    /// Creates a caret attached to the next logical cluster.
    pub const fn attached_to_next_cluster(index: usize) -> Self {
        Self {
            index,
            affinity: CaretAffinity::Downstream,
        }
    }

    /// Creates a caret attached to the previous logical cluster.
    pub const fn attached_to_previous_cluster(index: usize) -> Self {
        Self {
            index,
            affinity: CaretAffinity::Upstream,
        }
    }
}

/// A text selection with an anchor and a caret, each with its own affinity.
///
/// The anchor stays fixed while the caret moves. The caret can be on either side
/// of the anchor, so use [`CaretSelection::byte_range`] to get the range in logical order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaretSelection {
    /// The fixed position of the selection, which may be its logical start or end.
    pub anchor: CaretPosition,
    /// The active position of the selection. When it equals `anchor`, the selection is empty.
    pub caret: CaretPosition,
}

impl From<usize> for CaretSelection {
    fn from(index: usize) -> Self {
        CaretPosition::attached_to_next_cluster(index).into()
    }
}

impl From<CaretPosition> for CaretSelection {
    fn from(caret: CaretPosition) -> Self {
        Self {
            anchor: caret,
            caret,
        }
    }
}

impl From<Range<usize>> for CaretSelection {
    /// Creates a selection whose caret is at `range.start` and whose anchor is at `range.end`.
    fn from(range: Range<usize>) -> Self {
        Self {
            anchor: CaretPosition::attached_to_next_cluster(range.end),
            caret: CaretPosition::attached_to_next_cluster(range.start),
        }
    }
}

impl CaretSelection {
    /// Returns whether the anchor and the caret have the same byte index.
    ///
    /// Affinity is ignored, so a caret at a wrap point with two affinities is still empty.
    pub fn is_empty(&self) -> bool {
        self.anchor.index == self.caret.index
    }

    /// Compares the caret's byte index with the anchor's. `Less` means the caret is before the anchor.
    pub fn endpoint_ordering(self) -> std::cmp::Ordering {
        self.caret.index.cmp(&self.anchor.index)
    }

    /// Returns the selected UTF-8 byte range, with the smaller index first.
    pub fn byte_range(self) -> Range<usize> {
        self.anchor.index.min(self.caret.index)..self.anchor.index.max(self.caret.index)
    }

    /// Limits both selection endpoints to `max_idx` while preserving their affinities.
    pub fn min(mut self, max_idx: usize) -> Self {
        self.caret.index = self.caret.index.min(max_idx);
        self.anchor.index = self.anchor.index.min(max_idx);

        self
    }

    /// Moves the active end while preserving the anchor.
    pub fn with_caret(self, caret: CaretPosition) -> Self {
        Self {
            anchor: self.anchor,
            caret,
        }
    }
}

/// The destination of a caret movement, and the column to keep across vertical moves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaretMovement<T = CaretPosition> {
    /// The value at the requested destination.
    pub result: T,
    /// The horizontal position, from the layout's left edge, to keep across consecutive
    /// vertical movements. Pass it back in on the next move so the caret keeps its column.
    pub vertical_navigation_x: Option<Pixels>,
}

/// The result of moving a selection: the new anchor and caret, and the column to keep.
pub type CaretSelectionMovement = CaretMovement<CaretSelection>;

impl CaretMovement<CaretSelection> {
    /// Moves the caret to `caret`, or extends the selection to it when `extend` is true.
    ///
    /// Either way, the retained vertical coordinate is cleared.
    pub fn move_or_select_to(self, caret: CaretPosition, extend: bool) -> Self {
        let result = if extend {
            self.result.with_caret(caret)
        } else {
            caret.into()
        };

        Self {
            result,
            vertical_navigation_x: None,
        }
    }
}

/// Shaped text with its wrap width, which is how a layout is measured and queried.
///
/// Get one from [`WindowTextSystem::shape_text`](crate::WindowTextSystem::shape_text).
/// [`ShapedText`](crate::ShapedText) derefs to it. Its methods convert between
/// pixel positions and UTF-8 byte indices, so you can place a caret or find the
/// character under the mouse. All positions are in logical pixels, relative to
/// the top-left corner of the text.
#[derive(Debug)]
pub struct ShapedTextLayout {
    /// The laid-out document.
    pub layout: Arc<LineLayout>,

    /// The width that lines wrap at, or `None` when the text does not wrap.
    pub wrap_width: Option<Pixels>,
}

impl ShapedTextLayout {
    /// Returns the length of the text in UTF-8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.layout.platform_layout.len()
    }

    /// Returns the width of the widest line, capped at the wrap width.
    pub fn width(&self) -> Pixels {
        self.wrap_width
            .unwrap_or(Pixels::MAX)
            .min(self.layout.width)
    }

    /// Returns the size of the whole layout, with each line `line_height` tall.
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

    /// Returns the distance from the baseline up to the tallest glyphs.
    pub fn ascent(&self) -> Pixels {
        self.layout.ascent
    }

    /// Returns the distance from the baseline down to the lowest glyphs.
    pub fn descent(&self) -> Pixels {
        self.layout.descent
    }

    /// Returns the visual lines in paint order.
    pub fn visual_lines(&self) -> &[VisualLine] {
        &self.layout.visual_lines
    }

    /// Returns the fragments belonging to a visual line.
    pub fn fragments_for_line(&self, line: &VisualLine) -> &[PaintFragment] {
        &self.layout.paint_fragments[line.paint_fragment_range.clone()]
    }

    /// Returns the base font size that the text was shaped at, in logical pixels.
    pub fn font_size(&self) -> Pixels {
        self.layout.font_size
    }

    /// The UTF-8 byte index for a point in GPUI layout coordinates.
    ///
    /// The backend returns the logical start of the visual cluster under the point.
    /// Whitespace is hit like any other cluster. Positions outside the line return the boundary at
    /// that visual edge in `Err`.
    ///
    /// See also [`Self::closest_byte_index_for_pixel_point`].
    pub fn byte_index_for_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.layout
            .platform_layout
            .byte_index_from_pixel_point(pixel_point, line_height)
    }

    /// Returns the UTF-8 byte index of the cluster boundary closest to a point.
    ///
    /// Use this for a click or drag that should always land on the nearest character,
    /// even when the point is outside the text. For right-to-left clusters, the visual
    /// left edge maps to the logical end and the visual right edge maps to the logical
    /// start. Zero-width clusters share a stop with an adjacent cluster, and visual order
    /// breaks the tie.
    ///
    /// # Errors
    ///
    /// Returns `Err` with the index when the point is outside a visual line, and `Ok`
    /// otherwise. Both cases give an index, so use either value.
    pub fn closest_byte_index_for_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self.closest_caret_for_pixel_point(pixel_point, line_height)
            .map(|caret| caret.index)
            .map_err(|caret| caret.index)
    }

    /// Returns the closest backend-native caret for a point in GPUI layout coordinates.
    ///
    /// Returns `Err` with the caret at that edge when the point is outside a visual line,
    /// the same way as [`Self::closest_byte_index_for_pixel_point`].
    pub fn closest_caret_for_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<CaretPosition, CaretPosition> {
        self.layout
            .platform_layout
            .caret_from_pixel_point(pixel_point, line_height)
    }

    /// Returns the top-left corner of the caret for a UTF-8 byte index, or `None` if the index is past the end.
    ///
    /// The position is on the cluster boundary, at the visual edge that matches the text direction.
    /// An index inside an atomic cluster snaps to the start of that cluster. On a soft-wrap
    /// boundary, the index places the caret at the start of the following visual line.
    pub fn visual_position_for_byte_index(
        &self,
        byte_index: usize,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        if byte_index > self.len() {
            return None;
        }

        self.visual_position_for_caret(
            CaretPosition::attached_to_next_cluster(byte_index),
            line_height,
        )
    }

    /// Returns the top-left corner of the caret for `caret`, or `None` if the caret has no position.
    pub fn visual_position_for_caret(
        &self,
        caret: CaretPosition,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        self.layout
            .platform_layout
            .caret_bounds(caret, line_height)
            .map(|bounds| bounds.origin)
    }

    /// Moves a caret onto a valid cluster boundary, keeping its affinity when it can.
    pub fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
        self.layout.platform_layout.normalized_caret(caret)
    }

    /// Returns the next caret stop to the left or right of `caret` on screen.
    ///
    /// Returns `None` when there is no caret stop in that direction.
    pub fn adjacent_visual_caret(
        &self,
        caret: CaretPosition,
        direction: VisualDirection,
    ) -> Option<CaretPosition> {
        self.layout
            .platform_layout
            .adjacent_visual_caret(caret, direction)
    }

    /// Returns the byte range of the cluster before `caret` in logical order.
    pub fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.layout.platform_layout.logical_cluster_before(caret)
    }

    /// Returns the byte range of the cluster after `caret` in logical order.
    pub fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
        self.layout.platform_layout.logical_cluster_after(caret)
    }

    /// Returns where the caret lands after `movement`, without changing any state.
    ///
    /// Pass the `vertical_navigation_x` from the previous move to keep the caret's column
    /// across consecutive up or down moves. The result carries the new value.
    pub fn caret_movement(
        &self,
        caret: CaretPosition,
        movement: TextMovement,
        vertical_navigation_x: Option<Pixels>,
    ) -> CaretMovement {
        self.layout
            .platform_layout
            .caret_movement(caret, movement, vertical_navigation_x)
    }

    /// Returns where a selection lands after `movement`, using visual order.
    ///
    /// Without `extend`, a left or right cluster or word move on a non-empty selection collapses
    /// it to the visual edge in that direction. Other moves start from the caret. With `extend`, the
    /// anchor stays fixed and the caret moves.
    ///
    /// - `selection` is the current anchor and caret.
    /// - `movement` gives the direction and the unit.
    /// - `extend` keeps the anchor fixed when `true`, and collapses the selection when `false`.
    /// - `vertical_navigation_x` carries the column across vertical moves.
    /// - `line_height` converts caret positions into comparable visual coordinates.
    pub fn selection_movement(
        &self,
        selection: CaretSelection,
        movement: TextMovement,
        extend: bool,
        vertical_navigation_x: Option<Pixels>,
        line_height: Pixels,
    ) -> CaretSelectionMovement {
        let forward = movement.direction == TextDirection::Right;
        let horizontal = matches!(
            movement.direction,
            TextDirection::Left | TextDirection::Right
        ) && matches!(
            movement.boundary,
            TextBoundary::Cluster | TextBoundary::Word
        );

        if !extend && !selection.is_empty() && horizontal {
            let caret_visual_position =
                self.visual_position_for_caret(selection.caret, line_height);
            let anchor_visual_position =
                self.visual_position_for_caret(selection.anchor, line_height);
            let (visual_start, visual_end) = caret_visual_position
                .zip(anchor_visual_position)
                .map(|(caret_visual_position, anchor_visual_position)| {
                    if (caret_visual_position.y, caret_visual_position.x)
                        <= (anchor_visual_position.y, anchor_visual_position.x)
                    {
                        (selection.caret, selection.anchor)
                    } else {
                        (selection.anchor, selection.caret)
                    }
                })
                .unwrap_or_else(|| {
                    if selection.caret.index <= selection.anchor.index {
                        (selection.caret, selection.anchor)
                    } else {
                        (selection.anchor, selection.caret)
                    }
                });

            let caret = if forward { visual_end } else { visual_start };

            return CaretSelectionMovement {
                result: caret.into(),
                vertical_navigation_x: None,
            };
        }

        let CaretMovement {
            result: caret,
            vertical_navigation_x,
        } = self.caret_movement(selection.caret, movement, vertical_navigation_x);

        CaretSelectionMovement {
            result: if extend {
                selection.with_caret(caret)
            } else {
                caret.into()
            },
            vertical_navigation_x,
        }
    }

    /// Selects the word or line at a point in GPUI layout coordinates.
    pub fn selection_from_pixel_point(
        &self,
        pixel_point: Point<Pixels>,
        line_height: Pixels,
        kind: TextSelectionKind,
    ) -> Range<usize> {
        self.layout
            .platform_layout
            .selection_from_pixel_point(pixel_point, line_height, kind)
    }

    /// Returns rectangles covering all selected clusters in visual order.
    pub fn selection_bounds(
        &self,
        byte_range: Range<usize>,
        line_height: Pixels,
    ) -> SmallVec<[Bounds<Pixels>; 4]> {
        let mut result = SmallVec::new();

        if byte_range.is_empty() {
            return result;
        }

        result.extend(
            self.layout
                .platform_layout
                .selection_bounds(byte_range, line_height),
        );
        result
    }
}

impl std::ops::Deref for ShapedTextLayout {
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
    lines: FrameLayouts<CacheKey, LineLayout>,
    shaped_texts: FrameLayouts<CacheKey, ShapedTextLayout>,
    inline_layouts: FrameLayouts<InlineCacheKey, InlineLayout>,
}

struct FrameLayouts<Key, Value> {
    entries: FxHashMap<Arc<Key>, Arc<Value>>,
    used: Vec<Arc<Key>>,
}

impl<Key, Value> Default for FrameLayouts<Key, Value> {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
            used: Vec::new(),
        }
    }
}

impl<Key, Value> FrameLayouts<Key, Value>
where
    Key: Eq + Hash,
{
    fn get<Query>(&self, key: &Query) -> Option<&Arc<Value>>
    where
        Arc<Key>: Borrow<Query>,
        Query: Eq + Hash + ?Sized,
    {
        self.entries.get(key)
    }

    fn remove_entry<Query>(&mut self, key: &Query) -> Option<(Arc<Key>, Arc<Value>)>
    where
        Arc<Key>: Borrow<Query>,
        Query: Eq + Hash + ?Sized,
    {
        self.entries.remove_entry(key)
    }

    fn insert(&mut self, key: Arc<Key>, value: Arc<Value>) {
        self.entries.insert(key.clone(), value);
        self.used.push(key);
    }

    fn reuse(&mut self, previous: &mut Self, range: Range<usize>) {
        for key in &previous.used[range] {
            if let Some(layout) = previous.entries.remove(key) {
                self.entries.insert(key.clone(), layout);
            }

            self.used.push(key.clone());
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.used.clear();
    }
}

fn repaint_line_layout(layout: Arc<LineLayout>, runs: &[TextRun]) -> Arc<LineLayout> {
    let paint_matches = layout.paint_fragments.iter().all(|fragment| {
        runs.get(fragment.source_run)
            .is_some_and(|run| fragment.style == PaintStyle::from(run))
    });

    if paint_matches {
        return layout;
    }

    let mut repainted = (*layout).clone();

    for fragment in &mut repainted.paint_fragments {
        if let Some(run) = runs.get(fragment.source_run) {
            fragment.style = PaintStyle::from(run);
        }
    }

    Arc::new(repainted)
}

fn repaint_shaped_text_layout(
    layout: Arc<ShapedTextLayout>,
    runs: &[TextRun],
) -> Arc<ShapedTextLayout> {
    let repainted = repaint_line_layout(layout.layout.clone(), runs);

    if Arc::ptr_eq(&repainted, &layout.layout) {
        return layout;
    }

    Arc::new(ShapedTextLayout {
        layout: repainted,
        wrap_width: layout.wrap_width,
    })
}

#[derive(Clone, Default)]
pub(crate) struct LineLayoutIndex {
    lines_index: usize,
    shaped_texts_index: usize,
    inline_layouts_index: usize,
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
            lines_index: frame.lines.used.len(),
            shaped_texts_index: frame.shaped_texts.used.len(),
            inline_layouts_index: frame.inline_layouts.used.len(),
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

        current_frame.lines.reuse(
            &mut previous_frame.lines,
            range.start.lines_index..range.end.lines_index,
        );
        current_frame.shaped_texts.reuse(
            &mut previous_frame.shaped_texts,
            range.start.shaped_texts_index..range.end.shaped_texts_index,
        );
        current_frame.inline_layouts.reuse(
            &mut previous_frame.inline_layouts,
            range.start.inline_layouts_index..range.end.inline_layouts_index,
        );
    }

    pub fn truncate_layouts(&self, index: LineLayoutIndex) {
        let mut current_frame = &mut *self.current_frame.write();
        current_frame.lines.used.truncate(index.lines_index);
        current_frame
            .shaped_texts
            .used
            .truncate(index.shaped_texts_index);
        current_frame
            .inline_layouts
            .used
            .truncate(index.inline_layouts_index);
    }

    pub fn finish_frame(&self) {
        let mut prev_frame = self.previous_frame.lock();
        let mut curr_frame = self.current_frame.write();
        std::mem::swap(&mut *prev_frame, &mut *curr_frame);
        curr_frame.lines.clear();
        curr_frame.shaped_texts.clear();
        curr_frame.inline_layouts.clear();
    }

    pub fn layout_text_with_options<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[TextRun],
        options: TextLayoutOptions,
    ) -> Arc<ShapedTextLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        self.sync_font_generation();
        let wrap_width = options.wrap_width;
        let shaping_runs = runs.iter().map(ShapingRun::from).collect::<SmallVec<_>>();
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs: &shaping_runs,
            options,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.shaped_texts.get(key) {
            return repaint_shaped_text_layout(layout.clone(), runs);
        }

        let previous_frame_entry = self.previous_frame.lock().shaped_texts.remove_entry(key);
        if let Some((key, layout)) = previous_frame_entry {
            let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
            current_frame.shaped_texts.insert(key, layout.clone());
            repaint_shaped_text_layout(layout, runs)
        } else {
            drop(current_frame);
            let text = SharedString::from(text);
            let document_layout = if options != TextLayoutOptions::default() || text.contains('\n')
            {
                Arc::new(self.platform_text_system.layout_text(TextLayoutRequest {
                    text: &text,
                    font_size,
                    runs,
                    options,
                }))
            } else {
                self.layout_line::<&SharedString>(&text, font_size, runs)
            };

            let layout = Arc::new(ShapedTextLayout {
                layout: document_layout,
                wrap_width,
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: shaping_runs,
                options,
            });

            let mut current_frame = self.current_frame.write();
            current_frame.shaped_texts.insert(key, layout.clone());

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
        let shaping_runs = runs.iter().map(ShapingRun::from).collect::<SmallVec<_>>();
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs: &shaping_runs,
            options: TextLayoutOptions::default(),
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.lines.get(key) {
            return repaint_line_layout(layout.clone(), runs);
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
        if let Some((key, layout)) = self.previous_frame.lock().lines.remove_entry(key) {
            current_frame.lines.insert(key, layout.clone());
            repaint_line_layout(layout, runs)
        } else {
            let text = SharedString::from(text);
            let layout = self.platform_text_system.layout_text(TextLayoutRequest {
                text: &text,
                font_size,
                runs,
                options: TextLayoutOptions::default(),
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: shaping_runs,
                options: TextLayoutOptions::default(),
            });
            let layout = Arc::new(layout);
            current_frame.lines.insert(key, layout.clone());
            layout
        }
    }

    pub fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> Arc<InlineLayout> {
        self.sync_font_generation();
        let key = &request as &dyn AsInlineCacheKeyRef;
        let current_frame = self.current_frame.upgradable_read();

        if let Some(layout) = current_frame.inline_layouts.get(key) {
            return layout.clone();
        }

        let previous_frame_entry = self.previous_frame.lock().inline_layouts.remove_entry(key);

        if let Some((key, layout)) = previous_frame_entry {
            let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
            current_frame.inline_layouts.insert(key, layout.clone());

            return layout;
        }

        drop(current_frame);
        let layout = Arc::new(self.platform_text_system.layout_inline(request));
        let key = Arc::new(InlineCacheKey::from(request));

        let mut current_frame = self.current_frame.write();
        current_frame.inline_layouts.insert(key, layout.clone());

        layout
    }
}

trait AsInlineCacheKeyRef {
    fn as_inline_cache_key_ref(&self) -> InlineLayoutRequest<'_>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct InlineCacheKey {
    text: SharedString,
    runs: SmallVec<[TextRun; 1]>,
    text_styles: Vec<InlineTextStyle>,
    boxes: Vec<InlineBoxRequest>,
    font_size: Pixels,
    line_height: Pixels,
    text_metrics: InlineTextMetrics,
    options: TextLayoutOptions,
    bidi_scopes: Vec<InlineBidiScope>,
}

impl From<InlineLayoutRequest<'_>> for InlineCacheKey {
    fn from(request: InlineLayoutRequest<'_>) -> Self {
        Self {
            text: SharedString::new(request.text),
            runs: SmallVec::from(request.runs),
            text_styles: request.text_styles.to_vec(),
            boxes: request.boxes.to_vec(),
            font_size: request.font_size,
            line_height: request.line_height,
            text_metrics: request.text_metrics,
            options: request.options,
            bidi_scopes: request.bidi_scopes.to_vec(),
        }
    }
}

impl AsInlineCacheKeyRef for InlineCacheKey {
    fn as_inline_cache_key_ref(&self) -> InlineLayoutRequest<'_> {
        InlineLayoutRequest {
            text: &self.text,
            runs: &self.runs,
            text_styles: &self.text_styles,
            boxes: &self.boxes,
            font_size: self.font_size,
            line_height: self.line_height,
            text_metrics: self.text_metrics,
            options: self.options,
            bidi_scopes: &self.bidi_scopes,
        }
    }
}

impl AsInlineCacheKeyRef for InlineLayoutRequest<'_> {
    fn as_inline_cache_key_ref(&self) -> InlineLayoutRequest<'_> {
        *self
    }
}

impl PartialEq for dyn AsInlineCacheKeyRef + '_ {
    fn eq(&self, other: &dyn AsInlineCacheKeyRef) -> bool {
        self.as_inline_cache_key_ref() == other.as_inline_cache_key_ref()
    }
}

impl Eq for dyn AsInlineCacheKeyRef + '_ {}

impl Hash for dyn AsInlineCacheKeyRef + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_inline_cache_key_ref().hash(state);
    }
}

impl<'a> Borrow<dyn AsInlineCacheKeyRef + 'a> for Arc<InlineCacheKey> {
    fn borrow(&self) -> &(dyn AsInlineCacheKeyRef + 'a) {
        self.as_ref() as &dyn AsInlineCacheKeyRef
    }
}

trait AsCacheKeyRef {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_>;
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CacheKey {
    text: SharedString,
    font_size: Pixels,
    runs: SmallVec<[ShapingRun; 1]>,
    options: TextLayoutOptions,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: Pixels,
    runs: &'a [ShapingRun],
    options: TextLayoutOptions,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ShapingRun {
    len: usize,
    font: crate::Font,
    letter_spacing: Option<Pixels>,
}

impl From<&TextRun> for ShapingRun {
    fn from(run: &TextRun) -> Self {
        Self {
            len: run.len,
            font: run.font.clone(),
            letter_spacing: run.letter_spacing,
        }
    }
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
            options: self.options,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, px, size};

    #[test]
    fn caret_selection_endpoint_ordering_uses_byte_indices() {
        for (caret, anchor, expected) in [
            (
                CaretPosition::attached_to_next_cluster(2),
                CaretPosition::attached_to_next_cluster(5),
                std::cmp::Ordering::Less,
            ),
            (
                CaretPosition::attached_to_next_cluster(5),
                CaretPosition::attached_to_previous_cluster(5),
                std::cmp::Ordering::Equal,
            ),
            (
                CaretPosition::attached_to_previous_cluster(8),
                CaretPosition::attached_to_next_cluster(5),
                std::cmp::Ordering::Greater,
            ),
        ] {
            let selection = CaretSelection { anchor, caret };
            assert_eq!(selection.endpoint_ordering(), expected);
        }
    }

    #[test]
    fn paragraph_separators_match_unicode_bidi_class_b() {
        for character in [
            '\n', '\r', '\u{001c}', '\u{001d}', '\u{001e}', '\u{0085}', '\u{2029}',
        ] {
            assert!(is_paragraph_separator(character));
        }

        for character in [' ', '\u{2028}'] {
            assert!(!is_paragraph_separator(character));
        }
    }

    #[test]
    fn inserted_text_affinity_tracks_trailing_paragraph_separators() {
        for (text, expected) in [
            ("", CaretAffinity::Downstream),
            ("ordinary text", CaretAffinity::Upstream),
            ("\n", CaretAffinity::Downstream),
            ("\r", CaretAffinity::Downstream),
            ("\u{001c}", CaretAffinity::Downstream),
            ("\u{001d}", CaretAffinity::Downstream),
            ("\u{001e}", CaretAffinity::Downstream),
            ("\u{0085}", CaretAffinity::Downstream),
            ("\u{2028}", CaretAffinity::Downstream),
            ("\u{2029}", CaretAffinity::Downstream),
            ("\nordinary text", CaretAffinity::Upstream),
        ] {
            assert_eq!(CaretAffinity::from(text), expected, "From({text:?})");
            assert_eq!(
                CaretAffinity::for_inserted_text(text),
                expected,
                "for_inserted_text({text:?})"
            );
        }
    }

    #[test]
    fn inline_box_alignment_preserves_unsorted_boxes_and_empty_lines() {
        let mut lines = [InlineVisualLine::default(); 3];
        let mut boxes =
            [(20, 2, 14.), (10, 0, 18.), (90, 3, 100.)].map(|(id, line_index, height)| {
                PositionedInlineBox {
                    id,
                    line_index,
                    bounds: Bounds::new(point(px(7.), px(99.)), size(px(5.), px(height))),
                }
            });
        let mut layout_size = size(px(100.), px(999.));
        let original_lines = lines;
        let original_boxes = boxes;

        let requests = [InlineBoxRequest {
            id: 20,
            index: 0,
            size: boxes[0].bounds.size,
            vertical_align: VerticalAlign::Top,
        }];
        let metrics = InlineTextMetrics {
            ascent: px(8.),
            descent: px(2.),
            x_height: px(4.),
        };

        align_inline_boxes(
            &mut lines,
            &mut boxes,
            &mut layout_size,
            &requests,
            &[metrics],
            &[(px(-7.), px(3.)), (px(-11.), px(4.))],
            metrics,
            px(12.),
        );

        for ((line, mut expected_line), (origin_y, height, baseline)) in lines
            .iter()
            .zip(original_lines)
            .zip([(0., 21., 18.), (21., 15., 11.), (36., 14., 9.)])
        {
            expected_line.origin.y = px(origin_y);
            expected_line.size.height = px(height);
            expected_line.baseline = px(baseline);
            assert_eq!(*line, expected_line);
        }

        for ((inline_box, mut expected_box), origin_y) in
            boxes.iter().zip(original_boxes).zip([36., 0., 99.])
        {
            expected_box.bounds.origin.y = px(origin_y);
            assert_eq!(*inline_box, expected_box);
        }

        assert_eq!(layout_size, size(px(100.), px(50.)));
    }
}
