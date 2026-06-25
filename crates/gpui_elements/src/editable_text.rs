//! Implementation for editable-text elements (gpui equivalent of html
//! [`<input>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/input) and
//! [`<textarea>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/textarea)).
//!
//! TODO: More documentation
//! - caret blinking
//! - history
//! - storage
//! - selection
//! - ime
//! - navigation
//! - overflow (scroll vs clip)
//! - auto-sizing to content via min/max w/h
//! - mouse selection (click x2 x3 drag)
//!
//! Backlog of not-yet implemented features:
//! - text sanitation & validation (see no-op implementation of [`EditableTextState::validate_incoming_text`])
//! - nav & select via PageUp/PageDown
//! - masking text (e.g. for passwords)
//! - disabling `insert_tab` if favor of tab being used to change focus between elements (i.e. escaping the field)
//!

pub mod actions;
mod caret;
mod element;
mod history;
mod layout;
mod state;
mod storage;

pub use caret::*;
pub use element::*;
pub use state::*;
pub use storage::*;
