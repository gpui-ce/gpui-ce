//! Editing actions and their default keyboard bindings.
//!
//! `gpui::actions!` generates command structs independently of their shortcuts. Install
//! [`default_bindings`] at app startup; the supplied fields forward dispatched commands to
//! [`EditableTextActionHandler`] on [`EditableTextState`](crate::editable_text::EditableTextState).
//! Implementing the trait alone does not install key bindings.
//!
//! The standard [`EditableTextState`](crate::editable_text::EditableTextState) deletes the
//! selection or, without one, the requested cluster, word, or hard-line range. Hard lines end
//! at newlines, not soft wraps. With a current layout, cluster, word, and visual-line
//! navigation follows the laid-out text; otherwise it falls back to storage boundaries.
//! Document-start and document-end commands always use storage bounds. Deletion uses logical
//! ranges, so in bidirectional text `DeleteLeft` need not remove the cluster that `NavLeft`
//! would cross. Directional selection extends from the existing anchor.
//!
//! In the standard state, single-line fields propagate [`Enter`] and [`Tab`] with
//! `cx.propagate()`, allowing a parent binding to submit the form or move focus. Multi-line
//! fields insert the corresponding characters. With input disabled, navigation, selection, and
//! copying a non-empty selection remain available; insertion, deletion, cut, and paste do
//! nothing.
use gpui::{InteractiveElement, WeakEntity, Window};
use std::{cell::RefCell, rc::Rc};

/// The key context that editable text elements set for their default bindings.
pub const DEFAULT_INPUT_CONTEXT: &str = "EditableText";

gpui::actions!(
    actions,
    [
        /// Clears the selection, moves the caret to the start, and removes focus from the field.
        Escape,
        /// Inserts a newline at the caret position. In a single-line field, the action
        /// propagates to the surrounding view instead.
        Enter,
        /// Inserts a tab character at the caret position. In a single-line field, the action
        /// propagates to the surrounding view instead, so that Tab can move focus.
        Tab,
        /// Deletes the selection, or the preceding text cluster when nothing is selected.
        DeleteLeft,
        /// Deletes the selection, or the following text cluster when nothing is selected.
        DeleteRight,
        /// Deletes the selection, or back to the preceding word boundary.
        DeleteWordLeft,
        /// Deletes the selection, or forward to the following word boundary.
        DeleteWordRight,
        /// Deletes the selection, or from the caret to the hard-line start.
        DeleteToLineStart,
        /// Deletes the selection, or from the caret to the hard-line end.
        DeleteToLineEnd,
        /// Moves the caret one text cluster to the left.
        NavLeft,
        /// Moves the caret one text cluster to the right.
        NavRight,
        /// Moves up one visual line. A single-line field moves to its only visual line's start.
        NavUp,
        /// Moves down one visual line. A single-line field moves to its only visual line's end.
        NavDown,
        /// Moves to the start of the current hard line, ignoring soft wraps.
        NavLineStart,
        /// Moves to the end of the current hard line, ignoring soft wraps.
        NavLineEnd,
        /// Moves the caret to the beginning of the content.
        NavDocumentStart,
        /// Moves the caret to the end of the content.
        NavDocumentEnd,
        /// Moves the caret one word to the left.
        NavWordLeft,
        /// Moves the caret one word to the right.
        NavWordRight,
        /// Selects all text content.
        SelectAll,
        /// Extends the selection one text cluster to the left.
        SelectLeft,
        /// Extends the selection one text cluster to the right.
        SelectRight,
        /// Extends the selection up one visual line. In a single-line field, extends to the document start.
        SelectUp,
        /// Extends the selection down one visual line. In a single-line field, extends to the document end.
        SelectDown,
        /// Extends the selection to the beginning of the content.
        SelectDocumentStart,
        /// Extends the selection to the end of the content.
        SelectDocumentEnd,
        /// Extends the selection one word to the left.
        SelectWordLeft,
        /// Extends the selection one word to the right.
        SelectWordRight,
        /// Cuts the selection, or the current hard line and an adjacent line break.
        Cut,
        /// Copies selected text. With no selection, leaves the clipboard unchanged.
        Copy,
        /// Replaces marked text or the selection with clipboard text, or inserts at the caret.
        Paste,
        /// Undo the last edit. Edits made in quick succession undo together as one step.
        Undo,
        /// Redo the last undone edit.
        Redo,
        /// Show the platform character palette.
        ShowCharacterPalette,
    ]
);

