use crate::editable_text::{
    EditableTextActionHandler, TextBoundary, TextInputStateBase, TextStateNotifier,
    UnicodeTextStorage,
    notify::{TextChanged, TextHistoryPushed},
};
use gpui::{
    Bounds, Context, EntityInputHandler, EventEmitter, NavigationDirection, Pixels, Point,
    UTF16Selection, Window,
};
use std::ops::Range;

pub struct TextAreaState {
    internal: TextInputStateBase,
}

impl EventEmitter<TextChanged> for TextAreaState {}
impl EventEmitter<TextHistoryPushed> for TextAreaState {}

impl TextAreaState {
    pub fn new(storage: impl Into<Box<dyn UnicodeTextStorage>>, cx: &mut Context<Self>) -> Self {
        let internal = TextInputStateBase::new(storage, cx);
        Self { internal }
    }
}

impl TextStateNotifier for Context<'_, TextAreaState> {
    fn notify_changed(&mut self) {
        self.notify();
    }

    fn emit_text_changed(&mut self, event: TextChanged) {
        self.emit(event);
    }

    fn emit_history(&mut self, event: TextHistoryPushed) {
        self.emit(event);
    }
}

impl EntityInputHandler for TextAreaState {
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
            .replace_text_in_range_bytes(range_utf8, text_to_insert, cx);
        //self.mark_layout_dirty();
        //cx.emit(CursorTrigger::PauseBlinkingForUserAction);
        cx.emit_text_changed(TextChanged);
        cx.notify_changed();
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
            .replace_text_in_range_bytes(range.clone(), text_to_insert, cx);
        self.internal
            .ime_mark_text_in_range(&range, text_to_insert.len());
        self.internal.ime_mark_selected_range(
            &range,
            &new_selected_range_utf16,
            text_to_insert.len(),
        );
        //self.mark_layout_dirty();
        cx.emit_text_changed(TextChanged);
        cx.notify_changed();
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

impl<'app> EditableTextActionHandler<'app> for TextAreaState {
    type Context = gpui::Context<'app, Self>;

    fn escape(&mut self, _: &super::Escape, window: &mut Window, cx: &mut Self::Context) {
        self.internal.set_selected_range(0..0);
        cx.notify();

        window.blur();
    }

    fn insert_enter(&mut self, _: &super::Enter, window: &mut Window, cx: &mut Self::Context) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn insert_tab(&mut self, _: &super::Tab, window: &mut Window, cx: &mut Self::Context) {
        self.replace_text_in_range(None, "\t", window, cx);
    }

    fn backspace(&mut self, _: &super::Backspace, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .delete(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn delete(&mut self, _: &super::Delete, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .delete(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn delete_word_left(
        &mut self,
        _: &super::DeleteWordLeft,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .delete(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn delete_word_right(
        &mut self,
        _: &super::DeleteWordRight,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .delete(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &super::DeleteToBeginningOfLine,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .delete(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn delete_to_line_end(
        &mut self,
        _: &super::DeleteToEndOfLine,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .delete(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_left(&mut self, _: &super::Left, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn nav_right(&mut self, _: &super::Right, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn nav_up(&mut self, _: &super::Up, _w: &mut Window, cx: &mut Self::Context) {
        // TODO: implement
    }

    fn nav_down(&mut self, _: &super::Down, _w: &mut Window, cx: &mut Self::Context) {
        // TODO: implement
    }

    fn nav_line_start(&mut self, _: &super::Home, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Back, TextBoundary::Line, cx);
    }

    fn nav_line_end(&mut self, _: &super::End, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Forward, TextBoundary::Line, cx);
    }

    fn nav_start(&mut self, _: &super::MoveToBeginning, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn nav_end(&mut self, _: &super::MoveToEnd, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
    }

    fn nav_left_word(&mut self, _: &super::WordLeft, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn nav_right_word(&mut self, _: &super::WordRight, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .nav_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn select_all(&mut self, _: &super::SelectAll, _w: &mut Window, cx: &mut Self::Context) {
        self.internal.select_all(cx);
    }

    fn select_left(&mut self, _: &super::SelectLeft, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .select_linear(NavigationDirection::Back, TextBoundary::Graphmeme, cx);
    }

    fn select_right(&mut self, _: &super::SelectRight, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .select_linear(NavigationDirection::Forward, TextBoundary::Graphmeme, cx);
    }

    fn select_up(&mut self, _: &super::SelectUp, _w: &mut Window, cx: &mut Self::Context) {
        // TODO: implement
    }

    fn select_down(&mut self, _: &super::SelectDown, _w: &mut Window, cx: &mut Self::Context) {
        // TODO: implement
    }

    fn select_start(
        &mut self,
        _: &super::SelectToBeginning,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .select_linear(NavigationDirection::Back, TextBoundary::Document, cx);
    }

    fn select_end(&mut self, _: &super::SelectToEnd, _w: &mut Window, cx: &mut Self::Context) {
        self.internal
            .select_linear(NavigationDirection::Forward, TextBoundary::Document, cx);
    }

    fn select_left_word(
        &mut self,
        _: &super::SelectWordLeft,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .select_linear(NavigationDirection::Back, TextBoundary::Word, cx);
    }

    fn select_right_word(
        &mut self,
        _: &super::SelectWordRight,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        self.internal
            .select_linear(NavigationDirection::Forward, TextBoundary::Word, cx);
    }

    fn cut(&mut self, _: &super::Cut, _w: &mut Window, cx: &mut Self::Context) {
        self.internal.cut(cx);
    }

    fn copy(&mut self, _: &super::Copy, _w: &mut Window, cx: &mut Self::Context) {
        self.internal.copy(cx);
    }

    fn paste(&mut self, _: &super::Paste, _w: &mut Window, cx: &mut Self::Context) {
        self.internal.paste(cx);
    }

    fn undo(&mut self, _: &super::Undo, _w: &mut Window, _cx: &mut Self::Context) {
        // TODO: STUB
    }

    fn redo(&mut self, _: &super::Redo, _w: &mut Window, _cx: &mut Self::Context) {
        // TODO: STUB
    }

    fn on_mouse_down(
        &mut self,
        event: &gpui::MouseDownEvent,
        text_position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Self::Context,
    ) {
        let character_pos = self.internal.caret_pos(); // TODO: Should be index_for_pixel_point
        self.internal.on_mouse_down(
            text_position,
            character_pos,
            event.click_count,
            event.modifiers.shift,
            window,
            cx,
        );
    }

    fn on_mouse_up(
        &mut self,
        _event: &gpui::MouseUpEvent,
        _w: &mut Window,
        _cx: &mut Self::Context,
    ) {
        self.internal.on_mouse_up();
    }

    fn on_mouse_move(
        &mut self,
        _event: &gpui::MouseMoveEvent,
        text_position: Point<Pixels>,
        _w: &mut Window,
        cx: &mut Self::Context,
    ) {
        let character_pos = self.internal.caret_pos(); // TODO: Should be index_for_pixel_point
        self.internal.on_mouse_move(character_pos, cx);
    }
}
