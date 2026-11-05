# Contexts

Most GPUI callbacks receive a `cx` argument. It is a *context*: access to the
application, sometimes paired with the identity of the entity being processed.
The context type tells you which state and services the callback may use. Entity
operations come from the [`AppContext`](crate::AppContext) trait, which these
contexts implement.

| Context | You have it… | It adds… |
| --- | --- | --- |
| [`&mut App`](crate::App) | In the [`Application::run`](crate::Application::run) callback and in element event handlers | Everything application-wide: entities, windows, globals, key bindings, platform services |
| [`&mut Context<T>`](crate::Context) | While creating, updating or rendering the entity `T` | `notify`, `emit`, `observe`, `subscribe`, `listener` and `spawn` for that entity |
| [`&mut AsyncApp`](crate::AsyncApp) | Inside futures started with `spawn` | Access to the app across `.await` points |
| [`&mut AsyncWindowContext`](crate::AsyncWindowContext) | Inside futures started with `spawn_in` | The same, plus one window |

A separate [`&mut Window`](crate::Window) often accompanies the context. It
provides access to one window's focus, size, input state, and painting; it does
not replace the app context.

## `App`: the application

[`App`](crate::App) is the application-wide context. It owns entities and
windows, key bindings, global values and a connection to platform services.
GPUI uses a [`Platform`](crate::Platform) implementation for each supported
platform to connect to services such as windowing and input. A function that
takes `&mut App` can use those application-wide services:

```standalone_crate,no_run
use gpui::{App, KeyBinding, actions};

actions!(app, [Quit]);

fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

fn main() {
    gpui_platform::application().run(init);
}
```

This starts the platform event loop and registers a quit action. To open a
window too, follow [Hello, window](crate::_documentation::tutorial::ch1_hello_window).

## `Context<T>`: working on one entity

When you create or update an entity, GPUI gives you a
[`Context<T>`](crate::Context). It implements `AppContext` for entity access and
also dereferences to `App` for application-wide services. It knows which entity
it belongs to, so these methods do not need an entity argument:

* [`notify`](crate::Context::notify) marks this entity as changed.
* [`emit`](crate::Context::emit) sends an event from this entity.
* [`observe`](crate::Context::observe) and
  [`subscribe`](crate::Context::subscribe) register callbacks that receive this
  entity mutably.
* [`listener`](crate::Context::listener) wraps an element event handler so
  that it receives this entity mutably.
* [`spawn`](crate::Context::spawn) starts async work with a weak handle to this
  entity.

```
# use gpui::{AppContext, Context, TestApp};
struct Clock {
    ticks: u64,
}

impl Clock {
    fn tick(&mut self, cx: &mut Context<Self>) {
        self.ticks += 1;
        cx.notify();
    }
}

# TestApp::new().update(|cx| {
let clock = cx.new(|_cx| Clock { ticks: 0 });
clock.update(cx, |clock, cx| clock.tick(cx));
assert_eq!(clock.read(cx).ticks, 1);
# });
```

Entity methods like `tick` conventionally take `&mut self` to change their own
state and `cx: &mut Context<Self>` when they also need GPUI services. Call them
inside an entity update.

## `Window`: one window

A [`Window`](crate::Window) is *not* a context of its own, yet you'll see it traveling
through one, as render methods receive `(&mut self, &mut Window, &mut Context<Self>)`,
and element event handlers receive `(&Event, &mut Window, &mut App)`.

Keeping the window separate from the app lets one callback use both. You can
move focus with [`Window::focus`](crate::Window::focus) while updating an entity
through `cx`; neither operation has to hide behind the other context.

Use the window for things that only make sense in one window: its size and scale
factor, the focused element, the current modifier keys, the mouse position,
painting, and scheduling work associated with the window. With a
[`VisualContext`](crate::VisualContext),
[`Entity::update_in`](crate::Entity::update_in) updates an entity with access to
the window too. The trait also provides entity creation with window access,
root-view replacement, and focus changes; see
[Writing code for any context](#writing-code-for-any-context).

## Async contexts

A future cannot hold `&mut App` across an `.await`: while it waits, the rest of
the application must keep running and needs that access. Futures started with
[`App::spawn`](crate::App::spawn) or [`Context::spawn`](crate::Context::spawn)
receive an [`AsyncApp`](crate::AsyncApp) instead. It refers to the app weakly
and borrows it only for the duration of each call you make through it:

```
# use gpui::{AppContext, Context, Task};
# struct Search { results: Vec<String>, task: Option<Task<()>> }
# async fn fetch(query: String) -> Vec<String> { vec![query] }
impl Search {
    fn run(&mut self, query: String, cx: &mut Context<Self>) {
        self.task = Some(cx.spawn(async move |search, cx| {
            let results = fetch(query).await;
            search
                .update(cx, |search, cx| {
                    search.results = results;
                    cx.notify();
                })
                .ok();
        }));
    }
}
```

Two details in this example come from the async setting:

* `search` is a [`WeakEntity`](crate::WeakEntity). The task does not keep the
  view alive, so `update` returns a `Result` that is an error if the view was
  released while the task waited.
* Each `update` is a complete update: effects such as `notify` are flushed
  before it returns, as in a synchronous callback.

[`Context::spawn_in`](crate::Context::spawn_in) gives you an
[`AsyncWindowContext`](crate::AsyncWindowContext) instead, for work that needs
to reach the window after it awaits. Window operations through it return errors
once the window has closed.

An `AsyncApp` refers to the application weakly, like a [`WeakEntity`] does to an
entity. Using it after the application itself has been dropped panics.

## Writing code for any context

Entity operations are defined on the [`AppContext`](crate::AppContext) trait,
which every context implements. Write helpers against the trait when they only
need to create, read or update entities, and they'll work from synchronous and
asynchronous code alike:

```
use gpui::{AppContext, Entity};

fn reset<C: AppContext>(count: &Entity<u64>, cx: &mut C) {
    count.update(cx, |count, cx| {
        *count = 0;
        cx.notify();
    });
}
# gpui::TestApp::new().update(|cx| {
#     let count = cx.new(|_| 5_u64);
#     reset(&count, cx);
#     assert_eq!(*count.read(cx), 0);
# });
```

[`VisualContext`](crate::VisualContext) extends `AppContext` with operations
that need a window. Import both through [`gpui::prelude`](crate::prelude).

## Borrowing through a context

A context is a mutable borrow of the application, so the borrow checker applies
to it. You cannot hold a reference returned by
[`Entity::read`](crate::Entity::read) while you use the same context mutably:

```compile_fail
# use gpui::{AppContext, TestApp};
# TestApp::new().update(|cx| {
let name = cx.new(|_| String::from("GPUI"));
let borrowed: &String = name.read(cx);
name.update(cx, |name, _| name.push('!')); // error: `cx` is already borrowed
println!("{borrowed}");
# });
```

Copy what you need out of the entity first, or do the work inside a single
update. This rule is what makes entity access safe without locks: there is only
ever one `&mut App`, so only one piece of code can be changing state at a time.

Next, [the frame](crate::_documentation::concepts::the_frame) follows a change
from `notify` to the pixels on screen.
