# Manage focus

Each window tracks at most one focused [`FocusId`](crate::FocusId), which
determines where keystrokes and keyboard actions dispatch. Application code
works with [`FocusHandle`](crate::FocusHandle), a reference-counted handle to
that identity: allocate one with [`cx.focus_handle()`](crate::App::focus_handle)
when constructing a view, store it on the view, and attach it during `render`
with [`track_focus`](crate::InteractiveElement::track_focus).

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

`track_focus` binds the handle to the rendered dispatch node. When that handle
is focused, every enclosing element in the rendered tree lies on the focus path,
allowing an outer workspace or form to handle actions like `Save` while an inner
field holds the caret; see [input dispatch](crate::_documentation::concepts::input_dispatch).
Left-clicking an element with a tracked handle also moves focus to it unless a
handler calls `cx.prevent_default()`.

If a component only needs focus styling or local keyboard dispatch without
exposing a handle to its parent, call [`focusable`](crate::StatefulInteractiveElement::focusable)
on a stateful element (`div().id("item").focusable()`), and GPUI retains a
handle in element state across frames. Use [`FocusHandle::downgrade`] when
holding a [`WeakFocusHandle`](crate::WeakFocusHandle) that must not keep the
focus identity alive.

## Focus and blur from code

[`window.focus(&handle, cx)`](crate::Window::focus) and
[`handle.focus(window, cx)`](crate::FocusHandle::focus) both select the handle
immediately, while [`Window::blur`] clears the window's selection. Focus styles
and the action dispatch path update on the next draw, so calling `window.focus`
in a constructor makes `is_focused` return `true` immediately even before the
first frame renders the element. ([`Window::disable_focus`] permanently disables
focus changes on utility or overlay windows that must never take focus.)

Implement [`Focusable`](crate::Focusable) on a view so parents and [`VisualContext::focus`]
can focus it without reaching into its fields. Because [`Entity<V>`](crate::Entity)
implements `Focusable` whenever `V` does, `entity.focus_handle(cx)` reads the
entity to clone its handle—call it from outside an active `entity.update(...)`
closure, and use `self.focus_handle` directly inside the view's own methods:

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

let search = window.root();
let handle = app.read(|cx| search.focus_handle(cx));
window.update(|_, window, cx| window.focus(&handle, cx));
assert!(window.update(|search, window, _| search.focus_handle.is_focused(window)));

window.update(|_, window, cx| window.blur(cx));
assert!(!window.update(|search, window, _| search.focus_handle.is_focused(window)));
```

## Query and style focus

| Query | Returns |
| --- | --- |
| [`FocusHandle::is_focused`] | `true` when this exact handle is selected. |
| [`FocusHandle::contains_focused`] | `true` when this handle or a rendered descendant is selected. |
| [`FocusHandle::within_focused`] | `true` when this handle or a rendered ancestor is selected. |
| [`Window::focused`] | The currently selected `FocusHandle`, if a strong handle still exists. |

`is_focused` checks the selected ID directly. `contains_focused`,
`within_focused`, and [`FocusHandle::contains`] consult the dispatch tree from
the most recently drawn frame, so call them during `render`, in event handlers,
or after [`TestAppWindow::draw`](crate::TestAppWindow::draw).

Elements that track a handle can style focus states declaratively:

* [`focus_style`](crate::InteractiveElement::focus_style) applies when the element itself is
  focused.
* [`in_focus_style`](crate::InteractiveElement::in_focus_style) applies when the element or
  an ancestor is focused (matching `within_focused`). To highlight a parent
  container when a *child* has focus, branch on `handle.contains_focused(window, cx)`
  in `render`.
* [`focus_visible_style`](crate::InteractiveElement::focus_visible_style) applies when the
  element is focused *and* [`Window::last_input_was_keyboard`] is `true`, hiding
  focus rings after pointer clicks:

```rust
# use gpui::{Context, FocusHandle, Window, div, prelude::*, rgb};
# struct Field { focus_handle: FocusHandle }
# impl Render for Field {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
div()
    .track_focus(&self.focus_handle)
    .border_2()
    .border_color(rgb(0x3b3b3b))
    .focus_style(|style| style.border_color(rgb(0x3b82f6)))
    .focus_visible_style(|style| style.bg(rgb(0x1e3a8a)))
    .child("Name")
#     }
# }
```

## React to focus changes

Register focus listeners on [`Context`](crate::Context) and store the returned
[`Subscription`](crate::Subscription)s:

* [`Context::on_focus`] and [`Context::on_blur`] fire when the handle itself
  becomes or stops being the focused leaf (moving focus from a container to one
  of its children blurs the container leaf).
* [`Context::on_focus_in`] and [`Context::on_focus_out`] fire when focus enters
  or leaves the entire subtree rooted at the handle. `on_focus_out` receives a
  [`FocusOutEvent`](crate::FocusOutEvent) naming the blurred leaf.
* [`Context::on_focus_lost`] fires on the window when focus clears completely,
  giving workspaces a chance to restore a default focus target.

Focus listeners fire at the end of a frame while the window is active, so tests
must call [`TestAppWindow::activate`] and [`TestAppWindow::draw`]:

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

When a custom modal prompt is installed via [`Window::prompt`] and
[`PromptHandle::with_view`], GPUI saves a strong `FocusHandle` to the previously
focused control and restores it automatically when the prompt resolves.

## Tab stops and `Tab` navigation

A new `FocusHandle` starts with `tab_index(0)` and `tab_stop(false)`. Opt into
sequential navigation on the handle itself with `.tab_stop(true)` and order
stops with `.tab_index(order)` (lowest first, breaking ties in paint order).
Configure `tab_stop` and `tab_index` on the `FocusHandle` passed to
`track_focus`; the fluent [`InteractiveElement::tab_index`] and `tab_stop`
methods apply only to handles managed internally by `.focusable()`. Wrap a panel
in [`InteractiveElement::tab_group`] to scope child `tab_index` values locally
within that group.

GPUI does not bind `Tab` globally because code editors and text areas use `Tab`
for indentation. Bind `tab` and `shift-tab` in your form's key context to
actions that call [`Window::focus_next`] and [`Window::focus_prev`]:

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

Single-line `text_input` fields propagate `Tab` so form bindings like the one
above advance focus; multi-line `text_area` fields consume `Tab` to insert `\t`.

## See also

* [Handle keyboard shortcuts](crate::_documentation::guides::keyboard) — binding
  keys to actions in a `key_context`.
* [Accept text input](crate::_documentation::guides::text_input) — focus and
  `Tab` handling in single-line and multi-line text fields.
* [Input dispatch](crate::_documentation::concepts::input_dispatch) — how the
  rendered focus path drives capture and bubble dispatch.
