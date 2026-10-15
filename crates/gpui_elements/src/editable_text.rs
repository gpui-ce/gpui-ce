//! Implementation for editable-text elements, GPUI's equivalent of HTML
//! [`<input>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/input) and
//! [`<textarea>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/textarea)).
//!
//! Both [`text_input`] and [`text_area`] create an [`EditableTextElement`]. This element supports:
//! - navigating with the keyboard and mouse by character, word, line, or document
//! - selecting text with Shift, double- or triple-click, and mouse dragging
//! - typing with an Input Method Editor (IME), including Chinese, Japanese, and Korean UTF-16 input
//! - inserting newlines (`\n`) and tabs (`\t`) in multi-line fields
//! - cut/copy/paste
//! - caret / text cursor that can blink
//! - simple undo/redo within a single field
//! - a single-line mode, where Enter and Tab are passed to the surrounding view, so a form can
//!   submit or move focus; see [`text_input`]
//! - exposing its text and selection to accessibility APIs while an assistive technology is active
//!
//! For all input actions, see documentation in the [`actions`] module.
//!
//! Editable text elements use [`String`] by default (see [`StringStorage`]).
//! A standard library string is not ideal for a large document. For those documents, implement
//! [`UnicodeTextStorage`] with a storage structure such as a rope.
//!
//! Unlike other elements, editable text owns its [`FocusHandle`](gpui::FocusHandle) internally.
//! The field must be focusable before a user can interact with it, and
//! [`Interactivity`](gpui::Interactivity) cannot provide that handle on its own.
//!
//! ### Usage Samples
//!
//! A single-line text input with a fixed width and text that does not wrap
//! (overflow text is clipped and does not scroll).
//! ```
//! # use gpui::prelude::*;
//! # fn test() -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::text_input;
//! text_input("my_input")
//!     .placeholder("empty text")
//!     .w_5()
//!     .min_h_auto()
//!     .whitespace_nowrap()
//! # }
//! ```
//!
//! A single-line text input with a flexible width and text that does not wrap, but will scroll if overflowing.
//! ```
//! # use gpui::{prelude::*, Hsla};
//! # fn test() -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::text_input;
//! text_input("my_input")
//!     .placeholder("empty text")
//!     .border_1().rounded_lg().border_color(gpui::white()) // has a border
//!     .p_2() // padding between the text and border
//!     .min_w_10().max_w_128()
//!     .min_h_auto()
//!     .whitespace_nowrap()
//!     .overflow_x_scroll()
//! # }
//! ```
//!
//! A multi-line text area with flexible height, wrapping text, and scrolling overflow on both axes.
//! ```
//! # use gpui::{prelude::*, Hsla};
//! # fn test() -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::text_area;
//! text_area("message")
//!     .placeholder("empty text")
//!     .border_1().rounded_lg().border_color(gpui::white()) // has a border
//!     .p_2() // padding between the text and border
//!     .min_w_10().max_w_128()
//!     .min_h_24().max_h_128()
//!     .whitespace_normal() // default
//!     .overflow_y_scroll()
//! # }
//! ```
//!
//! The user-inputted text can be accessed via event callbacks on the element.
//! There is no callback for "editing finished." Debounce [`TextChanged`] events or observe focus
//! changes when you need to detect that the user has left the field.
//! ```
//! # use gpui::{prelude::*, App, Entity, Window, AppContext, ElementId};
//! # fn test(window: &mut Window, cx: &mut App) -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::{text_input, EditableTextState, TextChanged};
//!
//! // A unique id to the editable text element within the outer scope.
//! let id = ElementId::from("my_input");
//!
//! // Find or lazily create the state entity backing the element.
//! // Then attach the entity to the element, thereby keeping it alive across consecutive frames.
//! let state = EditableTextState::use_keyed(id.clone(), window, cx);
//!
//! // This will trigger on every character input or other mutation to the underlying string
//! cx.subscribe(&state, |state, _: &TextChanged, cx| {
//! 	println!("{:?}", state.read(cx).as_str());
//! }).detach();
//!
//! // Using state explicitly attaches the state we already have attached to the ElementId.
//! text_input(id).state(state.downgrade())
//! # }
//! ```
//!
//! You can configure the default value of the editable text by using [`EditableTextState::use_keyed_init`]:
//! ```
//! # use gpui::{prelude::*, App, Entity, Window, AppContext, ElementId};
//! # use gpui_ce_elements::editable_text::{text_input, EditableTextState, StringStorage};
//! # fn test(window: &mut Window, cx: &mut App) -> gpui_ce_elements::editable_text::EditableTextElement {
//! let id = ElementId::from("my_input");
//!
//! // The function parameter will only be called when the state is created/initialized.
//! // All successive renders across consecutive frames will re-use the existing state.
//! let _state = EditableTextState::use_keyed_init(id.clone(), window, cx,
//!     |_window, _cx| StringStorage::from("this is some default text content"));
//!
//! // You can omit the state call: the element looks up state by id. Passing the
//! // state explicitly makes the connection clear and helps when debugging.
//! text_input(id)
//! # }
//! ```
//!
//! To use a blinking caret, you can use one of the templated functions:
//! ```
//! # use gpui::{prelude::*, App, Entity, Window, AppContext, ElementId};
//! # fn test(window: &mut Window, cx: &mut App) -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::{text_input};
//! let id = ElementId::from("my_input");
//! text_input(id)
//!     .caret_blink_interval_500ms()
//!     // or use the parameterized one, e.g. 200ms
//!     .caret_blink_interval(std::time::Duration::from_millis(200))
//! # }
//! ```
//!
//! or construct a caret entity with a blinking interval when constructing the state:
//! ```
//! # use gpui::{prelude::*, App, Entity, Window, AppContext, ElementId};
//! # fn test(window: &mut Window, cx: &mut App) -> gpui_ce_elements::editable_text::EditableTextElement {
//! use gpui_ce_elements::editable_text::{text_input, EditableTextState, TextChanged, Caret};
//! let id = ElementId::from("my_input");
//!
//! let state = EditableTextState::use_keyed(id.clone(), window, cx);
//!
//! // Ensure the caret exists, linked to the input element by id.
//! window.use_keyed_state(id.clone(), cx, |window, cx| {
//!     // using the default interval of 500ms
//!     let mut caret = Caret::default().with_blink_interval_500ms();
//!     // ensures the caret receives events from the input state during typing & other actions
//!     caret.subscribe_to(&state, cx);
//!     caret
//! });
//!
//! text_input(id).state(state.downgrade())
//! # }
//! ```
//!
//! Full-text examples can be found in the [examples folder](https://github.com/gpui-ce/gpui-ce/tree/main/crates/gpui_elements/examples)
//!
//! ### Backlog of not-yet implemented features:
//! - a public callback for when a field loses focus (the state clears its selection on blur, but
//!   emits no event)
//! - text sanitization and validation beyond removing line breaks from single-line fields. The
//!   private `validate_incoming_text` method of `EditableTextState` is where this would go.
//! - navigation and selection via PageUp/PageDown
//! - fuller screen reader support, beyond the text and selection exposed while an assistive
//!   technology is active
//! - masking text (e.g. for passwords)
//!

pub mod actions;
mod caret;
mod element;
mod history;
pub use history::*;
mod layout;
mod state;
mod storage;

pub use caret::*;
pub use element::*;
pub use state::*;
pub use storage::*;
