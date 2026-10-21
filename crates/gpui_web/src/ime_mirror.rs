//! The hidden `<textarea>` that connects browser IMEs to GPUI.
//!
//! IMEs inspect the focused element's value and selection for backspace,
//! autocorrect, and suggestions. This module mirrors nearby document text
//! into the element so those edits arrive as interpretable events.
//!
//! Only [`sync`], reached through [`ImeMirror::schedule_sync`], writes the
//! value and selection. Each write restarts the IME connection, so requests
//! coalesce into at most one write per browser event-loop turn, after all
//! events for the gesture have dispatched.
//!
//! The element and its write path are private to this module, so the compiler
//! enforces this restriction.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration};
use wasm_bindgen::JsCast;

use crate::window::WebWindowInner;

/// UTF-16 code units mirrored on each side of the selection.
///
/// The browser answers IME context requests, such as Android's
/// `getTextBeforeCursor`, without notifying the page. Reserve context ahead
/// of time for sentence-scale reads of roughly a hundred units.
///
/// Larger windows cost more on every keystroke. Edits diff the full value,
/// and rebuilds copy the window and snapshot it across the browser-IME
/// boundary, all on the main thread between frames. Reads beyond the window
/// see truncated text, as they would in a short field.
const CONTEXT_CHARS: usize = 512;

/// Recenter when the selection gets this close to the mirror's edge, or on
/// desynchronization. Must exceed a gesture's read and edit span, including
/// a long word and autocorrect lookback. Every recenter restarts the IME.
const MIN_EDGE_CHARS: usize = 64;

/// The hidden `<textarea>` and its document mapping.
///
/// Value and selection writes stay private. Other code uses read accessors,
/// focus, read-only control, and [`ImeMirror::schedule_sync`].
pub(crate) struct ImeMirror {
    element: web_sys::HtmlTextAreaElement,
    /// The last synced or observed text, diffed against `input` events to
    /// recover IME edits.
    text: RefCell<String>,
    /// Selection after the last sync or imported edit, in element-local
    /// UTF-16 offsets. IME edits use caret-relative positions because
    /// document coordinates go stale during collaboration.
    selection: Cell<(u32, u32)>,
    /// Hint for the mirror's document offset, never used to position edits.
    /// Every use verifies the stored text against the document at this
    /// alignment, so a stale hint triggers a rebuild instead of a misplaced edit.
    window_hint: Cell<usize>,
    /// Whether a sync is scheduled for the next task. Coalescing requests
    /// into one write after the gesture preserves the keyboard's word model.
    sync_scheduled: Cell<bool>,
    /// Whether the `selectionchange` import saw an element selection move
    /// it could not apply (the app selection changed underneath). Sync
    /// normally defers to a pending import when the element's live
    /// selection has moved; a rejected import means no import is coming,
    /// so the next sync must reassert the app's state instead of waiting.
    selection_import_rejected: Cell<bool>,
}

/// Whether the device's primary pointer is coarse (a touch screen). The
/// distinction drives virtual-keyboard policy: touch-first browsers summon
/// the keyboard for any focused editable element on a user gesture.
fn primary_pointer_is_coarse() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(pointer: coarse)").ok().flatten())
        .is_some_and(|media_query_list| media_query_list.matches())
}

