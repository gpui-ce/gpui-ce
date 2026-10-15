# Handle keyboard shortcuts

An editor can expose Save through a shortcut, a menu item and a button. Give
all three the same action. The shortcut then has two jobs to complete: its
binding selects an action from the current key contexts, and dispatch finds
a handler for that action along the focus path.

## Declare actions

Save and Undo need names but no payload, so declare them with
[`actions!`](crate::actions). Its namespace becomes part of each keymap name,
as in `editor::Save`. A Find command can carry a query instead: derive
[`Action`](crate::Action) on its data type, with `Clone` and `PartialEq`.
Field-bearing actions also need `serde::Deserialize` and
`schemars::JsonSchema` for construction from keymap JSON.
`#[action(no_json)]` removes those two bounds and disables JSON construction.

```standalone_crate
use gpui::{Action, actions};

actions!(keyboard_declare, [Save, Undo]);

#[derive(Clone, PartialEq, serde::Deserialize, schemars::JsonSchema, Action)]
#[action(namespace = keyboard_declare)]
pub struct Find {
    pub query: String,
}
```

The namespace matters when features come from different crates. Two Rust
types can both register the name `editor::Save`, and creating an app then
panics. Give independent action sets different namespaces so their names
remain unique across the app.

## Bind a key to an action

The binding below connects `secondary-s` to `Save`, provided the focus path
contains the `Editor` context. [`KeyBinding::new`] takes those three pieces—
keystroke, action and optional context predicate—and [`App::bind_keys`]
registers the result. Register it during startup, then attach the context
and listener to the editor:

```standalone_crate
use gpui::{Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(keyboard_example, [Save]);

struct Editor {
    focus_handle: FocusHandle,
    saves: usize,
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|editor, _: &Save, _window, _cx| editor.saves += 1))
    }
}

let mut app = TestApp::new();
app.update(|cx| cx.bind_keys([KeyBinding::new("secondary-s", Save, Some("Editor"))]));

let mut window = app.open_window(|window, cx| {
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    Editor { focus_handle, saves: 0 }
});

window.simulate_keystrokes("secondary-s");
assert_eq!(window.read(|editor, _| editor.saves), 1);
```

The assertion checks the entire route: focusing the editor makes its
`key_context("Editor")` available to the predicate, the key selects `Save`,
and [`on_action`](crate::InteractiveElement::on_action) receives it.
`cx.listener` supplies mutable access to the owning view, where the callback
increments `saves`. The listener also covers focused descendants unless an
earlier handler consumes the action.

[`track_focus`](crate::InteractiveElement::track_focus) attaches the handle;
[`Window::focus`] selects it. The [focus guide](crate::_documentation::guides::focus)
explains that setup, and [input dispatch](crate::_documentation::concepts::input_dispatch)
explains how GPUI ranks matching bindings and walks the handler path.

## Write keystrokes

Write a keystroke as modifiers followed by a key, separated by dashes:
`ctrl-shift-z`, for example. The key can stand alone. These are the accepted
modifier names:

| Written as | Meaning |
| --- | --- |
| `ctrl` | Control |
| `alt` | Option on macOS, Alt elsewhere |
| `shift` | Shift |
| `fn` | The function modifier |
| `cmd` | The platform modifier: Command on macOS, Super or Windows elsewhere. `super` and `win` are aliases. |
| `secondary` | `cmd` on macOS, `ctrl` elsewhere. Use it for shortcuts such as save and copy. |

Keys are names such as `enter`, `escape`, `tab`, `space`, `backspace`, `left`, or
a single character. An uppercase letter means Shift plus that letter, so `S` is
`shift-s`; other names are case-insensitive.

```standalone_crate
use gpui::Keystroke;

let save = Keystroke::parse("secondary-s").unwrap();
assert!(save.modifiers.secondary());

let capital = Keystroke::parse("S").unwrap();
assert!(capital.modifiers.shift);
assert_eq!(capital.key, "s");

assert!(Keystroke::parse("ctrl-shift-z").is_ok());
assert!(Keystroke::parse("hyper-z").is_err());
```

For Save and Copy, `secondary` follows the platform convention without
conditional compilation. Use an explicit `cmd` or `ctrl` when that modifier
is part of the command's intended meaning. Invalid keystroke and context
strings make `KeyBinding::new` panic; parsing with `Keystroke::parse` as above
lets you inspect an error instead.

## Bind a sequence of keys

Spaces separate keystrokes in a sequence. This command runs only after both
keys arrive:

```standalone_crate
use gpui::{KeyBinding, actions};

actions!(keyboard_sequence, [OpenPalette]);

fn bindings() -> KeyBinding {
    KeyBinding::new("secondary-k secondary-o", OpenPalette, Some("Workspace"))
}
```

After registration with [`App::bind_keys`], `secondary-k` waits in a buffer
for `secondary-o`. That wait needs a limit if the prefix could also be a
complete shortcut or text for an accepting input handler: GPUI starts a
one-second timer, restarting it for further prefix keys. Expiry flushes and
replays the buffered input, ending the chance to finish the longer shortcut.
A prefix with neither alternative can wait without a timer. Changing or
clearing focus discards the pending keys.

## Use key contexts

[`key_context`](crate::InteractiveElement::key_context) declares names and
values on an element. The focused element and its ancestors supply a stack of
contexts. GPUI tests a binding's predicate at each depth: the deepest match
wins, and the last registered binding breaks a tie. A binding without a
predicate is treated as matching at the deepest level.

