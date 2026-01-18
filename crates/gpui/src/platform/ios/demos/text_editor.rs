//! Text Editor demo - text editing and cursor control.
//!
//! This demo showcases GPUI's text input capabilities on iOS:
//! - Multi-line text display with wrapping
//! - Cursor placement via touch
//! - Text selection
//! - Scrollable content
//! - Keyboard input via UITextInput protocol

use super::{BACKGROUND, GREEN, SUBTEXT, SURFACE, TEXT};
use crate::{
    App, Bounds, Context, EntityInputHandler, FocusHandle, Focusable,
    Hsla, Pixels, Point, SharedString, TextRun, UTF16Selection, Window,
    div, fill, hsla, point, prelude::*, px, rgb, size,
};
use std::ops::Range;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// Global counter for replace_text_in_range calls (for debugging)
static REPLACE_CALL_COUNT: AtomicU32 = AtomicU32::new(0);

/// Sample text content for the demo
const SAMPLE_TEXT: &str = r#"Welcome to GPUI on iOS!

This text editor demo showcases the text rendering and input capabilities of the GPUI framework.

Features demonstrated:
- Multi-line text rendering
- Touch-based cursor placement
- Text selection via drag
- Smooth scrolling

Try tapping anywhere in the text to place the cursor, or drag to select text.

GPUI uses GPU-accelerated rendering to achieve smooth 60fps performance even with complex UI layouts and animations.

The framework provides a declarative API similar to SwiftUI, making it easy to build responsive user interfaces that work across platforms.

Additional Content for Scrolling

This section contains extra text to demonstrate the scrolling functionality of the text editor. As you can see, when the content exceeds the visible area, you can scroll through it using two-finger gestures on a trackpad or by using Option+click and drag in the iOS Simulator.

The scroll indicator on the right side of the text area shows your current position within the document. It updates in real-time as you scroll through the content.

Technical Implementation Details

The text editor uses GPUI's text shaping system to accurately measure and render text. Each line is individually shaped using the platform's text system, which ensures proper character spacing and line breaks.

Word wrapping is implemented by measuring each word as it's added to a line. When a line would exceed the available width, the current line is committed and a new line begins with the overflowing word.

Touch handling converts screen coordinates to text offsets by finding the line at the touch Y position, then using the shaped line's closest_index_for_x method to find the character under the touch X position.

Selection is implemented by tracking the start and end offsets of the selected range. As you drag, the selection expands or contracts based on the touch position. The selection_reversed flag tracks whether you're extending the selection backwards.

More Text to Ensure Scrolling

Here is even more text to make absolutely sure that the content extends beyond the visible viewport, allowing you to test the scrolling functionality thoroughly.

The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump!

Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.

End of sample content. Try scrolling up and down to see the scroll indicator move!"#;

/// Font configuration
const FONT_SIZE: f32 = 18.0;
const LINE_HEIGHT_MULTIPLIER: f32 = 1.4;
const PADDING: f32 = 20.0;
const HEADER_HEIGHT: f32 = 100.0;
const FOOTER_HEIGHT: f32 = 60.0;

/// Cursor blink configuration
const CURSOR_BLINK_INTERVAL_MS: u64 = 500;

/// Text Editor demo view
pub struct TextEditor {
    content: SharedString,
    cursor_offset: usize,
    selected_range: Range<usize>,
    selection_reversed: bool,
    scroll_offset: f32,
    /// Cached line layout information
    lines: Vec<LineLayout>,
    content_height: f32,
    /// Last touch Y position for scroll delta calculation
    last_touch_y: f32,
    /// Start time for cursor blink animation
    cursor_blink_start: Instant,
    /// Whether cursor is currently visible (for blink)
    cursor_visible: bool,
    /// Scroll velocity for momentum
    scroll_velocity: f32,
    /// Whether we're currently in a touch/drag
    is_dragging: bool,
    /// Whether we're selecting text (vs scrolling)
    is_selecting: bool,
    /// Touch start offset for selection
    touch_start_offset: usize,
    /// Last scroll time for momentum calculation
    last_scroll_time: Instant,
    /// Focus handle for keyboard input
    focus_handle: Option<FocusHandle>,
    /// ID of the last replace_text_in_range operation (for deduplication)
    last_replace_id: u32,
    /// Last deletion range (for deduplication)
    last_delete_range: Option<Range<usize>>,
    /// Timestamp of last delete (for debounce)
    last_delete_time: Instant,
    /// Marked text range for IME composition
    marked_range: Option<Range<usize>>,
    /// Last known bounds for input handler
    last_bounds: Option<Bounds<Pixels>>,
}

