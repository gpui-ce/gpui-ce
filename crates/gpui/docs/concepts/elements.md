# Elements, components and views

Everything on screen is an *element*. GPUI gives you three ways to produce
elements; choosing the one that matches your state and control needs keeps code
simple:

| You want… | Write | It is |
| --- | --- | --- |
| A piece of interface with its own state | A **view**: a type that implements [`Render`] | An entity, kept between frames |
| A reusable piece of interface without state | A **component**: a type that implements [`RenderOnce`] | A value, consumed when rendered |
| Control over layout or painting | An **element**: a type that implements [`Element`] | A value, driven through the frame's phases |

Most code uses views and components built from [`div`]. Use a custom element
when you need control over layout or painting, such as for a text editor or
chart.

## Views

A view is an entity whose type implements [`Render`]. Its `render` method gets
`&mut self` for reading and updating view state, plus a [`Context`] that can
attach handlers back to that entity:

```
# use gpui::{Context, Window, div, prelude::*};
struct Toggle {
    on: bool,
}

impl Render for Toggle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("toggle")
            .child(if self.on { "On" } else { "Off" })
            .on_click(cx.listener(|toggle, _event, _window, cx| {
                toggle.on = !toggle.on;
                cx.notify();
            }))
    }
}
```

Use a view when interface state must survive between frames, or when other code
needs to hold a handle to it. A view shows another view by passing its
[`Entity`] as a child, as in `div().child(self.sidebar.clone())`.

## Components

A component is a plain value that describes some elements. Implement
[`RenderOnce`] and derive [`IntoElement`], and the component can be used
anywhere an element can:

```
# use gpui::{App, ClickEvent, SharedString, Window, div, prelude::*};
#[derive(IntoElement)]
struct Badge {
    label: SharedString,
    count: usize,
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .gap_1()
            .child(self.label)
            .child(self.count.to_string())
    }
}

fn inbox(unread: usize) -> impl IntoElement {
    div().child(Badge { label: "Inbox".into(), count: unread })
}
```

`RenderOnce::render` takes `self` by value: GPUI builds, renders and drops the
component on every frame. It has no `Context` of its own, so it cannot
update itself. To report events, give the component callbacks and let the view
that uses it decide what to do:

```
# use gpui::{App, ClickEvent, ElementId, SharedString, Window, div, prelude::*};
#[derive(IntoElement)]
struct Button {
    id: ElementId,
    label: SharedString,
    on_click: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .px_3()
            .py_1()
            .rounded_md()
            .child(self.label)
            .on_click(self.on_click)
    }
}
```

A view passes `Box::new(cx.listener(...))` as `on_click`, and the handler
updates the view. This separates a component library's presentation from its
state: components own their look, while views own the state.

## Elements are values

In every case, `render` returns values that GPUI consumes during the frame and
then drops. The next frame calls `render` again and receives new values. Build
those values from current state; keep state in views or entities, not in the
elements being rebuilt.

Because elements are ordinary values, normal Rust tools work for building them.
Child lists can come from iterators, and [`FluentBuilder`](prelude::FluentBuilder)
adds `when`, `when_some`, and `map` for conditional styling without breaking a
builder chain:

```
# use gpui::{IntoElement, div, prelude::*, rgb};
fn row(name: &str, selected: bool, note: Option<&str>) -> impl IntoElement {
    div()
        .child(name.to_string())
        .when(selected, |row| row.bg(rgb(0xdbeafe)))
        .when_some(note, |row, note| row.child(note.to_string()))
}
```

A function must return one concrete type. When branches produce different
element types, convert them with
[`into_any_element`](IntoElement::into_any_element) to the type-erased
[`AnyElement`].

## Identity and element state

Some state belongs to an element rather than to your program: whether it is
being pressed, how far it is scrolled, where an animation is. GPUI keeps this
state for you between frames. To find it again on the next frame it needs to
recognize the element, and for that the element needs an [`ElementId`]:

```
# use gpui::{IntoElement, div, prelude::*};
# fn list() -> impl IntoElement {
div()
    .id("messages")
    .overflow_y_scroll()
    .children((0..100usize).map(|index| {
        div().id(("message", index)).child(format!("Message {index}"))
    }))
# }
```

An element's identity is the path of ids from the window down to it, so an id
only has to be unique among the elements that share its nearest identified
ancestor. Ids can be strings, numbers, or a `(name, number)` pair as above. Two
rules follow:

* **Interactive elements need an id.** Calling [`id`](InteractiveElement::id)
  on a [`Div`] turns it into a [`Stateful<Div>`](Stateful), which unlocks
  clicks, scrolling, drag and drop, and active styles, because all of those keep
  state between frames.
* **Ids must be stable.** If a row's id changes from one frame to the next, its
  state is lost. Use something that identifies the row's *content*, such as a
  database id, rather than its position, when rows can move.

Your own code can keep state per element too.
[`Window::use_keyed_state`] returns an entity that lives as long as an element
with that id keeps being rendered, which is handy for components that need a
little private state.

## Custom elements

Implement [`Element`] when the built-in elements cannot express what you need:
a custom layout algorithm, a canvas with thousands of hit-testable shapes, a
text editor. The trait has one method per [phase of the frame](crate::_documentation::concepts::the_frame):

* [`request_layout`](Element::request_layout) tells GPUI how big the element
  wants to be.
* [`prepaint`](Element::prepaint) runs once bounds are known. Insert hitboxes
  and compute anything paint will need.
* [`paint`](Element::paint) adds primitives to the scene and registers event
  listeners.

The associated types carry data from one phase to the next. For one-off custom
drawing, [`canvas`] is simpler: it gives you prepaint and paint callbacks
without a new type. The [custom drawing guide](crate::_documentation::guides::custom_drawing)
and the `custom_element` example walk through a complete element.
