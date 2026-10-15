# Input and dispatch

A floating panel and an editor receive input for different reasons. The panel
receives a pointer event when its hitbox contains the pointer; the editor
receives a keyboard command when it lies on the focused element's ancestry.
Painting the panel over the editor changes pointer precedence without moving
keyboard focus.

GPUI records hitboxes and dispatch paths while drawing a frame, then consults
those records for input. Key bindings translate keystrokes into typed
[actions](Action), so a Save operation can be reached from a shortcut or a
menu. Text entry uses a further protocol for input methods and composition.

## Mouse events

An interactive element inserts its [`Hitbox`] during prepaint and registers
mouse listeners during [paint](crate::_documentation::concepts::the_frame).
For a later move or click, GPUI tests the pointer against those hitboxes,
rather than against the painted primitives. Listener order follows two passes:

1. **Capture**, from the back of the window to the front, in the order the
   elements were painted. Use [`capture_any_mouse_down`](InteractiveElement::capture_any_mouse_down)
   and friends for this pass. It is for special cases, such as closing a popover
   when the user clicks outside it.
2. **Bubble**, from the front to the back. Ordinary handlers such as
   [`on_mouse_down`](InteractiveElement::on_mouse_down) and
   [`on_click`](StatefulInteractiveElement::on_click) run here, so the element
   on top gets the first chance to respond.

Ordinary listeners require a matching hitbox. An outside-click listener such
as [`on_mouse_down_out`](InteractiveElement::on_mouse_down_out) deliberately
tests the opposite condition. In the bubble pass, [`App::stop_propagation`]
prevents later listeners behind the current element from receiving the event.
Here the small front box stops its click; a click elsewhere reaches the back:

```
# use gpui::{Context, MouseButton, TestApp, Window, div, point, prelude::*, px};
struct Layers {
    clicked: Vec<&'static str>,
}

impl Render for Layers {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_mouse_down(MouseButton::Left, cx.listener(|layers, _, _, _| {
                layers.clicked.push("back");
            }))
            .child(
                div()
                    .size(px(50.))
                    .on_mouse_down(MouseButton::Left, cx.listener(|layers, _, _, cx| {
                        layers.clicked.push("front");
                        cx.stop_propagation();
                    })),
            )
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Layers { clicked: Vec::new() });

window.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left);
window.simulate_mouse_down(point(px(200.), px(200.)), MouseButton::Left);
assert_eq!(window.read(|layers, _| layers.clicked.clone()), ["front", "back"]);
```

An opaque-looking panel is not automatically a mouse barrier. Apply
[`occlude`](InteractiveElement::occlude) when its hitbox should block elements
underneath; painted pixels alone do not establish that rule.

## Focus

A window can have one focused element or none. An element becomes
focusable when it tracks a [`FocusHandle`], which you create with
[`App::focus_handle`] and attach with
[`track_focus`](InteractiveElement::track_focus). Focus moves when the user
clicks a focusable element or when your code calls [`Window::focus`].
[`Window::focus_next`] and [`Window::focus_prev`] move through the elements
that have a [`tab_index`](InteractiveElement::tab_index); GPUI does not bind
Tab to them for you, so bind it in your app.

The focused handle identifies one target; the *focus path* connects that
target to the window root through the rendered dispatch tree. An ancestor
*contains* focus. This is why a panel can handle Save while a text field inside
it owns the caret: both lie on the same path.
See the
[focus guide](crate::_documentation::guides::focus) for tab order and focus
styles.

## From keystrokes to actions

Declare the operation before choosing its shortcut. These actions can be
dispatched by code as well as by the two bindings:

```standalone_crate
use gpui::{App, KeyBinding, actions};

actions!(editor, [Save, Undo]);

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, Some("Editor")),
        KeyBinding::new("cmd-z", Undo, Some("Editor")),
    ]);
}
```

The third argument restricts a binding to a *key context*, declared by an
element with [`key_context`](InteractiveElement::key_context). GPUI matches
contexts along the focus path. An Enter binding in a form can therefore submit
the value, while an Enter binding in an editor can insert a newline.

For a pressed key, GPUI resolves and dispatches in this order:

1. Looks up bindings for the keystroke whose context matches the focus path.
   A context deeper in the path wins over a shallower one. At the same depth,
   the binding added last wins. A binding without a context matches at the
   deepest level.
2. Dispatches the winning binding's action along the focus path. Handlers
   registered with [`on_action`](InteractiveElement::on_action) on elements in
   the path receive it.
3. If no action handles the press, offers the raw key-down event to
   [`on_key_down`](InteractiveElement::on_key_down) listeners along the path.

Key release is separate: [`on_key_up`](InteractiveElement::on_key_up) receives
the release event without resolving another binding. On a platform event that
prefers character input, an accepting text handler can also take precedence
over bindings, for example for AltGr text entry.

Action dispatch walks toward the target for capture, then back toward the root
for bubble. Register a capture listener with
[`capture_action`](InteractiveElement::capture_action), or a bubble listener
with [`on_action`](InteractiveElement::on_action). A bubble listener consumes
the action by default. Calling [`App::propagate`] declines it and allows
dispatch to continue, including trying the next binding for the keystroke.

```standalone_crate
use gpui::{Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(editor, [Save]);

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
app.update(|cx| cx.bind_keys([KeyBinding::new("ctrl-s", Save, Some("Editor"))]));

let mut window = app.open_window(|window, cx| {
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    Editor { focus_handle, saves: 0 }
});

window.simulate_keystrokes("ctrl-s");
assert_eq!(window.read(|editor, _| editor.saves), 1);
```

Bindings can also be sequences, such as `"cmd-k cmd-s"`. GPUI buffers a
matching prefix, with a one-second timeout when that input could also resolve
a complete binding or enter text through an accepting input handler. Timeout
flushes the buffered input. A pure prefix with neither alternative can remain
pending without a timer; changing or clearing focus discards it. See the
[keyboard guide](crate::_documentation::guides::keyboard#bind-a-sequence-of-keys)
for the binding syntax.

The handler above only knows about Save. A menu item, command palette or button
can dispatch the same action, and a settings file can replace the shortcut
without replacing that handler. [`Window::bindings_for_action`] retrieves the
current bindings for display in a menu or tooltip.

## Application-wide actions

Some actions do not belong to an element: Quit, Open Settings, and New Window.
Register those on the app with [`App::on_action`]. These handlers run in the
bubble phase after the element handlers, if propagation reaches them. They do
not run during capture. In a window's bubble pass, an app handler stops the
action unless it calls [`App::propagate`].

[`App::dispatch_action`] also works without an active window. It then calls app
handlers in reverse registration order, without an element path. In this case,
handlers continue by default until one calls [`App::stop_propagation`]. This
allows commands such as Quit to work with no window open.

## Text input

Typing text is a separate protocol from pressing shortcuts. Input methods may
compose characters across several keystrokes, and some keyboards produce text
that no single key represents. Text accepting elements, including the editable
elements in `gpui_ce_elements`, install an input handler that the platform talks
to directly. Text arrives through that protocol; an unhandled key event does
not necessarily produce text. See the [text input
guide](crate::_documentation::guides::text_input).