impl ImeMirror {
    pub(crate) fn new(
        document: &web_sys::Document,
        body: &web_sys::HtmlElement,
    ) -> anyhow::Result<Self> {
        // Use a textarea because single-line inputs strip newlines and would
        // desynchronize the mirror.
        let element: web_sys::HtmlTextAreaElement = document
            .create_element("textarea")
            .map_err(|e| anyhow::anyhow!("Failed to create textarea element: {e:?}"))?
            .dyn_into()
            .map_err(|e| anyhow::anyhow!("Created element is not a textarea: {e:?}"))?;
        let style = element.style();
        style.set_property("position", "fixed").ok();
        style.set_property("top", "0").ok();
        style.set_property("left", "0").ok();
        style.set_property("width", "1px").ok();
        style.set_property("height", "1px").ok();
        style.set_property("opacity", "0").ok();
        // Android Chrome zooms the visual viewport onto a focused text input
        // whose font is smaller than 16px; with page zoom disabled the user
        // can never zoom back out, so keep the hidden IME input at 16px.
        style.set_property("font-size", "16px").ok();
        body.append_child(&element)
            .map_err(|e| anyhow::anyhow!("Failed to append input to body: {e:?}"))?;
        element.focus().ok();
        // Keep focus for hardware keys and IME events. On touch devices, start
        // read-only so scrolling cannot summon the virtual keyboard. Only a
        // recognized text-input tap enables editing in `sync_virtual_keyboard`.
        if primary_pointer_is_coarse() {
            element.set_read_only(true);
        }

        let this = Self {
            element,
            text: RefCell::new(String::new()),
            selection: Cell::new((0, 0)),
            window_hint: Cell::new(0),
            sync_scheduled: Cell::new(false),
            selection_import_rejected: Cell::new(false),
        };
        // Disable browser text assistance until the input handler requests it,
        // to prevent edits outside the app's control.
        this.apply_configuration(&TextInputConfiguration::default());
        Ok(this)
    }

    /// Maps a [`TextInputConfiguration`] onto the element's text-assistance
    /// attributes. Callers must only invoke this on actual configuration
    /// changes (GPUI diffs before forwarding): mutating the focused element
    /// can restart the IME's input connection.
    pub(crate) fn apply_configuration(&self, configuration: &TextInputConfiguration) {
        let element: &web_sys::Element = self.element.as_ref();
        self.element.set_spellcheck(configuration.suggestions);
        let on_off = |enabled: bool| if enabled { "on" } else { "off" };
        element
            .set_attribute("autocomplete", on_off(configuration.suggestions))
            .ok();
        element
            .set_attribute("autocorrect", on_off(configuration.autocorrect))
            .ok();
        element
            .set_attribute(
                "autocapitalize",
                match configuration.autocapitalize {
                    Autocapitalize::None => "off",
                    Autocapitalize::Words => "words",
                    Autocapitalize::Sentences => "sentences",
                    Autocapitalize::Characters => "characters",
                },
            )
            .ok();
        let enter_key_hint = match configuration.input_action {
            TextInputAction::Unspecified => None,
            TextInputAction::Enter => Some("enter"),
            TextInputAction::Done => Some("done"),
            TextInputAction::Go => Some("go"),
            TextInputAction::Next => Some("next"),
            TextInputAction::Previous => Some("previous"),
            TextInputAction::Search => Some("search"),
            TextInputAction::Send => Some("send"),
        };
        match enter_key_hint {
            Some(hint) => element.set_attribute("enterkeyhint", hint).ok(),
            None => element.remove_attribute("enterkeyhint").ok(),
        };
    }

    pub(crate) fn event_target(&self) -> &web_sys::EventTarget {
        self.element.as_ref()
    }

    pub(crate) fn focus(&self) {
        self.element.focus().ok();
    }

    pub(crate) fn is_focused(&self) -> bool {
        let element: &web_sys::Element = self.element.as_ref();
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .is_some_and(|active| &active == element)
    }

    pub(crate) fn blur(&self) {
        self.element.blur().ok();
    }

    pub(crate) fn read_only(&self) -> bool {
        self.element.read_only()
    }

    pub(crate) fn set_read_only(&self, read_only: bool) {
        self.element.set_read_only(read_only);
    }

    pub(crate) fn remove(&self) {
        let element: &web_sys::Element = self.element.as_ref();
        element.remove();
    }

    pub(crate) fn value(&self) -> String {
        self.element.value()
    }

    pub(crate) fn selection_start(&self) -> Option<u32> {
        self.element.selection_start().ok().flatten()
    }

    pub(crate) fn element_selection_end(&self) -> Option<u32> {
        self.element.selection_end().ok().flatten()
    }

    pub(crate) fn stored_text(&self) -> String {
        self.text.borrow().clone()
    }

    pub(crate) fn stored_selection(&self) -> (u32, u32) {
        self.selection.get()
    }

