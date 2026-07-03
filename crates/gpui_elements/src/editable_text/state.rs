use crate::editable_text::{
    StringStorage, TextBoundary, UnicodeTextStorage,
    actions::EditableTextActionHandler,
    caret::{Caret, CaretNotify},
    history::EditableTextHistory,
    layout::EditableTextLayoutResult,
};
use gpui::{
    App, Bounds, ClipboardItem, Context, ElementId, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, NavigationDirection, Pixels, Point, UTF16Selection, Window, point,
};
use std::{borrow::Cow, ops::Range};

const CARET_PIXELS_EPSILON: Pixels = gpui::px(4.);

/// The utf-8 character range that is currently selected by the user.
/// Valid both when start < end and start > end (which dictates the direction of the selection).
/// Empty when start==end. The start of this range is always the current position of the caret (input cursor).
/// Diverges from the semantics/expectations of the Range type (since `Range` is incoherent if start > end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaretSelection {
    start: usize,
    end: usize,
}
impl From<usize> for CaretSelection {
    fn from(value: usize) -> Self {
        Self {
            start: value,
            end: value,
        }
    }
}
impl From<(usize, usize)> for CaretSelection {
    fn from((start, end): (usize, usize)) -> Self {
        Self { start, end }
    }
}
impl From<Range<usize>> for CaretSelection {
    fn from(value: Range<usize>) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}
impl Into<(usize, usize)> for CaretSelection {
    fn into(self) -> (usize, usize) {
        (self.start, self.end)
    }
}
impl CaretSelection {
    fn is_empty(&self) -> bool {
        self.start == self.end
    }

    fn range(&self) -> Range<usize> {
        self.start.min(self.end)..self.start.max(self.end)
    }
}

/// Internal state for EditableText elements.
pub struct EditableTextState {
    /// The storage medium backing this element-state. Hypothetically supports both
    /// std String and other crates (e.g. long document text).
    storage: Box<dyn UnicodeTextStorage>,
    /// The caret entity which has internal state for features like blinking
    caret: Entity<Caret>,

    /// The utf-8 character range that is currently selected by the user.
    /// Valid both when start < end and start > end (which dictates the direction of the selection).
    /// Empty when start==end. The start of this range is always the current position of the caret (input cursor).
    /// This means it breaks the semantics/expectations of the Range type.
    ///
    /// NOTE: because each input has its own selection state, its trivial for users to have
    /// multiple selections active across multiple inputs at the same time.
    /// This could be considered undesirable behavior, and could prompt the question of
    /// whether there should be a mechanism to clear selection when focus is lost.
    selected_range: CaretSelection,

    /// The utf-8 character range of `storage` which is being composed by IME
    marked_range: Option<Range<usize>>,

    /// True while the user is in the act of highlighting a section of the text (e.g. during mouse pressed & dragging).
    is_selecting: bool,
    /// The last ui location relative to the element that the user clicked. Used to filter when a user clicks multiple times in the same area.
    last_click_position: Option<Point<Pixels>>,
    /// The number of times the user has clicked `last_click_position`. Used to determine which click behavior to trigger, depending on single, double, or triple clicks.
    click_count: usize,

    focus_handle: FocusHandle,
    history: Option<EditableTextHistory>,

    pub(super) layout_data: EditableTextLayoutResult,
}

impl EventEmitter<CaretNotify> for EditableTextState {}

/// Event emitted when an `EditableTextState` is changed.
///
/// This is not suitable for input sanitation (which should occur before the mutation).
pub struct TextChanged;
impl EventEmitter<TextChanged> for EditableTextState {}

impl Focusable for EditableTextState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AsRef<str> for EditableTextState {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl EditableTextState {
    /// Uses a pre-existing state attached to the element at `key`, as long as the element has existed over consecutive frames.
    /// If the state does not yet exist, a new one is created using the default [`UnicodeTextStorage`] medium.
    pub fn use_keyed(key: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::use_keyed_init(key, window, cx, |_, _| StringStorage::default())
    }

    /// Uses a pre-existing state attached to the element at `key`, as long as the element has existed over consecutive frames.
    /// If the state does not yet exist, a new one is created calling `init` to create a [`UnicodeTextStorage`] medium.
    ///
    /// ```
    /// # use gpui::{RenderOnce, Window, App, IntoElement, ElementId};
    /// # use gpui_elements::editable_text::{EditableTextState, StringStorage, editable_text};
    /// pub struct Form;
    /// impl RenderOnce for Form {
    ///     fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
    ///         let field_a_id = ElementId::from("field_a");
    ///         let field_a = EditableTextState::use_keyed_init(field_a_id.clone(), window, cx,
    ///             |_window, _cx| StringStorage::from("this is some default editable text content"));
    ///         editable_text(field_a_id).state(field_a.downgrade())
    ///     }
    /// }
    /// ```
    pub fn use_keyed_init<F, StorageType>(
        key: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
        init: F,
    ) -> Entity<Self>
    where
        F: 'static + Fn(&mut Window, &mut Context<'_, EditableTextState>) -> StorageType,
        StorageType: 'static + UnicodeTextStorage,
    {
        window.use_keyed_state(key, cx, |window, cx| Self::new(init(window, cx), cx))
    }

    /// Creates a new EditableText state with a given storage medium.
    ///
    /// Does not intrinsicly handle the state being attached to an element
    /// over multiple frames (e.g. via [`RenderOnce`]). Use [`use_keyed`] or [`use_keyed_init`] for that.
    ///
    /// Expected to be called via [`AppContext::new`] such as:
    /// ```
    /// # use gpui::{AppContext, Window, App, Entity};
    /// # use gpui_elements::editable_text::{StringStorage, EditableTextState};
    /// # fn new(_window: &mut Window, cx: &mut App) -> Entity<EditableTextState> {
    /// cx.new(|cx| EditableTextState::new(StringStorage::default(), cx))
    /// # }
    /// ```
    pub fn new(storage: impl UnicodeTextStorage + 'static, cx: &mut Context<Self>) -> Self {
        use gpui::AppContext;
        let caret = cx.new({
            let state_entity = cx.entity();
            move |cx| {
                let mut caret = Caret::default().blink_interval_default();
                caret.subscribe_to(&state_entity, cx);
                caret
            }
        });
        Self {
            storage: Box::new(storage),
            caret,

            selected_range: 0.into(),
            marked_range: None,

            is_selecting: false,
            last_click_position: None,
            click_count: 0,

            focus_handle: cx.focus_handle(),
            // TODO: what is the best way to give users access to configure this via element
            history: Some(EditableTextHistory::default()),

            layout_data: EditableTextLayoutResult::default(),
        }
    }

    /// Returns the current contents of [`storage`] as a string slice.
    pub fn as_str(&self) -> &str {
        self.storage.content_utf8()
    }

    pub fn version(&self) -> u16 {
        self.storage.version()
    }