/// Information about a laid out line
#[derive(Clone, Debug)]
struct LineLayout {
    text: String,
    byte_range: Range<usize>,
    y_position: f32,
    height: f32,
}

/// Momentum deceleration rate (pixels per frame squared, like gravity)
const SCROLL_DECELERATION: f32 = 0.5;
/// Minimum velocity threshold to stop momentum (pixels per frame)
const MIN_VELOCITY: f32 = 0.5;

impl TextEditor {
    pub fn new() -> Self {
        Self {
            content: SAMPLE_TEXT.into(),
            cursor_offset: 0,
            selected_range: 0..0,
            selection_reversed: false,
            scroll_offset: 0.0,
            lines: Vec::new(),
            content_height: 0.0,
            last_touch_y: 0.0,
            cursor_blink_start: Instant::now(),
            cursor_visible: true,
            scroll_velocity: 0.0,
            is_dragging: false,
            is_selecting: false,
            touch_start_offset: 0,
            last_scroll_time: Instant::now(),
            focus_handle: None,
            last_replace_id: 0,
            last_delete_range: None,
            last_delete_time: Instant::now(),
            marked_range: None,
            last_bounds: None,
        }
    }

    /// Initialize focus handle from context
    pub fn init_focus(&mut self, cx: &mut Context<Self>) {
        if self.focus_handle.is_none() {
            self.focus_handle = Some(cx.focus_handle());
        }
    }