    /// Adopts the browser's edited value and selection without writing back,
    /// which would restart the IME mid-gesture. Used for imported IME edits
    /// and composition commits.
    pub(crate) fn adopt_element_state(&self) {
        *self.text.borrow_mut() = self.element.value();
        let selection_start = self.selection_start().unwrap_or(0);
        let selection_end = self
            .element
            .selection_end()
            .ok()
            .flatten()
            .unwrap_or(selection_start);
        self.selection.set((selection_start, selection_end));
    }

    /// Records that an element selection move could not be imported, so
    /// the next sync reasserts the app's state rather than deferring to an
    /// import that is no longer coming.
    pub(crate) fn reject_selection_import(&self) {
        self.selection_import_rejected.set(true);
    }

    /// Schedules a coalesced sync for the next task.
    ///
    /// Mid-gesture value or selection writes desynchronize the IME's field
    /// model. A zero-delay timeout coalesces requests into one write after
    /// the gesture's events, matching a plain textarea's behavior.
    ///
    /// Nesting `sync` here prevents direct calls from writing mid-gesture.
    pub(crate) fn schedule_sync(window: &Rc<WebWindowInner>) {
        if window.ime_mirror.sync_scheduled.replace(true) {
            return;
        }
        let closure = wasm_bindgen::closure::Closure::once_into_js({
            let window = Rc::clone(window);
            move || {
                window.ime_mirror.sync_scheduled.set(false);
                sync(&window);
            }
        });
        window
            .browser_window
            .set_timeout_with_callback(closure.unchecked_ref())
            .ok();

        /// Mirrors the text surrounding the selection into the hidden
        /// element.
        ///
        /// With an empty element, Gboard deletes against its private buffer
        /// (the keypress reaches the page only as an `"Unidentified"`
        /// placeholder) and its suggestion strip has no context. Mirroring
        /// a window of real text makes those operations arrive as
        /// interpretable `beforeinput` events.
        ///
        /// All offsets are UTF-16 code units on both sides: GPUI's
        /// input-handler protocol and JavaScript string indexing agree by
        /// construction.
        ///
        /// Value and selection writes restart the IME and reset keyboard state,
        /// breaking multi-step edits such as suggestion delete-then-insert pairs.
        /// Verify the current window against the document at its current alignment
        /// and skip every write while it remains valid, even if off-center.
        ///
        /// Rebuild only for independent app changes, such as remote edits within
        /// the window or caret moves by tap or keybinding, or when the selection
        /// gets too close to an edge to provide enough IME context.
        fn sync(window: &WebWindowInner) {
            if window.is_composing.get() {
                return;
            }
            let mirror = &window.ime_mirror;
            // A live element selection that differs from the stored baseline
            // while the value still matches is an IME-driven selection move
            // whose `selectionchange` import hasn't dispatched yet (the event
            // is asynchronous, and this sync may run first). The element owns
            // the selection until that import runs: writing now would clobber
            // an in-progress gesture, e.g. Android's slide-on-backspace
            // growing its selection. The import reconciles the two sides and
            // schedules a fresh sync when it cannot adopt the move.
            if !mirror.selection_import_rejected.replace(false)
                && *mirror.text.borrow() == mirror.element.value()
            {
                let live_start = mirror.selection_start().unwrap_or(0);
                let live_end = mirror.element_selection_end().unwrap_or(live_start);
                if (live_start, live_end) != mirror.selection.get() {
                    return;
                }
            }
            let selection = window
                .with_input_handler(|handler| handler.selected_text_range(false))
                .flatten();
            let Some(selection) = selection else {
                if !mirror.text.borrow().is_empty() {
                    mirror.element.set_value("");
                    mirror.text.borrow_mut().clear();
                }
                mirror.selection.set((0, 0));
                return;
            };
            // The mirrored window never crosses the handler's editable
            // range: everything in the element is reachable by multi-step
            // IME edit gestures (word deletion, autocorrect rewrites), so
            // text outside the range must not be mirrored at all. The IME
            // sees the range's edges as the field's edges.
            let editable_range = window
                .with_input_handler(|handler| handler.text_input_editable_range())
                .flatten();

            if is_consistent(
                window,
                &selection.range,
                editable_range.as_ref(),
                MIN_EDGE_CHARS,
            ) {
                return;
            }

            // A caret move within the existing window (a tap into nearby
            // text) must update only the element's selection, like a native
            // tap in a plain textarea. Rewriting the value restarts the IME
            // connection, which desynchronizes the keyboard's word model
            // right when it is about to act on the tapped word.
            if move_selection_within_window(
                window,
                &selection.range,
                editable_range.as_ref(),
                MIN_EDGE_CHARS,
            ) {
                return;
            }

            let mut window_range = selection.range.start.saturating_sub(CONTEXT_CHARS)
                ..selection.range.end + CONTEXT_CHARS;
            if let Some(editable_range) = &editable_range {
                window_range.start = window_range.start.max(editable_range.start);
                window_range.end = window_range
                    .end
                    .min(editable_range.end)
                    .max(window_range.start);
            }
            let mut adjusted = None;
            let text = window
                .with_input_handler(|handler| {
                    handler.text_for_range(window_range.clone(), &mut adjusted)
                })
                .flatten()
                .unwrap_or_default();
            let window_start = adjusted.unwrap_or(window_range).start;

            if *mirror.text.borrow() != text || mirror.element.value() != text {
                mirror.element.set_value(&text);
                *mirror.text.borrow_mut() = text;
            }

            mirror.window_hint.set(window_start);
            let selection_start = selection.range.start.saturating_sub(window_start) as u32;
            let selection_end = selection.range.end.saturating_sub(window_start) as u32;
            if mirror.element.selection_start().ok().flatten() != Some(selection_start)
                || mirror.element.selection_end().ok().flatten() != Some(selection_end)
            {
                mirror
                    .element
                    .set_selection_range(selection_start, selection_end)
                    .ok();
            }
            // Read the selection back rather than trusting the computed
            // values: the browser clamps out-of-bounds positions, and a
            // stored selection the element doesn't actually have would
            // corrupt the next diff.
            let actual_start = mirror.element.selection_start().ok().flatten();
            let actual_end = mirror.element.selection_end().ok().flatten();
            mirror.selection.set((
                actual_start.unwrap_or(selection_start),
                actual_end.unwrap_or(selection_end),
            ));
        }
    }
}