    /// Replaces the contents of the stored text with the provided string slice.
    pub fn emplace(&mut self, content: &str, cx: &mut Context<Self>) {
        let len = self.storage.content_utf8().len();
        self.replace_text(0..len, content);
        self.emit_text_changed(cx);
        cx.notify();
    }

    /// Returns the utf-8 character range that is currently selected within the current state of the text.
    /// Internally converts the stored direction-aware range into a canonical range.
    pub(super) fn selected_range(&self) -> Range<usize> {
        self.selected_range.range()
    }

    pub(super) fn selection_direction(&self) -> Option<NavigationDirection> {
        match self.selected_range.start.cmp(&self.selected_range.end) {
            std::cmp::Ordering::Less => Some(NavigationDirection::Forward),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(NavigationDirection::Back),
        }
    }

    /// Returns a reference to the entity owning the state of the [`Caret`] (e.g. its blinking state).
    pub(super) fn caret_entity(&self) -> &Entity<Caret> {
        &self.caret
    }

    /// Returns the position of the caret in utf8 character space.
    pub(super) fn caret_pos(&self) -> usize {
        self.selected_range.start
    }

    /// Returns the IME marked range for character operations.
    pub(super) fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }
}

impl EditableTextState {
    /// Validates/sanitizes incoming text according to the rules of the field.
    fn validate_incoming_text<'text>(
        &self,
        _range: &Range<usize>,
        text_to_insert: &'text str,
    ) -> Cow<'text, str> {
        // TODO: Apply text sanitization, ideally using externally-sourced implementations.
        // example optional/opt-in sanitations include:
        // - single-line fields should prune /n & /r
        // - maximum utf8 length
        // - numbers only
        // should also consider validation support, for features such as:
        // - total syntax evaluation (e.g. passwords)
        // - conforms to regex or math (e.g. ssn, phone number, email, etc)
        let mut text_to_insert = Cow::Borrowed(text_to_insert);

        if !self.layout_data.supports_multiline {
            text_to_insert = Cow::Owned(text_to_insert.replace("\n", "").replace("\r", ""));
        }

        /* A sample implementation of max-length sampled from gpuikit
        // Decide the effective new text up front (honouring `max_length`).
        // This avoids the "apply, then truncate" path which would leave the caret past the end.
        let max_length = None::<usize>;
        if let Some(cap) = max_length {
            let existing_len = self.as_str().len() - (range.end - range.start);
            let room = cap.saturating_sub(existing_len);
            text_to_insert = &text_to_insert[..text_to_insert.len().min(room)];
        }
        */

        // for now, this function is no-op
        text_to_insert
    }

    /// Internal method to record historical changes and perform text replacement in storage.
    /// Selection is moved to the end of the inserted text and ime marked range is cleared.
    fn replace_text(&mut self, range: Range<usize>, text_to_insert: &str) {
        let end_pos = range.start + text_to_insert.len();
        self.record_history(range.clone(), text_to_insert.len());
        self.storage.replace_range(range, text_to_insert);
        self.selected_range = end_pos.into();
        self.marked_range = None;
    }

    fn emit_text_changed(&self, cx: &mut Context<Self>) {
        cx.emit(TextChanged);
    }
}

// Screen space (text layout engine output) & String space transformers
impl EditableTextState {
    /// Returns the utf-8 character position of the start of the line that contains the provided pixel-point.
    fn index_for_pixel_point(&self, point: Point<Pixels>, line_height: Pixels) -> usize {
        let storage_len_utf8 = self.storage.content_utf8().len();
        if storage_len_utf8 == 0 {
            return 0;
        }

        for line in &self.layout_data.lines {
            let y_offset = line.pos_y * line_height;
            let line_height_total = line_height * line.row_count() as f32;

            if point.y >= y_offset && point.y < y_offset + line_height_total {
                if line.text_range.is_empty() {
                    return line.text_range.start;
                }
                let Some(wrapped) = &line.wrapped_line else {
                    return line.text_range.start;
                };

                let relative_y = point.y - y_offset;
                let relative_point = gpui::point(point.x, relative_y);

                let closest_result =
                    wrapped.closest_index_for_position(relative_point, line_height);

                let local_idx = closest_result.unwrap_or_else(|closest| closest);
                let clamped = local_idx.min(wrapped.text.len());
                return line.text_range.start + clamped;
            }
        }

        storage_len_utf8
    }

    fn line_index_and_point_at_caret(&self, line_height: Pixels) -> (usize, Point<Pixels>) {
        if self.layout_data.lines.is_empty() {
            return (0, Point::default());
        }

        let pos = self.caret_pos();

        // accumulated vertical line count (not literal lines, since they can be wrapped)
        let mut segment_index = 0;
        for segment in &self.layout_data.lines {
            if segment.text_range.is_empty() {
                if pos == segment.text_range.start {
                    return (segment_index, Point::default());
                }
            }

            if segment.text_range.contains(&pos) {
                if let Some(wrapped) = &segment.wrapped_line {
                    let pos_in_segment = (pos - segment.text_range.start).min(wrapped.text.len());
                    if let Some(point) = wrapped.position_for_index(pos_in_segment, line_height) {
                        let visual_line_within = (point.y / line_height).floor() as usize;
                        return (segment_index + visual_line_within, point);
                    }
                }
                return (segment_index, Point::default());
            }

            segment_index += segment.row_count();
        }

        (segment_index.saturating_sub(1), Point::default())
    }

    fn find_position_in_vertical_direction(
        &self,
        direction: i32,
        line_height: Pixels,
    ) -> Option<usize> {
        let (line_index, point) = self.line_index_and_point_at_caret(line_height);
        let line_index = line_index.saturating_add_signed(direction as isize);

        let mut current_visual_line = 0;
        for segment in &self.layout_data.lines {
            let wrap_boundary_len = segment.row_count();

            if line_index < current_visual_line + wrap_boundary_len {
                let visual_line_within_layout = line_index - current_visual_line;

                if segment.text_range.is_empty() {
                    return Some(segment.text_range.start);
                }

                if let Some(wrapped) = &segment.wrapped_line {
                    let y_within_wrapped = line_height * visual_line_within_layout as f32;
                    let target_point = gpui::point(point.x, y_within_wrapped);

                    let closest_result =
                        wrapped.closest_index_for_position(target_point, line_height);

                    let closest_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = closest_idx.min(wrapped.text.len());
                    let result = segment.text_range.start + clamped;
                    return Some(result);
                }

                return Some(segment.text_range.start);
            }

            current_visual_line += wrap_boundary_len;
        }

        (direction > 0).then(|| self.storage.content_utf8().len())
    }