Ordinary predicate terms inspect the context at the tested depth. `>` relates a
parent context to a descendant. Outside that operator, `!` checks all contexts
on the focus path; on the right side of `>`, negation checks only the
descendant slice. Thus `Panel > !ReadOnly` can match a writable child even if
`ReadOnly` appears above `Panel`. A predicate needs at least one context; use
`None` for an unconditional binding when the path may have no contexts.

| Predicate | Matches when |
| --- | --- |
| `Editor` | A context on the focus path declares `Editor`. |
| `mode == insert` | The context sets `mode` to `insert`. |
| `mode != insert` | `mode` has another value or is absent from the context. |
| `!ReadOnly` | No context on the focus path is named `ReadOnly`. |
| `Editor && mode == insert` | The same context declares `Editor` and `mode = insert`. |
| `Editor \|\| Preview` | Either holds. |
| `Panel > Editor` | An `Editor` context sits below a `Panel` context. |

Precedence from loosest to tightest is `>`, `||`, `&&`, `==` and `!=`, then `!`.
Add parentheses when you mix `>` with other operators.

```standalone_crate
use gpui::{KeyBindingContextPredicate, KeyContext};

let editor = KeyContext::parse("Editor mode = insert").unwrap();

let insert = KeyBindingContextPredicate::parse("Editor && mode == insert && !ReadOnly").unwrap();
assert!(insert.eval(&[editor.clone()]));

let not_insert = KeyBindingContextPredicate::parse("Editor && mode != insert").unwrap();
assert!(!not_insert.eval(&[editor]));

let nested = KeyBindingContextPredicate::parse("Panel > Editor").unwrap();
let panel = KeyContext::parse("Panel").unwrap();
let field = KeyContext::parse("Editor").unwrap();
assert!(nested.eval(&[panel, field]));
```

`Editor && mode == insert` requires both entries in one context, as above. If
the editor name is on a parent and the mode is on a child, use
`Editor > (mode == insert)` to express that relationship. A form and a
multiline editor can then give Enter different meanings through their local
contexts.

## Show the shortcut for an action

A tooltip for Save should reflect the binding the user can invoke now. Ask
[`Window::keystroke_text_for`] for the highest-precedence binding on the
current focus path; it formats the answer for the platform.
[`Window::bindings_for_action`] supplies the complete set when one shortcut
is not enough.

```standalone_crate
use gpui::{Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(keyboard_display, [Save]);

struct Editor {
    focus_handle: FocusHandle,
}

impl Render for Editor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().key_context("Editor").track_focus(&self.focus_handle)
    }
}

let mut app = TestApp::new();
app.update(|cx| cx.bind_keys([KeyBinding::new("secondary-s", Save, Some("Editor"))]));

let mut window = app.open_window(|window, cx| {
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    Editor { focus_handle }
});

let shortcut = window.update(|_, window, _| window.keystroke_text_for(&Save));
assert!(shortcut.ends_with('S'));
```

The result is already platform-formatted (`⌘S` on macOS and `ctrl-S` elsewhere),
so display it without parsing. The lookup uses the last rendered frame; before
the first frame, `keystroke_text_for` returns the action name instead.

## Handle app-wide actions

Register commands such as Quit or Open Settings with [`App::on_action`]. With
an active window, app handlers run during bubbling after the element handlers,
if propagation reaches them. Each action handler consumes the action unless it
calls [`App::propagate`]. If every handler declines it, GPUI tries the next
matching binding for that keystroke. This lets a single-line input pass Enter
to its form's Submit binding, or Tab to a navigation binding.

Without an active window, [`App::dispatch_action`] calls app handlers in
reverse registration order. That dispatch continues by default until a
handler calls [`App::stop_propagation`]; it has no element path to walk.

```standalone_crate
use std::{cell::Cell, rc::Rc};

use gpui::{App, Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(keyboard_app, [Quit]);

struct Root {
    focus_handle: FocusHandle,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle)
    }
}

let quits = Rc::new(Cell::new(0));
let mut app = TestApp::new();
app.update(|cx| {
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    let quits = quits.clone();
    cx.on_action(move |_: &Quit, _: &mut App| quits.set(quits.get() + 1));
});

let mut window = app.open_window(|window, cx| {
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    Root { focus_handle }
});

window.simulate_keystrokes("secondary-q");
assert_eq!(quits.get(), 1);
```

## Handle raw key events

Raw listeners suit input where the key press matters, such as movement in a
canvas or game. [`on_key_down`](crate::InteractiveElement::on_key_down) runs for
presses left unhandled by key bindings.
[`on_key_up`](crate::InteractiveElement::on_key_up) receives the separate raw
release event; GPUI does not perform another binding lookup for key-up. Both
listeners follow the focus path, and the key is available as
`event.keystroke`.

```standalone_crate
use gpui::{FocusHandle, KeyDownEvent, div, prelude::*};

fn pad(focus: &FocusHandle) -> impl IntoElement {
    div().track_focus(focus).on_key_down(|event: &KeyDownEvent, _window, _cx| {
        if event.keystroke.key == "escape" {
            // Close the pad here.
        }
    })
}
```

For a named command, prefer an action: users can rebind it, and menus or
tooltips can look up its current shortcut.
For typed text and IME composition, use the
[text input API](crate::_documentation::guides::text_input) rather than
constructing text from key-down events.

## See also

- [Input and dispatch](crate::_documentation::concepts::input_dispatch) explains
  how GPUI picks the handler for a keystroke.
- [Manage focus](crate::_documentation::guides::focus) covers the focus path that
  actions travel.
- [Accept text input](crate::_documentation::guides::text_input) covers typed text
  and the default editing bindings.
- The [keyboard example](crate::_documentation::examples::keyboard) shows these
  pieces in a running app. Run it with `cargo run --example keyboard`.