/// Attempts to represent a changed app selection as a pure element
/// selection move within the existing mirror window.
///
/// The stored window-start hint is re-verified textually against the
/// document before use, so a stale hint (remote edit, any drift) fails
/// verification and falls through to a full window rebuild rather than
/// mispositioning the selection.
fn move_selection_within_window(
    window: &WebWindowInner,
    app_selection: &std::ops::Range<usize>,
    editable_range: Option<&std::ops::Range<usize>>,
    min_edge: usize,
) -> bool {
    let mirror = &window.ime_mirror;
    let stored_text = mirror.text.borrow().clone();
    let stored_length = stored_text.encode_utf16().count();
    if stored_length == 0 || mirror.element.value() != stored_text {
        return false;
    }
    let window_start = mirror.window_hint.get();

    // Keep context on both sides unless the window reaches a document or
    // editable-range boundary, such as a caret at a chat thread's end.
    //
    // Never mirror text outside the editable range, which the IME must not edit.
    if let Some(range) = editable_range
        && (window_start < range.start || window_start + stored_length > range.end)
    {
        return false;
    }
    let left_boundary = editable_range.map_or(0, |range| range.start);
    let Some(selection_start) = app_selection.start.checked_sub(window_start) else {
        return false;
    };
    let selection_end = selection_start + (app_selection.end - app_selection.start);
    if selection_end > stored_length {
        return false;
    }
    if selection_start < min_edge && window_start > left_boundary {
        return false;
    }

    // Verify the hint: the stored window text must still equal the
    // document at this alignment. Asking for one unit extra also
    // determines whether the window reaches the document's end, which
    // excuses a missing right margin, as does reaching the editable
    // range's end.
    let mut adjusted = None;
    let document_text = window
        .with_input_handler(|handler| {
            handler.text_for_range(
                window_start..window_start + stored_length + 1,
                &mut adjusted,
            )
        })
        .flatten()
        .unwrap_or_default();
    let document_text_length = document_text.encode_utf16().count();
    let window_at_right_boundary = document_text_length == stored_length
        || editable_range.is_some_and(|range| window_start + stored_length >= range.end);
    if selection_end + min_edge > stored_length && !window_at_right_boundary {
        return false;
    }
    if !document_text.starts_with(stored_text.as_str()) || document_text_length > stored_length + 1
    {
        return false;
    }

    mirror
        .element
        .set_selection_range(selection_start as u32, selection_end as u32)
        .ok();
    let actual_start = mirror.element.selection_start().ok().flatten();
    let actual_end = mirror.element.selection_end().ok().flatten();
    if actual_start != Some(selection_start as u32) || actual_end != Some(selection_end as u32) {
        return false;
    }
    mirror
        .selection
        .set((selection_start as u32, selection_end as u32));
    true
}

