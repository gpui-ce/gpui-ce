# Accept text input

The `gpui_ce_elements` crate provides `editable_text::text_input`
for single-line fields and `editable_text::text_area`
for multi-line editors. Both render an `EditableTextElement` backed by an
`EditableTextState` entity that owns the text buffer, selection, undo history,
IME composition range, and [`FocusHandle`](gpui::FocusHandle).

## Hold state and install editing bindings

Create `EditableTextState` once when constructing your view and pass a weak
handle (`state.downgrade()`) plus a stable element ID to `text_input`:

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

If surrounding code never reads or mutates the buffer directly, omit `.state(...)`
(or call `EditableTextState::use_keyed` / `use_keyed_init` during `render`) so
the window retains the state entity under the element's ID for as long as that
ID remains mounted. `StringStorage` wraps a `String` for everyday forms;
implement `UnicodeTextStorage` over a rope when editing large documents so
`len_utf16` does not scan the full string.

Typed characters arrive through the platform input handler automatically, while
Backspace, Delete, arrow navigation, selection, clipboard commands, and
Undo/Redo dispatch as GPUI actions scoped to `DEFAULT_INPUT_CONTEXT`
(`"EditableText"`). Register the default editing keymap once at startup before
opening windows:

```rust
# use gpui::App;
use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};

# fn setup(cx: &mut App) {
cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
# }
```

Always pass `Some(DEFAULT_INPUT_CONTEXT)` so editing bindings activate only
while a text field lies on the focus path (`secondary` in those bindings maps to
`Cmd` on macOS and `Ctrl` elsewhere).

## Read, replace, subscribe, and submit

Read the current string with `EditableTextState::as_str`. Replace it from
application code with `EditableTextState::emplace`, which swaps the buffer,
moves the caret to the end, pushes an undo entry, emits `TextChanged`, and
notifies the entity (bypassing `accepts_input(false)` and single-line newline
stripping). Programmatic caret and selection methods—`move_to`, `select_to`,
`select_document`, and `select_linear`—take **UTF-8 byte offsets** into
`as_str`, clamping offsets past the end.

Subscribe to `TextChanged` with `cx.subscribe` and store the
[`Subscription`](gpui::Subscription) on your view. Because `TextChanged` fires
after each edit (including intermediate IME composition steps) and carries no
payload, read `as_str` inside the callback and guard against recursion if you
call `emplace` in response.

On a single-line `text_input`, both `Enter` and `Tab` call `cx.propagate()`,
letting an enclosing form handle submission and focus traversal. (`Escape`
collapses the selection and blurs the field.) On a multi-line `text_area`,
`Enter` inserts `\n`, `Tab` inserts `\t`, and `Up`/`Down` move across wrapped
visual lines:

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

