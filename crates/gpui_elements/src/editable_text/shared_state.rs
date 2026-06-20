use crate::editable_text::{
    TextBoundary, UnicodeTextStorage,
    notify::{TextChanged, TextHistoryPushed},
};
use gpui::{
    App, Bounds, ClipboardItem, Entity, FocusHandle, Focusable, Hsla, NavigationDirection, Pixels,
    Point, ShapedLine, SharedString, TextRun, TextStyle, UTF16Selection, Window, WrappedLine,
};
use std::{ops::Range, sync::Arc};

pub trait TextStateNotifier {
    fn notify_changed(&mut self);
    fn emit_text_changed(&mut self, event: TextChanged);
    fn emit_history(&mut self, event: TextHistoryPushed);
}

pub(super) trait StateBackedElement {
    type State: 'static;
    type InitProps: 'static;

    fn init_props(&self) -> Self::InitProps;

    fn get_or_init_state(
        init_props: &Self::InitProps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self::State>;

    fn get_state(&self, window: &mut Window, cx: &mut App) -> Entity<Self::State> {
        Self::get_or_init_state(&self.init_props(), window, cx)
    }
}

pub struct TextInputStateBase {
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

    layout_wrapping: TextLayoutWrapping,
    layout_data: TextInputLayoutData,
}

#[derive(Default)]
pub(super) struct TextLayoutWrapping {
    text_style: TextStyle,
    wrap_width: Option<Pixels>,
    dirty: bool,
}
impl TextLayoutWrapping {
    pub fn new(text_style: TextStyle, wrap_width: Option<Pixels>) -> Self {
        Self {
            text_style,
            wrap_width,
            dirty: false,
        }
    }
}

#[derive(Default)]
pub(super) struct TextInputLayoutData {
    /// The `ShapedLine` produced by the painter's `prepaint`.
    /// Cached so IME `bounds_for_range` / `character_index_for_point` can evaluate without re-shaping.
    pub lines: Vec<TextLineSegment>,
    /// The bounds of the text area, in window coordinates.
    /// Cached for IME operations.
    pub bounds: Bounds<Pixels>,
}
pub(super) struct TextLineSegment {
    /// The utf8 byte range in the content string that this line covers.
    pub text_range: Range<usize>,
    /// The shaped and wrapped text for this line, if available.
    pub wrapped_line: Option<Arc<WrappedLine>>,

    /// The y-coordinate of this segment which can be multiplied by the line_height
    /// to get its pixel location relative to the bounds of the text area.
    pub pos_y: usize,
    /// The number of segments up to and including this segment in the literal line that has been wrapped.
    /// There may be other segments after this one with a larger counter.
    pub num_visual_lines: usize,
}

impl Focusable for TextInputStateBase {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInputStateBase {
    pub fn new(storage: impl Into<Box<dyn UnicodeTextStorage>>, cx: &mut App) -> Self {
        Self {
            storage: storage.into(),

            selected_range: 0..0,
            marked_range: None,

            is_selecting: false,
            last_click_position: None,
            click_count: 0,

            focus_handle: cx.focus_handle(),

            layout_wrapping: TextLayoutWrapping {
                dirty: true,
                ..TextLayoutWrapping::default()
            },
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

    pub(super) fn layout_data(&self) -> &TextInputLayoutData {
        &self.layout_data
    }

    pub(super) fn layout_data_mut(&mut self) -> &mut TextInputLayoutData {
        &mut self.layout_data
    }
}

impl TextInputStateBase {
    pub fn apply_wrapping(
        &mut self,
        wrapping: TextLayoutWrapping,
        display_text: &str,
        window: &Window,
    ) {
        let dirty = self.layout_wrapping.dirty
            || self.layout_wrapping.wrap_width != wrapping.wrap_width
            || self.layout_wrapping.text_style != wrapping.text_style;
        self.layout_wrapping = wrapping;
        if dirty {
            self.layout_data.lines = self.build_wrapped_lines(display_text, window);
        }
    }

    pub fn line_segments(&self) -> &Vec<TextLineSegment> {
        &self.layout_data.lines
    }

    fn build_wrapped_lines(&self, content: &str, window: &Window) -> Vec<TextLineSegment> {
        let text_style = &self.layout_wrapping.text_style;
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let mut lines = Vec::new();

        if content.is_empty() {
            lines.push(TextLineSegment {
                text_range: 0..0,
                wrapped_line: None,
                pos_y: 0,
                num_visual_lines: 1,
            });
            return lines;
        }

        let mut pos_y = 0;
        let mut current_pos = 0;

        while current_pos < content.len() {
            let line_end = content[current_pos..]
                .find('\n')
                .map(|pos| current_pos + pos)
                .unwrap_or(content.len());

            let line_slice = &content[current_pos..line_end];

            if line_slice.is_empty() {
                lines.push(TextLineSegment {
                    text_range: current_pos..current_pos,
                    wrapped_line: None,
                    pos_y,
                    num_visual_lines: 1,
                });
                pos_y += 1;
            } else {
                let run = TextRun {
                    len: line_slice.len(),
                    font: text_style.font(),
                    // TODO: This is the actual text color that is stored in WrappedLine,
                    // needs to reflect the style color provided by paint
                    color: Hsla::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };

                let wrapped_lines = window
                    .text_system()
                    .shape_text(
                        SharedString::from(line_slice.to_string()),
                        font_size,
                        &[run],
                        self.layout_wrapping.wrap_width,
                        None,
                    )
                    .unwrap_or_default();

                for wrapped in wrapped_lines {
                    let num_visual_lines = wrapped.wrap_boundaries().len() + 1;
                    lines.push(TextLineSegment {
                        text_range: current_pos..line_end,
                        wrapped_line: Some(Arc::new(wrapped)),
                        pos_y,
                        num_visual_lines,
                    });
                    pos_y += num_visual_lines;
                }
            }

            current_pos = if line_end < content.len() {
                line_end + 1
            } else {
                content.len()
            };
        }

        if content.ends_with('\n') {
            lines.push(TextLineSegment {
                text_range: content.len()..content.len(),
                wrapped_line: None,
                pos_y,
                num_visual_lines: 1,
            });
        }

        lines
    }
}

impl TextInputStateBase {
    pub fn ime_text_for_range(
        &self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
    ) -> Option<String> {
        let range = self.storage.utf_range_16to8(&range_utf16);
        let storage_len_utf8 = self.storage.content_utf8().len();
        let clamped_range = range.start.min(storage_len_utf8)..range.end.min(storage_len_utf8);
        adjusted_range.replace(self.storage.utf_range_8to16(&clamped_range));
        Some(self.storage.content_utf8()[clamped_range].to_string())
    }

    pub fn ime_selected_text_range(&self, _ignore_disabled_input: bool) -> Option<UTF16Selection> {
        let selection_range = self.selected_range();
        let direction = self.selection_direction();
        Some(UTF16Selection {
            range: self.storage.utf_range_8to16(&selection_range),
            reversed: direction == Some(NavigationDirection::Back),
        })
    }

    pub fn ime_marked_text_range(&self) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.storage.utf_range_8to16(range))
    }

