use super::actions::*;
use crate::input::InputLayoutStyle;
use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, EntityId, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, NavigationDirection, Pixels, Point, SharedString, Size, Subscription,
    TextRun, TextStyle, Window, WrappedLine, point, px,
};
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

/// Events emitted by InputState when significant changes occur.
#[derive(Clone, Debug)]
pub enum InputStateEvent {
    /// Emitted when the input gains focus.
    Focus,
    /// Emitted when the input loses focus.
    Blur,
    /// Emitted when the text content changes.
    TextChanged,
    /// Emitted when an undo operation is performed.
    Undo,
    /// Emitted when a redo operation is performed.
    Redo,
}

impl EventEmitter<InputStateEvent> for InputState {}

/// `Input` is the state model for text input components. It handles:
/// - Text content storage and manipulation
/// - Selection and cursor management
/// - Keyboard navigation and editing actions
/// - IME (Input Method Editor) support via `EntityInputHandler`
pub struct InputState {
    entity_id: EntityId,
    focus_handle: FocusHandle,
    content: String,
    pub(super) selected_range: Range<usize>,
    pub(super) selection_direction: NavigationDirection,
    pub(super) marked_range: Option<Range<usize>>,
    pub(super) logical_lines: Vec<InputLogicalLine>,
    // refreshed each update by the element, for conveinent access in mutations and painting
    pub(super) layout_data: InputLayoutData,
    is_selecting: bool,
    last_click_position: Option<Point<Pixels>>,
    click_count: usize,
    /// Scroll offset - vertical for multiline, horizontal for single-line
    pub(super) scroll_offset: Pixels,
    pub(super) layout_style: InputLayoutStyle,
    history_grouping_interval: Duration,
    /// Stack of previous states for undo.
    undo_stack: Vec<super::HistoryEntry>,
    /// Stack of undone states for redo.
    redo_stack: Vec<super::HistoryEntry>,
    /// Optional entity and subscription tracking the blinking of the text cursor.
    cursor_blink: Option<(Entity<super::CursorBlink>, Subscription)>,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
    /// Cached UTF-16 length of content for faster IME operations.
    /// Lazily computed when None.
    pub(super) cached_utf16_len: Option<usize>,
}

/// Data built during element prepaint that is stored in InputState for conveinence
pub(super) struct InputLayoutData {
    pub text_style: TextStyle,
    pub wrap_width: Option<Pixels>,
    pub available_size: Size<Pixels>,
    pub line_height: Pixels,
    pub dirty: bool,
}
impl Default for InputLayoutData {
    fn default() -> Self {
        Self {
            text_style: Default::default(),
            wrap_width: Default::default(),
            available_size: Default::default(),
            line_height: Default::default(),
            dirty: true,
        }
    }
}

/// Layout information for a single logical line of text in an input.
///
/// A logical line corresponds to content between newlines in the input text.
/// When text wrapping is enabled, a logical line may span multiple visual lines.
#[derive(Clone, Debug)]
pub(super) struct InputLogicalLine {
    /// The utf8 byte range in the content string that this line covers.
    pub text_range: Range<usize>,
    /// The shaped and wrapped text for this line, if available.
    pub wrapped_line: Option<Arc<WrappedLine>>,
    /// The vertical offset from the top of the text area in pixels.
    pub y_offset: Pixels, // TODO: replace with a counter such that the offset is determined by multipling the counter by the line_height
    /// The number of visual lines this logical line spans (due to wrapping).
    pub visual_line_count: usize,
}

pub enum CursorBlinkType<'app> {
    Disabled,
    Enabled {
        app: &'app mut App,
        interval: Option<Duration>,
    },
}

