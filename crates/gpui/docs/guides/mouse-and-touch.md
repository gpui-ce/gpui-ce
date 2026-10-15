# Respond to the mouse and touch

GPUI targets mouse input at the frontmost element under the pointer. Give a
clickable element an [`id`](crate::InteractiveElement) and attach
[`on_click`](crate::StatefulInteractiveElement::on_click); use the pointer-level
handlers when you need presses or movement. Touch is translated to the mouse
and scroll events supported by the current backend. The public
`ClickEvent::Touch` variant is reserved for future gesture backends; current
touch taps arrive as primary mouse clicks.

## Handle a click

[`on_click`](crate::StatefulInteractiveElement::on_click) needs an id, so call
[`id`](crate::InteractiveElement::id) first. GPUI invokes the handler on release
only when the press began on that element and the release is still over it. The
handler receives a [`ClickEvent`]; wrap it with `cx.listener` when it must edit
view state.

The example below clicks a button twice. The second press is a double click with
Shift held, which is how you read the click count and modifiers.

```standalone_crate
use gpui::{
    ClickEvent, Context, MouseButton, MouseDownEvent, MouseUpEvent, Modifiers, TestApp, Window,
    div, point, prelude::*, px,
};

struct Toolbar {
    clicks: Vec<(usize, bool)>,
}

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().child(
            div()
                .id("save")
                .w(px(120.))
                .h(px(40.))
                .on_click(cx.listener(|toolbar, event: &ClickEvent, _window, _cx| {
                    toolbar.clicks.push((event.click_count(), event.modifiers().shift));
                }))
                .child("Save"),
        )
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Toolbar { clicks: Vec::new() });

let inside = point(px(10.), px(10.));
window.simulate_click(inside, MouseButton::Left);

let shift = Modifiers { shift: true, ..Default::default() };
window.simulate_event(MouseDownEvent {
    position: inside,
    button: MouseButton::Left,
    modifiers: shift,
    click_count: 2,
    first_mouse: false,
});
window.simulate_event(MouseUpEvent {
    position: inside,
    button: MouseButton::Left,
    modifiers: shift,
    click_count: 2,
});

assert_eq!(window.read(|toolbar, _| toolbar.clicks.clone()), [(1, false), (2, true)]);
```

[`ClickEvent`] tells you how the click happened:

- [`click_count`](ClickEvent::click_count) is 1 for a single click and 2 for a
  double click. Keyboard clicks always report 1.
- [`modifiers`](ClickEvent::modifiers) are the keys held on release. Keyboard clicks
  never have modifiers.
- [`position`](ClickEvent::position) is where the pointer was released.
- [`is_right_click`](ClickEvent::is_right_click) and
  [`is_secondary`](ClickEvent::is_secondary) tell you when the click is a context
  menu request.

`on_click` handles the left button only. For right and middle clicks, use
[`on_aux_click`](crate::StatefulInteractiveElement::on_aux_click). Touch long
presses produce [`LongPressEvent`] instead; a custom element can receive them
through [`Window::on_mouse_event`] during paint.

Enter and Space also produce a click when a focused element with `on_click` has
focus. The event is a [`ClickEvent::Keyboard`], so check
[`is_keyboard`](ClickEvent::is_keyboard) if the handler needs to tell them apart.

## Track the pointer

For presses and movement that a click does not describe, use
[`on_mouse_down`](crate::InteractiveElement::on_mouse_down),
[`on_mouse_up`](crate::InteractiveElement::on_mouse_up) and
[`on_mouse_move`](crate::InteractiveElement::on_mouse_move). They don't need an id.
`on_mouse_down` and `on_mouse_up` take the button to listen for. Use
[`on_mouse_down_out`](crate::InteractiveElement::on_mouse_down_out) to react to a
press outside an element, such as closing a popover, and
[`capture_any_mouse_down`](crate::InteractiveElement::capture_any_mouse_down) when
you must see a press before any element does.

If a drag must continue after the pointer leaves its original hitbox, call
[`Window::capture_pointer`] with that hitbox's id. Captured move and release
events continue to reach the captured element until release or
[`Window::release_pointer`]. This is different from
[`InteractiveElement::occlude`], which only changes which hitboxes behind an
element can receive input.

## Respond to hover

Use [`hover`](crate::InteractiveElement::hover) for a style that follows the
pointer. Use [`on_hover`](crate::StatefulInteractiveElement::on_hover) when code
needs the transition; it needs an id and fires when the state changes, not for
every move. While a button is held down over the element, GPUI reports it as not
hovered.

```standalone_crate
use gpui::{Context, MouseMoveEvent, TestApp, Window, div, point, prelude::*, px, rgb};

struct Hover {
    hovered: bool,
    moves: usize,
}

impl Render for Hover {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("hover-target")
            .w(px(120.))
            .h(px(40.))
            .hover(|style| style.bg(rgb(0x334155)))
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, _window, _cx| {
                this.moves += 1;
            }))
            .child("Hover me")
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Hover { hovered: false, moves: 0 });

window.simulate_mouse_move(point(px(10.), px(10.)));
assert_eq!(window.read(|hover, _| (hover.hovered, hover.moves)), (true, 1));

window.simulate_mouse_move(point(px(500.), px(500.)));
assert!(!window.read(|hover, _| hover.hovered));
```

## Style pressed and hovered elements

[`active`](crate::StatefulInteractiveElement::active) styles an element while it's
pressed. Like `on_hover`, it needs an id. Use [`cursor`](crate::Styled) with a
[`CursorStyle`] to choose the pointer shape, or a shorthand such as
`cursor_pointer()`.

