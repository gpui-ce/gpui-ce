use crate::{Pixels, Point, ResolvedDirection, Size, TextLayoutOptions, TextRun, UnicodeBidi};
use std::ops::Range;

/// A directional scope within an inline text document.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InlineBidiScope {
    /// UTF-8 source range covered by this scope.
    pub range: Range<usize>,
    /// Resolved direction used by the scope.
    pub direction: ResolvedDirection,
    /// The CSS `unicode-bidi` behavior for the scope.
    pub unicode_bidi: UnicodeBidi,
}

/// An atomic element inserted at a UTF-8 boundary in an inline document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InlineBoxRequest {
    /// Identifier returned with the positioned box.
    pub id: u64,
    /// UTF-8 byte index at which to insert the box.
    pub index: usize,
    /// Measured size of the element.
    pub size: Size<Pixels>,
    /// Vertical alignment within the line containing this element.
    pub vertical_align: crate::VerticalAlign,
}

/// Resolved font size and line height for part of an inline document.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InlineTextStyle {
    /// UTF-8 range receiving these resolved metrics.
    pub range: Range<usize>,
    /// Font size for this range.
    pub font_size: Pixels,
    /// Absolute line height for this range.
    pub line_height: Pixels,
}

/// Complete input for a document containing text and element boxes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InlineLayoutRequest<'a> {
    /// UTF-8 source text.
    pub text: &'a str,
    /// Complete shaping and paint styles covering `text`.
    pub runs: &'a [TextRun],
    /// Resolved font sizes and line heights within the document.
    pub text_styles: &'a [InlineTextStyle],
    /// Atomic boxes inserted into the text.
    pub boxes: &'a [InlineBoxRequest],
    /// Base font size.
    pub font_size: Pixels,
    /// Requested height of an ordinary text row.
    pub line_height: Pixels,
    /// Metrics for the inline container's base font.
    pub text_metrics: InlineTextMetrics,
    /// Wrapping, alignment, and bidirectional layout options.
    pub options: TextLayoutOptions,
    /// Directional scopes established by nested inline elements.
    pub bidi_scopes: &'a [InlineBidiScope],
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
    /// The row size before it is placed in its containing element.
    pub size: Size<Pixels>,
    /// The baseline of the row, measured down from `origin.y`.
    pub baseline: Pixels,
}
