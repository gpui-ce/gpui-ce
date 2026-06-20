use gpui::{App, Window};

/// The key context used for input element keybindings.
pub const DEFAULT_INPUT_CONTEXT: &str = "Input";

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
        DeleteToBeginningOfLine,
        /// Delete from the cursor to the end of the line.
        DeleteToEndOfLine,
        /// Move the cursor one character to the left.
        Left,
        /// Move the cursor one character to the right.
        Right,
        /// Move the cursor up one visual line.
        Up,
        /// Move the cursor down one visual line.
        Down,
        /// Move cursor to the start of the current line.
        Home,
        /// Move cursor to the end of the current line.
        End,
        /// Move cursor to the beginning of the content.
        MoveToBeginning,
        /// Move cursor to the end of the content.
        MoveToEnd,
        /// Move cursor one word to the left.
        WordLeft,
        /// Move cursor one word to the right.
        WordRight,
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
        SelectToBeginning,
        /// Extend selection to the end of the content.
        SelectToEnd,
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
        .with::<Left>("left")
        .with::<Right>("right")
        .with::<Up>("up")
        .with::<Down>("down")
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
            .with::<DeleteToBeginningOfLine>("cmd-backspace")
            .with::<DeleteToEndOfLine>("ctrl-k")
            // Mac keyboards don't have Home/End keys, so cmd-left/right are standard
            .with::<Home>("cmd-left")
            .with::<End>("cmd-right")
            .with::<MoveToBeginning>("cmd-up")
            .with::<MoveToEnd>("cmd-down")
            .with::<SelectToBeginning>("cmd-shift-up")
            .with::<SelectToEnd>("cmd-shift-down")
            .with::<WordLeft>("alt-left")
            .with::<WordRight>("alt-right")
            .with::<SelectWordLeft>("alt-shift-left")
            .with::<SelectWordRight>("alt-shift-right");
    }

    #[cfg(not(target_os = "macos"))]
    {
        bindings = bindings
            .with::<DeleteWordLeft>("ctrl-backspace")
            .with::<DeleteWordRight>("ctrl-delete")
            .with::<DeleteToBeginningOfLine>("ctrl-shift-backspace")
            .with::<DeleteToEndOfLine>("ctrl-shift-delete")
            .with::<Home>("home")
            .with::<End>("end")
            .with::<MoveToBeginning>("ctrl-home")
            .with::<MoveToEnd>("ctrl-end")
            .with::<SelectToBeginning>("ctrl-shift-home")
            .with::<SelectToEnd>("ctrl-shift-end")
            .with::<WordLeft>("ctrl-left")
            .with::<WordRight>("ctrl-right")
            .with::<SelectWordLeft>("ctrl-shift-left")
            .with::<SelectWordRight>("ctrl-shift-right");
    }

    bindings
}

pub trait EditableTextActionHandler {
    fn escape(&mut self, _: &Escape, _w: &mut Window, _cx: &mut App) {}

    fn insert_enter(&mut self, _: &Enter, _w: &mut Window, _cx: &mut App) {}
    fn insert_tab(&mut self, _: &Tab, _w: &mut Window, _cx: &mut App) {}

    fn backspace(&mut self, _: &Backspace, _w: &mut Window, _cx: &mut App) {}
    fn delete(&mut self, _: &Delete, _w: &mut Window, _cx: &mut App) {}

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _w: &mut Window, _cx: &mut App) {}
    fn delete_word_right(&mut self, _: &DeleteWordRight, _w: &mut Window, _cx: &mut App) {}
    fn delete_to_line_start(
        &mut self,
        _: &DeleteToBeginningOfLine,
        _w: &mut Window,
        _cx: &mut App,
    ) {
    }
    fn delete_to_line_end(&mut self, _: &DeleteToEndOfLine, _w: &mut Window, _cx: &mut App) {}

    fn nav_left(&mut self, _: &Left, _w: &mut Window, _cx: &mut App) {}
    fn nav_right(&mut self, _: &Right, _w: &mut Window, _cx: &mut App) {}
    fn nav_up(&mut self, _: &Up, _w: &mut Window, _cx: &mut App) {}
    fn nav_down(&mut self, _: &Down, _w: &mut Window, _cx: &mut App) {}
    fn nav_line_start(&mut self, _: &Home, _w: &mut Window, _cx: &mut App) {}
    fn nav_line_end(&mut self, _: &End, _w: &mut Window, _cx: &mut App) {}
    fn nav_start(&mut self, _: &MoveToBeginning, _w: &mut Window, _cx: &mut App) {}
    fn nav_end(&mut self, _: &MoveToEnd, _w: &mut Window, _cx: &mut App) {}
    fn nav_left_word(&mut self, _: &WordLeft, _w: &mut Window, _cx: &mut App) {}
    fn nav_right_word(&mut self, _: &WordRight, _w: &mut Window, _cx: &mut App) {}

    fn select_all(&mut self, _: &SelectAll, _w: &mut Window, _cx: &mut App) {}
    fn select_left(&mut self, _: &SelectLeft, _w: &mut Window, _cx: &mut App) {}
    fn select_right(&mut self, _: &SelectRight, _w: &mut Window, _cx: &mut App) {}
    fn select_up(&mut self, _: &SelectUp, _w: &mut Window, _cx: &mut App) {}
    fn select_down(&mut self, _: &SelectDown, _w: &mut Window, _cx: &mut App) {}
    fn select_start(&mut self, _: &SelectToBeginning, _w: &mut Window, _cx: &mut App) {}
    fn select_end(&mut self, _: &SelectToEnd, _w: &mut Window, _cx: &mut App) {}
    fn select_left_word(&mut self, _: &SelectWordLeft, _w: &mut Window, _cx: &mut App) {}
    fn select_right_word(&mut self, _: &SelectWordRight, _w: &mut Window, _cx: &mut App) {}

    fn cut(&mut self, _: &Cut, _w: &mut Window, _cx: &mut App) {}
    fn copy(&mut self, _: &Copy, _w: &mut Window, _cx: &mut App) {}
    fn paste(&mut self, _: &Paste, _w: &mut Window, _cx: &mut App) {}

    fn undo(&mut self, _: &Undo, _w: &mut Window, _cx: &mut App) {}
    fn redo(&mut self, _: &Redo, _w: &mut Window, _cx: &mut App) {}

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        _position: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut App,
    ) {
    }
    fn on_mouse_up(&mut self, _event: &gpui::MouseUpEvent, _w: &mut Window, _cx: &mut App) {}
    fn on_mouse_move(&mut self, _event: &gpui::MouseMoveEvent, _w: &mut Window, _cx: &mut App) {}
}