    pub fn ime_unmark_text(&mut self) {
        self.marked_range = None;
    }

    pub fn ime_resolve_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
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

    fn emit_change_for_undo(
        &self,
        cx: &mut impl TextStateNotifier,
        range: Range<usize>,
        length: usize,
    ) {
        cx.emit_history(TextHistoryPushed::new(
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
        cx: &mut impl TextStateNotifier,
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

        self.emit_change_for_undo(cx, range.clone(), text_to_insert.len());
        self.storage.replace_range(range, text_to_insert);
        self.marked_range = None;
    }

    pub fn ime_mark_text_in_range(&mut self, range: &Range<usize>, text_len: usize) {
        self.marked_range = match text_len {
            0 => None,
            _ => Some(range.start..range.start + text_len),
        };
    }

    pub fn ime_mark_selected_range(
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

impl TextInputStateBase {
    fn move_to(&mut self, caret_pos: usize) {
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        let caret_pos = caret_pos.min(self.storage.content_utf8().len());
        self.selected_range = caret_pos..caret_pos;
        //self.scroll_to_cursor();
        //cx.notify_changed();
    }

    fn select_to(&mut self, caret_pos: usize) {
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        let caret_pos = caret_pos.min(self.storage().content_utf8().len());
        self.selected_range = caret_pos..self.selected_range.start;
        //self.scroll_to_cursor();
        //cx.notify_changed();
    }

    pub fn delete(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        cx: &mut impl TextStateNotifier,
    ) {
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

        cx.emit_text_changed(TextChanged);
        cx.notify_changed();
    }

    pub fn nav_linear(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        _cx: &mut impl TextStateNotifier,
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
        self.move_to(caret_pos);
    }

    pub fn select_all(&mut self, _cx: &mut impl TextStateNotifier) {
        self.selected_range = 0..self.storage.content_utf8().len();
    }

    pub fn select_linear(
        &mut self,
        direction: NavigationDirection,
        boundary: TextBoundary,
        _cx: &mut impl TextStateNotifier,
    ) {
        let caret_pos = self
            .storage
            .offset_from_caret(self.caret_pos(), direction, boundary);
        self.select_to(caret_pos);
    }

    pub fn cut<T>(&mut self, cx: &mut T)
    where
        T: TextStateNotifier + std::ops::Deref<Target = App>,
    {
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
        cx.emit_text_changed(TextChanged);
        cx.notify_changed();
    }

    pub fn copy(&mut self, app: &mut App) {
        if !self.selected_range.is_empty() {
            let slice = &self.storage.content_utf8()[self.selected_range.clone()];
            app.write_to_clipboard(ClipboardItem::new_string(slice.to_string()));
        }
    }

    pub fn paste<T>(&mut self, cx: &mut T)
    where
        T: TextStateNotifier + std::ops::Deref<Target = App>,
    {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.replace_text_in_range_bytes(self.ime_resolve_range(None), &text, cx);
        cx.emit_text_changed(TextChanged);
        cx.notify_changed();
    }

    pub fn on_mouse_down<Context>(
        &mut self,
        position: Point<Pixels>,
        character_pos: usize,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context,
    ) where
        Context: TextStateNotifier + std::ops::DerefMut<Target = App>,
    {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        let is_same_position = self
            .last_click_position
            .map(|last| {
                let threshold = gpui::px(4.);
                (position.x - last.x).abs() < threshold && (position.y - last.y).abs() < threshold
            })
            .unwrap_or(false);

        if is_same_position && click_count > 1 {
            self.click_count = click_count;
        } else {
            self.click_count = 1;
        }
        self.last_click_position = Some(position);

        match self.click_count {
            2 => {
                let (word_start, word_end) = self.storage.word_range_at(character_pos);
                self.selected_range = word_start..word_end;
                //cx.notify();
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
                //cx.notify();
            }
            _ => {
                if shift {
                    self.select_to(character_pos);
                } else {
                    self.move_to(character_pos);
                }
            }
        }
    }

    pub fn on_mouse_up(&mut self) {
        self.is_selecting = false;
    }

    pub fn on_mouse_move(&mut self, character_pos: usize, _cx: &mut impl TextStateNotifier) {
        if self.is_selecting && self.click_count == 1 {
            self.select_to(character_pos);
        }
    }
}
