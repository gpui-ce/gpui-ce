use super::unicode::UnicodeString;
use crate::input::InputStateEvent;
use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window, point, px};
use std::ops::Range;

impl EntityInputHandler for super::InputState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.utf_range_16to8(&range_utf16);
        let clamped_range =
            range.start.min(self.content().len())..range.end.min(self.content().len());
        adjusted_range.replace(self.utf_range_8to16(&clamped_range));
        Some(self.content()[clamped_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf_range_8to16(&self.selected_range),
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
            .map(|range| self.utf_range_8to16(range))
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
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.utf_range_16to8(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let range = range.start.min(self.content().len())..range.end.min(self.content().len());

        let text_to_insert = self.layout_style.sanitize_content(new_text);

        // Record patch for undo before modifying content
        self.push_undo_patch(range.clone(), text_to_insert.len());

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content()[range.clone()]
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
            .map(|range_utf16| self.utf_range_16to8(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let range = range.start.min(self.content().len())..range.end.min(self.content().len());

        let text_to_insert = self.layout_style.sanitize_content(new_text);

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content()[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.replace_range(range.clone(), &text_to_insert);

        if !text_to_insert.is_empty() {
            self.marked_range = Some(range.start..range.start + text_to_insert.len());
        } else {
            self.marked_range = None;
        }

        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.utf_range_16to8(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| {
                range.start + text_to_insert.len()..range.start + text_to_insert.len()
            });

        self.layout_data.dirty = true;
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.utf_range_16to8(&range_utf16);

        for line in &self.logical_lines {
            if line.text_range.is_empty() {
                if range.start == line.text_range.start {
                    return Some(Bounds::from_corners(
                        point(bounds.left(), bounds.top() + line.y_offset),
                        point(
                            bounds.left() + px(4.),
                            bounds.top() + line.y_offset + self.line_height(),
                        ),
                    ));
                }
            } else if line.text_range.contains(&range.start) {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_start = range.start - line.text_range.start;
                    let local_end = (range.end - line.text_range.start).min(wrapped.text.len());

                    let start_pos = wrapped
                        .position_for_index(local_start, self.line_height())
                        .unwrap_or(point(px(0.), px(0.)));
                    let end_pos = wrapped
                        .position_for_index(local_end, self.line_height())
                        .unwrap_or_else(|| {
                            let last_line_y =
                                self.line_height() * (line.visual_line_count - 1) as f32;
                            point(wrapped.width(), last_line_y)
                        });

                    let start_visual_line = (start_pos.y / self.line_height()).floor() as usize;
                    let end_visual_line = (end_pos.y / self.line_height()).floor() as usize;

                    if start_visual_line == end_visual_line {
                        return Some(Bounds::from_corners(
                            point(
                                bounds.left() + start_pos.x,
                                bounds.top() + line.y_offset + start_pos.y,
                            ),
                            point(
                                bounds.left() + end_pos.x,
                                bounds.top() + line.y_offset + start_pos.y + self.line_height(),
                            ),
                        ));
                    } else {
                        return Some(Bounds::from_corners(
                            point(
                                bounds.left() + start_pos.x,
                                bounds.top() + line.y_offset + start_pos.y,
                            ),
                            point(
                                bounds.left() + wrapped.width(),
                                bounds.top() + line.y_offset + start_pos.y + self.line_height(),
                            ),
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
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self.index_for_pixel_point(point);
        Some(self.utf_offset_8to16(index))
    }
}
