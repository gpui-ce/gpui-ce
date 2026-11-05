# Elements, components and views

Everything on screen is an *element*. GPUI gives you three ways to produce
elements; choosing the one that matches your state and control needs keeps code
simple:

| You want… | Write | It is |
| --- | --- | --- |
| Interface state that other code can access through a handle | A **view**: a type that implements [`Render`] | An entity, kept between frames |
| Reusable presentation with inputs, callbacks, and optional private state | A **component**: a type that implements [`RenderOnce`] | A value, consumed when rendered |
| Direct control over layout, prepaint, and paint phases | An **element**: a type that implements [`Element`] | A value, driven through the frame's phases |

Most code uses views and components built from [`div`]. For custom drawing,
[`canvas`] provides prepaint and paint callbacks, including hitbox insertion. A
chart can use a canvas even when its shapes need hit testing. Implement
[`Element`] when you also need to define layout or carry your own data between
the layout, prepaint, and paint phases.

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

Use a view when other code needs a handle to its state, methods, or events. A
parent view can store `toggle: Entity<Toggle>` and show it with
`div().child(self.toggle.clone())`. Private state can also survive between
frames in a component through [`Window::use_state`] or
[`Window::use_keyed_state`]; persistence alone does not require a view.

## Components

A component is a plain value that describes some elements. Implement
[`RenderOnce`] and derive [`IntoElement`], and the component can be used
anywhere an element can:

```
# use gpui::{App, SharedString, Window, div, prelude::*};
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

`RenderOnce::render` takes `self` by value, so the component value is consumed
when rendered. It has no entity-specific `Context` of its own. For private
state, use [`Window::use_state`] or [`Window::use_keyed_state`], which return
state entities retained by the window across consecutive frames. To expose
changes to the caller, accept callbacks and let the owning view update its
state:

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
updates the view. Callbacks let the same button presentation work with different
application state.

## Elements are values

Views and components produce elements through their render methods; custom
elements implement the frame phases directly. GPUI consumes the resulting
element values during the frame. Build new elements from current state when
rendering again. Keep persistent state in entities or window-managed element
state rather than fields of the element values being rebuilt.

A function must return one concrete type. When branches produce different
element types, use [`into_any_element`](IntoElement::into_any_element) to
convert them to the type-erased [`AnyElement`].

Because elements are ordinary values, normal Rust tools work for building them.
Child lists can come from iterators, and
[`FluentBuilder`](prelude::FluentBuilder) adds `when`, `when_some`, and `map`
for conditional styling without breaking a builder chain:

```
# use gpui::{IntoElement, div, prelude::*, rgb};
fn row(name: &str, selected: bool, note: Option<&str>, tags: &[&str]) -> impl IntoElement {
    div()
        .child(name.to_string())
        .children(tags.iter().map(|tag| div().child((*tag).to_string())))
        .when(selected, |row| row.bg(rgb(0xdbeafe)))
        .when_some(note, |row, note| row.child(note.to_string()))
}
```

## Identity and element state

GPUI retains interaction state, such as pressed state and scroll offsets, by an
element's identity. Your own code can retain private state through
[`Window::use_keyed_state`], using an explicit key, or [`Window::use_state`],
using the caller's source location as the key. Both return an entity and reuse
it while the same identity is rendered in consecutive frames. Notifying that
entity redraws the view that uses it. Inside a loop, use a different stable key
for each item; repeated calls to `use_state` at one source location share a key.

An [`ElementId`] lets GPUI find an element's retained state on the next frame:

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

An element's identity is a path within its window. It includes explicit ancestor
IDs, view entity IDs for `Render` views, and a type-name namespace for
`RenderOnce` components. Multiple components of the same type still need
distinct IDs for their state when they share a parent namespace. Custom
elements can add namespaces with [`Window::with_element_namespace`].

IDs can represent entity IDs, integers, strings, UUIDs, focus handles,
`(name, number)` pairs, file-system paths, source locations, named children of
other IDs, or opaque byte IDs; [`ElementId`] lists the variants. An ID only
needs to be unique within its parent namespace. Two rules follow:

* **Interactive elements need an id.** Calling [`id`](InteractiveElement::id)
  on a [`Div`] turns it into a [`Stateful<Div>`](Stateful), which unlocks
  clicks, scrolling, drag and drop, and active styles, because all of those keep
  state between frames.
* **Ids must be stable.** If a row's id changes from one frame to the next, its
  state is lost. Use something that identifies the row's *content*, such as a
  database id, rather than its position, when rows can move.

## Custom elements

Implement [`Element`] to define layout and pass your own state between frame
phases, as in a text editor or a custom layout container. The trait has one
method per [phase of the frame](crate::_documentation::concepts::the_frame):

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