    fn find_point_for_character_position(&self, character_pos: usize) -> Point<Pixels> {
        let line_height = self.layout_data.line_height;
        let mut row_count = 0;
        for segment in &self.layout_data.lines {
            if !segment.contains_position(character_pos, false) {
                row_count += segment.row_count();
                continue;
            }

            let line_origin = point(Pixels::ZERO, row_count * line_height);
            return match &segment.wrapped_line {
                None => line_origin,
                Some(wrapped) => {
                    let local_offset = character_pos.saturating_sub(segment.text_range.start);
                    let position = wrapped.position_for_index(local_offset, line_height);
                    position.unwrap_or_default() + line_origin
                }
            };
        }
        Point::default()
    }
}

// Internal user action / logical processors
impl EditableTextState {
    fn scroll_to_caret(&mut self) {
        if self.layout_data.scroll_bounds.is_empty() {
            return;
        }
        let Some(content_size) = self.layout_data.state.size else {
            return;
        };

        // point will be relative to content_size, and may or may not be within the current scroll_bounds
        let point = self.find_point_for_character_position(self.caret_pos());

        // this scroll_offset diverges from the rest of gpui, as it is stored in the
        // positive real number space (interactivity stores it in the negatives)
        let mut scroll_offset = Cow::Borrowed(&self.layout_data.scroll_bounds.origin);

        if self.layout_data.scroll_bounds.contains(&point) {
            return;
        }

        // No existing "shift bounds origin so <point> is contained", but that is effectively what this does
        if point.x < self.layout_data.scroll_bounds.left() {
            scroll_offset.to_mut().x = point.x;
        }
        if point.y < self.layout_data.scroll_bounds.top() {
            scroll_offset.to_mut().y = point.y;
        }
        let right = self.layout_data.scroll_bounds.right();
        if point.x > right {
            scroll_offset.to_mut().x += point.x - right;
        }
        let bottom = self.layout_data.scroll_bounds.bottom();
        let point_bottom = point.y + self.layout_data.line_height;
        if point_bottom > bottom {
            let delta = point_bottom - bottom;
            scroll_offset.to_mut().y += delta;
        }

        if let Cow::Owned(mut offset) = scroll_offset {
            offset.x = offset.x.clamp(Pixels::ZERO, content_size.width);
            offset.y = offset.y.clamp(Pixels::ZERO, content_size.height);
            println!("shift to {offset:?}");
            self.layout_data.next_scroll_offset = Some(offset);
        }
    }

    /// Moves the caret to the provided position.
    ///
    /// Will cause the current scroll position/offset to update on the next frame,
    /// if the line the carent is on is out of view.
    pub fn move_to(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        cx.emit(CaretNotify::PauseBlinking);
        let caret_pos = caret_pos.min(self.storage.content_utf8().len());
        self.selected_range = caret_pos.into();
        self.scroll_to_caret();
        cx.notify();
    }

    /// Changes the current selection to extend to the provided position.
    ///
    /// Will cause the current scroll position/offset to update on the next frame,
    /// if the line the carent is on is out of view.
    pub fn select_to(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        cx.emit(CaretNotify::PauseBlinking);
        let caret_pos = caret_pos.min(self.as_str().len());
        self.selected_range.start = caret_pos;
        self.scroll_to_caret();
        cx.notify();
    }

    /// Removes a chunk of text at the cursor/selection.
    /// No-op if the element is currently not accepting input.
    ///
    /// If there is a selection of multiple characters, the slice of text represented
    /// by range is replaced with an empty string.
    /// If there is no selection, `direction` and `boundary` are used to determine the slice of text to remove.
    ///
    /// [`NavigationDirection::Back`] represents scanning earlier in the text string from the caret.
    ///
    /// [`NavigationDirection::Forward`] represents scanning later in the text string from the caret.
    ///
    /// [`TextBoundary`] describes how far to jump from the caret.
    pub fn delete_linear(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        cx: &mut Context<Self>,
    ) {
        if !self.layout_data.accepts_input {
            return;
        }

        let range = self.selected_range();
        let range = match range.is_empty() {
            false => range,
            true => self
                .storage
                .range_from_caret(self.caret_pos(), direction, boundary),
        };
        let storage_len_utf8 = self.storage.content_utf8().len();
        let start = range.start.min(storage_len_utf8);
        let end = range.end.max(start).min(storage_len_utf8);

        self.replace_text(start..end, "");

        self.emit_text_changed(cx);
        cx.notify();
    }

    /// Moves the caret somewhere relative to its current location, according to `direction` and `boundary`.
    ///
    /// If there is currently a selection, the cursor will jump to the start/end of that selection based on `direction`.
    ///
    /// [`NavigationDirection::Back`] represents scanning earlier in the text string from the current caret.
    ///
    /// [`NavigationDirection::Forward`] represents scanning later in the text string from the current caret.
    ///
    /// [`TextBoundary`] describes how far to jump from the current caret
    pub fn nav_linear(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        cx: &mut Context<Self>,
    ) {
        let caret_pos = match self.selected_range.is_empty() {
            false => match direction {
                NavigationDirection::Back => self.selected_range.start,
                NavigationDirection::Forward => self.selected_range.end,
            },
            true => self
                .storage
                .offset_from_caret(self.caret_pos(), direction, boundary),
        };
        self.move_to(caret_pos, cx);
    }

    /// Sets the current selection to be the entire text in the storage medium
    pub fn select_document(&mut self, cx: &mut Context<Self>) {
        self.selected_range = (0, self.storage.content_utf8().len()).into();
        cx.notify();
    }

    /// Extends the current selection to include some amount of textrelative the current
    /// location of the caret, according to `direction` and `boundary`.
    ///
    /// [`NavigationDirection::Back`] represents scanning earlier in the text string from the current caret.
    ///
    /// [`NavigationDirection::Forward`] represents scanning later in the text string from the current caret.
    ///
    /// [`TextBoundary`] describes how far to jump from the current caret
    pub fn select_linear(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        cx: &mut Context<Self>,
    ) {
        let caret_pos = self
            .storage
            .offset_from_caret(self.caret_pos(), direction, boundary);
        self.select_to(caret_pos, cx);
    }

    /// Updates the mouse-click tracker so we can detect when a mouse click results in different actions.
    fn apply_click(&mut self, click_count: usize, text_position: Point<Pixels>) {
        let should_continue_click =
            click_count > 1 && self.is_position_nearly_at_previous_click(text_position);
        self.click_count = if should_continue_click {
            click_count
        } else {
            1
        };
        self.last_click_position = Some(text_position);
    }

    fn is_position_nearly_at_previous_click(&self, point: Point<Pixels>) -> bool {
        match self.last_click_position {
            None => false,
            Some(previous_pos) => point.is_nearly_eq(&previous_pos, CARET_PIXELS_EPSILON),
        }
    }

    fn select_word_at(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        self.selected_range = self.storage.word_range_at(caret_pos).into();
        cx.notify();
    }

