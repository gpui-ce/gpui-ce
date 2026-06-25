use gpui::{Action, Context, InteractiveElement, WeakEntity, Window};
use std::{cell::RefCell, rc::Rc};

/// The key context used for EditableText element keybindings.
pub const DEFAULT_INPUT_CONTEXT: &str = "EditableText";

gpui::actions!(
    actions,
    [
        /// Blur focus from the input.
        Escape,
        /// Insert a newline at the cursor position.
        Enter,
        /// Insert a tab character at the cursor position.
        Tab,
        /// Delete the character before the cursor.
        Backspace,
        /// Delete the character after the cursor.
        Delete,
        /// Delete the word before the cursor.
        DeleteWordLeft,
        /// Delete the word after the cursor.
        DeleteWordRight,
        /// Delete from the cursor to the beginning of the line.
        DeleteToLineStart,
        /// Delete from the cursor to the end of the line.
        DeleteToLineEnd,
        /// Move the cursor one character to the left.
        NavLeft,
        /// Move the cursor one character to the right.
        NavRight,
        /// Move the cursor up one visual line.
        NavUp,
        /// Move the cursor down one visual line.
        NavDown,
        /// Move cursor to the start of the current line.
        Home,
        /// Move cursor to the end of the current line.
        NavLineEnd,
        /// Move cursor to the beginning of the content.
        NavDocumentStart,
        /// Move cursor to the end of the content.
        NavDocumentEnd,
        /// Move cursor one word to the left.
        NavWordLeft,
        /// Move cursor one word to the right.
        NavWordRight,
        /// Select all text content.
        SelectAll,
        /// Extend selection one character to the left.
        SelectLeft,
        /// Extend selection one character to the right.
        SelectRight,
        /// Extend selection up one visual line.
        SelectUp,
        /// Extend selection down one visual line.
        SelectDown,
        /// Extend selection to the beginning of the content.
        SelectDocumentStart,
        /// Extend selection to the end of the content.
        SelectDocumentEnd,
        /// Extend selection one word to the left.
        SelectWordLeft,
        /// Extend selection one word to the right.
        SelectWordRight,
        /// Cut selected text to clipboard.
        Cut,
        /// Copy selected text to clipboard.
        Copy,
        /// Paste from clipboard at the cursor position.
        Paste,
        /// Undo the last edit.
        Undo,
        /// Redo the last undone edit.
        Redo,
        /// Show the platform character palette.
        ShowCharacterPalette,
    ]
);

pub fn default_bindings() -> gpui::ActionBindingCollection {
    let mut bindings = gpui::ActionBindingCollection::default()
        .with::<Backspace>("backspace")
        .with::<Delete>("delete")
        .with::<Tab>("tab")
        .with::<Enter>("enter")
        .with::<NavLeft>("left")
        .with::<NavRight>("right")
        .with::<NavUp>("up")
        .with::<NavDown>("down")
        .with::<SelectAll>("secondary-a")
        .with::<SelectLeft>("shift-left")
        .with::<SelectRight>("shift-right")
        .with::<SelectUp>("shift-up")
        .with::<SelectDown>("shift-down")
        .with::<Copy>("secondary-c")
        .with::<Cut>("secondary-x")
        .with::<Paste>("secondary-v")
        .with::<Undo>("secondary-z")
        .with::<Redo>("secondary-shift-z")
        .with::<Escape>("escape")
        .with::<ShowCharacterPalette>("secondary-space");

    #[cfg(target_os = "macos")]
    {
        bindings = bindings
            .with::<DeleteWordLeft>("alt-backspace")
            .with::<DeleteWordRight>("alt-delete")
            .with::<DeleteToLineStart>("cmd-backspace")
            .with::<DeleteToLineEnd>("ctrl-k")
            // Mac keyboards don't have Home/End keys, so cmd-left/right are standard
            .with::<Home>("cmd-left")
            .with::<NavLineEnd>("cmd-right")
            .with::<NavDocumentStart>("cmd-up")
            .with::<NavDocumentEnd>("cmd-down")
            .with::<SelectDocumentStart>("cmd-shift-up")
            .with::<SelectDocumentEnd>("cmd-shift-down")
            .with::<NavWordLeft>("alt-left")
            .with::<NavWordRight>("alt-right")
            .with::<SelectWordLeft>("alt-shift-left")
            .with::<SelectWordRight>("alt-shift-right");
    }

    #[cfg(not(target_os = "macos"))]
    {
        bindings = bindings
            .with::<DeleteWordLeft>("ctrl-backspace")
            .with::<DeleteWordRight>("ctrl-delete")
            .with::<DeleteToLineStart>("ctrl-shift-backspace")
            .with::<DeleteToLineEnd>("ctrl-shift-delete")
            .with::<Home>("home")
            .with::<NavLineEnd>("end")
            .with::<NavDocumentStart>("ctrl-home")
            .with::<NavDocumentEnd>("ctrl-end")
            .with::<SelectDocumentStart>("ctrl-shift-home")
            .with::<SelectDocumentEnd>("ctrl-shift-end")
            .with::<NavWordLeft>("ctrl-left")
            .with::<NavWordRight>("ctrl-right")
            .with::<SelectWordLeft>("ctrl-shift-left")
            .with::<SelectWordRight>("ctrl-shift-right");
    }

    bindings
}