window.simulate_keystrokes("secondary-z");
assert_eq!(window.read(|form, cx| form.name.read(cx).as_str().to_string()), "Ada");
```

## Field styling, read-only mode, and IME composition

Alongside [`Styled`](gpui::Styled) box and typography methods, editable fields
expose field-specific setters:

* `placeholder` and `placeholder_color` style the hint shown when storage is
  empty.
* `caret_color`, `caret_w` (default `2px`), `caret_h` (default `relative(1.)` of
  line height), and `caret_blink_interval_500ms` configure the cursor (blinking
  is off until an interval is set).
* `selection_color` and `marked_color` color the selection highlight and the
  underline drawn beneath active IME pre-edit text.
* `accepts_input(false)` makes the field read-only: typing, paste, cut, delete,
  pointer focus/selection, and IME registration are disabled, while programmatic
  focus (`window.focus`), arrow navigation, Copy, Undo/Redo, and single-line
  `Enter`/`Tab` propagation remain active.

Neither `text_input` nor `text_area` sets overflow clipping automatically: add
`whitespace_nowrap().overflow_x_scroll()` on single-line inputs that can exceed
their width, and pair `text_area` with a bounded height and
`overflow_y_scroll()`.

During IME composition (such as Japanese kana-to-kanji or dead-key accents),
the platform calls `replace_and_mark_text_in_range` for each pre-edit step and
finishes with `replace_text_in_range` or `unmark_text`. `EditableTextState`
suppresses per-keystroke undo entries while a marked range is active so
committing a composition records cleanly on the undo stack.

## Implement a custom `EntityInputHandler`

When building an editor that cannot use `EditableTextState`, implement
[`EntityInputHandler`](gpui::EntityInputHandler) on your view and register an
[`ElementInputHandler`](gpui::ElementInputHandler) with
[`Window::handle_input`](gpui::Window::handle_input) on every paint frame while
focused.

Every offset and range in [`EntityInputHandler`](gpui::EntityInputHandler) is
measured in **UTF-16 code units** to match macOS, Windows, and web IME APIs,
whereas Rust strings slice on **UTF-8 byte offsets**. Convert at the boundary
with [`utf16_to_utf8_offset`](gpui::utf16_to_utf8_offset) and
[`utf8_to_utf16_offset`](gpui::utf8_to_utf16_offset). Note two protocol rules:

* When `range` is `None` in `replace_text_in_range` or
  `replace_and_mark_text_in_range`, replace the current `marked` range if one
  exists, or the current `selection` otherwise.
* In `replace_and_mark_text_in_range`, `new_selected_range` is measured in
  UTF-16 code units *relative to `new_text`*, not from the start of the
  document—convert it against `new_text` and add the replacement's start byte.

```rust
use std::ops::Range;

use gpui::{
    Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Pixels, Point,
    UTF16Selection, Window, canvas, div, prelude::*, utf8_to_utf16_offset, utf16_to_utf8_offset,
};

struct RawField {
    focus_handle: FocusHandle,
    text: String,
    selection: Range<usize>,
    marked: Option<Range<usize>>,
}

impl RawField {
    fn to_utf8_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        match range {
            Some(r) => utf16_to_utf8_offset(&self.text, r.start)..utf16_to_utf8_offset(&self.text, r.end),
            None => self.marked.clone().unwrap_or_else(|| self.selection.clone()),
        }
    }

    fn to_utf16_range(&self, range: &Range<usize>) -> Range<usize> {
        utf8_to_utf16_offset(&self.text, range.start)..utf8_to_utf16_offset(&self.text, range.end)
    }
}

impl EntityInputHandler for RawField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.to_utf8_range(Some(range_utf16));
        *adjusted_range = Some(self.to_utf16_range(&range));
        self.text.get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.to_utf16_range(&self.selection),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.to_utf16_range(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.to_utf8_range(range_utf16);
        let caret = range.start + text.len();
        self.text.replace_range(range, text);
        self.selection = caret..caret;
        self.marked = None;
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
        let range = self.to_utf8_range(range_utf16);
        let start = range.start;
        self.text.replace_range(range, new_text);
        self.marked = (!new_text.is_empty()).then_some(start..start + new_text.len());
        self.selection = match new_selected_range_utf16 {
            Some(sel) => {
                (start + utf16_to_utf8_offset(new_text, sel.start))
                    ..(start + utf16_to_utf8_offset(new_text, sel.end))
            }
            None => (start + new_text.len())..(start + new_text.len()),
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
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
        Some(utf8_to_utf16_offset(&self.text, self.selection.end))
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

In a full custom field, `bounds_for_range` and `character_index_for_point` query
your [`TextLayout`](gpui::TextLayout) so IME candidate windows anchor to the
caret and pointer clicks place the cursor accurately.

## See also

* [Handle keyboard shortcuts](crate::_documentation::guides::keyboard) — key
  contexts, bindings, and action propagation.
* [Manage focus](crate::_documentation::guides::focus) — `FocusHandle`s and
  `Tab` traversal across form fields.
* [Text input example](crate::_documentation::examples::text_input) — a runnable
  form combining `text_input` and `text_area`.
