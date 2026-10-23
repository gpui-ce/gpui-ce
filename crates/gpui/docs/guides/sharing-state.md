# Share state between views

Several views can read the same [`Entity`] without keeping separate copies of
its data. Clone the handle when a view should retain the model; borrow it when
the view only needs an initial read or a subscription. The
[ownership concept](crate::_documentation::concepts::ownership) explains how
strong and weak handles control the model's lifetime.

## Share a model through entity handles

The app below retains a `Cart`. `Badge` stores a derived count and a
subscription that refreshes it. Appending an item changes the cart's value;
calling `cx.notify()` announces that change to the observer:

```rust
use gpui::{AppContext, Context, Entity, Subscription, TestApp};

#[derive(Default)]
struct Cart {
    items: Vec<String>,
}

struct Badge {
    count: usize,
    _observation: Subscription,
}

impl Badge {
    fn new(cart: &Entity<Cart>, cx: &mut Context<Self>) -> Self {
        Self {
            count: cart.read(cx).items.len(),
            _observation: cx.observe(cart, |badge, cart, cx| {
                badge.count = cart.read(cx).items.len();
            }),
        }
    }
}

let mut app = TestApp::new();
let (cart, badge) = app.update(|cx| {
    let cart = cx.new(|_| Cart::default());
    let badge = cx.new(|cx| Badge::new(&cart, cx));
    (cart, badge)
});

app.update(|cx| {
    cart.update(cx, |cart, cx| {
        cart.items.push("Keyboard".into());
        cx.notify();
    });
});

// The observer runs when the update above finishes, so check in the next update.
app.update(|cx| assert_eq!(badge.read(cx).count, 1));
```

`Badge::new` borrows the cart handle to read its initial count and register the
listener. The badge retains no strong cart handle. Releasing the badge drops
its subscription; releasing the cart's final strong handle releases the model.

## React to every change with `observe`

[`Context::observe`] receives the changed entity's handle, so the listener can
recompute from its current state. Here one notification covers two additions:

```rust
# use gpui::{AppContext, Context, Entity, Subscription, TestApp};
# #[derive(Default)]
# struct Cart { items: Vec<String> }
# struct Badge { count: usize, _observation: Subscription }
# impl Badge {
#     fn new(cart: &Entity<Cart>, cx: &mut Context<Self>) -> Self {
#         Self {
#             count: cart.read(cx).items.len(),
#             _observation: cx.observe(cart, |badge, cart, cx| {
#                 badge.count = cart.read(cx).items.len();
#             }),
#         }
#     }
# }
# let mut app = TestApp::new();
# let (cart, badge) = app.update(|cx| {
#     let cart = cx.new(|_| Cart::default());
#     let badge = cx.new(|cx| Badge::new(&cart, cx));
#     (cart, badge)
# });
app.update(|cx| {
    cart.update(cx, |cart, cx| {
        cart.items.push("Mouse".into());
        cart.items.push("Cable".into());
        cx.notify();
    });
});
app.update(|cx| assert_eq!(badge.read(cx).count, 2));
```

[`Context::notify`] schedules observers and invalidates rendering that depends
on the entity; mutation alone does neither. Notifications coalesce within an
update, so even several `notify` calls produce one observer callback. Delivery
waits until the model's mutable borrow is released and the update has finished.

Caching needs an explicit connection. A view rendered through [`Entity::cached`]
is not invalidated merely because it reads another entity. Notify the cached
view's own entity when the output must change. An observer still receives the
model's notifications independently of that rendering cache.

## Send typed events with `emit` and `subscribe`

Use an event when the occurrence itself matters. A “finished” event can carry a
file name; an observer would only tell you that some state changed. Implement
[`EventEmitter`] for each event type, call [`Context::emit`] in the model, and
subscribe from the listener. A subscription receives only its named event type,
by reference.

```rust
use gpui::{AppContext, Context, Entity, EventEmitter, Subscription, TestApp};

enum UploadEvent {
    Finished { name: String },
}

struct Uploads;

impl EventEmitter<UploadEvent> for Uploads {}

struct Notifications {
    finished: Vec<String>,
    _subscription: Subscription,
}

impl Notifications {
    fn new(uploads: &Entity<Uploads>, cx: &mut Context<Self>) -> Self {
        Self {
            finished: Vec::new(),
            _subscription: cx.subscribe(uploads, |this, _uploads, event, _cx| match event {
                UploadEvent::Finished { name } => this.finished.push(name.clone()),
            }),
        }
    }
}

let mut app = TestApp::new();
let (uploads, notifications) = app.update(|cx| {
    let uploads = cx.new(|_| Uploads);
    let notifications = cx.new(|cx| Notifications::new(&uploads, cx));
    (uploads, notifications)
});

app.update(|cx| {
    uploads.update(cx, |_uploads, cx| {
        cx.emit(UploadEvent::Finished { name: "logo.png".into() });
    });
});

app.update(|cx| {
    assert_eq!(notifications.read(cx).finished, vec!["logo.png".to_string()]);
});
```

Events are delivered after the emitting update, like notifications. A model
can implement `EventEmitter` for several event types, with separate
subscribers for each. The [events example](crate::_documentation::examples::events)
combines a mutating view, a derived view and an event consumer.

