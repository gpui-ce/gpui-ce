use gpui::{Bounds, Pixels, Point, Size, WrappedLine};
use std::{ops::Range, sync::Arc};

/// Data used across successive layout requests to gauge whether layout must be recomputed.
#[derive(Default, Clone, Copy)]
pub(super) struct EditableTextLayoutState {
    /// The last known width at which the lines were wrapped.
    pub wrap_width: Option<Pixels>,
    /// The last known size of the text, as generated during layout.
    pub size: Option<Size<Pixels>>,
    /// The last seen version of `storage` (for tracking when lines need to be reprocessed during layout)
    pub last_seen_storage_version: u16,
}

/// Internal state/result after the element has recomputed layout.
#[derive(Default)]
pub(super) struct EditableTextLayoutResult {
    /// Whether the element supports multiple lines of text
    pub supports_multiline: bool,
    /// Whether the element is currently accepting inputs
    pub accepts_input: bool,
    /// The last seen scroll position and size of the element
    pub scroll_bounds: Bounds<Pixels>,
    pub state: EditableTextLayoutState,
    /// The `ShapedLine` produced by the painter's `prepaint`.
    /// Cached so IME `bounds_for_range` / `character_index_for_point` can evaluate without re-shaping.
    pub lines: Vec<TextLineSegment>,
    pub line_height: Pixels,
    /// The next position the scroll view should move to.
    /// Set by the state in response to user actions.
    pub next_scroll_offset: Option<Point<Pixels>>,
}

/// A segment of text that is a single logical/document line but can take up multiple rows due to wrapping.
pub(super) struct TextLineSegment {
    /// The utf8 byte range in the content string that this line covers.
    pub text_range: Range<usize>,
    /// The shaped and wrapped text for this line, if available.
    pub wrapped_line: Option<Arc<WrappedLine>>,

    /// The y-coordinate of this segment which can be multiplied by the line_height
    /// to get its pixel location relative to the bounds of the text area.
    pub pos_y: usize,
}

impl TextLineSegment {
    /// The number of visual lines this segment encapsulates,
    /// since it can occupy multiple rows due to wrapping.
    pub fn row_count(&self) -> usize {
        let count = self
            .wrapped_line
            .as_ref()
            .map(|line| line.wrap_boundaries().len());
        count.unwrap_or_default() + 1
    }

    /// Returns true if the line contains a given position (e.g. for finding the line containing the caret).
    /// If `includes_end` is true, the end of the line is treated as inclusive instead of exclusive.
    pub fn contains_position(&self, pos: usize, include_end: bool) -> bool {
        if self.text_range.is_empty() {
            return pos == self.text_range.start;
        }

        if include_end {
            (self.text_range.start..=self.text_range.end).contains(&pos)
        } else {
            self.text_range.contains(&pos)
        }
    }
}
