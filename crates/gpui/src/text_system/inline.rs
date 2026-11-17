#[cfg(any(test, feature = "test-support"))]
use crate::VerticalAlign;
use crate::{
    Font, Pixels, Point, ResolvedDirection, Size, TextLayoutOptions, TextRun, UnicodeBidi,
};
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

#[cfg(any(test, feature = "test-support"))]
impl InlineBoxRequest {
    pub(crate) fn y_offset(
        &self,
        baseline: Pixels,
        line_height: Pixels,
        x_height: Pixels,
    ) -> Pixels {
        match self.vertical_align {
            VerticalAlign::Baseline => baseline - self.size.height,
            VerticalAlign::Middle => baseline - x_height / 2. - self.size.height / 2.,
            VerticalAlign::Top => Pixels::ZERO,
            VerticalAlign::Bottom => line_height - self.size.height,
        }
    }
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
    /// Font of the inline container, used for its line strut and box alignment.
    pub base_font: &'a Font,
    /// Wrapping, alignment, and bidirectional layout options.
    pub options: TextLayoutOptions,
    /// Directional scopes established by nested inline elements.
    pub bidi_scopes: &'a [InlineBidiScope],
}

#[cfg(any(test, feature = "test-support"))]
impl InlineLayoutRequest<'_> {
    pub(crate) fn line_extents(
        &self,
        ascent: Pixels,
        descent: Pixels,
        x_height: Pixels,
    ) -> (Pixels, Pixels) {
        let half_leading = (self.line_height - ascent - descent) / 2.;
        let mut top = -ascent - half_leading;
        let mut bottom = descent + half_leading;
        let mut top_height = Pixels::ZERO;
        let mut bottom_height = Pixels::ZERO;

        for inline_box in self.boxes {
            let height = inline_box.size.height;

            match inline_box.vertical_align {
                VerticalAlign::Baseline => top = top.min(-height),
                VerticalAlign::Middle => {
                    top = top.min(-x_height / 2. - height / 2.);
                    bottom = bottom.max(-x_height / 2. + height / 2.);
                }
                VerticalAlign::Top => top_height = top_height.max(height),
                VerticalAlign::Bottom => bottom_height = bottom_height.max(height),
            }
        }

        bottom = bottom.max(top + top_height);
        top = top.min(bottom - bottom_height);

        (top, bottom)
    }
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