    /// Get or create the focus handle
    pub fn get_focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone().unwrap_or_else(|| {
            // This shouldn't happen in practice as init_focus should be called first
            panic!("TextEditor focus_handle not initialized. Call init_focus first.")
        })
    }

    /// Request focus for this text editor
    pub fn focus(&self, window: &mut Window) {
        if let Some(handle) = &self.focus_handle {
            window.focus(handle);
        }
    }

    /// Move cursor left by one character
    pub fn move_left(&mut self) {
        if self.selected_range.is_empty() {
            if self.cursor_offset > 0 {
                self.cursor_offset = self.previous_char_boundary(self.cursor_offset);
            }
        } else {
            self.cursor_offset = self.selected_range.start;
        }
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.reset_cursor_blink();
    }

    /// Move cursor right by one character
    pub fn move_right(&mut self) {
        if self.selected_range.is_empty() {
            if self.cursor_offset < self.content.len() {
                self.cursor_offset = self.next_char_boundary(self.cursor_offset);
            }
        } else {
            self.cursor_offset = self.selected_range.end;
        }
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.reset_cursor_blink();
    }

    /// Move cursor up by one line
    pub fn move_up(&mut self) {
        if let Some(current_line_idx) = self.line_for_offset(self.cursor_offset) {
            if current_line_idx > 0 {
                let prev_line = &self.lines[current_line_idx - 1];
                let current_line = &self.lines[current_line_idx];
                let offset_in_line = self.cursor_offset - current_line.byte_range.start;
                self.cursor_offset =
                    prev_line.byte_range.start + offset_in_line.min(prev_line.text.len());
            }
        }
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.reset_cursor_blink();
    }

    /// Move cursor down by one line
    pub fn move_down(&mut self) {
        if let Some(current_line_idx) = self.line_for_offset(self.cursor_offset) {
            if current_line_idx < self.lines.len() - 1 {
                let next_line = &self.lines[current_line_idx + 1];
                let current_line = &self.lines[current_line_idx];
                let offset_in_line = self.cursor_offset - current_line.byte_range.start;
                self.cursor_offset =
                    next_line.byte_range.start + offset_in_line.min(next_line.text.len());
            }
        }
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.reset_cursor_blink();
    }

    /// Handle backspace key
    pub fn backspace(&mut self) {
        if !self.selected_range.is_empty() {
            self.delete_selection();
        } else if self.cursor_offset > 0 {
            let prev = self.previous_char_boundary(self.cursor_offset);
            let mut content = self.content.to_string();
            content.drain(prev..self.cursor_offset);
            self.content = content.into();
            self.cursor_offset = prev;
            self.selected_range = self.cursor_offset..self.cursor_offset;
            self.lines.clear(); // Force re-layout
        }
        self.reset_cursor_blink();
    }

    /// Handle delete key
    pub fn delete(&mut self) {
        if !self.selected_range.is_empty() {
            self.delete_selection();
        } else if self.cursor_offset < self.content.len() {
            let next = self.next_char_boundary(self.cursor_offset);
            let mut content = self.content.to_string();
            content.drain(self.cursor_offset..next);
            self.content = content.into();
            self.selected_range = self.cursor_offset..self.cursor_offset;
            self.lines.clear(); // Force re-layout
        }
        self.reset_cursor_blink();
    }

    /// Insert text at cursor position (used by key event fallback when input handler is unavailable)
    pub fn insert_text(&mut self, text: &str) {
        println!(
            "TextEditor::insert_text ENTRY - text={:?}, cursor={}, selected={:?}",
            text, self.cursor_offset, self.selected_range
        );

        // Delete selection first if any
        if !self.selected_range.is_empty() {
            self.delete_selection();
        }

        // Insert the text at cursor
        let mut content = self.content.to_string();
        content.insert_str(self.cursor_offset, text);
        self.content = content.into();
        self.cursor_offset += text.len();
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.lines.clear(); // Force re-layout

        println!(
            "TextEditor::insert_text EXIT - new cursor={}, new content_len={}",
            self.cursor_offset,
            self.content.len()
        );
        self.reset_cursor_blink();
    }

    /// Extend selection to the left
    pub fn select_left(&mut self) {
        let new_offset = self.previous_char_boundary(self.cursor_offset);
        self.extend_selection_to(new_offset);
        self.reset_cursor_blink();
    }

    /// Extend selection to the right
    pub fn select_right(&mut self) {
        let new_offset = self.next_char_boundary(self.cursor_offset);
        self.extend_selection_to(new_offset);
        self.reset_cursor_blink();
    }

    /// Select all text
    pub fn select_all(&mut self) {
        self.selected_range = 0..self.content.len();
        self.cursor_offset = self.content.len();
        self.selection_reversed = false;
        self.reset_cursor_blink();
    }

    /// Delete the currently selected text
    fn delete_selection(&mut self) {
        if self.selected_range.is_empty() {
            return;
        }
        let mut content = self.content.to_string();
        content.drain(self.selected_range.clone());
        self.content = content.into();
        self.cursor_offset = self.selected_range.start;
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.lines.clear(); // Force re-layout
    }

    /// Extend selection to the given offset
    fn extend_selection_to(&mut self, offset: usize) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.cursor_offset = if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        };
    }

    // ========================================
    // UTF-8 / UTF-16 conversion methods
    // ========================================

    /// Convert UTF-8 byte offset to UTF-16 offset
    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    /// Convert UTF-16 offset to UTF-8 byte offset
    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    /// Convert a UTF-8 range to UTF-16 range
    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// Convert a UTF-16 range to UTF-8 range
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    /// Find the previous character boundary
    fn previous_char_boundary(&self, offset: usize) -> usize {
        let content = self.content.as_ref();
        if offset == 0 {
            return 0;
        }
        let mut new_offset = offset - 1;
        while new_offset > 0 && !content.is_char_boundary(new_offset) {
            new_offset -= 1;
        }
        new_offset
    }

    /// Find the next character boundary
    fn next_char_boundary(&self, offset: usize) -> usize {
        let content = self.content.as_ref();
        if offset >= content.len() {
            return content.len();
        }
        let mut new_offset = offset + 1;
        while new_offset < content.len() && !content.is_char_boundary(new_offset) {
            new_offset += 1;
        }
        new_offset
    }

    /// Find which line contains the given offset
    fn line_for_offset(&self, offset: usize) -> Option<usize> {
        for (idx, line) in self.lines.iter().enumerate() {
            if offset >= line.byte_range.start && offset <= line.byte_range.end {
                return Some(idx);
            }
        }
        None
    }

    /// Reset cursor blink (show cursor immediately after interaction)
    fn reset_cursor_blink(&mut self) {
        self.cursor_blink_start = Instant::now();
        self.cursor_visible = true;
    }

    /// Update cursor visibility based on blink interval
    fn update_cursor_blink(&mut self) {
        let elapsed_ms = self.cursor_blink_start.elapsed().as_millis() as u64;
        let blink_cycle = elapsed_ms / CURSOR_BLINK_INTERVAL_MS;
        self.cursor_visible = blink_cycle.is_multiple_of(2);
    }

    /// Get last touch Y position
    pub fn last_touch_y(&self) -> f32 {
        self.last_touch_y
    }

    /// Set last touch Y position
    pub fn set_last_touch_y(&mut self, y: f32) {
        self.last_touch_y = y;
    }

    /// Layout text into lines with word wrapping
    fn layout_lines(&mut self, max_width: f32, window: &mut Window) {
        self.lines.clear();

        let style = window.text_style();
        let font = style.font();
        let font_size = px(FONT_SIZE);
        let line_height = FONT_SIZE * LINE_HEIGHT_MULTIPLIER;

        // Clone content to avoid borrow issues
        let content = self.content.to_string();

        let mut y = 0.0;
        let mut byte_offset = 0;

        for paragraph in content.split('\n') {
            if paragraph.is_empty() {
                // Empty line
                self.lines.push(LineLayout {
                    text: String::new(),
                    byte_range: byte_offset..byte_offset,
                    y_position: y,
                    height: line_height,
                });
                y += line_height;
                byte_offset += 1; // newline character
                continue;
            }

            // Convert to owned string for SharedString
            let paragraph_str = paragraph.to_string();
            let paragraph_shared: SharedString = paragraph_str.clone().into();

            // Shape the paragraph to get accurate measurements
            let run = TextRun {
                len: paragraph_str.len(),
                font: font.clone(),
                color: rgb(TEXT).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };

            let shaped = window
                .text_system()
                .shape_line(paragraph_shared, font_size, &[run], None);

            let paragraph_width = shaped.width.0;

            if paragraph_width <= max_width {
                // Paragraph fits on one line
                self.lines.push(LineLayout {
                    text: paragraph_str.clone(),
                    byte_range: byte_offset..byte_offset + paragraph_str.len(),
                    y_position: y,
                    height: line_height,
                });
                y += line_height;
                byte_offset += paragraph_str.len() + 1; // +1 for newline
            } else {
                // Need to wrap - track actual byte positions in original paragraph
                let paragraph_start = byte_offset;

                // Find word boundaries with their byte positions
                let mut words_with_positions: Vec<(usize, &str)> = Vec::new();
                let mut word_start: Option<usize> = None;

                for (i, ch) in paragraph.char_indices() {
                    if ch.is_whitespace() {
                        if let Some(start) = word_start {
                            words_with_positions.push((start, &paragraph[start..i]));
                            word_start = None;
                        }
                    } else if word_start.is_none() {
                        word_start = Some(i);
                    }
                }
                // Handle last word if paragraph doesn't end with whitespace
                if let Some(start) = word_start {
                    words_with_positions.push((start, &paragraph[start..]));
                }

                let mut line_text = String::new();
                let mut line_start_in_para = 0usize;
                let mut line_end_in_para = 0usize;

                for (word_idx, (word_pos, word)) in words_with_positions.iter().enumerate() {
                    let test_text = if line_text.is_empty() {
                        word.to_string()
                    } else {
                        format!("{} {}", line_text, word)
                    };

                    // Shape test line
                    let test_shared: SharedString = test_text.clone().into();
                    let test_run = TextRun {
                        len: test_text.len(),
                        font: font.clone(),
                        color: rgb(TEXT).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };

                    let test_shaped = window
                        .text_system()
                        .shape_line(test_shared, font_size, &[test_run], None);

                    if test_shaped.width.0 > max_width && !line_text.is_empty() {
                        // Current line is full, emit it
                        // line_end_in_para points to end of last word added
                        self.lines.push(LineLayout {
                            text: line_text.clone(),
                            byte_range: paragraph_start + line_start_in_para
                                ..paragraph_start + line_end_in_para,
                            y_position: y,
                            height: line_height,
                        });
                        y += line_height;

                        // Start new line from this word
                        line_start_in_para = *word_pos;
                        line_text = word.to_string();
                        line_end_in_para = word_pos + word.len();
                    } else {
                        // Word fits, update line
                        if line_text.is_empty() {
                            line_start_in_para = *word_pos;
                        }
                        line_text = test_text;
                        line_end_in_para = word_pos + word.len();
                    }

                    // Handle last word
                    if word_idx == words_with_positions.len() - 1 && !line_text.is_empty() {
                        self.lines.push(LineLayout {
                            text: line_text.clone(),
                            byte_range: paragraph_start + line_start_in_para
                                ..paragraph_start + line_end_in_para,
                            y_position: y,
                            height: line_height,
                        });
                        y += line_height;
                    }
                }
                byte_offset += paragraph_str.len() + 1;
            }
        }

        self.content_height = y;
    }

    /// Find the byte offset for a touch position
    fn offset_for_position(&self, position: Point<f32>, window: &mut Window) -> usize {
        let adjusted_y = position.y + self.scroll_offset - PADDING - HEADER_HEIGHT;

        // Find the line at this y position
        for line in &self.lines {
            if adjusted_y >= line.y_position && adjusted_y < line.y_position + line.height {
                if line.text.is_empty() {
                    return line.byte_range.start;
                }

                // Use text shaping for accurate x position
                let style = window.text_style();
                let font = style.font();
                let font_size = px(FONT_SIZE);

                let run = TextRun {
                    len: line.text.len(),
                    font,
                    color: rgb(TEXT).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };

                let shaped = window
                    .text_system()
                    .shape_line(line.text.clone().into(), font_size, &[run], None);

                let x_in_line = px((position.x - PADDING).max(0.0));
                let char_index = shaped.closest_index_for_x(x_in_line);

                return line.byte_range.start + char_index;
            }
        }

        // Below all lines, return end
        self.content.len()
    }

    pub fn handle_touch_down(&mut self, position: Point<f32>, window: &mut Window) {
        let offset = self.offset_for_position(position, window);
        self.cursor_offset = offset.min(self.content.len());
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.selection_reversed = false;
        self.is_selecting = true;
        self.touch_start_offset = self.cursor_offset;
        self.reset_cursor_blink();
    }

    /// Check if we're in selection mode
    pub fn is_selecting(&self) -> bool {
        self.is_selecting
    }

    /// End selection mode
    pub fn end_selection(&mut self) {
        self.is_selecting = false;
    }

    pub fn handle_touch_move(&mut self, position: Point<f32>, window: &mut Window) {
        let offset = self.offset_for_position(position, window).min(self.content.len());

        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }

        // Normalize range direction
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }

        self.cursor_offset = if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        };
    }

    /// Handle scroll wheel events
    pub fn handle_scroll(&mut self, delta_y: f32, viewport_height: f32) {
        let max_scroll = (self.content_height - viewport_height + PADDING * 2.0).max(0.0);
        self.scroll_offset = (self.scroll_offset - delta_y).clamp(0.0, max_scroll);

        // Track velocity for momentum - use exponential moving average
        let now = Instant::now();
        let dt = now.duration_since(self.last_scroll_time).as_secs_f32();
        if dt > 0.0 && dt < 0.1 {
            // Calculate instantaneous velocity (pixels per second)
            let instant_velocity = delta_y / dt;
            // Blend with previous velocity for smoother tracking
            self.scroll_velocity = self.scroll_velocity * 0.3 + instant_velocity * 0.7;
        }
        self.last_scroll_time = now;
        self.is_dragging = true;
    }

    /// Start dragging (called on touch down)
    pub fn start_drag(&mut self) {
        self.is_dragging = true;
        self.scroll_velocity = 0.0;
        self.last_scroll_time = Instant::now();
    }

    /// End dragging (called on touch up) - starts momentum
    pub fn end_drag(&mut self) {
        self.is_dragging = false;
        // Scale velocity to per-frame (assuming ~60fps = 16.67ms per frame)
        self.scroll_velocity *= 0.016;
    }

    /// Update scroll momentum - returns true if still animating
    pub fn update_momentum(&mut self, viewport_height: f32) -> bool {
        if self.is_dragging {
            return false;
        }

        if self.scroll_velocity.abs() < MIN_VELOCITY {
            self.scroll_velocity = 0.0;
            return false;
        }

        // Apply velocity to scroll
        let max_scroll = (self.content_height - viewport_height + PADDING * 2.0).max(0.0);
        self.scroll_offset = (self.scroll_offset - self.scroll_velocity).clamp(0.0, max_scroll);

        // Apply deceleration (like friction/air resistance)
        if self.scroll_velocity > 0.0 {
            self.scroll_velocity = (self.scroll_velocity - SCROLL_DECELERATION).max(0.0);
        } else {
            self.scroll_velocity = (self.scroll_velocity + SCROLL_DECELERATION).min(0.0);
        }

        // Stop if we hit bounds
        if self.scroll_offset <= 0.0 || self.scroll_offset >= max_scroll {
            self.scroll_velocity = 0.0;
            return false;
        }

        true
    }

    pub fn render_with_back_button<F>(
        &mut self,
        window: &mut Window,
        _on_back: F,
    ) -> impl IntoElement
    where
        F: Fn(&(), &mut Window, &mut App) + 'static,
    {
        // Request animation frame for cursor blink and momentum
        window.request_animation_frame();

        // Update cursor blink state
        self.update_cursor_blink();

        // Update scroll momentum
        let viewport = window.viewport_size();
        let content_viewport_height = viewport.height.0 - HEADER_HEIGHT - FOOTER_HEIGHT;
        self.update_momentum(content_viewport_height);

        // Layout text if needed
        let max_width = viewport.width.0 - PADDING * 2.0;
        if self.lines.is_empty() {
            self.layout_lines(max_width, window);
        }

        // Clone state for canvas closure
        let lines = self.lines.clone();
        let cursor_offset = self.cursor_offset;
        let selected_range = self.selected_range.clone();
        let scroll_offset = self.scroll_offset;
        let content_height = self.content_height;
        let cursor_visible = self.cursor_visible;

        div()
            .size_full()
            .bg(rgb(BACKGROUND))
            .flex()
            .flex_col()
            // Header - with left padding to avoid back button
            .child(
                div()
                    .h(px(HEADER_HEIGHT))
                    .pt(px(50.0))
                    .pl(px(100.0)) // Leave room for back button
                    .pr(px(PADDING))
                    .bg(rgb(SURFACE))
                    .shadow_sm() // Add subtle shadow below header
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .text_xl()
                            .text_color(rgb(TEXT))
                            .child("Text Editor"),
                    ),
            )
            // Text content area
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        crate::canvas(
                            move |bounds, window, _cx| {
                                // Prepaint: prepare rendering data
                                let style = window.text_style();
                                let font = style.font();
                                let font_size = px(FONT_SIZE);

                                // Shape all visible lines
                                let mut shaped_lines = Vec::new();
                                for line in &lines {
                                    let y = PADDING + line.y_position - scroll_offset;

                                    // Skip lines outside viewport
                                    if y + line.height < 0.0 || y > bounds.size.height.0 {
                                        shaped_lines.push(None);
                                        continue;
                                    }

                                    if line.text.is_empty() {
                                        shaped_lines.push(Some((line.clone(), None)));
                                        continue;
                                    }

                                    let run = TextRun {
                                        len: line.text.len(),
                                        font: font.clone(),
                                        color: rgb(TEXT).into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };

                                    let shaped = window
                                        .text_system()
                                        .shape_line(line.text.clone().into(), font_size, &[run], None);

                                    shaped_lines.push(Some((line.clone(), Some(shaped))));
                                }

                                (bounds, shaped_lines, cursor_offset, selected_range.clone(), scroll_offset, content_height, cursor_visible)
                            },
                            move |_bounds,
                                  (bounds, shaped_lines, cursor_offset, selected_range, scroll_offset, content_height, cursor_visible),
                                  window,
                                  cx| {
                                let cursor_color: Hsla = rgb(GREEN).into();
                                let selection_color = hsla(0.33, 0.7, 0.5, 0.3);
                                let line_height = px(FONT_SIZE * LINE_HEIGHT_MULTIPLIER);

                                for maybe_line in shaped_lines.iter() {
                                    let Some((line, maybe_shaped)) = maybe_line else {
                                        continue;
                                    };

                                    let y = PADDING + line.y_position - scroll_offset;

                                    // Draw selection background if this line intersects selection
                                    if !selected_range.is_empty()
                                        && selected_range.start < line.byte_range.end
                                        && selected_range.end > line.byte_range.start
                                    {
                                        let sel_start = selected_range.start.max(line.byte_range.start);
                                        let sel_end = selected_range.end.min(line.byte_range.end);

                                        if let Some(shaped) = maybe_shaped {
                                            let local_start = sel_start - line.byte_range.start;
                                            let local_end = sel_end - line.byte_range.start;

                                            let x_start = shaped.x_for_index(local_start);
                                            let x_end = shaped.x_for_index(local_end);

                                            let sel_bounds = Bounds {
                                                origin: point(
                                                    px(bounds.origin.x.0 + PADDING) + x_start,
                                                    px(bounds.origin.y.0 + y),
                                                ),
                                                size: size(x_end - x_start, line_height),
                                            };
                                            window.paint_quad(fill(sel_bounds, selection_color));
                                        }
                                    }

                                    // Draw text
                                    if let Some(shaped) = maybe_shaped {
                                        let origin = point(
                                            px(bounds.origin.x.0 + PADDING),
                                            px(bounds.origin.y.0 + y),
                                        );
                                        shaped.paint(origin, line_height, window, cx).ok();
                                    }

                                    // Draw cursor if on this line and cursor is visible (blinking)
                                    if cursor_visible
                                        && cursor_offset >= line.byte_range.start
                                        && cursor_offset <= line.byte_range.end
                                    {
                                        let cursor_x = if let Some(shaped) = maybe_shaped {
                                            let local_offset = cursor_offset - line.byte_range.start;
                                            shaped.x_for_index(local_offset)
                                        } else {
                                            px(0.0)
                                        };

                                        let cursor_bounds = Bounds {
                                            origin: point(
                                                px(bounds.origin.x.0 + PADDING) + cursor_x,
                                                px(bounds.origin.y.0 + y),
                                            ),
                                            size: size(px(2.0), line_height),
                                        };
                                        window.paint_quad(fill(cursor_bounds, cursor_color));
                                    }
                                }

                                // Draw scroll indicator
                                let viewport_height = bounds.size.height.0;
                                if content_height > viewport_height {
                                    let scroll_track_height = viewport_height - 20.0; // 10px margin top and bottom
                                    let scroll_ratio = viewport_height / content_height;
                                    let indicator_height = (scroll_track_height * scroll_ratio).max(30.0);
                                    let max_scroll = content_height - viewport_height;
                                    let scroll_progress = if max_scroll > 0.0 {
                                        scroll_offset / max_scroll
                                    } else {
                                        0.0
                                    };
                                    let indicator_y = 10.0 + (scroll_track_height - indicator_height) * scroll_progress;

                                    let indicator_bounds = Bounds {
                                        origin: point(
                                            px(bounds.origin.x.0 + bounds.size.width.0 - 6.0),
                                            px(bounds.origin.y.0 + indicator_y),
                                        ),
                                        size: size(px(4.0), px(indicator_height)),
                                    };
                                    window.paint_quad(fill(indicator_bounds, hsla(0.0, 0.0, 0.5, 0.5)));
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            // Footer
            .child(
                div()
                    .h(px(FOOTER_HEIGHT))
                    .px(px(PADDING))
                    .bg(rgb(SURFACE))
                    .shadow_sm() // Add subtle shadow above footer
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(SUBTEXT))
                            .child("Tap to place cursor, drag to select"),
                    ),
            )
    }
}

