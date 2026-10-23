# Ownership and data flow

A UI often shows one value in several places: a sidebar and editor share a
document, a button changes a setting used by many views, and a network response
updates a model that a window is drawing.

`Rc<RefCell<T>>` can share mutable state, but it does not tell GPUI which views
depend on that state. Changing a field therefore does not schedule a redraw.
Reference cycles keep values alive, and conflicting borrows fail at runtime.

GPUI takes a different approach: **the [`App`](crate::App) owns your state**,
and your code holds *handles* to it. The sections that follow show how handles,
updates, events, and lifetimes fit together.

## Entities

You give a value to the app with [`new`](crate::AppContext::new). The app
stores it and hands back an [`Entity<T>`](crate::Entity):

```
# use gpui::{AppContext, Entity, TestApp};
# TestApp::new().update(|cx| {
struct Counter {
    count: usize,
}

let counter: Entity<Counter> = cx.new(|_cx| Counter { count: 0 });
# });
```

An `Entity<T>` is a typed, reference-counted handle, a little like an `Rc<T>`.
Cloning it is cheap and gives you another handle to the same value. Unlike an
`Rc`, it cannot reach the value on its own. You need a context to read or update
it:

```
# use gpui::{AppContext, TestApp};
# struct Counter { count: usize }
# TestApp::new().update(|cx| {
let counter = cx.new(|_cx| Counter { count: 0 });

counter.update(cx, |counter, _cx| counter.count += 1);
assert_eq!(counter.read(cx).count, 1);
# });
```

The context is the boundary that makes this design work. Every read and write
passes through the app, while the handle can live inside another entity, a
closure, or an async task without holding a borrow of the stored value.

## Updating an entity

[`Entity::update`](crate::Entity::update) gives your closure a `&mut T` and a
[`Context<T>`](crate::Context). That context has the app's services plus the
identity of the entity being updated, so the closure can notify its observers
or emit an event without passing the entity handle separately.

While the closure runs, GPUI has lent the value to you. Reading or updating the
*same* entity again from inside the closure is an error, and GPUI panics with
`cannot update Counter while it is already being updated`. Updating *other*
entities is fine. If you need the same value again, use the `&mut T` you were
given.

## Telling GPUI what changed

GPUI does not watch your fields. When an update changes something that other
parts of the program care about, say so with
[`Context::notify`](crate::Context::notify):

```
# use gpui::{AppContext, TestApp};
# struct Counter { count: usize }
# TestApp::new().update(|cx| {
# let counter = cx.new(|_cx| Counter { count: 0 });
counter.update(cx, |counter, cx| {
    counter.count += 1;
    cx.notify();
});
# });
```

Notifying schedules two effects that run after the current update:

* Every window that read `counter` while rendering its last frame is marked for
  redraw. GPUI tracks those reads for you, so a view that displays the count is
  re-rendered and one that does not is left alone.
* Every observer registered with [`Context::observe`](crate::Context::observe)
  is called.

GPUI queues these effects until the outermost update finishes, in queue order.
In a running app, every event handler, timer, and frame runs inside one update,
so the effects run after the handler returns. Three notifications in one update
cost the same as one, and observers never run while the notifying entity is
still lent out.

[`TestApp`](crate::TestApp) makes this visible, because each call to its
`update` is one turn of the event loop:

```
use gpui::{AppContext, Context, TestApp};

struct Counter {
    count: usize,
}

struct Doubler {
    doubled: usize,
}

let mut app = TestApp::new();

let (counter, doubler) = app.update(|cx| {
    let counter = cx.new(|_cx| Counter { count: 0 });
    let doubler = cx.new(|cx: &mut Context<Doubler>| {
        cx.observe(&counter, |doubler, counter, cx| {
            doubler.doubled = counter.read(cx).count * 2;
        })
        .detach();
        Doubler { doubled: 0 }
    });
    (counter, doubler)
});

app.update(|cx| {
    counter.update(cx, |counter, cx| {
        counter.count = 21;
        cx.notify();
    });
    // This update is still running, so the observer hasn't been called yet.
    assert_eq!(doubler.read(cx).doubled, 0);
});

// It was called as soon as the previous update finished.
app.update(|cx| assert_eq!(doubler.read(cx).doubled, 42));
```

## Events

Notifying says "I changed; look at me again". Sometimes you want to say what
happened instead. Implement [`EventEmitter`](crate::EventEmitter) for each event
type an entity can emit, emit with [`Context::emit`](crate::Context::emit), and
listen with [`Context::subscribe`](crate::Context::subscribe):

```
# use gpui::{AppContext, Context, EventEmitter, TestApp};
struct Downloader;

enum DownloadEvent {
    Finished { bytes: usize },
}

impl EventEmitter<DownloadEvent> for Downloader {}

struct StatusBar {
    message: String,
}

# let mut app = TestApp::new();
# let (downloader, status) = app.update(|cx| {
let downloader = cx.new(|_cx| Downloader);

let status = cx.new(|cx: &mut Context<StatusBar>| {
    cx.subscribe(&downloader, |status, _downloader, event, _cx| match event {
        DownloadEvent::Finished { bytes } => {
            status.message = format!("Downloaded {bytes} bytes");
        }
    })
    .detach();
    StatusBar { message: String::new() }
});
# (downloader, status)
# });
# app.update(|cx| {

downloader.update(cx, |_downloader, cx| {
    cx.emit(DownloadEvent::Finished { bytes: 512 });
});
# });
# app.update(|cx| {

// Later, once the update that emitted the event has finished:
assert_eq!(status.read(cx).message, "Downloaded 512 bytes");
# });
```