/// Whether the hidden element, at its current window alignment, is still
/// an accurate mirror of the document around the app selection with
/// enough context on both sides. When this holds, a sync must not touch
/// the element (see [`sync`] on why writes are harmful).
fn is_consistent(
    window: &WebWindowInner,
    app_selection: &std::ops::Range<usize>,
    editable_range: Option<&std::ops::Range<usize>>,
    min_edge: usize,
) -> bool {
    let mirror = &window.ime_mirror;
    let (element_selection_start, element_selection_end) = mirror.selection.get();
    let element_selection_start = element_selection_start as usize;
    let element_selection_end = element_selection_end as usize;
    let stored_text = mirror.text.borrow().clone();
    let stored_length = stored_text.encode_utf16().count();

    if stored_length == 0 {
        return false;
    }
    // The element's real selection must match what we believe it is.
    if mirror.element.selection_start().ok().flatten() != Some(element_selection_start as u32)
        || mirror.element.selection_end().ok().flatten() != Some(element_selection_end as u32)
    {
        return false;
    }
    // Require context on both sides unless the window starts at the document
    // or editable range's left boundary, or ends at its right boundary.
    let app_window_start = match app_selection.start.checked_sub(element_selection_start) {
        Some(start) => start,
        None => return false,
    };
    // A window that leaks outside the editable range mirrors text the IME
    // must not reach, however consistent it is.
    if let Some(range) = editable_range
        && (app_window_start < range.start || app_window_start + stored_length > range.end)
    {
        return false;
    }
    let left_boundary = editable_range.map_or(0, |range| range.start);
    let has_left_context = element_selection_start >= min_edge || app_window_start <= left_boundary;
    let right_context = stored_length.saturating_sub(element_selection_end);
    if !has_left_context || right_context < min_edge {
        let window_end = app_window_start + stored_length;
        let at_right_boundary = if let Some(range) = editable_range {
            window_end >= range.end
        } else {
            // Verify the window genuinely reaches the end of the document
            // by asking for one unit past the stored window.
            let mut adjusted = None;
            window
                .with_input_handler(|handler| {
                    handler.text_for_range(window_end..window_end + 1, &mut adjusted)
                })
                .flatten()
                .unwrap_or_default()
                .is_empty()
        };
        if !has_left_context || !at_right_boundary {
            return false;
        }
    }
    // The stored window must still equal the document at this alignment
    // (a remote edit inside the window invalidates it), and the element
    // must still hold exactly the stored text.
    let mut adjusted = None;
    let document_text = window
        .with_input_handler(|handler| {
            handler.text_for_range(
                app_window_start..app_window_start + stored_length,
                &mut adjusted,
            )
        })
        .flatten()
        .unwrap_or_default();
    if document_text != stored_text {
        return false;
    }
    if mirror.element.value() != stored_text {
        return false;
    }
    // The element selection corresponds to the app selection end too?
    app_selection.end.checked_sub(app_window_start) == Some(element_selection_end)
}