## Keep subscriptions alive

[`Context::observe`], [`Context::subscribe`] and [`App::observe_global`] return a
[`Subscription`]. The handle controls local cancellation. GPUI also removes a
callback when an entity it depends on is released, so choose both the handle's
owner and the dependency's lifetime deliberately:

* **Store it** in a field of the listening view when the listener should stop
  with the view. Name the field with a leading underscore if you never read it,
  as `_observation` above.
* **Call [`Subscription::detach`]** when the local handle should not own
  cancellation. A GPUI subscription can still be removed when an entity it
  depends on is released; detaching does not make it independent of those
  entities. Use it for a subscription made in a constructor where no field can
  store the handle.

If a subscription is neither stored nor detached, dropping it ends the listener
at the end of its scope. A temporary subscription therefore silently disables
the callback; retain it when the listener is meant to live longer.

## Share app-wide state with a global

A global is one value of a type for the whole [`App`]. Use it for state that is
singular by definition, such as settings, a theme, or a feature flag. Implement
[`Global`] for the type, then read and write it through the app.

```rust
use std::{cell::Cell, rc::Rc};

use gpui::{BorrowAppContext, Global, TestApp};

#[derive(Default)]
struct Settings {
    font_size: f32,
}

impl Global for Settings {}

let mut app = TestApp::new();
let changes = Rc::new(Cell::new(0));

// Set the global in its own update. Global notifications from `set_global` are
// queued, and an observer added in the same update would miss them.
app.update(|cx| cx.set_global(Settings { font_size: 14.0 }));

app.update(|cx| {
    let changes = changes.clone();
    cx.observe_global::<Settings>(move |_cx| changes.set(changes.get() + 1))
        .detach();

    cx.update_global::<Settings, _>(|settings, _cx| settings.font_size = 16.0);
});

app.update(|cx| {
    assert_eq!(cx.global::<Settings>().font_size, 16.0);
    assert_eq!(changes.get(), 1);
});
```

[`App::global`] panics if no value of the type has been set; initialize globals
before a view reads them, or use [`App::try_global`] to handle absence. An
[`App::observe_global`] callback runs after the current update when the global is
set, mutated, removed, or restored after a global lease. `global_mut` and
`default_global` notify on every call, even when the value does not change, so a
call is a change announcement even when the fields compare equal. A view can
use [`Context::observe_global`] to receive its own state and context. Prefer an
entity when the state may have more than one instance.

## Avoid cycles with weak handles

An [`Entity`] is strong and keeps its value alive. If a parent holds a child and
the child holds the parent through another strong handle, neither is released.
Store a
[`WeakEntity`] for the back-reference. Reach the value through it with
[`WeakEntity::update`], which returns a `Result` that fails once the value is gone.

```rust
use gpui::{AppContext, Entity, TestApp, WeakEntity};

struct Parent {
    title: String,
    child: Option<Entity<Child>>,
}

struct Child {
    parent: WeakEntity<Parent>,
}

let mut app = TestApp::new();
let (parent, child) = app.update(|cx| {
    let parent = cx.new(|_| Parent {
        title: "Inbox".into(),
        child: None,
    });
    let child = cx.new(|_| Child {
        parent: parent.downgrade(),
    });
    parent.update(cx, |parent, _cx| parent.child = Some(child.clone()));
    (parent, child)
});

app.update(|cx| {
    let weak = child.read(cx).parent.clone();
    let title = weak.update(cx, |parent, _cx| parent.title.clone());
    assert_eq!(title.unwrap(), "Inbox");
});

// Drop the last strong handle to the parent.
drop(parent);

app.update(|cx| {
    let weak = child.read(cx).parent.clone();
    assert!(weak.update(cx, |parent, _cx| parent.title.clone()).is_err());
});
```

Use a weak handle when a callback should not keep the entity alive. The
closures that [`Context::listener`] and [`Context::spawn`] create supply weak
handles for this reason. Other values captured by your closure retain their
usual ownership, so capturing another strong handle can still keep the view
alive. A weak update returns an error after the last strong handle is gone;
handle that result as the normal shutdown path for the callback.

## Choose a mechanism

| You want to | Use |
| --- | --- |
| Show the same data in several views | Handles to a model as `Entity<T>` |
| Recompute something when a model changes | [`Context::observe`] and [`Context::notify`] |
| Tell listeners what happened | [`EventEmitter`], [`Context::emit`] and [`Context::subscribe`] |
| Hold one value for the whole app | [`Global`] with [`App::set_global`] and [`BorrowAppContext::update_global`] |
| Refer to an entity that may be gone | [`WeakEntity`] |

The notification choices are observers and events. Globals select the scope
of state, while weak handles control lifetime.

## See also

- [Ownership and data flow](crate::_documentation::concepts::ownership), for how entities,
  updates and effects work.
- [Contexts](crate::_documentation::concepts::contexts), for `Context<T>` and `App`.
- [Run work in the background](crate::_documentation::guides::async_work), for tasks that
  update a model later.
- [The events example](crate::_documentation::examples::events).