/// Creates the default keystroke bindings for editable-text actions.
/// See [`ActionBindingCollection`](gpui::ActionBindingCollection) for how bindings are built.
///
/// On macOS, Command-arrow bindings handle line and document navigation. On Linux and
/// Windows, Home and End handle line navigation, while Ctrl+Home and Ctrl+End handle
/// document navigation. Enter and Tab are bound on every platform, but single-line fields
/// propagate them to the parent view.
/// In the bindings below, `secondary` means Command on macOS and Control on
/// other platforms.
///
/// | Action                | All         | Linux & Windows           | macOS           |
/// | --------------------- | ----------- | ------------------------- | --------------- |
/// | Escape                | escape      |                           |                 |
/// | Enter                 | enter       |                           |                 |
/// | Tab                   | tab         |                           |                 |
/// | DeleteLeft            | backspace   |                           |                 |
/// | DeleteRight           | delete      |                           |                 |
/// | DeleteWordLeft        |             | ctrl + backspace          | alt + backspace |
/// | DeleteWordRight       |             | ctrl + delete             | alt + delete    |
/// | DeleteToLineStart     |             | ctrl + shift + backspace  | cmd + backspace |
/// | DeleteToLineEnd       |             | ctrl + shift + delete     | ctrl + k        |
/// | NavLeft               | 🡄          |                           |                  |
/// | NavRight              | 🡆          |                           |                  |
/// | NavUp                 | 🡅          |                           |                  |
/// | NavDown               | 🡇          |                           |                  |
/// | NavLineStart          | shift + home | home                      | cmd + 🡄, cmd + shift + 🡄 |
/// | NavLineEnd            | shift + end  | end                       | cmd + 🡆, cmd + shift + 🡆 |
/// | NavDocumentStart      |             | ctrl + home               | cmd + 🡅         |
/// | NavDocumentEnd        |             | ctrl + end                | cmd + 🡇         |
/// | NavWordLeft           |             | ctrl + 🡄                 | alt + 🡄         |
/// | NavWordRight          |             | ctrl + 🡆                 | alt + 🡆         |
/// | SelectAll             |             | ctrl + a                  | cmd + a         |
/// | SelectLeft            | shift + 🡄  |                           |                  |
/// | SelectRight           | shift + 🡆  |                           |                  |
/// | SelectUp              | shift + 🡅  |                           |                  |
/// | SelectDown            | shift + 🡇  |                           |                  |
/// | SelectDocumentStart   |             | ctrl + shift + home       | cmd + shift + 🡅 |
/// | SelectDocumentEnd     |             | ctrl + shift + end        | cmd + shift + 🡇 |
/// | SelectWordLeft        |             | ctrl + shift + 🡄         | alt + shift + 🡄 |
/// | SelectWordRight       |             | ctrl + shift + 🡆         | alt + shift + 🡆 |
/// | Cut                   |             | ctrl + x                  | cmd + x          |
/// | Copy                  |             | ctrl + c                  | cmd + c          |
/// | Paste                 |             | ctrl + v                  | cmd + v          |
/// | Undo                  |             | ctrl + z                  | cmd + z          |
/// | Redo                  |             | ctrl + shift + z          | cmd + shift + z  |
/// | ShowCharacterPalette  |             | ctrl + space              | cmd + space      |
///
/// The collection cannot remove a default keystroke yet; callers can add additional
/// keystrokes with [`ActionBindingCollection::with`](gpui::ActionBindingCollection::with)
/// before converting the collection to key bindings.
pub fn default_bindings() -> gpui::ActionBindingCollection {
    let mut bindings = gpui::ActionBindingCollection::default()
        .with::<DeleteLeft>("backspace")
        .with::<DeleteRight>("delete")
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
        // Shift+Home and Shift+End intentionally use navigation actions so they collapse the
        // selection at the requested line boundary.
        .with::<NavLineStart>("shift-home")
        .with::<NavLineEnd>("shift-end")
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
            // macOS uses Command-arrows for line and document navigation.
            .with::<NavLineStart>("cmd-left")
            .with::<NavLineEnd>("cmd-right")
            .with::<NavDocumentStart>("cmd-up")
            .with::<NavDocumentEnd>("cmd-down")
            .with::<NavLineStart>("cmd-shift-left")
            .with::<NavLineEnd>("cmd-shift-right")
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
            .with::<NavLineStart>("home")
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

/// Handles the editable-text actions for the state entity of an element.
///
/// The text element installs listeners that forward dispatched actions through its state
/// entity to these methods.
///
/// Editing and pointer methods do nothing by default; override the operations your state
/// supports. [`Self::show_character_palette`] instead opens the platform palette by default.
/// [`EditableTextState`](crate::editable_text::EditableTextState) supplies the standard editing
/// behavior described in this module.
pub trait EditableTextActionHandler<Context>: Sized {
    /// Handles Escape. The standard state clears the selection, moves the caret to the start,
    /// and blurs the window.
    fn escape(&mut self, _: &Escape, _w: &mut Window, _cx: &mut Context) {}

    /// Handles Enter. The standard state inserts a newline, or propagates the action from a
    /// single-line field.
    fn insert_enter(&mut self, _: &Enter, _w: &mut Window, _cx: &mut Context) {}
    /// Handles Tab. The standard state inserts a tab, or propagates the action from a
    /// single-line field.
    fn insert_tab(&mut self, _: &Tab, _w: &mut Window, _cx: &mut Context) {}

    /// Handles DeleteLeft. The standard state deletes the selection or the preceding text cluster.
    fn delete_left(&mut self, _: &DeleteLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles DeleteRight. The standard state deletes the selection or the following text cluster.
    fn delete_right(&mut self, _: &DeleteRight, _w: &mut Window, _cx: &mut Context) {}
    /// Handles DeleteWordLeft. The standard state deletes the selection or back to the preceding word boundary.
    fn delete_word_left(&mut self, _: &DeleteWordLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles DeleteWordRight. The standard state deletes the selection or forward to the following word boundary.
    fn delete_word_right(&mut self, _: &DeleteWordRight, _w: &mut Window, _cx: &mut Context) {}
    /// Handles DeleteToLineStart. The standard state deletes the selection or back to the hard-line start.
    fn delete_to_line_start(&mut self, _: &DeleteToLineStart, _w: &mut Window, _cx: &mut Context) {}
    /// Handles DeleteToLineEnd. The standard state deletes the selection or forward to the hard-line end.
    fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _w: &mut Window, _cx: &mut Context) {}

    /// Handles NavLeft. The standard state moves one visual text cluster to the left when layout is current.
    fn nav_left(&mut self, _: &NavLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavRight. The standard state moves one visual text cluster to the right when layout is current.
    fn nav_right(&mut self, _: &NavRight, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavUp. The standard state moves up one visual line, or to the only line's start in a single-line field.
    fn nav_up(&mut self, _: &NavUp, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavDown. The standard state moves down one visual line, or to the only line's end in a single-line field.
    fn nav_down(&mut self, _: &NavDown, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavLineStart. The standard state moves to the current hard-line start, ignoring soft wraps.
    fn nav_line_start(&mut self, _: &NavLineStart, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavLineEnd. The standard state moves to the current hard-line end, ignoring soft wraps.
    fn nav_line_end(&mut self, _: &NavLineEnd, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavDocumentStart. The standard state moves to the document start.
    fn nav_start(&mut self, _: &NavDocumentStart, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavDocumentEnd. The standard state moves to the document end.
    fn nav_end(&mut self, _: &NavDocumentEnd, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavWordLeft. The standard state moves to the preceding word boundary.
    fn nav_left_word(&mut self, _: &NavWordLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles NavWordRight. The standard state moves to the following word boundary.
    fn nav_right_word(&mut self, _: &NavWordRight, _w: &mut Window, _cx: &mut Context) {}

    /// Handles SelectAll. The standard state selects the entire document.
    fn select_all(&mut self, _: &SelectAll, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectLeft. The standard state extends the selection one visual text cluster to the left.
    fn select_left(&mut self, _: &SelectLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectRight. The standard state extends the selection one visual text cluster to the right.
    fn select_right(&mut self, _: &SelectRight, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectUp. The standard state extends one visual line, or to the document start in a single-line field.
    fn select_up(&mut self, _: &SelectUp, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectDown. The standard state extends one visual line, or to the document end in a single-line field.
    fn select_down(&mut self, _: &SelectDown, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectDocumentStart. The standard state extends the selection to document start.
    fn select_start(&mut self, _: &SelectDocumentStart, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectDocumentEnd. The standard state extends the selection to document end.
    fn select_end(&mut self, _: &SelectDocumentEnd, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectWordLeft. The standard state extends the selection to the preceding word boundary.
    fn select_left_word(&mut self, _: &SelectWordLeft, _w: &mut Window, _cx: &mut Context) {}
    /// Handles SelectWordRight. The standard state extends the selection to the following word boundary.
    fn select_right_word(&mut self, _: &SelectWordRight, _w: &mut Window, _cx: &mut Context) {}

    /// Handles Cut. The standard state cuts the selection, or the current hard line and an
    /// adjacent line break when no selection exists.
    fn cut(&mut self, _: &Cut, _w: &mut Window, _cx: &mut Context) {}
    /// Handles Copy. The standard state copies selected text and leaves the clipboard unchanged
    /// when the selection is empty.
    fn copy(&mut self, _: &Copy, _w: &mut Window, _cx: &mut Context) {}
    /// Handles Paste. The standard state replaces an IME marked range or selection, or inserts
    /// clipboard text at an empty caret.
    fn paste(&mut self, _: &Paste, _w: &mut Window, _cx: &mut Context) {}

    /// Handles Undo. The standard state undoes the last edit, grouping quick successive edits.
    fn undo(&mut self, _: &Undo, _w: &mut Window, _cx: &mut Context) {}
    /// Handles Redo. The standard state redoes the last undone edit.
    fn redo(&mut self, _: &Redo, _w: &mut Window, _cx: &mut Context) {}

    /// Shows the platform character palette.
    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut Context,
    ) {
        window.show_character_palette();
    }

    /// Called on a mouse-down over the text. `text_position` is the pointer
    /// position relative to the top-left of the text.
    fn on_mouse_down(
        &mut self,
        _event: &gpui::MouseDownEvent,
        _text_position: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context,
    ) {
    }
    /// Called when a mouse button is released over the field.
    fn on_mouse_up(&mut self, _event: &gpui::MouseUpEvent, _w: &mut Window, _cx: &mut Context) {}
    /// Called when the pointer moves over the field. `text_position` is the
    /// pointer position relative to the top-left of the text.
    fn on_mouse_move(
        &mut self,
        _event: &gpui::MouseMoveEvent,
        _text_position: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context,
    ) {
    }
}

/// Registers an action listener that upgrades the element's state entity and forwards each
/// action to the corresponding [`EditableTextActionHandler`] method.
macro_rules! register_action {
    ($action_element:expr, $func:ident) => {{
        let entity_rc = $action_element.state_entity_rc().clone();
        $action_element
            .interactivity()
            .on_action(move |action, window, cx| {
                let weak_entity = entity_rc.borrow();
                if let Some(entity) = weak_entity.upgrade() {
                    entity.update(cx, |state, cx| {
                        state.$func(action, window, cx);
                    });
                }
            });
    }};
}

/// Connects an interactive element to its state entity and registers listeners for all
/// editable-text actions.
pub(super) trait EditableTextActionElement<State> {
    fn state_entity_rc(&self) -> &Rc<RefCell<WeakEntity<State>>>;

    fn register_actions(&mut self)
    where
        Self: InteractiveElement,
        State: for<'app> EditableTextActionHandler<gpui::Context<'app, State>>,
        State: 'static,
    {
        register_action!(self, escape);
        register_action!(self, insert_enter);
        register_action!(self, insert_tab);
        register_action!(self, delete_left);
        register_action!(self, delete_right);
        register_action!(self, delete_word_left);
        register_action!(self, delete_word_right);
        register_action!(self, delete_to_line_start);
        register_action!(self, delete_to_line_end);
        register_action!(self, nav_left);
        register_action!(self, nav_right);
        register_action!(self, nav_up);
        register_action!(self, nav_down);
        register_action!(self, nav_line_start);
        register_action!(self, nav_line_end);
        register_action!(self, nav_start);
        register_action!(self, nav_end);
        register_action!(self, nav_left_word);
        register_action!(self, nav_right_word);
        register_action!(self, select_all);
        register_action!(self, select_left);
        register_action!(self, select_right);
        register_action!(self, select_up);
        register_action!(self, select_down);
        register_action!(self, select_start);
        register_action!(self, select_end);
        register_action!(self, select_left_word);
        register_action!(self, select_right_word);
        register_action!(self, cut);
        register_action!(self, copy);
        register_action!(self, paste);
        register_action!(self, undo);
        register_action!(self, redo);
        register_action!(self, show_character_palette);
    }
}
