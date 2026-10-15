use gpui::{Bounds, Pixels, Point, ShapedText, SharedString, Size, TextStyle};
use std::sync::Arc;

/// Data used across successive layout requests to gauge whether layout must be recomputed.
#[derive(Default, Clone, Copy)]
pub(super) struct EditableTextLayoutState {
    /// The last known size of the text, as generated during layout.
    pub size: Option<Size<Pixels>>,
    /// The last seen version of `storage` (for tracking when lines need to be reprocessed during layout)
    pub last_seen_storage_version: u16,
    /// Direction and alignment inputs used for this layout.
    pub options: gpui::TextLayoutOptions,
}

/// The inputs, besides storage version and layout options, that shaped the cached document.
/// Any change to them must reshape the text rather than reuse the cached size.
#[derive(Clone, PartialEq)]
pub(super) struct EditableTextLayoutKey {
    /// The text style with the effective (placeholder or content) color applied.
    pub text_style: TextStyle,
    pub source: SharedString,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub supports_multiline: bool,
    pub accepts_input: bool,
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
    /// The document layout produced by the painter's `prepaint`.
    /// Cached so IME `bounds_for_range` / `character_index_for_point` can evaluate without re-shaping.
    pub document: Option<Arc<ShapedText>>,
    pub line_height: Pixels,
    /// Translation that keeps aligned overflow in the scrollable coordinate space.
    pub document_offset: Point<Pixels>,
    /// The next position the scroll view should move to.
    /// Set by the state in response to user actions.
    pub next_scroll_offset: Option<Point<Pixels>>,
    /// The inputs that produced `document`.
    pub layout_key: Option<EditableTextLayoutKey>,
}
