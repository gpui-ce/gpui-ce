use crate::editable_text::{
    TextBoundary, UnicodeTextStorage,
    actions::EditableTextActionHandler,
    notify::{TextChanged, TextHistoryPushed},
};
use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    NavigationDirection, Pixels, Point, Size, UTF16Selection, Window, WrappedLine, point,
};
use std::{ops::Range, sync::Arc};

pub struct EditableTextState {
    storage: Box<dyn UnicodeTextStorage>,

    /// The utf-8 character range that is currently selected by the user.
    /// Valid both when start < end and start > end (which dictates the direction of the selection). Empty when start==end.
    /// The start of this range is always the current position of the caret (input cursor).
    ///
    /// NOTE: because each input has its own selection state, its trivial for users to have multiple selections active across multiple inputs at the same time.
    /// This could be considered undesirable behavior, and could prompt the question of whether there should be a mechanism to clear selection when focus is lost.
    selected_range: Range<usize>,

    /// The utf-8 character range of `storage` which is being composed by IME
    marked_range: Option<Range<usize>>,

    /// True while the user is in the act of highlighting a section of the text (e.g. during mouse pressed & dragging).
    is_selecting: bool,
    /// The last ui location relative to the element that the user clicked. Used to filter when a user clicks multiple times in the same area.
    last_click_position: Option<Point<Pixels>>,
    /// The number of times the user has clicked `last_click_position`. Used to determine which click behavior to trigger, depending on single, double, or triple clicks.
    click_count: usize,

    focus_handle: FocusHandle,

    pub(super) layout_data: TextInputLayoutData,
}

#[derive(Default)]
pub(super) struct TextInputLayoutData {
    pub supports_multiline: bool,
    pub accepts_input: bool,
    /// The last known width at which the lines were wrapped.
    pub wrap_width: Option<Pixels>,
    /// The last known size of the text, as generated during layout.
    pub size: Option<Size<Pixels>>,
    /// The last seen version of `storage` (for tracking when lines need to be reprocessed during layout)
    pub last_seen_storage_version: u16,
    /// The `ShapedLine` produced by the painter's `prepaint`.
    /// Cached so IME `bounds_for_range` / `character_index_for_point` can evaluate without re-shaping.
    pub lines: Vec<TextLineSegment>,
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
}

impl EventEmitter<TextChanged> for EditableTextState {}
impl EventEmitter<TextHistoryPushed> for EditableTextState {}

impl Focusable for EditableTextState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditableTextState {
    pub fn new(storage: impl Into<Box<dyn UnicodeTextStorage>>, cx: &mut Context<Self>) -> Self {
        Self {
            storage: storage.into(),

            selected_range: 0..0,
            marked_range: None,

            is_selecting: false,
            last_click_position: None,
            click_count: 0,

            focus_handle: cx.focus_handle(),

            layout_data: TextInputLayoutData::default(),
        }
    }

    pub fn storage(&self) -> &Box<dyn UnicodeTextStorage> {
        &self.storage
    }

    /// Returns the utf-8 character range that is currently selected within the current state of the text.
    /// Internally converts the stored direction-aware range into a canonical range.
    pub fn selected_range(&self) -> Range<usize> {
        self.selected_range.start.min(self.selected_range.end)
            ..self.selected_range.start.max(self.selected_range.end)
    }

    pub fn selection_direction(&self) -> Option<NavigationDirection> {
        match self.selected_range.start.cmp(&self.selected_range.end) {
            std::cmp::Ordering::Less => Some(NavigationDirection::Forward),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(NavigationDirection::Back),
        }
    }

    pub fn caret_pos(&self) -> usize {
        self.selected_range.start
    }

