# Accept text input

An editable field has two parts: an `EditableTextState` entity that survives
between frames, and an element that lays out and paints it. Store the entity
in your view. It holds the text, selection, undo history, focus handle and
input-method composition while `render` rebuilds the element.

The `gpui_ce_elements` crate supplies `text_input` for a single line and
`text_area` for several lines. They share the state type, so a form can use
either without changing how it reads the value or subscribes to edits.

## Hold the state

Create the entity once in the view's constructor and pass it to the field
when rendering. This field starts with `"Ada"`; `StringStorage::default()`
would start empty:

```rust
# use gpui::{AppContext as _, Context, Entity, Window, div, prelude::*};
# use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, text_input};
# struct Profile { name: Entity<EditableTextState> }
# impl Profile {
fn new(cx: &mut Context<Self>) -> Self {
    let name = cx.new(|cx| EditableTextState::new(StringStorage::from("Ada"), cx));
    Self { name }
}
# }
# impl Render for Profile {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
div().child(text_input("name").state(self.name.downgrade()))
#     }
# }
```

The field takes a [`WeakEntity`](gpui::WeakEntity); `Profile::name` keeps the
strong handle alive. Creating a replacement entity on every render would also
replace its selection, history, composition and [`FocusHandle`](gpui::FocusHandle).
Retaining a handle does not itself focus the field: the element tracks the
handle, and the app selects it with `window.focus`, as in the submit example.
Give the field an ID unique within the view (`"name"` here).

If only the element needs the state, `EditableTextState::use_keyed` can retain
it under that ID instead. Call it during `render`; it finds the existing state
or creates it, and retains it while the keyed element is rendered.
`use_keyed_init` supplies initial content. Its initializer runs on creation,
not on every render, so later changes to that initializer do not replace the
user's text.

## Install the editing keys

Typing can work even when Backspace and undo do nothing. Text comes through
the platform input handler; editing commands come through key bindings. The
element sets the `EditableText` key context, but the app must install its
`default_bindings()`. Do this once at startup, before opening a window:

```rust
# use gpui::App;
use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};

# fn setup(cx: &mut App) {
cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
# }
```

The [keystroke guide](crate::_documentation::guides::keyboard) explains how
GPUI chooses a binding from the focused path. The `default_bindings` reference
lists the editing shortcuts for each platform.

## Read and replace the text

Read the entity and call `EditableTextState::as_str` to obtain the current
value. To replace it from application code, call `EditableTextState::emplace`.
Clearing a submitted field with `emplace("")` is an edit: it moves the caret
to the end, records undo history and emits a change event. Use `emplace` for
later programmatic changes, including changes to a field created with
`use_keyed_init`.

## React to changes

An edit, undo, redo or `emplace` emits `TextChanged`. A `cx.subscribe` callback
can read `as_str` to update a count, enable a button, start a search or display
validation feedback. Keep its [`Subscription`](gpui::Subscription) in the view;
dropping the subscription unregisters the callback. The form below uses one
to request a new render as the user types.