    fn select_line_at(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        use NavigationDirection::*;
        use TextBoundary::*;

        let line_start = self.storage.offset_from_caret(caret_pos, Back, Line);
        let line_end = self.storage.offset_from_caret(caret_pos, Forward, Line);
        let line_end_with_newline = if line_end < self.storage.content_utf8().len() {
            line_end + 1
        } else {
            line_end
        };
        self.selected_range = (line_start, line_end_with_newline).into();
        cx.notify();
    }
}

// History management
impl EditableTextState {
    /// Returns the history log of the element, which is the data that supports undo/redo operations.
    pub fn history(&self) -> Option<&EditableTextHistory> {
        self.history.as_ref()
    }

    fn record_history(&mut self, range: Range<usize>, new_text_len: usize) {
        // Don't record during IME composition
        if self.marked_range.is_some() {
            return;
        }

        let Some(history) = &mut self.history else {
            return;
        };

        // Capture the text that will be replaced
        let old_text = &self.storage.content_utf8()[range.clone()];
        history.record(range, old_text, new_text_len, self.selected_range.into());
    }

    fn apply_from_history(&mut self, src: HistoryKind, dst: HistoryKind, cx: &mut Context<Self>) {
        let Some(history) = &mut self.history else {
            return;
        };
        let Some(entry) = history.take(src) else {
            return;
        };

        let range = entry.char_range(self.storage.content_utf8().len());
        // Snapshot the sub-slice that is being replaced
        let removed_text = self.storage.content_utf8()[range.clone()].to_string();

        // Replace the slice with the history value
        self.storage.replace_range(range, &entry.old_text);
        self.selected_range = entry.selected_range.into();

        // Push the entry onto the redo stack so the undo can be undone
        history.push(dst, entry.as_inverted(removed_text));

        self.scroll_to_caret();
        cx.notify();
    }
}

impl EditableTextState {
    fn ime_resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        // Use a series of fallbacks to pick the range to operate on.
        // Fallback order: IME provided range, active IME marked range, selection
        let range = range_utf16.map(|range_utf16| self.storage.utf_range_16to8(&range_utf16));
        let range = range.or_else(|| self.marked_range.clone());
        let range = range.unwrap_or_else(|| self.selected_range());

        let storage_len_utf8 = self.as_str().len();
        range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8)
    }

    fn ime_mark_text_in_range(&mut self, range: &Range<usize>, text_len: usize) {
        self.marked_range = match text_len {
            0 => None,
            _ => Some(range.start..range.start + text_len),
        };
    }

    fn ime_mark_selected_range(
        &mut self,
        range_overwritten: &Range<usize>,
        new_selected_range_utf16: &Option<Range<usize>>,
        text_len: usize,
    ) {
        // NOTE: Differs from yororen-ui
        // https://github.com/MeowLynxSea/yororen-ui/blob/346502ac654b77fdaff3be2d7444fca8783acfc9/crates/yororen-ui-core/src/headless/text_input_core.rs#L359-L371
        self.selected_range = {
            let new_range = new_selected_range_utf16.as_ref();
            let new_range = new_range.map(|range_utf16| self.storage.utf_range_16to8(range_utf16));
            let new_range = new_range.map(|new_range| {
                new_range.start + range_overwritten.start..new_range.end + range_overwritten.start
            });
            let new_range = new_range.unwrap_or_else(|| {
                range_overwritten.start + text_len..range_overwritten.start + text_len
            });
            new_range.into()
        };
    }
}