/// Declares stubs for all editable-text actions that an element's state entity can implement.
pub trait EditableTextActionHandler<Context>: Sized {
    fn escape(&mut self, _: &Escape, _w: &mut Window, _cx: &mut Context) {}

    fn insert_enter(&mut self, _: &Enter, _w: &mut Window, _cx: &mut Context) {}
    fn insert_tab(&mut self, _: &Tab, _w: &mut Window, _cx: &mut Context) {}

    fn backspace(&mut self, _: &Backspace, _w: &mut Window, _cx: &mut Context) {}
    fn delete(&mut self, _: &Delete, _w: &mut Window, _cx: &mut Context) {}

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _w: &mut Window, _cx: &mut Context) {}
    fn delete_word_right(&mut self, _: &DeleteWordRight, _w: &mut Window, _cx: &mut Context) {}
    fn delete_to_line_start(&mut self, _: &DeleteToLineStart, _w: &mut Window, _cx: &mut Context) {}
    fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _w: &mut Window, _cx: &mut Context) {}

    fn nav_left(&mut self, _: &NavLeft, _w: &mut Window, _cx: &mut Context) {}
    fn nav_right(&mut self, _: &NavRight, _w: &mut Window, _cx: &mut Context) {}
    fn nav_up(&mut self, _: &NavUp, _w: &mut Window, _cx: &mut Context) {}
    fn nav_down(&mut self, _: &NavDown, _w: &mut Window, _cx: &mut Context) {}
    fn nav_line_start(&mut self, _: &Home, _w: &mut Window, _cx: &mut Context) {}
    fn nav_line_end(&mut self, _: &NavLineEnd, _w: &mut Window, _cx: &mut Context) {}
    fn nav_start(&mut self, _: &NavDocumentStart, _w: &mut Window, _cx: &mut Context) {}
    fn nav_end(&mut self, _: &NavDocumentEnd, _w: &mut Window, _cx: &mut Context) {}
    fn nav_left_word(&mut self, _: &NavWordLeft, _w: &mut Window, _cx: &mut Context) {}
    fn nav_right_word(&mut self, _: &NavWordRight, _w: &mut Window, _cx: &mut Context) {}

    fn select_all(&mut self, _: &SelectAll, _w: &mut Window, _cx: &mut Context) {}
    fn select_left(&mut self, _: &SelectLeft, _w: &mut Window, _cx: &mut Context) {}
    fn select_right(&mut self, _: &SelectRight, _w: &mut Window, _cx: &mut Context) {}
    fn select_up(&mut self, _: &SelectUp, _w: &mut Window, _cx: &mut Context) {}
    fn select_down(&mut self, _: &SelectDown, _w: &mut Window, _cx: &mut Context) {}
    fn select_start(&mut self, _: &SelectDocumentStart, _w: &mut Window, _cx: &mut Context) {}
    fn select_end(&mut self, _: &SelectDocumentEnd, _w: &mut Window, _cx: &mut Context) {}
    fn select_left_word(&mut self, _: &SelectWordLeft, _w: &mut Window, _cx: &mut Context) {}
    fn select_right_word(&mut self, _: &SelectWordRight, _w: &mut Window, _cx: &mut Context) {}

    fn cut(&mut self, _: &Cut, _w: &mut Window, _cx: &mut Context) {}
    fn copy(&mut self, _: &Copy, _w: &mut Window, _cx: &mut Context) {}
    fn paste(&mut self, _: &Paste, _w: &mut Window, _cx: &mut Context) {}

    fn undo(&mut self, _: &Undo, _w: &mut Window, _cx: &mut Context) {}
    fn redo(&mut self, _: &Redo, _w: &mut Window, _cx: &mut Context) {}

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut Context,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        _event: &gpui::MouseDownEvent,
        _text_position: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context,
    ) {
    }
    fn on_mouse_up(&mut self, _event: &gpui::MouseUpEvent, _w: &mut Window, _cx: &mut Context) {}
    fn on_mouse_move(
        &mut self,
        _event: &gpui::MouseMoveEvent,
        _text_position: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context,
    ) {
    }
}

