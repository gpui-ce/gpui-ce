# Manage focus

Focus tells a window which control owns keyboard input. GPUI records that
control as one selected handle, or `None`. Switching to another window does
not erase the selection: an editor can keep its caret while its window is
inactive. Check [`Window::is_window_active`] for OS activation, which governs
whether the window receives keys.

## Create and track a focus handle

Keep a [`FocusHandle`] in the view, created with [`App::focus_handle`]. That
handle identifies the control across renders; allocating another on each
render would leave the previous selection pointing to an old identity.
[`track_focus`](crate::InteractiveElement::track_focus) places it on the
element's dispatch node during `render`:

```standalone_crate
use gpui::{Context, FocusHandle, TestApp, Window, div, prelude::*};

struct Search {
    focus_handle: FocusHandle,
}

impl Render for Search {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle).child("Search")
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, cx| Search { focus_handle: cx.focus_handle() });

window.update(|search, window, cx| window.focus(&search.focus_handle, cx));
assert!(window.update(|search, window, _| search.focus_handle.is_focused(window)));
```

The enclosing elements become its dispatch ancestors. An editor can handle
Save when a child field owns the caret because the editor lies on that path.
[Input dispatch](crate::_documentation::concepts::input_dispatch) describes
the capture and bubble phases.

## Focus an element

Select a handle with [`Window::focus`] or [`FocusHandle::focus`], and clear
focus with [`Window::blur`]. These operations take `cx` to update window and
app state. Selection changes immediately; the next draw applies focus styles.
For example, a constructor may choose the Search handle before its first
render. `is_focused` can already return `true`, yet there is no rendered
Search node to receive a keyboard action until drawing records it and its
ancestors.

After `blur`, another call to `focus` can select Search again.
[`Window::disable_focus`] closes that route: it clears selection and makes
later `focus`, `focus_next` and `focus_prev` calls do nothing. There is no
matching enable operation. Use it for a display-only window or teardown.

If a view implements [`Focusable`], you can focus it without knowing its handle.
An `Entity<V>` implements `Focusable` when `V` does, so `view.focus_handle(cx)`
works on the entity you hold.

```standalone_crate
use gpui::{App, Context, FocusHandle, Focusable, TestApp, Window, div, prelude::*};

struct Search {
    focus_handle: FocusHandle,
}

impl Focusable for Search {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Search {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle)
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, cx| Search { focus_handle: cx.focus_handle() });

window.update(|search, window, cx| window.focus(&search.focus_handle(cx), cx));
assert!(window.update(|search, window, _| search.focus_handle.is_focused(window)));

window.update(|_, window, cx| window.blur(cx));
assert!(!window.update(|search, window, _| search.focus_handle.is_focused(window)));
```

## Ask what has focus

| Query | Result |
| --- | --- |
| [`FocusHandle::is_focused`] | Whether this handle is selected |
| [`FocusHandle::contains_focused`] | Whether this handle or a descendant is selected |
| [`FocusHandle::within_focused`] | Whether this handle or an ancestor is selected |
| [`Window::focused`] | The selected handle, or `None` |

For an editor containing two fields, a toolbar may need to stay visible while
either field has focus. Query
`contains_focused` on the editor's handle. That query needs `cx` and the last
rendered ancestry; `is_focused` instead compares the selected handle directly.
The selected leaf can change before the ancestry is redrawn. Query from
`render`, an event handler, or [`TestAppWindow::update`](crate::TestAppWindow::update),
where the window is available.

## Style focused elements

A style closure chooses appearance without changing the focus selection.
These methods require a tracked handle:

- [`focus`](crate::InteractiveElement::focus) applies when the element itself is
  focused.
- [`in_focus`](crate::InteractiveElement::in_focus) applies when the element or
  an ancestor is focused, matching `within_focused`. To highlight a parent
  panel while a child has focus, query `contains_focused` during `render` and
  choose the panel style from that result.
- [`focus_visible`](crate::InteractiveElement::focus_visible) applies when the
  element is focused and the last input was from the keyboard. Use it for focus
  rings that shouldn't appear after a mouse click.

```rust
# use gpui::{Context, FocusHandle, Window, div, prelude::*, rgb};
# struct Field { focus_handle: FocusHandle }
# impl Render for Field {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
div()
    .track_focus(&self.focus_handle)
    .border_2()
    .border_color(rgb(0x3b3b3b))
    .focus(|style| style.border_color(rgb(0x3b82f6)))
    .focus_visible(|style| style.bg(rgb(0x1e3a8a)))
    .child("Name")
#     }
# }
```

## React to focus changes

Use a style closure for appearance alone. For state changes, register a
listener through the view's [`Context`](crate::Context):

| Listener | Runs when |
| --- | --- |
| [`Context::on_focus`] | The handle becomes the focused leaf |
| [`Context::on_blur`] | It stops being the focused leaf, including when focus moves into its child |
| [`Context::on_focus_in`], [`Context::on_focus_out`] | Focus enters or leaves the handle's subtree |
| [`Context::on_focus_lost`] | The rendered focus path disappears, for example when its element is removed; a fallback can be selected here |

Consider an editor with two fields. Moving from the editor's own handle into
the first field blurs the editor as a leaf, but focus remains inside it:
`on_blur` runs and `on_focus_out` does not. Moving from that field to the
second leaves the editor's subtree membership unchanged, so neither
`on_focus_in` nor `on_focus_out` runs for the editor. Use those subtree
listeners for behavior such as showing a toolbar whenever any field is active.

Retain the returned [`Subscription`](crate::Subscription) for as long as the
listener should run. For callbacks independent of a view, use
[`Window::on_focus_in`] or [`Window::on_focus_out`].