    pub fn set_selected_range(&mut self, range: Range<usize>) {
        self.selected_range = range;
    }

    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }
}

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

    fn ime_resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        // Use a series of fallbacks to pick the range to operate on.
        // Fallback order: IME provided range, active IME marked range, selection
        let range = range_utf16.map(|range_utf16| self.storage.utf_range_16to8(&range_utf16));
        let range = range.or_else(|| self.marked_range.clone());
        let range = range.unwrap_or_else(|| self.selected_range());

        let storage_len_utf8 = self.storage().content_utf8().len();
        range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8)
    }

    pub fn replace_text(&mut self, range: &Range<usize>, new_text: &str) {
        let storage_len_utf8 = self.storage.content_utf8().len();
        let start = range.start.min(storage_len_utf8);
        let end = range.end.max(start).min(storage_len_utf8);
        self.storage.replace_range(start..end, new_text);

        let new_caret = start + new_text.len();
        self.selected_range = new_caret..new_caret;
    }

    fn emit_change_for_undo(&self, cx: &mut Context<Self>, range: Range<usize>, length: usize) {
        cx.emit(TextHistoryPushed::new(
            range.clone(),
            length,
            &*self.storage,
            self.selected_range.clone(),
        ));
    }

    pub fn replace_text_in_range_bytes(
        &mut self,
        range: Range<usize>,
        mut text_to_insert: &str,
        cx: &mut Context<Self>,
    ) {
        // TODO: Apply text sanitization
        // single-line fields should prune \n and \r
        // fields should be able to provide a max_length or other validations on text-input

        let max_length = None::<usize>;

        // Decide the effective new text up front (honouring `max_length`).
        // This avoids the "apply, then truncate" path which would leave the caret past the end.
        if let Some(cap) = max_length {
            let existing_len = self.storage().content_utf8().len() - (range.end - range.start);
            let room = cap.saturating_sub(existing_len);
            text_to_insert = &text_to_insert[..text_to_insert.len().min(room)];
        }

        let end_pos = range.start + text_to_insert.len();

        self.emit_change_for_undo(cx, range.clone(), text_to_insert.len());
        self.storage.replace_range(range, text_to_insert);
        self.selected_range = end_pos..end_pos;
        self.marked_range = None;
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
            new_range.unwrap_or_else(|| {
                range_overwritten.start + text_len..range_overwritten.start + text_len
            })
        };
    }
}

impl EditableTextState {
    pub fn move_to(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        let caret_pos = caret_pos.min(self.storage.content_utf8().len());
        self.selected_range = caret_pos..caret_pos;
        //self.scroll_to_cursor();
        cx.notify();
    }

    pub fn select_to(&mut self, caret_pos: usize, cx: &mut Context<Self>) {
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        let caret_pos = caret_pos.min(self.storage().content_utf8().len());
        self.selected_range.start = caret_pos;
        //self.scroll_to_cursor();
        cx.notify();
    }

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

        self.emit_change_for_undo(cx, range.clone(), 0);

        self.replace_text(&range, "");
        self.marked_range = None;

        cx.emit(TextChanged);
        cx.notify();
    }

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

    pub fn line_index_and_point_at_caret(&self, line_height: Pixels) -> (usize, Point<Pixels>) {
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

    pub fn find_position_in_vertical_direction(
        &self,
        direction: i32,
        line_height: Pixels,
    ) -> Option<usize> {
        let (line_index, point) = self.line_index_and_point_at_caret(line_height);
        println!("{line_index:?} {point:?}");
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
                    println!("{result:?}");
                    return Some(result);
                }

                return Some(segment.text_range.start);
            }

            current_visual_line += wrap_boundary_len;
        }

        (direction > 0).then(|| self.storage.content_utf8().len())
    }

    pub fn select_document(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.storage.content_utf8().len();
        cx.notify();
    }

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
}

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
        self.replace_text_in_range_bytes(range_utf8, text_to_insert, cx);
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        cx.emit(TextChanged);
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
        self.replace_text_in_range_bytes(range.clone(), text_to_insert, cx);
        self.ime_mark_text_in_range(&range, text_to_insert.len());
        self.ime_mark_selected_range(&range, &new_selected_range_utf16, text_to_insert.len());
        cx.emit(TextChanged);
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
            let y_offset = line.pos_y * line_height;
            if line.text_range.is_empty() {
                if range.start == line.text_range.start {
                    return Some(Bounds::from_corners(
                        bounds.origin + point(Pixels::ZERO, y_offset),
                        bounds.origin + point(gpui::px(4.), y_offset + line_height),
                    ));
                }
            } else if line.text_range.contains(&range.start) {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_start = range.start - line.text_range.start;
                    let local_end = (range.end - line.text_range.start).min(wrapped.text.len());

                    let start_pos = wrapped
                        .position_for_index(local_start, line_height)
                        .unwrap_or(point(Pixels::ZERO, Pixels::ZERO));
                    let end_pos = wrapped
                        .position_for_index(local_end, line_height)
                        .unwrap_or_else(|| {
                            let last_line_y = line_height * (line.row_count() - 1) as f32;
                            point(wrapped.width(), last_line_y)
                        });

                    let start_visual_line = (start_pos.y / line_height).floor() as usize;
                    let end_visual_line = (end_pos.y / line_height).floor() as usize;

                    if start_visual_line == end_visual_line {
                        return Some(Bounds::from_corners(
                            bounds.origin + start_pos + point(Pixels::ZERO, y_offset),
                            bounds.origin + point(end_pos.x, y_offset + start_pos.y + line_height),
                        ));
                    } else {
                        return Some(Bounds::from_corners(
                            bounds.origin + start_pos + point(Pixels::ZERO, y_offset),
                            bounds.origin
                                + point(wrapped.width(), y_offset + start_pos.y + line_height),
                        ));
                    }
                }
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
        Some(self.storage().utf_offset_8to16(index))
    }
}