Events are typed: an entity can emit several kinds, while each subscriber hears
only the kind it requested. Like notifications, events are delivered after the
current update finishes.

## Subscriptions

`observe` and `subscribe` return a [`Subscription`](crate::Subscription). The
callback stays registered for as long as you keep the subscription. Drop it to
stop listening:

```
# use gpui::{AppContext, Context, Subscription, TestApp};
# struct Settings;
struct Panel {
    _settings_changed: Subscription,
}
# TestApp::new().update(|cx| {
# let settings = cx.new(|_| Settings);
# let _panel = cx.new(|cx: &mut Context<Panel>| Panel {
#     _settings_changed: cx.observe(&settings, |_, _, _| {}),
# });
# });
```

Storing the subscription in a field ties it to the lifetime of the entity that
owns the field, which is usually what you want. If the listening entity should
outlive the local subscription handle, call [`detach`](crate::Subscription::detach)
instead. GPUI-owned subscriptions may still remove themselves when an entity
they depend on is released; detaching does not make a callback independent of
those entities.

## Lifetimes and weak handles

An entity lives as long as any strong handle to it exists. When the last
`Entity<T>` is dropped, GPUI releases the value at the end of the current
update, after running any callbacks registered with
[`Context::on_release`](crate::Context::on_release).

That makes cycles a leak, as with `Rc`: if a parent view holds its child
and the child holds its parent, neither is ever released. Use a
[`WeakEntity<T>`](crate::WeakEntity) for back-references and for long-lived
callbacks. A weak handle does not keep the value alive, and reaching the value
through it returns a `Result` because the value may be gone:

```
# use gpui::{AppContext, TestApp};
# struct Counter { count: usize }
# let mut app = TestApp::new();
# let weak = app.update(|cx| {
let counter = cx.new(|_cx| Counter { count: 0 });
let weak = counter.downgrade();
assert!(weak.update(cx, |counter, _cx| counter.count += 1).is_ok());

drop(counter);
# weak
# });
// After the update that dropped the last strong handle, the counter is gone.
# app.update(|cx| {
assert!(weak.update(cx, |counter, _cx| counter.count += 1).is_err());
# });
```

GPUI's own callback helpers lean on this. The closure that
[`Context::listener`](crate::Context::listener) builds for an event handler
holds a weak handle to your view, so a button never keeps its view alive.
[`Context::spawn`](crate::Context::spawn) passes your async code a weak handle
for the same reason.

## Views are entities too

A *view* is an entity whose type implements [`Render`](crate::Render). There is
nothing else to it: a view is created with `cx.new`, held by an `Entity`, and
updated and notified like any other entity. A window holds its root view, and a
view shows another view by passing a handle as a child:

```
# use gpui::{AppContext, Context, Entity, Window, div, prelude::*};
struct Sidebar;

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Files")
    }
}

struct Workspace {
    sidebar: Entity<Sidebar>,
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().child(self.sidebar.clone()).child("Editor")
    }
}
```

When `Sidebar` notifies, its window is redrawn. By default a redraw renders
the window's whole tree again, starting from the root view, because building
elements is cheap. A view that is expensive to render can opt out: embed it
with [`Entity::cached`](crate::Entity::cached) and GPUI reuses its previous
output until the view is notified, or its bounds, content mask, text style, text direction or
Unicode bidirectional text mode changes. If the cached view reads another entity, observe that
entity and notify the cached view when its value changes. [The frame](crate::_documentation::concepts::the_frame)
explains the rendering side of this in detail.

## Globals

Some state belongs to the whole application: settings, a theme, a connection
pool. Implement [`Global`](crate::Global) for the type and store one value of it
in the app:

```
# use gpui::{Global, TestApp, prelude::*};
struct Theme {
    dark: bool,
}

impl Global for Theme {}

# TestApp::new().update(|cx| {
cx.set_global(Theme { dark: false });
cx.update_global::<Theme, _>(|theme, _cx| theme.dark = true);
assert!(cx.global::<Theme>().dark);
# });
```

[`App::observe_global`](crate::App::observe_global) lets entities react when a
global is updated. Prefer entities for state that more than one part of your
program owns or that can have more than one instance. Globals are for things
there is exactly one of.

## Summary

* The app owns every entity. You hold `Entity<T>` handles and reach the value
  through a context.
* Updates lend you the value. Don't reach for the same entity again inside its
  own update.
* Call `notify` when you change something others display or observe. Use
  events when listeners need to know *what* happened.
* Keep subscriptions in fields, or detach them.
* Strong handles keep entities alive. Use `WeakEntity` to break cycles.
* A view is an entity that implements `Render`.

Next, [contexts](crate::_documentation::concepts::contexts) explains the different context types you'll
meet and when you have each one.