Drawing compares the previous and current focus paths to deliver these
callbacks. Leaf and subtree listeners treat an inactive window as having an
empty path: deactivation can produce blur/out events and activation focus/in
events even though the selected handle remains the same. `on_focus_lost`
instead reports the disappearance of the rendered path, independent of OS
activation.

A test window starts inactive. The example activates it, retains both
subscriptions, changes selection, and draws before checking the callbacks.

```standalone_crate
use gpui::{Context, FocusHandle, Subscription, TestApp, Window, div, prelude::*};

struct Field {
    focus_handle: FocusHandle,
    events: Vec<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl Render for Field {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus_handle)
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, cx| Field {
    focus_handle: cx.focus_handle(),
    events: Vec::new(),
    _subscriptions: Vec::new(),
});
window.activate();

window.update(|field, window, cx| {
    let handle = field.focus_handle.clone();
    let focus = cx.on_focus(&handle, window, |field, _, _| field.events.push("focus"));
    let blur = cx.on_blur(&handle, window, |field, _, _| field.events.push("blur"));
    field._subscriptions.extend([focus, blur]);
    window.focus(&handle, cx);
});
window.draw();
assert_eq!(window.read(|field, _| field.events.clone()), ["focus"]);

window.update(|_, window, cx| window.blur(cx));
window.draw();
assert_eq!(window.read(|field, _| field.events.clone()), ["focus", "blur"]);
```

## Set the tab order

A handle needs [`FocusHandle::tab_stop`] set to `true` to participate in tab
navigation. Its [`tab_index`](FocusHandle::tab_index) then determines order,
lowest first. An index alone does not enable the stop. Conversely,
`tab_stop(false)` only removes the handle from tab traversal: code can still
focus it directly. Use that distinction for a control reached by another
navigation scheme, rather than treating it as disabled.

The order follows nested groups. All stops in a group with index 1 precede
those in a sibling group with index 2, even if a child in group 2 has a lower
index. GPUI compares the complete parent-and-child index path, then uses paint
order for equal paths. `focus_next` and `focus_prev` read this map from the
last rendered frame.

Set both on the handle you pass to `track_focus`. The
[`InteractiveElement::tab_index`](crate::InteractiveElement::tab_index) method only
applies to elements that create their own handle, which is an element with no
`track_focus` call. If you use `track_focus`, the index belongs on the handle:

```rust
# use gpui::{App, FocusHandle};
fn field(cx: &App, order: isize) -> FocusHandle {
    cx.focus_handle().tab_index(order).tab_stop(true)
}
```

Choose indices for the navigation sequence; a field's screen position has no
say in it. Adding, removing or regrouping fields changes the map only after a
draw, so redraw before navigating the changed form.

## Move focus with Tab

Choose the Tab behavior for your form; GPUI does not assign a default binding.
Bind Tab and Shift-Tab to actions, then call
[`Window::focus_next`] or [`Window::focus_prev`] from the handlers. The form
below puts three fields in the tab map, scopes the actions to `Form`, and wraps
at either end. With no selected handle, `focus_next` starts at the first stop
and `focus_prev` at the last.

```standalone_crate
use gpui::{Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(focus_tab, [NextField, PreviousField]);

struct Form {
    focus_handle: FocusHandle,
    fields: Vec<FocusHandle>,
}

impl Render for Form {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Form")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &NextField, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &PreviousField, window, cx| window.focus_prev(cx)))
            .children(self.fields.iter().map(|handle| {
                div().track_focus(handle).child("Field")
            }))
    }
}

let mut app = TestApp::new();
app.update(|cx| {
    cx.bind_keys([
        KeyBinding::new("tab", NextField, Some("Form")),
        KeyBinding::new("shift-tab", PreviousField, Some("Form")),
    ]);
});

let mut window = app.open_window(|_, cx| Form {
    focus_handle: cx.focus_handle(),
    fields: (0..3).map(|order| cx.focus_handle().tab_index(order).tab_stop(true)).collect(),
});

let focused_field = |window: &mut gpui::TestAppWindow<Form>| {
    window.update(|form, window, _| form.fields.iter().position(|field| field.is_focused(window)))
};

window.update(|form, window, cx| window.focus(&form.focus_handle, cx));

window.simulate_keystrokes("tab");
assert_eq!(focused_field(&mut window), Some(0));

window.simulate_keystrokes("tab tab");
assert_eq!(focused_field(&mut window), Some(2));

window.simulate_keystrokes("tab");
assert_eq!(focused_field(&mut window), Some(0));

window.simulate_keystrokes("shift-tab");
assert_eq!(focused_field(&mut window), Some(2));
```

A multiline text field consumes Tab for its editing binding, inserting a tab
character before the form can see the key. A single-line field propagates Tab,
so the form binding runs. Give multiline editors another navigation key, or
provide a button that calls `focus_next`.

## Restore focus after a prompt

With a custom prompt builder, `Window::prompt` moves focus into the prompt and
records the previously focused handle. When the user answers, GPUI restores
that handle if it still exists; otherwise the window is blurred. Opening a
newer prompt supersedes the older one, so an answer from the older prompt is
ignored. Native dialogs follow the platform's own focus behavior.

## See also

- [Handle keyboard shortcuts](crate::_documentation::guides::keyboard) covers the
  actions that Tab and Shift-Tab are bound to.
- [Respond to the mouse and touch](crate::_documentation::guides::mouse_and_touch)
  covers clicks, which also move focus to an element that tracks a handle.
- [Accept text input](crate::_documentation::guides::text_input) covers the text
  fields that keep Tab for themselves.
- [Input and dispatch](crate::_documentation::concepts::input_dispatch) explains the
  focus path that keys follow.