A change subscriber sees the value after mutation and cannot veto that edit.
Calling `emplace` there causes a second edit and a second event, so any such
correction needs a guard against recursion. Validate application-supplied
values before calling `emplace`. To filter typed input before mutation, write
an [input handler](#implement-your-own-text-element), including its selection
and input-method protocol.

## Submit with Enter

A single-line field's Enter action calls `propagate`, allowing GPUI to try the
next binding for the keystroke. Add an Enter binding in the enclosing view's
key context to submit the form. A multi-line area handles Enter by inserting
a newline instead.

Here Enter appends the name to a list and clears the field. The test installs
both sets of bindings, focuses the field, types `Ada` and submits it:

```standalone_crate
use gpui::{
    AppContext as _, Context, Entity, Focusable, KeyBinding, Subscription, TestApp, Window,
    actions, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};

actions!(text_input_submit, [Submit]);

struct SignUp {
    name: Entity<EditableTextState>,
    submitted: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl SignUp {
    fn new(cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let subscription = cx.subscribe(&name, |_, _, _: &TextChanged, cx| cx.notify());
        Self {
            name,
            submitted: Vec::new(),
            _subscriptions: vec![subscription],
        }
    }

    fn submit(&mut self, _: &Submit, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).as_str().trim().to_string();
        if !name.is_empty() {
            self.submitted.push(name);
            self.name.update(cx, |state, cx| state.emplace("", cx));
        }
        cx.notify();
    }
}

impl Render for SignUp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("SignUp")
            .on_action(cx.listener(Self::submit))
            .child(
                text_input("name")
                    .state(self.name.downgrade())
                    .placeholder("Your name")
                    .placeholder_color(gpui::rgb(0x8a8a8a))
                    .caret_blink_interval_500ms()
                    .w(px(240.))
                    .px_3()
                    .py_2()
                    .border_1()
                    .rounded_md(),
            )
    }
}

let mut app = TestApp::new();
app.update(|cx| {
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    cx.bind_keys([KeyBinding::new("enter", Submit, Some("SignUp"))]);
});

let mut window = app.open_window(|_, cx| SignUp::new(cx));
window.update(|form, window, cx| {
    let focus = form.name.focus_handle(cx);
    window.focus(&focus, cx);
});

window.simulate_input("Ada");
window.simulate_keystrokes("enter");

assert_eq!(window.read(|form, _| form.submitted.clone()), ["Ada"]);
assert_eq!(window.read(|form, cx| form.name.read(cx).as_str().to_string()), "");

// `emplace` is an edit, so the default Undo action restores the previous text.
window.simulate_keystrokes("secondary-z");
assert_eq!(window.read(|form, cx| form.name.read(cx).as_str().to_string()), "Ada");
```

The handler operates on `SignUp::name`, the entity used by the element.
Its final assertion exercises the undo consequence: clearing via `emplace`
leaves an edit that Undo can reverse.

## Style the field

Padding, borders, size and colors use the usual element styles. For the
editing parts, use these setters:

- `placeholder` supplies the empty-field hint; `placeholder_color` colors it.
- `caret_color`, `selection_color` and `caret_w` style the caret and selection.
- `caret_blink_interval_500ms` blinks the caret. Without an interval, the caret
  remains visible.
- `accepts_input(false)` makes the field read-only while leaving it focusable,
  so the user can still select and copy its text. Typing, paste, cut, delete,
  and, in a multi-line field, Enter and Tab do nothing until input is enabled
  again.

Set `whitespace_nowrap()` on a single-line field to keep its content on one
line. A multi-line area wraps by default; give it a height, or minimum and
maximum heights, and use `overflow_y_scroll()` when more text can fit than
the height allows.

## Use a multi-line area

Switch to `text_area` when the value can contain several lines. It accepts the
same setters and state as `text_input`; Enter inserts a newline and the arrow
keys move between lines. The [text input example](crate::_documentation::examples::text_input)
puts both controls in one form.

## Input methods

`EditableTextState` implements [`EntityInputHandler`](gpui::EntityInputHandler)
for input methods: it maintains marked text while the platform displays
candidates and eventually commits a choice. Composition can emit several
`TextChanged` events before that commit. Replacing an existing marked range
does not create a separate undo step for every update.

Subscribers must therefore accept intermediate text. A `TextChanged` event
alone is not evidence that composition has finished; submit through an
explicit action, and let validation feedback tolerate an unfinished value.

The default bindings map Cmd-Space on macOS and Ctrl-Space elsewhere to
`ShowCharacterPalette`, which asks the platform to open its character palette.
The host may reserve that shortcut before the app receives it.

## Implement your own text element

Use the supplied fields unless the editing behavior requires a custom one.
For a custom field, implement [`EntityInputHandler`](gpui::EntityInputHandler)
on the view and register it during paint with
[`ElementInputHandler`](gpui::ElementInputHandler). The window can then send
typed text and IME updates to that view.

The sketch below demonstrates registration and range conversion only. It
reports the caret at the end and ignores the selection requested within an
IME composition (`new_selected_range`). It is incomplete as an editor: a
working field must retain and honor the selection, including that argument.

The Rust string uses UTF-8 byte offsets; the platform input protocol uses
UTF-16 code units. Convert at the handler boundary with
[`utf8_to_utf16_offset`](gpui::utf8_to_utf16_offset) and
[`utf16_to_utf8_offset`](gpui::utf16_to_utf8_offset). The two units are not
interchangeable. In particular, `selected_text_range` and
`marked_text_range` return UTF-16 ranges, while `replace_text_in_range` and
`replace_and_mark_text_in_range` receive UTF-16 replacement ranges. Convert
those inputs before slicing `self.text`, and convert the model's byte positions
back before returning selections or marked ranges to the platform. The
`new_selected_range` argument is also UTF-16 units within `new_text`; it is not
a byte range into the model.

```rust
use std::ops::Range;

use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Pixels, Point,
    UTF16Selection, Window, canvas, div, prelude::*, utf8_to_utf16_offset, utf16_to_utf8_offset,
};

struct RawField {
    focus_handle: FocusHandle,
    text: String,
    marked: Option<Range<usize>>,
}

impl RawField {
    fn utf8_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        match range {
            Some(range) => {
                utf16_to_utf8_offset(&self.text, range.start)
                    ..utf16_to_utf8_offset(&self.text, range.end)
            }
            None => self.marked.clone().unwrap_or(self.text.len()..self.text.len()),
        }
    }
}

impl EntityInputHandler for RawField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        *adjusted_range = Some(range.clone());
        self.text.get(self.utf8_range(Some(range))).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = utf8_to_utf16_offset(&self.text, self.text.len());
        Some(UTF16Selection { range: end..end, reversed: false })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| {
            utf8_to_utf16_offset(&self.text, range.start)..utf8_to_utf16_offset(&self.text, range.end)
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.utf8_range(range);
        self.text.replace_range(range, text);
        self.marked = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.utf8_range(range);
        let start = range.start;
        self.text.replace_range(range, new_text);
        self.marked = Some(start..start + new_text.len());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(utf8_to_utf16_offset(&self.text, self.text.len()))
    }
}

impl Render for RawField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let focus_handle = self.focus_handle.clone();
        div().track_focus(&self.focus_handle).size_full().child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    window.handle_input(&focus_handle, ElementInputHandler::new(bounds, view), cx);
                },
            )
            .size_full(),
        )
    }
}
```

`handle_input` must run during paint on every frame, and the registered handler
is active only while its focus handle is focused. The window queries it for
text, selection and composition state, then calls its replacement methods to
insert text. See [`EntityInputHandler`](gpui::EntityInputHandler) for each
method's contract before extending this sketch.

## See also

- [Handle keyboard shortcuts](crate::_documentation::guides::keyboard) covers the
  bindings that Enter and the editing keys use.
- [Manage focus](crate::_documentation::guides::focus) covers how a field takes
  focus, and why Tab belongs to it.
- [Input and dispatch](crate::_documentation::concepts::input_dispatch) explains how
  typed text reaches the focused input handler.
- The [text input example](crate::_documentation::examples::text_input) and the
  [tutorial's text input chapter](crate::_documentation::tutorial) build a working
  form step by step.