// ============================================
// Focusable trait implementation
// ============================================

impl Focusable for TextEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.get_focus_handle()
    }
}

// ============================================
// EntityInputHandler trait implementation
// This enables keyboard input from the iOS UITextInput protocol
// ============================================

impl EntityInputHandler for TextEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        let clamped_range = range.start.min(self.content.len())..range.end.min(self.content.len());
        actual_range.replace(self.range_to_utf16(&clamped_range));
        Some(self.content[clamped_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let call_id = REPLACE_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        let now = Instant::now();

        // Detailed logging for debugging double-deletion
        println!(
            "TextEditor::replace_text_in_range [#{}] ENTRY - range_utf16={:?}, new_text={:?}, content_len={}, cursor={}, selected={:?}",
            call_id, range_utf16, new_text, self.content.len(), self.cursor_offset, self.selected_range
        );

        let range = range_utf16
            .as_ref()
            .map(|r| {
                let converted = self.range_from_utf16(r);
                println!(
                    "TextEditor::replace_text_in_range [#{}] - UTF16 {:?} -> UTF8 {:?}",
                    call_id, r, converted
                );
                converted
            })
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        // Clamp range to valid bounds
        let clamped_range = range.start.min(self.content.len())..range.end.min(self.content.len());

        // Deduplication: if this is a deletion (empty new_text, non-empty range),
        // check if we just did the exact same deletion within 100ms
        let is_deletion = new_text.is_empty() && !clamped_range.is_empty();
        if is_deletion {
            let elapsed = now.duration_since(self.last_delete_time);
            if elapsed.as_millis() < 100 {
                if let Some(last_range) = &self.last_delete_range {
                    // If the range starts at the same position or one position before
                    // (accounting for the cursor having moved), skip the duplicate
                    let likely_duplicate =
                        clamped_range.start == last_range.start ||
                        clamped_range.end == last_range.start;
                    if likely_duplicate {
                        println!(
                            "TextEditor::replace_text_in_range [#{}] SKIPPED - duplicate deletion (last={:?}, this={:?}, elapsed={}ms)",
                            call_id, last_range, clamped_range, elapsed.as_millis()
                        );
                        return;
                    }
                }
            }
        }

        // Log what we're about to do
        if new_text.is_empty() {
            println!(
                "TextEditor::replace_text_in_range [#{}] - DELETING range={:?}, text={:?}",
                call_id, clamped_range,
                &self.content[clamped_range.clone()]
            );
        } else {
            println!(
                "TextEditor::replace_text_in_range [#{}] - INSERTING {:?} at range={:?}",
                call_id, new_text, clamped_range
            );
        }

        // Replace text
        let mut content = self.content.to_string();
        content.replace_range(clamped_range.clone(), new_text);
        self.content = content.into();

        // Update cursor position
        self.cursor_offset = clamped_range.start + new_text.len();
        self.selected_range = self.cursor_offset..self.cursor_offset;
        self.marked_range = None;
        self.lines.clear(); // Force re-layout
        self.reset_cursor_blink();

        // Track this deletion for deduplication
        if is_deletion {
            self.last_delete_range = Some(clamped_range.clone());
            self.last_delete_time = now;
        } else {
            // Clear deletion tracking on non-deletion operations
            self.last_delete_range = None;
        }

        println!(
            "TextEditor::replace_text_in_range [#{}] EXIT - new cursor={}, new content_len={}",
            call_id, self.cursor_offset, self.content.len()
        );

        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        // Clamp range to valid bounds
        let clamped_range = range.start.min(self.content.len())..range.end.min(self.content.len());

        // Replace text
        let mut content = self.content.to_string();
        content.replace_range(clamped_range.clone(), new_text);
        self.content = content.into();

        // Set marked range
        if !new_text.is_empty() {
            self.marked_range = Some(clamped_range.start..clamped_range.start + new_text.len());
        } else {
            self.marked_range = None;
        }

        // Update selection
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|new_range| {
                clamped_range.start + new_range.start..clamped_range.start + new_range.end
            })
            .unwrap_or_else(|| {
                let pos = clamped_range.start + new_text.len();
                pos..pos
            });

        self.cursor_offset = if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        };

        self.lines.clear(); // Force re-layout
        self.reset_cursor_blink();

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Return a reasonable default bounds for the cursor position
        // This is used for IME positioning
        let range = self.range_from_utf16(&range_utf16);

        // Find which line contains this range
        for line in &self.lines {
            if range.start >= line.byte_range.start && range.start <= line.byte_range.end {
                let y = PADDING + line.y_position - self.scroll_offset;
                let line_height = px(FONT_SIZE * LINE_HEIGHT_MULTIPLIER);

                return Some(Bounds {
                    origin: point(
                        bounds.origin.x + px(PADDING),
                        bounds.origin.y + px(y),
                    ),
                    size: size(px(100.0), line_height),
                });
            }
        }

        // Default to top-left area
        Some(Bounds {
            origin: bounds.origin + point(px(PADDING), px(PADDING)),
            size: size(px(100.0), px(FONT_SIZE * LINE_HEIGHT_MULTIPLIER)),
        })
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // Convert point to text offset
        // This is a simplified version - could be more accurate with proper hit testing
        let y = point.y.0 + self.scroll_offset - PADDING - HEADER_HEIGHT;

        for line in &self.lines {
            if y >= line.y_position && y < line.y_position + line.height {
                // Found the line, estimate character position based on x
                let x = (point.x.0 - PADDING).max(0.0);
                let char_width = FONT_SIZE * 0.6; // Approximate character width
                let char_index = (x / char_width) as usize;
                let offset = line.byte_range.start + char_index.min(line.text.len());
                return Some(self.offset_to_utf16(offset));
            }
        }

        Some(self.offset_to_utf16(self.content.len()))
    }
}