// IME handler
impl EntityInputHandler for EditableTextState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.storage.utf_range_16to8(&range_utf16);
        let storage_len_utf8 = self.storage.content_utf8().len();
        let clamped_range = range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8);
        adjusted_range.replace(self.storage.utf_range_8to16(&clamped_range));
        Some(self.storage.content_utf8()[clamped_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection_range = self.selected_range();
        let direction = self.selection_direction();
        Some(UTF16Selection {
            range: self.storage.utf_range_8to16(&selection_range),
            reversed: direction == Some(NavigationDirection::Back),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.storage.utf_range_8to16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text_to_insert: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range_utf8 = self.ime_resolve_range(range_utf16);
        let text_to_insert = self.validate_incoming_text(&range_utf8, text_to_insert);
        self.replace_text(range_utf8, text_to_insert.as_ref());
        cx.emit(CaretNotify::PauseBlinking);
        self.emit_text_changed(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text_to_insert: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.ime_resolve_range(range_utf16);
        let text_to_insert = self.validate_incoming_text(&range, text_to_insert);
        self.replace_text(range.clone(), text_to_insert.as_ref());
        self.ime_mark_text_in_range(&range, text_to_insert.len());
        self.ime_mark_selected_range(&range, &new_selected_range_utf16, text_to_insert.len());
        self.emit_text_changed(cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.storage.utf_range_16to8(&range_utf16);
        let line_height = window.line_height();

        for line in &self.layout_data.lines {
            // The vertical offset of the text-segment from the start of the virtual box (which could be scrolled).
            // Scrolling is not relevant here, so we are just operating on the internal virtualized space.
            let y_offset = line.pos_y * line_height;
            // The start of the line in screen space as if there was no scrolling.
            let line_origin = bounds.origin + point(Pixels::ZERO, y_offset);
            if line.text_range.is_empty() {
                if range.start == line.text_range.start {
                    return Some(Bounds::from_corners(
                        line_origin,
                        line_origin + point(CARET_PIXELS_EPSILON, line_height),
                    ));
                }
            } else if line.text_range.contains(&range.start)
                && let Some(wrapped) = &line.wrapped_line
            {
                let local_start = range.start - line.text_range.start;
                let local_end = (range.end - line.text_range.start).min(wrapped.text.len());

                // The start of the line in screen-space pixels
                let line_start_screen_pos = wrapped
                    .position_for_index(local_start, line_height)
                    .unwrap_or_default();
                // The end of the line in screen-space pixels
                let line_end_screen_pos = wrapped
                    .position_for_index(local_end, line_height)
                    .unwrap_or_else(|| {
                        // the y-height/position of the last line
                        let last_line_y = line_height * (line.row_count() - 1) as f32;
                        point(wrapped.width(), last_line_y)
                    });

                // The number of rows this text-segment spans
                let line_height_range = (line_start_screen_pos.y / line_height).floor() as usize
                    ..(line_end_screen_pos.y / line_height).floor() as usize;
                // The width of the line-segment that may span multiple rows
                let width = match line_height_range.is_empty() {
                    true => line_end_screen_pos.x,
                    false => wrapped.width(),
                };
                return Some(Bounds::from_corners(
                    line_origin + line_start_screen_pos,
                    line_origin + point(width, line_start_screen_pos.y + line_height),
                ));
            }
        }
        None
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self.index_for_pixel_point(point, window.line_height());
        Some(self.storage.utf_offset_8to16(index))
    }
}

// Input Action handler
use super::{actions::*, history::HistoryKind};
impl<'app> EditableTextActionHandler<Context<'app, Self>> for EditableTextState {
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<'app, Self>) {
        self.selected_range = 0.into();
        cx.notify();

        window.blur();
    }

    fn insert_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            return;
        }
        if !self.layout_data.accepts_input {
            return;
        }
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn insert_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.accepts_input {
            return;
        }
        self.replace_text_in_range(None, "\t", window, cx);
    }

    fn delete_left(&mut self, _: &DeleteLeft, _: &mut Window, cx: &mut Context<'app, Self>) {
        self.delete_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn delete_right(&mut self, _: &DeleteRight, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.delete_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn delete_to_line_end(
        &mut self,
        _: &DeleteToLineEnd,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_left(&mut self, _: &NavLeft, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn nav_right(&mut self, _: &NavRight, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn nav_up(&mut self, _: &NavUp, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            // semantically equivalent to line
            self.nav_linear(NavigationDirection::Back, TextBoundary::Line, cx);
            return;
        }

        if let Some(caret_pos) = self.find_position_in_vertical_direction(-1, window.line_height())
        {
            self.move_to(caret_pos, cx);
        }
    }

    fn nav_down(&mut self, _: &NavDown, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            // semantically equivalent to line
            self.nav_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
            return;
        }

        if let Some(caret_pos) = self.find_position_in_vertical_direction(1, window.line_height()) {
            self.move_to(caret_pos, cx);
        }
    }

    fn nav_line_start(&mut self, _: &NavLineStart, _w: &mut Window, cx: &mut Context<'app, Self>) {
        // [when not multiline] semantically equivalent to document
        self.nav_linear(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn nav_line_end(&mut self, _: &NavLineEnd, _w: &mut Window, cx: &mut Context<'app, Self>) {
        // [when not multiline] semantically equivalent to document
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_start(&mut self, _: &NavDocumentStart, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn nav_end(&mut self, _: &NavDocumentEnd, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
    }

    fn nav_left_word(&mut self, _: &NavWordLeft, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn nav_right_word(&mut self, _: &NavWordRight, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.select_document(cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.select_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.select_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            // semantically equivalent to select document
            self.select_linear(NavigationDirection::Back, TextBoundary::Document, cx);
            return;
        }

        if let Some(caret_pos) = self.find_position_in_vertical_direction(-1, window.line_height())
        {
            self.select_to(caret_pos, cx);
        }
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            // semantically equivalent to select document
            self.select_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
            return;
        }

        if let Some(caret_pos) = self.find_position_in_vertical_direction(1, window.line_height()) {
            self.select_to(caret_pos, cx);
        }
    }

    fn select_start(
        &mut self,
        _: &SelectDocumentStart,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.select_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn select_end(&mut self, _: &SelectDocumentEnd, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.select_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
    }

    fn select_left_word(
        &mut self,
        _: &SelectWordLeft,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.select_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn select_right_word(
        &mut self,
        _: &SelectWordRight,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.select_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn cut(&mut self, _: &Cut, _w: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.accepts_input {
            return;
        }

        let range_to_cut = match self.selected_range.is_empty() {
            // selection is more than a caret, use that range of text
            false => self.selected_range.range(),
            // No selection: cut the entire current line (including newline)
            true => {
                use NavigationDirection::*;
                use TextBoundary::*;

                let caret = self.caret_pos();
                let line_start = self.storage.offset_from_caret(caret, Back, Line);
                let line_end = self.storage.offset_from_caret(caret, Forward, Line);
                let storage_len_utf8 = self.as_str().len();

                // Include the newline character if there is one after the line
                let cut_end = if line_end < storage_len_utf8 {
                    line_end + 1 // Include the newline
                } else if line_start > 0 {
                    // Last line with no trailing newline - include preceding newline instead
                    line_end
                } else {
                    line_end
                };

                // For last line, also remove the preceding newline if it exists
                let cut_start = if line_end >= storage_len_utf8 && line_start > 0 {
                    line_start - 1 // Include preceding newline for last line
                } else {
                    line_start
                };

                cut_start..cut_end
            }
        };

        // Cut selected text
        let slice = &self.storage.content_utf8()[range_to_cut.clone()];
        cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
        self.replace_text(range_to_cut, "");

        self.emit_text_changed(cx);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _w: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.selected_range.is_empty() {
            let slice = &self.storage.content_utf8()[self.selected_range.range()];
            cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
        }
    }

    fn paste(&mut self, _: &Paste, _w: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.accepts_input {
            return;
        }

        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };

        let range = self.ime_resolve_range(None);
        let text_to_insert = self.validate_incoming_text(&range, &text);
        self.replace_text(range, text_to_insert.as_ref());
        self.emit_text_changed(cx);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.apply_from_history(HistoryKind::Undo, HistoryKind::Redo, cx);
    }

    fn redo(&mut self, _: &Redo, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.apply_from_history(HistoryKind::Redo, HistoryKind::Undo, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        text_position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        const DOUBLE_CLICK: usize = 2;
        const TRIPLE_CLICK: usize = 3;

        let caret_pos = self.index_for_pixel_point(text_position, window.line_height());

        self.is_selecting = true;
        self.apply_click(event.click_count, text_position);

        match self.click_count {
            DOUBLE_CLICK => self.select_word_at(caret_pos, cx),
            TRIPLE_CLICK => self.select_line_at(caret_pos, cx),
            _ if event.modifiers.shift => self.select_to(caret_pos, cx),
            _ => self.move_to(caret_pos, cx),
        }
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _w: &mut Window,
        _cx: &mut Context<'app, Self>,
    ) {
        self.is_selecting = false;
    }

    fn on_mouse_move(
        &mut self,
        _event: &gpui::MouseMoveEvent,
        text_position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        let character_pos = self.index_for_pixel_point(text_position, window.line_height());
        if self.is_selecting && self.click_count == 1 {
            self.select_to(character_pos, cx);
        }
    }
}

/// Backlog:
/// - tests for text layout (generating TextLineSegment and wrap-boundaries);
///     permutations of: single and multiline fields, wrap vs no-wrap, overflow scroll vs no scroll
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::editable_text::StringStorage;
    use gpui::{AppContext, Entity, IntoElement, Render, TestAppContext, WindowHandle, div};

    struct TestView {
        input: Entity<EditableTextState>,
    }

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn default_state(content: &str, cx: &mut Context<EditableTextState>) -> EditableTextState {
        EditableTextState::new(StringStorage::from(content), cx)
    }

    fn create_test_input(
        cx: &mut TestAppContext,
        content: &str,
        range: impl Into<CaretSelection>,
    ) -> WindowHandle<TestView> {
        cx.add_window(|_window, cx| {
            let input = cx.new(|cx| {
                let mut input = default_state(content, cx);
                input.selected_range = range.into();
                input.layout_data.accepts_input = true;
                input
            });
            TestView { input }
        })
    }

    // Disable grouping for predictable test behavior
    fn without_history_grouping(state: &mut EditableTextState) {
        state
            .history
            .get_or_insert_default()
            .set_grouping_interval(Duration::from_secs(0));
    }

    fn is_history_kind_available(state: &EditableTextState, kind: HistoryKind) -> bool {
        state
            .history()
            .map(|history| history.has_next(kind))
            .unwrap_or_default()
    }

    // ============================================================
    // BASIC MOVEMENT
    // ============================================================

    #[gpui::test]
    fn test_left_at_start_of_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_moves_by_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 2.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_collapses_selection_to_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", (1, 4));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 1.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_stops_at_end_of_line(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 3 (start of "cd", after newline)
        // Pressing left should move to position 2 (end of "ab", before newline)
        let view = create_test_input(cx, "ab\ncd", 3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 2.into()); // cursor at end of line 1
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_at_end_of_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_moves_by_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 3.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_collapses_selection_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", (1, 4));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 4.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_stops_at_end_of_line(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 1 (after 'a')
        // Pressing right should move to position 2 (end of "ab", before newline)
        let view = create_test_input(cx, "ab\ncd", 1);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 2.into()); // cursor at end of line 1
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_crosses_newline(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 2 (end of "ab", before newline)
        // Pressing right should move to position 3 (after newline, start of "cd")
        let view = create_test_input(cx, "ab\ncd", 2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 3.into()); // cursor at start of line 2
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_crosses_newline(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 2 (end of "ab", before newline)
        // Pressing left should move to position 1 (after 'a')
        let view = create_test_input(cx, "ab\ncd", 2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 1.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_home_moves_to_line_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond", 9);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_line_start(&NavLineStart, window, cx);
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_end_moves_to_line_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond", 8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_line_end(&NavLineEnd, window, cx);
                assert_eq!(input.selected_range, 12.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_move_to_beginning(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 9);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_start(&NavDocumentStart, window, cx);
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_move_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_end(&NavDocumentEnd, window, cx);
                assert_eq!(input.selected_range, 18.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // WORD MOVEMENT
    // ============================================================

    #[gpui::test]
    fn test_word_left_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left_word(&NavWordLeft, window, cx);
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_left_stops_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world test", 11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left_word(&NavWordLeft, window, cx);
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_right_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right_word(&NavWordRight, window, cx);
                assert_eq!(input.selected_range, 11.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_right_stops_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world test", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right_word(&NavWordRight, window, cx);
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // SELECTION
    // ============================================================

    #[gpui::test]
    fn test_select_left_extends_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_left(&SelectLeft, window, cx);
                assert_eq!(input.selected_range, (2, 3).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_right_extends_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 2..2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, (3, 2).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_all(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello\nworld", 3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_all(&SelectAll, window, cx);
                assert_eq!(input.selected_range, (0, 11).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_to_beginning(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_start(&SelectDocumentStart, window, cx);
                assert_eq!(input.selected_range, (0, 6).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_end(&SelectDocumentEnd, window, cx);
                assert_eq!(input.selected_range, (11, 6).into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - BACKSPACE
    // ============================================================

    #[gpui::test]
    fn test_backspace_deletes_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (6, 11));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "hello ");
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_previous_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "hell");
                assert_eq!(input.selected_range, 4.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_at_start_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "hello");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_entire_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "Hi 👋", 7);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "Hi ");
                assert_eq!(input.selected_range, 3.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - DELETE
    // ============================================================

    #[gpui::test]
    fn test_delete_deletes_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_deletes_next_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), "ello");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_at_end_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), "hello");
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - ENTER
    // ============================================================

    #[gpui::test]
    fn test_enter_inserts_newline(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.layout_data.supports_multiline = true;
                input.insert_enter(&Enter, window, cx);
                assert_eq!(input.as_str(), "hello\n world");
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_enter_replaces_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (5, 6));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.layout_data.supports_multiline = true;
                input.insert_enter(&Enter, window, cx);
                assert_eq!(input.as_str(), "hello\nworld");
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // CLIPBOARD
    // ============================================================

    #[gpui::test]
    fn test_copy_with_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (6, 11));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.copy(&Copy, window, cx);
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert!(clipboard.is_some());
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("world"));
    }

    #[gpui::test]
    fn test_cut_with_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("hello"));
    }

    #[gpui::test]
    fn test_paste_inserts_text(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        cx.write_to_clipboard(ClipboardItem::new_string(" there".to_string()));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.paste(&Paste, window, cx);
                assert_eq!(input.as_str(), "hello there world");
                assert_eq!(input.selected_range, 11.into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // UNICODE / GRAPHEME HANDLING
    // ============================================================

    #[gpui::test]
    fn test_movement_with_multibyte_utf8(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "café", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 1.into());
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 2.into());
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 3.into());
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_movement_with_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a👋b", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 1.into());
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 5.into());
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 6.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_selection_with_multibyte_characters(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "日本語", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, (3, 0).into());
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, (6, 0).into());
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, (9, 0).into());
            });
        })
        .unwrap();
    }

    // ============================================================
    // NEWLINE HANDLING
    // ============================================================

    #[gpui::test]
    fn test_find_line_start_and_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                use NavigationDirection::*;
                use TextBoundary::*;
                let storage = &input.storage;

                assert_eq!(storage.offset_from_caret(0, Back, Line), 0);
                assert_eq!(storage.offset_from_caret(3, Back, Line), 0);
                assert_eq!(storage.offset_from_caret(6, Back, Line), 6);
                assert_eq!(storage.offset_from_caret(13, Back, Line), 13);

                assert_eq!(storage.offset_from_caret(0, Forward, Line), 5);
                assert_eq!(storage.offset_from_caret(6, Forward, Line), 12);
                assert_eq!(storage.offset_from_caret(13, Forward, Line), 18);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDGE CASES
    // ============================================================

    #[gpui::test]
    fn test_operations_on_empty_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range, 0.into());

                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range, 0.into());

                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "");

                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), "");

                input.select_all(&SelectAll, window, cx);
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_set_content_resets_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (3, 8));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.marked_range = Some(5..7);
                input.replace_text_in_range(Some(0..11), "new content", window, cx);
                assert_eq!(input.as_str(), "new content");
                assert_eq!(input.selected_range, 11.into());
                assert_eq!(input.marked_range, None);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cursor_clamped_to_content_length(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 100);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, cx| {
                input.move_to(1000, cx);
                assert_eq!(input.selected_range, 5.into());

                input.selected_range = 0.into();
                input.select_to(1000, cx);
                assert_eq!(input.selected_range, (5, 0).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_previous_boundary_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                use NavigationDirection::*;
                use TextBoundary::*;
                assert_eq!(input.storage.offset_from_caret(0, Back, Graphmeme), 0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_next_boundary_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                use NavigationDirection::*;
                use TextBoundary::*;
                let storage = &input.storage;
                assert_eq!(storage.offset_from_caret(5, Forward, Graphmeme), 5);
                assert_eq!(storage.offset_from_caret(100, Forward, Graphmeme), 5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_range_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                let range = input.storage.word_range_at(5);
                assert_eq!(range.start, 0);
                assert_eq!(range.end, 5);

                let range = input.storage.word_range_at(8);
                assert_eq!(range.start, 6);
                assert_eq!(range.end, 11);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EMOJI & GRAPHEME CLUSTERS
    // ============================================================

    #[gpui::test]
    fn test_simple_emoji_navigation(cx: &mut TestAppContext) {
        // 😀 is 4 bytes in UTF-8
        let view = create_test_input(cx, "a😀b", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                // Move right through: a -> 😀 -> b
                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range.start, 1); // after 'a'

                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range.start, 5); // after 😀 (1 + 4 bytes)

                input.nav_right(&NavRight, window, cx);
                assert_eq!(input.selected_range.start, 6); // after 'b'

                // Move left back
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range.start, 5); // before 'b'

                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range.start, 1); // before 😀
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_emoji_with_skin_tone_modifier(cx: &mut TestAppContext) {
        // 👋🏽 = 👋 (U+1F44B, 4 bytes) + 🏽 (U+1F3FD, 4 bytes) = 8 bytes total
        let emoji = "👋🏽";
        assert_eq!(emoji.len(), 8);

        let view = create_test_input(cx, &format!("a{}b", emoji), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.nav_right(&NavRight, window, cx); // past entire emoji with modifier
                assert_eq!(input.selected_range.start, 9); // 1 + 8

                input.nav_left(&NavLeft, window, cx); // back before emoji
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_zwj_family_emoji(cx: &mut TestAppContext) {
        // 👨‍👩‍👧 = man + ZWJ + woman + ZWJ + girl
        // Each person emoji is 4 bytes, ZWJ is 3 bytes
        // Total: 4 + 3 + 4 + 3 + 4 = 18 bytes
        let family = "👨‍👩‍👧";
        assert_eq!(family.len(), 18);

        let view = create_test_input(cx, &format!("x{}y", family), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'x'
                assert_eq!(input.selected_range.start, 1);

                input.nav_right(&NavRight, window, cx); // past entire ZWJ sequence
                assert_eq!(input.selected_range.start, 19); // 1 + 18

                input.nav_right(&NavRight, window, cx); // past 'y'
                assert_eq!(input.selected_range.start, 20);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_emoji_between_ascii(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 5); // cursor after emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "ab");
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_zwj_sequence(cx: &mut TestAppContext) {
        let family = "👨‍👩‍👧";
        let content = format!("a{}b", family);
        let cursor_pos = 1 + family.len(); // after the family emoji

        let view = create_test_input(cx, &content, cursor_pos..cursor_pos);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "ab");
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_removes_entire_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 1); // cursor before emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), "ab");
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_flag_emoji_navigation(cx: &mut TestAppContext) {
        // 🇯🇵 = Regional Indicator J (4 bytes) + Regional Indicator P (4 bytes)
        let flag = "🇯🇵";
        assert_eq!(flag.len(), 8);

        let view = create_test_input(cx, &format!("x{}y", flag), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'x'
                input.nav_right(&NavRight, window, cx); // past flag (should be single grapheme)
                assert_eq!(input.selected_range.start, 9); // 1 + 8
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_combining_diacritical_marks(cx: &mut TestAppContext) {
        // é as e + combining acute accent (U+0301)
        let combining = "e\u{0301}"; // 1 + 2 = 3 bytes
        assert_eq!(combining.len(), 3);

        let view = create_test_input(cx, &format!("a{}b", combining), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.nav_right(&NavRight, window, cx); // past e + combining mark (single grapheme)
                assert_eq!(input.selected_range.start, 4); // 1 + 3

                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_multiple_combining_marks(cx: &mut TestAppContext) {
        // ë́ = e + combining diaeresis (U+0308) + combining acute (U+0301)
        let multi_combining = "e\u{0308}\u{0301}"; // 1 + 2 + 2 = 5 bytes
        assert_eq!(multi_combining.len(), 5);

        let view = create_test_input(cx, &format!("x{}y", multi_combining), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'x'
                input.nav_right(&NavRight, window, cx); // past entire combined character
                assert_eq!(input.selected_range.start, 6); // 1 + 5
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_emoji_with_shift(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 1); // cursor before emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, (5, 1).into()); // selected the entire emoji
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cjk_characters(cx: &mut TestAppContext) {
        // 你好 - each character is 3 bytes in UTF-8
        let view = create_test_input(cx, "a你好b", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.nav_right(&NavRight, window, cx); // past 你
                assert_eq!(input.selected_range.start, 4); // 1 + 3

                input.nav_right(&NavRight, window, cx); // past 好
                assert_eq!(input.selected_range.start, 7); // 4 + 3

                input.nav_right(&NavRight, window, cx); // past 'b'
                assert_eq!(input.selected_range.start, 8);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_mixed_script_text(cx: &mut TestAppContext) {
        // Mix of ASCII, CJK, and emoji
        let view = create_test_input(cx, "Hi你😀", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'H'
                assert_eq!(input.selected_range.start, 1);

                input.nav_right(&NavRight, window, cx); // past 'i'
                assert_eq!(input.selected_range.start, 2);

                input.nav_right(&NavRight, window, cx); // past 你 (3 bytes)
                assert_eq!(input.selected_range.start, 5);

                input.nav_right(&NavRight, window, cx); // past 😀 (4 bytes)
                assert_eq!(input.selected_range.start, 9);

                // Now go back
                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range.start, 5);

                input.nav_left(&NavLeft, window, cx);
                assert_eq!(input.selected_range.start, 2);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_variation_selector_emoji(cx: &mut TestAppContext) {
        // ☺️ = ☺ (U+263A, 3 bytes) + variation selector-16 (U+FE0F, 3 bytes)
        let emoji_presentation = "☺\u{FE0F}";
        assert_eq!(emoji_presentation.len(), 6);

        let view = create_test_input(cx, &format!("a{}b", emoji_presentation), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'a'
                input.nav_right(&NavRight, window, cx); // past emoji with variation selector
                assert_eq!(input.selected_range.start, 7); // 1 + 6
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_keycap_emoji(cx: &mut TestAppContext) {
        // 1️⃣ = 1 + variation selector + combining enclosing keycap
        let keycap = "1\u{FE0F}\u{20E3}";

        let view = create_test_input(cx, &format!("x{}y", keycap), 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_right(&NavRight, window, cx); // past 'x'
                input.nav_right(&NavRight, window, cx); // past keycap sequence
                let expected_pos = 1 + keycap.len();
                assert_eq!(input.selected_range.start, expected_pos);
            });
        })
        .unwrap();
    }

    // Single-line input tests

    fn create_single_line_input(
        cx: &mut TestAppContext,
        content: &str,
        selected_range: impl Into<CaretSelection>,
    ) -> WindowHandle<TestView> {
        cx.add_window(|_window, cx| {
            let input = cx.new(|cx| {
                let mut input = default_state(content, cx);
                input.selected_range = selected_range.into();
                input
            });
            TestView { input }
        })
    }

    #[gpui::test]
    fn test_single_line_enter_does_nothing(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.insert_enter(&Enter, window, cx);
                assert_eq!(input.as_str(), "hello");
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_up_moves_to_start(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_up(&NavUp, window, cx);
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_down_moves_to_end(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.nav_down(&NavDown, window, cx);
                assert_eq!(input.selected_range, 11.into()); // "hello world".len() == 11
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_select_up_selects_to_start(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_up(&SelectUp, window, cx);
                assert_eq!(input.selected_range, (0, 5).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_select_down_selects_to_end(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_down(&SelectDown, window, cx);
                assert_eq!(input.selected_range, (11, 5).into()); // "hello world".len() == 11
            });
        })
        .unwrap();
    }

    // ============================================================
    // UNDO / REDO
    // ============================================================

    #[gpui::test]
    fn test_undo_restores_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                // Make an edit
                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.as_str(), "hello world");

                // Undo should restore original content
                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_redo_restores_undone_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.as_str(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");

                input.redo(&Redo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_undo_with_no_history_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                assert!(!is_history_kind_available(input, HistoryKind::Undo));
                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_redo_with_no_history_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                assert!(!is_history_kind_available(input, HistoryKind::Redo));
                input.redo(&Redo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_undo_restores_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                // Delete selection
                input.replace_text_in_range(None, "", window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());

                // Undo should restore content and selection
                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
                assert_eq!(input.selected_range, (0, 5).into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_multiple_undo_redo(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.replace_text_in_range(None, "a", window, cx);
                input.replace_text_in_range(None, "b", window, cx);
                input.replace_text_in_range(None, "c", window, cx);
                assert_eq!(input.as_str(), "abc");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "ab");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "a");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "");

                input.redo(&Redo, window, cx);
                assert_eq!(input.as_str(), "a");

                input.redo(&Redo, window, cx);
                assert_eq!(input.as_str(), "ab");

                input.redo(&Redo, window, cx);
                assert_eq!(input.as_str(), "abc");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_new_edit_clears_redo_stack(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.as_str(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
                assert!(is_history_kind_available(input, HistoryKind::Redo));

                // New edit should clear redo stack
                input.replace_text_in_range(None, "!", window, cx);
                assert_eq!(input.as_str(), "hello!");
                assert!(!is_history_kind_available(input, HistoryKind::Redo));
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_can_undo_can_redo(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                assert!(!is_history_kind_available(input, HistoryKind::Undo));
                assert!(!is_history_kind_available(input, HistoryKind::Redo));

                input.replace_text_in_range(None, "!", window, cx);
                assert!(is_history_kind_available(input, HistoryKind::Undo));
                assert!(!is_history_kind_available(input, HistoryKind::Redo));

                input.undo(&Undo, window, cx);
                assert!(!is_history_kind_available(input, HistoryKind::Undo));
                assert!(is_history_kind_available(input, HistoryKind::Redo));

                input.redo(&Redo, window, cx);
                assert!(is_history_kind_available(input, HistoryKind::Undo));
                assert!(!is_history_kind_available(input, HistoryKind::Redo));
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_left(&DeleteLeft, window, cx);
                assert_eq!(input.as_str(), "hell");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_right(&DeleteRight, window, cx);
                assert_eq!(input.as_str(), "ello");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cut_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cut_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor in middle line, no selection - should cut entire line including newline
        let view = create_test_input(cx, "line1\nline2\nline3", 8); // cursor in "line2"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "line1\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("line2\n"));
    }

    #[gpui::test]
    fn test_cut_first_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor on first line, no selection
        let view = create_test_input(cx, "line1\nline2\nline3", 2); // cursor in "line1"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "line2\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("line1\n"));
    }

    #[gpui::test]
    fn test_cut_last_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor on last line, no selection - should include preceding newline
        let view = create_test_input(cx, "line1\nline2\nline3", 14); // cursor in "line3"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "line1\nline2");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("\nline3"));
    }

    #[gpui::test]
    fn test_cut_empty_line(cx: &mut TestAppContext) {
        // Cursor on empty line - should remove that line
        let view = create_test_input(cx, "line1\n\nline3", 6); // cursor on empty line
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "line1\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("\n"));
    }

    #[gpui::test]
    fn test_cut_only_line_with_no_selection(cx: &mut TestAppContext) {
        // Single line content, no selection - should cut entire content
        let view = create_test_input(cx, "hello", 2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("hello"));
    }

    #[gpui::test]
    fn test_cut_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "line1\nline2\nline3", 8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.cut(&Cut, window, cx);
                assert_eq!(input.as_str(), "line1\nline3");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "line1\nline2\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_paste_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5);
        cx.write_to_clipboard(ClipboardItem::new_string(" world".to_string()));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.paste(&Paste, window, cx);
                assert_eq!(input.as_str(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_enter_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);
                input.layout_data.supports_multiline = true;

                input.insert_enter(&Enter, window, cx);
                assert_eq!(input.as_str(), "hello\n world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left(cx: &mut TestAppContext) {
        // Cursor at end of "hello" in "hello world"
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_with_selection(cx: &mut TestAppContext) {
        // Selection from 0 to 5 ("hello")
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.as_str(), " world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right(cx: &mut TestAppContext) {
        // Cursor at start
        let view = create_test_input(cx, "hello world", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_with_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", (0, 5));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.as_str(), " world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_start(&DeleteToLineStart, window, cx);
                assert_eq!(input.as_str(), " world");
                assert_eq!(input.selected_range, 0.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_multiline(cx: &mut TestAppContext) {
        // Cursor at position 8 (middle of "line2")
        let view = create_test_input(cx, "line1\nline2\nline3", 8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_start(&DeleteToLineStart, window, cx);
                assert_eq!(input.as_str(), "line1\nne2\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_start(&DeleteToLineStart, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_end(&DeleteToLineEnd, window, cx);
                assert_eq!(input.as_str(), "hello");
                assert_eq!(input.selected_range, 5.into());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_multiline(cx: &mut TestAppContext) {
        // Cursor at position 8 (middle of "line2")
        let view = create_test_input(cx, "line1\nline2\nline3", 8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_end(&DeleteToLineEnd, window, cx);
                assert_eq!(input.as_str(), "line1\nli\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_line_end(&DeleteToLineEnd, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.as_str(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.as_str(), "hello ");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_to_line_start(&DeleteToLineStart, window, cx);
                assert_eq!(input.as_str(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                without_history_grouping(input);

                input.delete_to_line_end(&DeleteToLineEnd, window, cx);
                assert_eq!(input.as_str(), "hello");

                input.undo(&Undo, window, cx);
                assert_eq!(input.as_str(), "hello world");
            });
        })
        .unwrap();
    }
}