impl Focusable for InputState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// External API
impl InputState {
    /// Creates a new `Input` with the specified multiline setting.
    /// Cursor blinking is enabled by default.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            entity_id: cx.entity_id(),
            focus_handle: cx.focus_handle(),
            content: String::default(),
            selected_range: 0..0,
            selection_direction: NavigationDirection::Forward,
            marked_range: None,
            layout_data: InputLayoutData::default(),
            logical_lines: Vec::new(),
            is_selecting: false,
            last_click_position: None,
            click_count: 0,
            scroll_offset: px(0.),
            layout_style: InputLayoutStyle::SingleLine,
            history_grouping_interval: super::DEFAULT_GROUP_INTERVAL,
            undo_stack: Vec::new(),
            cached_utf16_len: None,
            redo_stack: Vec::new(),
            cursor_blink: None,
            was_focused: false,
        };
        this = this.cursor_blink(CursorBlinkType::Enabled {
            app: cx,
            interval: None,
        });
        this
    }

    /// Configure how often the cursor should blink when the input element has focus.
    pub fn cursor_blink<'app>(mut self, args: CursorBlinkType<'app>) -> Self {
        self.cursor_blink = match args {
            CursorBlinkType::Disabled => None,
            CursorBlinkType::Enabled { app: cx, interval } => {
                let interval = interval.unwrap_or(super::DEFAULT_BLINK_INTERVAL);
                let cursor_blink = cx.new(|cx| super::CursorBlink::new(interval, cx));
                let entity_id = self.entity_id;
                let subscription = cx.observe(&cursor_blink, move |_, cx| cx.notify(entity_id));
                Some((cursor_blink, subscription))
            }
        };
        self
    }

    /// Returns the current text content.
    pub fn content(&self) -> &String {
        &self.content
    }

    /// Sets the text content, resetting selection to the beginning.
    /// This clears the undo/redo history.
    pub fn set_content(&mut self, content: impl AsRef<str>, cx: &mut Context<Self>) {
        let content = self.layout_style.sanitize_content(content.as_ref());
        self.content = content.to_string().into();
        self.selected_range = 0..0;
        self.selection_direction = NavigationDirection::Forward;
        self.marked_range = None;
        self.layout_data.dirty = true;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.cached_utf16_len = None;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Sets the input's layout style (single-line or multi-line/area).
    pub fn with_layout_style(mut self, layout_style: InputLayoutStyle) -> Self {
        self.layout_style = layout_style;
        self
    }

    /// Returns the input's layout style.
    pub fn layout_style(&self) -> InputLayoutStyle {
        self.layout_style
    }

    /// Returns the utf-8 character range that is currently selected within the current state of the text.
    pub fn selected_range(&self) -> &Range<usize> {
        &self.selected_range
    }

    /// Sets the selection range directly.
    pub fn set_selected_range(&mut self, range: Range<usize>) {
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.selected_range = range;
        self.selection_direction = NavigationDirection::Forward;
    }

    /// Returns the current position of the cursor within the utf-8 character range of the current state of the text.
    pub fn cursor_position(&self) -> usize {
        match self.selection_direction {
            NavigationDirection::Back => self.selected_range.start,
            NavigationDirection::Forward => self.selected_range.end,
        }
    }

    /// Returns the marked text range (for IME composition). Marked text range represents a collection of utf-8 characters that are treated as one group. (TBD need better explanation of marked text)
    pub fn marked_range(&self) -> Option<&Range<usize>> {
        self.marked_range.as_ref()
    }

    /// Returns true if the scroll position is at the top.
    pub fn at_top(&self) -> bool {
        self.scroll_offset <= px(0.)
    }

    /// Returns true if the scroll position is at the bottom.
    pub fn at_bottom(&self) -> bool {
        let content_height = self.total_content_height();
        let visible_height = self.layout_data.available_size.height;

        if content_height <= visible_height {
            return true;
        }

        self.scroll_offset + visible_height >= content_height
    }

    /// Returns the scroll progress as a value from 0.0 (top) to 1.0 (bottom).
    pub fn scroll_progress(&self) -> f32 {
        let content_height = self.total_content_height();
        let visible_height = self.layout_data.available_size.height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return 0.0;
        }

        (self.scroll_offset / max_scroll).clamp(0.0, 1.0)
    }

    /// Returns how far the content is scrolled from the top in pixels.
    pub fn distance_from_top(&self) -> Pixels {
        self.scroll_offset.max(px(0.))
    }

    /// Returns how far the content is from the bottom in pixels.
    pub fn distance_from_bottom(&self) -> Pixels {
        let content_height = self.total_content_height();
        let visible_height = self.layout_data.available_size.height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return px(0.);
        }

        (max_scroll - self.scroll_offset).max(px(0.))
    }

    /// Configures how long the input will wait between user-input changes to create new logs in the history for undo/redo.
    /// The interval by default is 300ms (defined by `DEFAULT_GROUP_INTERVAL`).
    pub fn set_history_group_interval(&mut self, interval: Duration) {
        self.history_grouping_interval = interval;
    }

    /// Configures how long the input will wait between user-input changes to create new logs in the history for undo/redo.
    /// The interval by default is 300ms (defined by `DEFAULT_GROUP_INTERVAL`).
    pub fn with_history_group_interval(mut self, interval: Duration) -> Self {
        self.set_history_group_interval(interval);
        self
    }

    /// Returns whether undo is available based on the recorded states.
    pub fn is_undo_available(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns whether redo is currently available based on the recorded states.
    pub fn is_redo_available(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Inserts text at the current cursor position, replacing any content that is currently selected (i.e. `selection_range` is non-empty).
    /// If any of the text is marked, that range will be replaced instead of the selected range.
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or(self.selected_range.clone());
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());

        let text_to_insert = self.layout_style.sanitize_content(text);

        // Record patch for undo before modifying content
        self.push_undo_patch(range.clone(), text_to_insert.len());

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.replace_range(range.clone(), &text_to_insert);

        self.selected_range =
            range.start + text_to_insert.len()..range.start + text_to_insert.len();
        self.marked_range.take();
        self.layout_data.dirty = true;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Deletes the character before the cursor (convenience method for benchmarks).
    pub fn delete_backward(&mut self, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_position()), cx);
        }
        self.insert_text("", cx);
    }

    /// Reverts the last edit.
    pub fn undo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            let selected_range = entry.selected_range.clone();
            let selection_direction = entry.selection_direction;

            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            self.selected_range = selected_range;
            self.selection_direction = selection_direction;
            self.layout_data.dirty = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    /// Restores the last edit reverted by `undo_action` (or the undo action binding).
    pub fn redo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            let undo_entry = entry.apply_redo(&mut self.content);

            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_direction = NavigationDirection::Forward;

            self.undo_stack.push(undo_entry);
            self.layout_data.dirty = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }
}

// Action implementations
impl InputState {
    pub(super) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            // Remember selection to restore
            let selected_range = entry.selected_range.clone();
            let selection_direction = entry.selection_direction;

            // Apply the undo patch and get the redo patch
            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            // Restore selection state
            self.selected_range = selected_range;
            self.selection_direction = selection_direction;
            self.layout_data.dirty = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    pub(super) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            // Apply the redo patch and get the undo patch
            let undo_entry = entry.apply_redo(&mut self.content);

            // The undo entry contains the selection state after the original edit
            // We need to restore cursor to end of inserted text
            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_direction = NavigationDirection::Forward;