Tooltips need an id too. [`tooltip`](crate::StatefulInteractiveElement::tooltip)
takes a function that builds a view. The view appears after a short delay while the
pointer rests on the element, and hides when the pointer leaves.

```standalone_crate
use gpui::{AppContext as _, Context, Render, Window, div, prelude::*, rgb};

struct Tip;

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Save the file")
    }
}

struct Toolbar;

impl Render for Toolbar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("save")
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x334155)))
            .active(|style| style.bg(rgb(0x1e293b)))
            .tooltip(|_, cx| cx.new(|_| Tip).into())
            .child("Save")
    }
}
```

## Drag and drop

Start a drag with [`on_drag`](crate::StatefulInteractiveElement::on_drag), which
needs an id, a value, and a constructor for the preview drawn under the pointer.
A target declares its accepted type with
[`on_drop`](crate::InteractiveElement::on_drop); [`drag_over`](crate::InteractiveElement::drag_over)
can style that target while a matching value passes over it. The example drags
one card into a typed drop zone and checks the received value.

```standalone_crate
use gpui::{
    AppContext as _, Context, MouseButton, Render, TestApp, Window, div, point, prelude::*, px, rgb,
};

#[derive(Clone)]
struct Card {
    title: &'static str,
}

struct CardPreview(&'static str);

impl Render for CardPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().px_2().bg(rgb(0x334155)).child(self.0)
    }
}

struct Board {
    dropped: Vec<&'static str>,
}

impl Render for Board {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("card")
                    .w(px(200.))
                    .h(px(40.))
                    .child("Card A")
                    .on_drag(Card { title: "Card A" }, |card, _offset, _window, cx| {
                        cx.new(|_| CardPreview(card.title))
                    }),
            )
            .child(
                div()
                    .w(px(200.))
                    .h(px(100.))
                    .drag_over::<Card>(|style, _, _, _| style.bg(rgb(0x1e293b)))
                    .on_drop(cx.listener(|board, card: &Card, _window, cx| {
                        board.dropped.push(card.title);
                        cx.notify();
                    })),
            )
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Board { dropped: Vec::new() });

window.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left);
window.simulate_mouse_move(point(px(10.), px(90.)));
window.simulate_mouse_up(point(px(10.), px(90.)), MouseButton::Left);

assert_eq!(window.read(|board, _| board.dropped.clone()), ["Card A"]);
```

A drop target only receives values of the type it was registered for. To refuse a
value of that type, for example when the zone is full, use
[`can_drop`](crate::InteractiveElement::can_drop).

To expose a disabled control, set [`aria_disabled`](crate::StatefulInteractiveElement::aria_disabled)
for assistive technology and make the input handler ignore activation while the
control is disabled. The accessibility flag describes availability; it does not
replace the application condition in a click or action handler.

## Scroll, pinch and touch

[`on_scroll_wheel`](crate::InteractiveElement::on_scroll_wheel) receives a
[`ScrollWheelEvent`] for each scroll. Its `delta` is either pixels or lines, so
convert it with [`ScrollDelta::pixel_delta`](ScrollDelta::pixel_delta) to get one
unit. The listener runs for each element under the pointer, from the front, until
one stops propagation or occludes the elements behind it.

[`on_pinch`](crate::InteractiveElement::on_pinch) receives a [`PinchEvent`] for
each zoom gesture. Its `delta` is the change in zoom, so `0.1` is a 10 percent
increase.

```standalone_crate
use gpui::{Context, PinchEvent, ScrollWheelEvent, TestApp, Window, div, point, prelude::*, px};

struct Viewer {
    scrolled: f32,
    zoom: f32,
}

impl Render for Viewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size(px(200.))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _window, _cx| {
                this.scrolled += f32::from(event.delta.pixel_delta(px(20.)).y);
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _window, _cx| {
                this.zoom += event.delta;
            }))
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Viewer { scrolled: 0., zoom: 1. });

window.simulate_scroll(point(px(10.), px(10.)), point(px(0.), px(-30.)));
assert_eq!(window.read(|viewer, _| viewer.scrolled), -30.);
```

Touch input is translated into the same event types, so most components work on
touch without changes. A tap becomes a synthesized left mouse press and release,
so it reaches
`on_click` as a [`ClickEvent::Mouse`]. A pan becomes [`ScrollWheelEvent`]s with a
[`TouchPhase`](crate::TouchPhase). A long press is reported as a `LongPressEvent`
instead of a click. GPUI doesn't recognize pinch from touch yet. The `gestures`
module in the GPUI source describes how GPUI recognizes them.

## Block the pointer from elements behind

A panel that floats over other content should stop the pointer from reaching
what's beneath it. Call [`occlude`](crate::InteractiveElement::occlude) on it.
For a scrollable panel, use
[`block_mouse_except_scroll`](crate::InteractiveElement::block_mouse_except_scroll)
instead, so scrolling still reaches the content behind.

## See also

- [Input and dispatch](crate::_documentation::concepts::input_dispatch) explains the
  capture and bubble phases and how the pointer finds its target.
- [Handle keyboard shortcuts](crate::_documentation::guides::keyboard) covers keys.
- [Manage focus](crate::_documentation::guides::focus) covers the focus that a click
  gives an element with a tracked handle.
- The [interaction example](crate::_documentation::examples::interaction) shows
  these handlers in a running app. Run it with `cargo run --example interaction`.
