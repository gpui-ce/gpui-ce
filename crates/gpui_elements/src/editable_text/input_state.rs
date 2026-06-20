use crate::editable_text::{TextInputStateBase, notify::TextChanged};
use gpui::{
    Bounds, Context, EntityInputHandler, EventEmitter, Pixels, Point, UTF16Selection, Window,
};
use std::ops::Range;

pub struct TextInputState {
    internal: TextInputStateBase,
}

impl EventEmitter<TextChanged> for TextInputState {}

impl EntityInputHandler for TextInputState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        self.internal
            .ime_text_for_range(range_utf16, adjusted_range)
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        self.internal.ime_selected_text_range(ignore_disabled_input)
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.internal.ime_marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.internal.ime_unmark_text();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text_to_insert: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range_utf8 = self.internal.ime_resolve_range(range_utf16);
        self.internal
            .replace_text_in_range_bytes(range_utf8, text_to_insert);
        //self.mark_layout_dirty();
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
        let range = self.internal.ime_resolve_range(range_utf16);
        self.internal
            .replace_text_in_range_bytes(range.clone(), text_to_insert);
        self.internal
            .ime_mark_text_in_range(&range, text_to_insert.len());
        self.internal.ime_mark_selected_range(
            &range,
            &new_selected_range_utf16,
            text_to_insert.len(),
        );
        //self.mark_layout_dirty();
        cx.emit(TextChanged);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        unimplemented!()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        unimplemented!()
    }
}