use super::actions::*;
impl<'app> EditableTextActionHandler<Context<'app, Self>> for EditableTextState {
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<'app, Self>) {
        self.set_selected_range(0..0);
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
        // TODO: Why is the cursor appearing at the start of the entire field instead of on the new line?
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn insert_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.accepts_input {
            return;
        }
        self.replace_text_in_range(None, "\t", window, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<'app, Self>) {
        self.delete_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn delete(&mut self, _: &Delete, _w: &mut Window, cx: &mut Context<'app, Self>) {
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
        _: &DeleteToBeginningOfLine,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn delete_to_line_end(
        &mut self,
        _: &DeleteToEndOfLine,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.delete_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_left(&mut self, _: &Left, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn nav_right(&mut self, _: &Right, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn nav_up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<'app, Self>) {
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

    fn nav_down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.layout_data.supports_multiline {
            // semantically equivalent to line
            self.nav_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
            return;
        }

        if let Some(caret_pos) = self.find_position_in_vertical_direction(1, window.line_height()) {
            self.move_to(caret_pos, cx);
        }
    }

    fn nav_line_start(&mut self, _: &Home, _w: &mut Window, cx: &mut Context<'app, Self>) {
        // [when not multiline] semantically equivalent to document
        self.nav_linear(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn nav_line_end(&mut self, _: &End, _w: &mut Window, cx: &mut Context<'app, Self>) {
        // [when not multiline] semantically equivalent to document
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_start(&mut self, _: &MoveToBeginning, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn nav_end(&mut self, _: &MoveToEnd, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
    }

    fn nav_left_word(&mut self, _: &WordLeft, _w: &mut Window, cx: &mut Context<'app, Self>) {
        self.nav_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn nav_right_word(&mut self, _: &WordRight, _w: &mut Window, cx: &mut Context<'app, Self>) {
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
            //self.scroll_to_cursor();
            cx.notify();
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
            //self.scroll_to_cursor();
            cx.notify();
        }
    }

    fn select_start(
        &mut self,
        _: &SelectToBeginning,
        _w: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        self.select_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn select_end(&mut self, _: &SelectToEnd, _w: &mut Window, cx: &mut Context<'app, Self>) {
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

        if !self.selected_range.is_empty() {
            // Cut selected text
            let slice = &self.storage.content_utf8()[self.selected_range.clone()];
            cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
            self.replace_text_in_range_bytes(self.selected_range.clone(), "", cx);
        } else {
            // No selection: cut the entire current line (including newline)
            let caret = self.caret_pos();
            let line_start = self.storage.find_line_start(caret);
            let line_end = self.storage.find_line_end(caret);
            let storage_len_utf8 = self.storage.content_utf8().len();

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

            self.selected_range = cut_start..cut_end;

            let slice = &self.storage.content_utf8()[self.selected_range.clone()];
            cx.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));

            self.replace_text_in_range_bytes(self.selected_range.clone(), "", cx);
        }
        cx.emit(TextChanged);
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _w: &mut Window, cx: &mut Context<'app, Self>) {
        if !self.selected_range.is_empty() {
            let slice = &self.storage.content_utf8()[self.selected_range.clone()];
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
        self.replace_text_in_range_bytes(self.ime_resolve_range(None), &text, cx);
        cx.emit(TextChanged);
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _w: &mut Window, _cx: &mut Context<'app, Self>) {
        // TODO: STUB
    }

    fn redo(&mut self, _: &Redo, _w: &mut Window, _cx: &mut Context<'app, Self>) {
        // TODO: STUB
    }

    fn on_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        text_position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<'app, Self>,
    ) {
        let character_pos = self.index_for_pixel_point(text_position, window.line_height());

        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        let is_same_position = self
            .last_click_position
            .map(|last| {
                let threshold = gpui::px(4.);
                (text_position.x - last.x).abs() < threshold
                    && (text_position.y - last.y).abs() < threshold
            })
            .unwrap_or(false);

        if is_same_position && event.click_count > 1 {
            self.click_count = event.click_count;
        } else {
            self.click_count = 1;
        }
        self.last_click_position = Some(text_position);

        match self.click_count {
            2 => {
                let (word_start, word_end) = self.storage.word_range_at(character_pos);
                self.selected_range = word_start..word_end;
                cx.notify();
            }
            3 => {
                let line_start = self.storage.find_line_start(character_pos);
                let line_end = self.storage.find_line_end(character_pos);
                let line_end_with_newline = if line_end < self.storage.content_utf8().len() {
                    line_end + 1
                } else {
                    line_end
                };
                self.selected_range = line_start..line_end_with_newline;
                cx.notify();
            }
            _ => {
                if event.modifiers.shift {
                    self.select_to(character_pos, cx);
                } else {
                    self.move_to(character_pos, cx);
                }
            }
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