            self.undo_stack.push(undo_entry);
            self.layout_data.dirty = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_direction = NavigationDirection::Forward;
        cx.notify();
    }

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.previous_boundary(self.cursor_position());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.next_boundary(self.cursor_position());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    pub(super) fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout_style {
            InputLayoutStyle::SingleLine => {
                // In single-line mode, up moves to start
                self.selected_range = 0..0;
                self.selection_direction = NavigationDirection::Forward;
                self.scroll_to_cursor();
                cx.notify();
            }
            InputLayoutStyle::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_position(), -1) {
                    self.selected_range = new_offset..new_offset;
                    self.selection_direction = NavigationDirection::Forward;
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout_style {
            InputLayoutStyle::SingleLine => {
                // In single-line mode, down moves to end
                let end = self.content.len();
                self.selected_range = end..end;
                self.selection_direction = NavigationDirection::Forward;
                self.scroll_to_cursor();
                cx.notify();
            }
            InputLayoutStyle::MultiLine => {
                if let Some(new_offset) = self.move_vertically(self.cursor_position(), 1) {
                    self.selected_range = new_offset..new_offset;
                    self.selection_direction = NavigationDirection::Forward;
                    self.scroll_to_cursor();
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_position()), cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_position()), cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        match self.layout_style {
            InputLayoutStyle::SingleLine => {
                // In single-line mode, select_up selects to start
                self.select_to(0, cx);
            }
            InputLayoutStyle::MultiLine => {
                let Some(new_offset) = self.move_vertically(self.cursor_position(), -1) else {
                    return;
                };
                self.apply_selection_offset(new_offset);
                self.scroll_to_cursor();
                cx.notify();
            }
        }
    }

    pub(super) fn select_down(
        &mut self,
        _: &SelectDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pause_cursor_blink(cx);
        match self.layout_style {
            InputLayoutStyle::SingleLine => {
                // In single-line mode, select_down selects to end
                self.select_to(self.content.len(), cx);
            }
            InputLayoutStyle::MultiLine => {
                let Some(new_offset) = self.move_vertically(self.cursor_position(), 1) else {
                    return;
                };
                self.apply_selection_offset(new_offset);
                self.scroll_to_cursor();
                cx.notify();
            }
        }
    }

    pub(super) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line_start = self.find_line_start(self.cursor_position());
        self.move_to(line_start, cx);
    }

    pub(super) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let line_end = self.find_line_end(self.cursor_position());
        self.move_to(line_end, cx);
    }

    pub(super) fn move_to_beginning(
        &mut self,
        _: &MoveToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(0, cx);
    }

    pub(super) fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub(super) fn select_to_beginning(
        &mut self,
        _: &SelectToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    pub(super) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.content.len(), cx);
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.previous_word_boundary(self.cursor_position());
        self.move_to(new_pos, cx);
    }

    pub(super) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.next_word_boundary(self.cursor_position());
        self.move_to(new_pos, cx);
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.previous_word_boundary(self.cursor_position());
        self.select_to(new_pos, cx);
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.next_word_boundary(self.cursor_position());
        self.select_to(new_pos, cx);
    }

    pub(super) fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(&self.layout_style, InputLayoutStyle::MultiLine) {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    pub(super) fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\t", window, cx);
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_boundary(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_word_boundary(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_start(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_end(self.cursor_position()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = self.layout_style.sanitize_content(&text);
        self.replace_text_in_range(None, &text, window, cx);
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            // Cut selected text
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        } else {
            // No selection: cut the entire current line (including newline)
            let cursor = self.cursor_position();
            let line_start = self.find_line_start(cursor);
            let line_end = self.find_line_end(cursor);

            // Include the newline character if there is one after the line
            let cut_end = if line_end < self.content.len() {
                line_end + 1 // Include the newline
            } else if line_start > 0 {
                // Last line with no trailing newline - include preceding newline instead
                line_end
            } else {
                line_end
            };

            // For last line, also remove the preceding newline if it exists
            let cut_start = if line_end >= self.content.len() && line_start > 0 {
                line_start - 1 // Include preceding newline for last line
            } else {
                line_start
            };

            let line_text = self.content[cut_start..cut_end].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(line_text));

            self.selected_range = cut_start..cut_end;
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    pub(super) fn on_mouse_down(
        &mut self,
        position: Point<Pixels>,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        let is_same_position = self
            .last_click_position
            .map(|last| {
                let threshold = px(4.);
                (position.x - last.x).abs() < threshold && (position.y - last.y).abs() < threshold
            })
            .unwrap_or(false);

        if is_same_position && click_count > 1 {
            self.click_count = click_count;
        } else {
            self.click_count = 1;
        }
        self.last_click_position = Some(position);

        let character_pos = self.index_for_pixel_point(position);

        match self.click_count {
            2 => {
                let (word_start, word_end) = self.word_range_at(character_pos);
                self.selected_range = word_start..word_end;
                self.selection_direction = NavigationDirection::Forward;
                cx.notify();
            }
            3 => {
                let line_start = self.find_line_start(character_pos);
                let line_end = self.find_line_end(character_pos);
                let line_end_with_newline = if line_end < self.content.len() {
                    line_end + 1
                } else {
                    line_end
                };
                self.selected_range = line_start..line_end_with_newline;
                self.selection_direction = NavigationDirection::Forward;
                cx.notify();
            }
            _ => {
                if shift {
                    self.select_to(character_pos, cx);
                } else {
                    self.move_to(character_pos, cx);
                }
            }
        }
    }

    pub(super) fn on_mouse_up(&mut self, _cx: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(super) fn on_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.is_selecting && self.click_count == 1 {
            self.select_to(self.index_for_pixel_point(position), cx);
        }
    }
}

// Internal implementations
impl InputState {
    pub(super) fn line_height(&self) -> Pixels {
        self.layout_data.line_height
    }

    /// Replaces the provided utf-8 character range with the provided text
    pub(super) fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.content.replace_range(range, &text);
    }

    /// Pauses cursor blinking temporarily (e.g., during typing).
    pub(super) fn pause_cursor_blink(&self, cx: &mut Context<Self>) {
        if let Some((cursor_blink, _)) = &self.cursor_blink {
            cursor_blink.update(cx, |cb, cx| cb.pause_blinking(cx));
        }
    }

    /// Records a patch for undo. Called before making changes to content.
    /// Returns true if a new entry was created, false if grouped with previous.
    pub(super) fn push_undo_patch(&mut self, range: Range<usize>, new_text_len: usize) {
        // Don't record during IME composition
        if self.marked_range.is_some() {
            return;
        }

        let now = Instant::now();

        // Check if we should group with the last entry
        if let Some(last) = self.undo_stack.last() {
            if now.duration_since(last.timestamp) < self.history_grouping_interval {
                // Within group interval - extend the existing patch
                // We need to merge this edit with the previous one
                return;
            }
        }

        // Capture the text that will be replaced
        let old_text = self.content[range.clone()].to_string();

        self.undo_stack.push(super::HistoryEntry {
            range: range.start..range.start + new_text_len,
            old_text,
            new_text_len,
            selected_range: self.selected_range.clone(),
            selection_direction: self.selection_direction,
            timestamp: now,
        });

        // Limit history size
        if self.undo_stack.len() > super::MAX_HISTORY_LEN {
            self.undo_stack.remove(0);
        }

        // New edit invalidates redo stack
        self.redo_stack.clear();
    }

    /// Returns the utf-8 character position of first character after the first new-line preceeding the character at the provided utf-8 character position.
    pub(super) fn find_line_start(&self, position: usize) -> usize {
        self.content[..position.min(self.content.len())]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    /// Returns the utf-8 character position of the character immediately before the first new-line character after the character at the provided utf-8 character position.
    pub(super) fn find_line_end(&self, position: usize) -> usize {
        self.content[position.min(self.content.len())..]
            .find('\n')
            .map(|pos| position + pos)
            .unwrap_or(self.content.len())
    }

    /// Returns the utf-8 character position of the start of the line that contains the provided pixel-point.
    pub(super) fn index_for_pixel_point(&self, point: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        for line in self.logical_lines.iter() {
            let line_height_total = self.line_height() * line.visual_line_count as f32;

            if point.y >= line.y_offset && point.y < line.y_offset + line_height_total {
                if line.text_range.is_empty() {
                    return line.text_range.start;
                }
                let Some(wrapped) = &line.wrapped_line else {
                    return line.text_range.start;
                };

                let relative_y = point.y - line.y_offset;
                let relative_point = gpui::point(point.x, relative_y);

                let closest_result =
                    wrapped.closest_index_for_position(relative_point, self.line_height());

                let local_idx = closest_result.unwrap_or_else(|closest| closest);
                let clamped = local_idx.min(wrapped.text.len());
                return line.text_range.start + clamped;
            }
        }

        self.content.len()
    }

    pub(super) fn scroll_to_cursor(&mut self) {
        if self.logical_lines.is_empty() {
            return;
        }

        let cursor_offset = self.cursor_position();
        match self.layout_style {
            InputLayoutStyle::SingleLine => {
                if self.layout_data.available_size.width <= px(0.) {
                    return;
                }

                // For single-line input, get cursor x position from the first (only) line
                let Some(line) = self.logical_lines.first() else {
                    return;
                };

                let cursor_x = if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = cursor_offset.saturating_sub(line.text_range.start);
                    wrapped
                        .position_for_index(local_offset, self.line_height())
                        .map(|p| p.x)
                        .unwrap_or(px(0.))
                } else {
                    px(0.)
                };

                let visible_left = self.scroll_offset;
                let visible_right = self.scroll_offset + self.layout_data.available_size.width;

                // Add some padding so cursor isn't right at the edge
                let padding = px(2.0);

                if cursor_x < visible_left + padding {
                    self.scroll_offset = (cursor_x - padding).max(px(0.));
                } else if cursor_x > visible_right - padding {
                    self.scroll_offset = cursor_x - self.layout_data.available_size.width + padding;
                }

                self.scroll_offset = self.scroll_offset.max(px(0.));
            }
            InputLayoutStyle::MultiLine => {
                if self.layout_data.available_size.height <= px(0.) {
                    return;
                }

                let line_height = self.line_height();

                for line in &self.logical_lines {
                    let is_cursor_in_line = if line.text_range.is_empty() {
                        cursor_offset == line.text_range.start
                    } else {
                        line.text_range.contains(&cursor_offset)
                            || (cursor_offset == line.text_range.end
                                && cursor_offset == self.content.len())
                    };

                    if is_cursor_in_line {
                        let cursor_visual_y = if let Some(wrapped) = &line.wrapped_line {
                            let local_offset = cursor_offset.saturating_sub(line.text_range.start);
                            if let Some(position) =
                                wrapped.position_for_index(local_offset, self.line_height())
                            {
                                line.y_offset + position.y
                            } else {
                                line.y_offset
                            }
                        } else {
                            line.y_offset
                        };

                        let visible_top = self.scroll_offset;
                        let visible_bottom =
                            self.scroll_offset + self.layout_data.available_size.height;

                        if cursor_visual_y < visible_top {
                            self.scroll_offset = cursor_visual_y;
                        } else if cursor_visual_y + line_height > visible_bottom {
                            self.scroll_offset = (cursor_visual_y + line_height)
                                - self.layout_data.available_size.height;
                        }

                        self.scroll_offset = self.scroll_offset.max(px(0.));
                        break;
                    }
                }
            }
        }
    }

    /// Called internally during window prepaint to layout the content into logical lines based on viewport bounds wrapping.
    pub(super) fn build_logical_lines(
        content: &str,
        window: &mut Window,
        layout_data: &InputLayoutData,
    ) -> Vec<InputLogicalLine> {
        let text_style = &layout_data.text_style;
        let mut logical_lines = Vec::new();

        let text_color = text_style.color;
        let font_size = text_style.font_size.to_pixels(window.rem_size());

        if content.is_empty() {
            logical_lines.push(InputLogicalLine {
                text_range: 0..0,
                wrapped_line: None,
                y_offset: px(0.),
                visual_line_count: 1,
            });
            return logical_lines;
        }

        let mut y_offset = px(0.);
        let mut current_pos = 0;

        while current_pos < content.len() {
            let line_end = content[current_pos..]
                .find('\n')
                .map(|pos| current_pos + pos)
                .unwrap_or(content.len());

            let line_slice = &content[current_pos..line_end];

            if line_slice.is_empty() {
                logical_lines.push(InputLogicalLine {
                    text_range: current_pos..current_pos,
                    wrapped_line: None,
                    y_offset,
                    visual_line_count: 1,
                });
                y_offset += layout_data.line_height;
            } else {
                let run = TextRun {
                    len: line_slice.len(),
                    font: text_style.font(),
                    color: text_color,
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
                        layout_data.wrap_width,
                        None,
                    )
                    .unwrap_or_default();

                for wrapped in wrapped_lines {
                    let visual_line_count = wrapped.wrap_boundaries().len() + 1;
                    let line_height_total = layout_data.line_height * visual_line_count as f32;

                    logical_lines.push(InputLogicalLine {
                        text_range: current_pos..line_end,
                        wrapped_line: Some(Arc::new(wrapped)),
                        y_offset,
                        visual_line_count,
                    });

                    y_offset += line_height_total;
                }
            }

            current_pos = if line_end < content.len() {
                line_end + 1
            } else {
                content.len()
            };
        }

        if content.ends_with('\n') {
            logical_lines.push(InputLogicalLine {
                text_range: content.len()..content.len(),
                wrapped_line: None,
                y_offset,
                visual_line_count: 1,
            });
        }

        logical_lines
    }

    /// Processes a focus-flag update during window paint, returning whether the cursor should be visible in this frame.
    /// Returns false if the cursor is blinking and not currently visible.
    pub(super) fn toggle_cursor_on_focus_change(
        &mut self,
        is_focused: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        // Update cursor blink based on focus changes
        let was_focused = self.was_focused;
        self.was_focused = is_focused;

        match &self.cursor_blink {
            None => true,
            Some((cursor_blink, _)) => match (is_focused, was_focused) {
                (true, false) => {
                    cursor_blink.update(cx, |cursor, cx| cursor.enable(cx));
                    cx.emit(InputStateEvent::Focus);
                    true
                }
                (false, true) => {
                    cursor_blink.update(cx, |cursor, cx| cursor.disable(cx));
                    cx.emit(InputStateEvent::Blur);
                    false
                }
                _ => cursor_blink.read(cx).visible(),
            },
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_direction = NavigationDirection::Forward;
        self.scroll_to_cursor();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        self.apply_selection_offset(offset);
        self.scroll_to_cursor();
        cx.notify();
    }

    fn apply_selection_offset(&mut self, offset: usize) {
        match self.selection_direction {
            NavigationDirection::Forward => self.selected_range.end = offset,
            NavigationDirection::Back => self.selected_range.start = offset,
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_direction = match self.selection_direction {
                NavigationDirection::Forward => NavigationDirection::Back,
                NavigationDirection::Back => NavigationDirection::Forward,
            };
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
    }

    fn find_visual_line_and_x_offset(&self, offset: usize) -> (usize, f32) {
        if self.logical_lines.is_empty() {
            return (0, 0.0);
        }

        let mut visual_line_idx = 0;

        for line in &self.logical_lines {
            if line.text_range.is_empty() {
                if offset == line.text_range.start {
                    return (visual_line_idx, 0.0);
                }
            } else if offset >= line.text_range.start && offset <= line.text_range.end {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = (offset - line.text_range.start).min(wrapped.text.len());
                    if let Some(position) =
                        wrapped.position_for_index(local_offset, self.line_height())
                    {
                        let visual_line_within = (position.y / self.line_height()).floor() as usize;
                        return (visual_line_idx + visual_line_within, position.x.into());
                    }
                }
                return (visual_line_idx, 0.0);
            }
            visual_line_idx += line.visual_line_count;
        }

        (visual_line_idx.saturating_sub(1), 0.0)
    }

    fn move_vertically(&self, offset: usize, direction: i32) -> Option<usize> {
        let (visual_line_idx, x_pixels) = self.find_visual_line_and_x_offset(offset);
        let target_visual_line_idx = (visual_line_idx as i32 + direction).max(0) as usize;

        let mut current_visual_line = 0;
        for layout in self.logical_lines.iter() {
            let visual_lines_in_layout = layout.visual_line_count;

            if target_visual_line_idx < current_visual_line + visual_lines_in_layout {
                let visual_line_within_layout = target_visual_line_idx - current_visual_line;

                if layout.text_range.is_empty() {
                    return Some(layout.text_range.start);
                }

                if let Some(wrapped) = &layout.wrapped_line {
                    let y_within_wrapped = self.line_height() * visual_line_within_layout as f32;
                    let target_point = point(px(x_pixels), y_within_wrapped);

                    let closest_result =
                        wrapped.closest_index_for_position(target_point, self.line_height());

                    let closest_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = closest_idx.min(wrapped.text.len());
                    let result = layout.text_range.start + clamped;

                    return Some(result);
                }

                return Some(layout.text_range.start);
            }

            current_visual_line += visual_lines_in_layout;
        }

        if direction > 0 {
            Some(self.content.len())
        } else {
            None
        }
    }

    pub(super) fn total_content_height(&self) -> Pixels {
        self.logical_lines
            .last()
            .map(|last| last.y_offset + self.line_height() * last.visual_line_count as f32)
            .unwrap_or(px(0.))
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];
        text_before
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];
        text_after
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| offset + i)
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];

        let mut last_word_start = 0;
        for (idx, _) in text_before.unicode_word_indices() {
            if idx < offset {
                last_word_start = idx;
            }
        }

        if last_word_start == 0 && offset > 0 {
            let trimmed = text_before.trim_end();
            if trimmed.is_empty() {
                return 0;
            }
            for (idx, _) in trimmed.unicode_word_indices() {
                last_word_start = idx;
            }
        }

        last_word_start
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];

        for (idx, word) in text_after.unicode_word_indices() {
            let word_end = offset + idx + word.len();
            if word_end > offset {
                return word_end;
            }
        }

        self.content.len()
    }

    fn word_range_at(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());

        for (idx, word) in self.content.unicode_word_indices() {
            let word_end = idx + word.len();
            if offset >= idx && offset <= word_end {
                return (idx, word_end);
            }
        }

        (offset, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Entity, IntoElement, Render, TestAppContext, WindowHandle, div};

    struct TestView {
        input: Entity<InputState>,
    }

    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn create_test_input(
        cx: &mut TestAppContext,
        content: &str,
        range: std::ops::Range<usize>,
    ) -> WindowHandle<TestView> {
        cx.add_window(|_window, cx| {
            let input = cx.new(|cx| {
                let mut input = InputState::new(cx).with_layout_style(InputLayoutStyle::MultiLine);
                input.content = content.to_string().into();
                input.selected_range = range;
                input
            });
            TestView { input }
        })
    }

    #[allow(dead_code)]
    fn create_test_input_with_layout(
        cx: &mut TestAppContext,
        content: &str,
        range: std::ops::Range<usize>,
    ) -> WindowHandle<TestView> {
        let view = cx.add_window(|window, cx| {
            let input = cx.new(|cx| {
                let mut input = InputState::new(cx).with_layout_style(InputLayoutStyle::MultiLine);
                input.content = content.to_string().into();
                input.selected_range = range;
                input.layout_data.line_height = px(20.);
                input.layout_data.wrap_width = Some(px(500.));
                input.logical_lines =
                    InputState::build_logical_lines(&input.content, window, &input.layout_data);
                input
            });
            TestView { input }
        });
        view
    }

    // ============================================================
    // BASIC MOVEMENT
    // ============================================================

    #[gpui::test]
    fn test_left_at_start_of_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_moves_by_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 3..3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 2..2);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_collapses_selection_to_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 1..4);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 1..1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_stops_at_end_of_line(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 3 (start of "cd", after newline)
        // Pressing left should move to position 2 (end of "ab", before newline)
        let view = create_test_input(cx, "ab\ncd", 3..3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 2..2); // cursor at end of line 1
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_at_end_of_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_moves_by_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 2..2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 3..3);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_collapses_selection_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 1..4);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 4..4);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_stops_at_end_of_line(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 1 (after 'a')
        // Pressing right should move to position 2 (end of "ab", before newline)
        let view = create_test_input(cx, "ab\ncd", 1..1);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 2..2); // cursor at end of line 1
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_right_crosses_newline(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 2 (end of "ab", before newline)
        // Pressing right should move to position 3 (after newline, start of "cd")
        let view = create_test_input(cx, "ab\ncd", 2..2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 3..3); // cursor at start of line 2
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_left_crosses_newline(cx: &mut TestAppContext) {
        // "ab\ncd" - cursor at position 2 (end of "ab", before newline)
        // Pressing left should move to position 1 (after 'a')
        let view = create_test_input(cx, "ab\ncd", 2..2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 1..1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_home_moves_to_line_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond", 9..9);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.home(&Home, window, cx);
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_end_moves_to_line_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond", 8..8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.end(&End, window, cx);
                assert_eq!(input.selected_range, 12..12);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_move_to_beginning(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 9..9);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.move_to_beginning(&MoveToBeginning, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_move_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.move_to_end(&MoveToEnd, window, cx);
                assert_eq!(input.selected_range, 18..18);
            });
        })
        .unwrap();
    }

    // ============================================================
    // WORD MOVEMENT
    // ============================================================

    #[gpui::test]
    fn test_word_left_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.word_left(&WordLeft, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_left_stops_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world test", 11..11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.word_left(&WordLeft, window, cx);
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_right_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11..11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.word_right(&WordRight, window, cx);
                assert_eq!(input.selected_range, 11..11);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_right_stops_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world test", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.word_right(&WordRight, window, cx);
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    // ============================================================
    // SELECTION
    // ============================================================

    #[gpui::test]
    fn test_select_left_extends_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 3..3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_left(&SelectLeft, window, cx);
                assert_eq!(input.selected_range, 2..3);
                assert_eq!(input.selection_direction, NavigationDirection::Back);
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
                assert_eq!(input.selected_range, 2..3);
                assert_eq!(input.selection_direction, NavigationDirection::Forward);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_all(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello\nworld", 3..3);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_all(&SelectAll, window, cx);
                assert_eq!(input.selected_range, 0..11);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_to_beginning(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6..6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_to_beginning(&SelectToBeginning, window, cx);
                assert_eq!(input.selected_range, 0..6);
                assert_eq!(input.selection_direction, NavigationDirection::Back);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_to_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6..6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_to_end(&SelectToEnd, window, cx);
                assert_eq!(input.selected_range, 6..11);
                assert_eq!(input.selection_direction, NavigationDirection::Forward);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - BACKSPACE
    // ============================================================

    #[gpui::test]
    fn test_backspace_deletes_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6..11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "hello ");
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_previous_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "hell");
                assert_eq!(input.selected_range, 4..4);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_at_start_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "hello");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_entire_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "Hi 👋", 7..7);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "Hi ");
                assert_eq!(input.selected_range, 3..3);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - DELETE
    // ============================================================

    #[gpui::test]
    fn test_delete_deletes_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_deletes_next_grapheme(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), "ello");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_at_end_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), "hello");
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDITING - ENTER
    // ============================================================

    #[gpui::test]
    fn test_enter_inserts_newline(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.enter(&Enter, window, cx);
                assert_eq!(input.content(), "hello\n world");
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_enter_replaces_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.enter(&Enter, window, cx);
                assert_eq!(input.content(), "hello\nworld");
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    // ============================================================
    // CLIPBOARD
    // ============================================================

    #[gpui::test]
    fn test_copy_with_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6..11);
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
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("hello"));
    }

    #[gpui::test]
    fn test_paste_inserts_text(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        cx.write_to_clipboard(ClipboardItem::new_string(" there".to_string()));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.paste(&Paste, window, cx);
                assert_eq!(input.content(), "hello there world");
                assert_eq!(input.selected_range, 11..11);
            });
        })
        .unwrap();
    }

    // ============================================================
    // UNICODE / GRAPHEME HANDLING
    // ============================================================

    #[gpui::test]
    fn test_movement_with_multibyte_utf8(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "café", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 1..1);
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 2..2);
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 3..3);
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_movement_with_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a👋b", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 1..1);
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 5..5);
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 6..6);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_selection_with_multibyte_characters(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "日本語", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, 0..3);
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, 0..6);
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, 0..9);
            });
        })
        .unwrap();
    }

    // ============================================================
    // NEWLINE HANDLING
    // ============================================================

    #[gpui::test]
    fn test_find_line_start_and_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "first\nsecond\nthird", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                assert_eq!(input.find_line_start(0), 0);
                assert_eq!(input.find_line_start(3), 0);
                assert_eq!(input.find_line_start(6), 6);
                assert_eq!(input.find_line_start(13), 13);

                assert_eq!(input.find_line_end(0), 5);
                assert_eq!(input.find_line_end(6), 12);
                assert_eq!(input.find_line_end(13), 18);
            });
        })
        .unwrap();
    }

    // ============================================================
    // EDGE CASES
    // ============================================================

    #[gpui::test]
    fn test_operations_on_empty_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range, 0..0);

                input.right(&Right, window, cx);
                assert_eq!(input.selected_range, 0..0);

                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "");

                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), "");

                input.select_all(&SelectAll, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_set_content_resets_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 3..8);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, cx| {
                input.selection_direction = NavigationDirection::Back;
                input.marked_range = Some(5..7);
                input.set_content("new content", cx);
                assert_eq!(input.content(), "new content");
                assert_eq!(input.selected_range, 0..0);
                assert_eq!(input.selection_direction, NavigationDirection::Forward);
                assert_eq!(input.marked_range, None);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cursor_clamped_to_content_length(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 100..100);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, cx| {
                input.move_to(1000, cx);
                assert_eq!(input.selected_range, 5..5);

                input.selected_range = 0..0;
                input.select_to(1000, cx);
                assert_eq!(input.selected_range, 0..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_previous_boundary_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                assert_eq!(input.previous_boundary(0), 0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_next_boundary_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                assert_eq!(input.next_boundary(5), 5);
                assert_eq!(input.next_boundary(100), 5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_word_range_at_boundary(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                let (start, end) = input.word_range_at(5);
                assert_eq!(start, 0);
                assert_eq!(end, 5);

                let (start, end) = input.word_range_at(8);
                assert_eq!(start, 6);
                assert_eq!(end, 11);
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
        let view = create_test_input(cx, "a😀b", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                // Move right through: a -> 😀 -> b
                input.right(&Right, window, cx);
                assert_eq!(input.selected_range.start, 1); // after 'a'

                input.right(&Right, window, cx);
                assert_eq!(input.selected_range.start, 5); // after 😀 (1 + 4 bytes)

                input.right(&Right, window, cx);
                assert_eq!(input.selected_range.start, 6); // after 'b'

                // Move left back
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range.start, 5); // before 'b'

                input.left(&Left, window, cx);
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

        let view = create_test_input(cx, &format!("a{}b", emoji), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.right(&Right, window, cx); // past entire emoji with modifier
                assert_eq!(input.selected_range.start, 9); // 1 + 8

                input.left(&Left, window, cx); // back before emoji
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

        let view = create_test_input(cx, &format!("x{}y", family), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'x'
                assert_eq!(input.selected_range.start, 1);

                input.right(&Right, window, cx); // past entire ZWJ sequence
                assert_eq!(input.selected_range.start, 19); // 1 + 18

                input.right(&Right, window, cx); // past 'y'
                assert_eq!(input.selected_range.start, 20);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_deletes_emoji_between_ascii(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 5..5); // cursor after emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "ab");
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
                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "ab");
                assert_eq!(input.selected_range.start, 1);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_removes_entire_emoji(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 1..1); // cursor before emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), "ab");
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

        let view = create_test_input(cx, &format!("x{}y", flag), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'x'
                input.right(&Right, window, cx); // past flag (should be single grapheme)
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

        let view = create_test_input(cx, &format!("a{}b", combining), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.right(&Right, window, cx); // past e + combining mark (single grapheme)
                assert_eq!(input.selected_range.start, 4); // 1 + 3

                input.left(&Left, window, cx);
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

        let view = create_test_input(cx, &format!("x{}y", multi_combining), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'x'
                input.right(&Right, window, cx); // past entire combined character
                assert_eq!(input.selected_range.start, 6); // 1 + 5
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_select_emoji_with_shift(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "a😀b", 1..1); // cursor before emoji
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_right(&SelectRight, window, cx);
                assert_eq!(input.selected_range, 1..5); // selected the entire emoji
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cjk_characters(cx: &mut TestAppContext) {
        // 你好 - each character is 3 bytes in UTF-8
        let view = create_test_input(cx, "a你好b", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'a'
                assert_eq!(input.selected_range.start, 1);

                input.right(&Right, window, cx); // past 你
                assert_eq!(input.selected_range.start, 4); // 1 + 3

                input.right(&Right, window, cx); // past 好
                assert_eq!(input.selected_range.start, 7); // 4 + 3

                input.right(&Right, window, cx); // past 'b'
                assert_eq!(input.selected_range.start, 8);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_mixed_script_text(cx: &mut TestAppContext) {
        // Mix of ASCII, CJK, and emoji
        let view = create_test_input(cx, "Hi你😀", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'H'
                assert_eq!(input.selected_range.start, 1);

                input.right(&Right, window, cx); // past 'i'
                assert_eq!(input.selected_range.start, 2);

                input.right(&Right, window, cx); // past 你 (3 bytes)
                assert_eq!(input.selected_range.start, 5);

                input.right(&Right, window, cx); // past 😀 (4 bytes)
                assert_eq!(input.selected_range.start, 9);

                // Now go back
                input.left(&Left, window, cx);
                assert_eq!(input.selected_range.start, 5);

                input.left(&Left, window, cx);
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

        let view = create_test_input(cx, &format!("a{}b", emoji_presentation), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'a'
                input.right(&Right, window, cx); // past emoji with variation selector
                assert_eq!(input.selected_range.start, 7); // 1 + 6
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_keycap_emoji(cx: &mut TestAppContext) {
        // 1️⃣ = 1 + variation selector + combining enclosing keycap
        let keycap = "1\u{FE0F}\u{20E3}";

        let view = create_test_input(cx, &format!("x{}y", keycap), 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.right(&Right, window, cx); // past 'x'
                input.right(&Right, window, cx); // past keycap sequence
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
        selected_range: Range<usize>,
    ) -> WindowHandle<TestView> {
        cx.add_window(|_window, cx| {
            let input = cx.new(|cx| {
                let mut input = InputState::new(cx).with_layout_style(InputLayoutStyle::SingleLine);
                input.content = content.to_string().into();
                input.selected_range = selected_range;
                input
            });
            TestView { input }
        })
    }

    #[gpui::test]
    fn test_single_line_enter_does_nothing(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.enter(&Enter, window, cx);
                assert_eq!(input.content(), "hello");
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_set_content_strips_newlines(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_content("hello\nworld\r\nfoo", cx);
                assert_eq!(input.content(), "hello world foo");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_up_moves_to_start(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.up(&Up, window, cx);
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_down_moves_to_end(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.down(&Down, window, cx);
                assert_eq!(input.selected_range, 11..11); // "hello world".len() == 11
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_select_up_selects_to_start(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_up(&SelectUp, window, cx);
                assert_eq!(input.selected_range, 0..5);
                assert_eq!(input.selection_direction, NavigationDirection::Back);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_select_down_selects_to_end(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.select_down(&SelectDown, window, cx);
                assert_eq!(input.selected_range, 5..11); // "hello world".len() == 11
                assert_eq!(input.selection_direction, NavigationDirection::Forward);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_single_line_multiline_getter(cx: &mut TestAppContext) {
        let view = create_single_line_input(cx, "hello", 0..0);
        view.update(cx, |view, _window, cx| {
            view.input.update(cx, |input, _cx| {
                assert_eq!(input.layout_style(), InputLayoutStyle::SingleLine);
            });
        })
        .unwrap();

        let multiline_view = create_test_input(cx, "hello", 0..0);
        multiline_view
            .update(cx, |view, _window, cx| {
                view.input.update(cx, |input, _cx| {
                    assert_eq!(input.layout_style(), InputLayoutStyle::MultiLine);
                });
            })
            .unwrap();
    }

    // ============================================================
    // UNDO / REDO
    // ============================================================

    #[gpui::test]
    fn test_undo_restores_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                // Disable grouping for predictable test behavior
                input.set_history_group_interval(Duration::from_secs(0));

                // Make an edit
                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.content(), "hello world");

                // Undo should restore original content
                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_redo_restores_undone_content(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.content(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");

                input.redo(&Redo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_undo_with_no_history_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                assert!(!input.is_undo_available());
                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_redo_with_no_history_does_nothing(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                assert!(!input.is_redo_available());
                input.redo(&Redo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_undo_restores_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                // Delete selection
                input.replace_text_in_range(None, "", window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);

                // Undo should restore content and selection
                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
                assert_eq!(input.selected_range, 0..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_multiple_undo_redo(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.replace_text_in_range(None, "a", window, cx);
                input.replace_text_in_range(None, "b", window, cx);
                input.replace_text_in_range(None, "c", window, cx);
                assert_eq!(input.content(), "abc");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "ab");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "a");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "");

                input.redo(&Redo, window, cx);
                assert_eq!(input.content(), "a");

                input.redo(&Redo, window, cx);
                assert_eq!(input.content(), "ab");

                input.redo(&Redo, window, cx);
                assert_eq!(input.content(), "abc");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_new_edit_clears_redo_stack(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.replace_text_in_range(None, " world", window, cx);
                assert_eq!(input.content(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
                assert!(input.is_redo_available());

                // New edit should clear redo stack
                input.replace_text_in_range(None, "!", window, cx);
                assert_eq!(input.content(), "hello!");
                assert!(!input.is_redo_available());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_set_content_clears_history(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.replace_text_in_range(None, " world", window, cx);
                assert!(input.is_undo_available());

                input.set_content("new content", cx);
                assert!(!input.is_undo_available());
                assert!(!input.is_redo_available());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_can_undo_can_redo(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                assert!(!input.is_undo_available());
                assert!(!input.is_redo_available());

                input.replace_text_in_range(None, "!", window, cx);
                assert!(input.is_undo_available());
                assert!(!input.is_redo_available());

                input.undo(&Undo, window, cx);
                assert!(!input.is_undo_available());
                assert!(input.is_redo_available());

                input.redo(&Redo, window, cx);
                assert!(input.is_undo_available());
                assert!(!input.is_redo_available());
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_backspace_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.backspace(&Backspace, window, cx);
                assert_eq!(input.content(), "hell");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.delete(&Delete, window, cx);
                assert_eq!(input.content(), "ello");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cut_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_cut_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor in middle line, no selection - should cut entire line including newline
        let view = create_test_input(cx, "line1\nline2\nline3", 8..8); // cursor in "line2"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "line1\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("line2\n"));
    }

    #[gpui::test]
    fn test_cut_first_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor on first line, no selection
        let view = create_test_input(cx, "line1\nline2\nline3", 2..2); // cursor in "line1"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "line2\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("line1\n"));
    }

    #[gpui::test]
    fn test_cut_last_line_with_no_selection(cx: &mut TestAppContext) {
        // Cursor on last line, no selection - should include preceding newline
        let view = create_test_input(cx, "line1\nline2\nline3", 14..14); // cursor in "line3"
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "line1\nline2");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("\nline3"));
    }

    #[gpui::test]
    fn test_cut_empty_line(cx: &mut TestAppContext) {
        // Cursor on empty line - should remove that line
        let view = create_test_input(cx, "line1\n\nline3", 6..6); // cursor on empty line
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "line1\nline3");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("\n"));
    }

    #[gpui::test]
    fn test_cut_only_line_with_no_selection(cx: &mut TestAppContext) {
        // Single line content, no selection - should cut entire content
        let view = create_test_input(cx, "hello", 2..2);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "");
            });
        })
        .unwrap();

        let clipboard = cx.read_from_clipboard();
        assert_eq!(clipboard.unwrap().text().as_deref(), Some("hello"));
    }

    #[gpui::test]
    fn test_cut_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "line1\nline2\nline3", 8..8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.cut(&Cut, window, cx);
                assert_eq!(input.content(), "line1\nline3");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "line1\nline2\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_paste_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello", 5..5);
        cx.write_to_clipboard(ClipboardItem::new_string(" world".to_string()));
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.paste(&Paste, window, cx);
                assert_eq!(input.content(), "hello world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_enter_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.enter(&Enter, window, cx);
                assert_eq!(input.content(), "hello\n world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left(cx: &mut TestAppContext) {
        // Cursor at end of "hello" in "hello world"
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_with_selection(cx: &mut TestAppContext) {
        // Selection from 0 to 5 ("hello")
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.content(), " world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right(cx: &mut TestAppContext) {
        // Cursor at start
        let view = create_test_input(cx, "hello world", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_with_selection(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.content(), " world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11..11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_beginning_of_line(&DeleteToBeginningOfLine, window, cx);
                assert_eq!(input.content(), " world");
                assert_eq!(input.selected_range, 0..0);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_multiline(cx: &mut TestAppContext) {
        // Cursor at position 8 (middle of "line2")
        let view = create_test_input(cx, "line1\nline2\nline3", 8..8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_beginning_of_line(&DeleteToBeginningOfLine, window, cx);
                assert_eq!(input.content(), "line1\nne2\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_at_start(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 0..0);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_beginning_of_line(&DeleteToBeginningOfLine, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_end_of_line(&DeleteToEndOfLine, window, cx);
                assert_eq!(input.content(), "hello");
                assert_eq!(input.selected_range, 5..5);
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_multiline(cx: &mut TestAppContext) {
        // Cursor at position 8 (middle of "line2")
        let view = create_test_input(cx, "line1\nline2\nline3", 8..8);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_end_of_line(&DeleteToEndOfLine, window, cx);
                assert_eq!(input.content(), "line1\nli\nline3");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_at_end(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 11..11);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.delete_to_end_of_line(&DeleteToEndOfLine, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_left_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.delete_word_left(&DeleteWordLeft, window, cx);
                assert_eq!(input.content(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_word_right_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 6..6);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.delete_word_right(&DeleteWordRight, window, cx);
                assert_eq!(input.content(), "hello ");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_beginning_of_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.delete_to_beginning_of_line(&DeleteToBeginningOfLine, window, cx);
                assert_eq!(input.content(), " world");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_delete_to_end_of_line_is_undoable(cx: &mut TestAppContext) {
        let view = create_test_input(cx, "hello world", 5..5);
        view.update(cx, |view, window, cx| {
            view.input.update(cx, |input, cx| {
                input.set_history_group_interval(Duration::from_secs(0));

                input.delete_to_end_of_line(&DeleteToEndOfLine, window, cx);
                assert_eq!(input.content(), "hello");

                input.undo(&Undo, window, cx);
                assert_eq!(input.content(), "hello world");
            });
        })
        .unwrap();
    }
}