/// Generic trait to support an element backed by an internal state entity to bind to all editable-text input actions.
pub(super) trait EditableTextActionElement<State> {
    fn state_entity_rc(&self) -> &Rc<RefCell<WeakEntity<State>>>;

    fn register_action<ActionType>(
        &mut self,
        listener: fn(&mut State, &ActionType, &mut Window, &mut Context<State>),
    ) where
        Self: InteractiveElement,
        ActionType: Action + std::fmt::Debug,
        State: 'static,
    {
        let entity_rc = self.state_entity_rc().clone();
        self.interactivity()
            .on_action::<ActionType>(move |action, window, cx| {
                let weak_entity = entity_rc.borrow();
                if let Some(entity) = weak_entity.upgrade() {
                    entity.update(cx, |state, cx| {
                        listener(state, action, window, cx);
                    });
                }
            });
    }

    fn register_actions(&mut self)
    where
        Self: InteractiveElement,
        State: for<'app> EditableTextActionHandler<gpui::Context<'app, State>>,
        State: 'static,
    {
        self.register_action(|state, action, window, cx| state.escape(action, window, cx));
        self.register_action(|state, action, window, cx| state.insert_enter(action, window, cx));
        self.register_action(|state, action, window, cx| state.insert_tab(action, window, cx));
        self.register_action(|state, action, window, cx| state.backspace(action, window, cx));
        self.register_action(|state, action, window, cx| state.delete(action, window, cx));
        self.register_action(|state, action, window, cx| {
            state.delete_word_left(action, window, cx)
        });
        self.register_action(|state, action, window, cx| {
            state.delete_word_right(action, window, cx)
        });
        self.register_action(|state, action, window, cx| {
            state.delete_to_line_start(action, window, cx)
        });
        self.register_action(|state, action, window, cx| {
            state.delete_to_line_end(action, window, cx)
        });
        self.register_action(|state, action, window, cx| state.nav_left(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_right(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_up(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_down(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_line_start(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_line_end(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_start(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_end(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_left_word(action, window, cx));
        self.register_action(|state, action, window, cx| state.nav_right_word(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_all(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_left(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_right(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_up(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_down(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_start(action, window, cx));
        self.register_action(|state, action, window, cx| state.select_end(action, window, cx));
        self.register_action(|state, action, window, cx| {
            state.select_left_word(action, window, cx)
        });
        self.register_action(|state, action, window, cx| {
            state.select_right_word(action, window, cx)
        });
        self.register_action(|state, action, window, cx| state.cut(action, window, cx));
        self.register_action(|state, action, window, cx| state.copy(action, window, cx));
        self.register_action(|state, action, window, cx| state.paste(action, window, cx));
        self.register_action(|state, action, window, cx| state.undo(action, window, cx));
        self.register_action(|state, action, window, cx| state.redo(action, window, cx));
        self.register_action(|state, action, window, cx| {
            state.show_character_palette(action, window, cx)
        });
    }
}
