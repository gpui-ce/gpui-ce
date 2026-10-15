# Troubleshooting

Start with the symptom. For missing input, check that the target has a hit area
or a focus path before investigating dispatch order. For missing updates or
async work, check notification and handle ownership. Each entry links to the
concept that explains the underlying rule.

## The view doesn't update

**Cause.** A field assignment changes the value but sends no notification. An
unrelated redraw can expose the new value later, making the bug intermittent.
[`Context::notify`] schedules observers and invalidates ordinary views that
read the entity; cached subtrees have a separate rule below.

**Fix.** Call `cx.notify()` after the change, using the context belonging to the
entity whose state changed:

```rust
use gpui::Context;

struct Counter {
    count: usize,
}

impl Counter {
    fn increment(&mut self, cx: &mut Context<Self>) {
        self.count += 1;
        cx.notify();
    }
}
```

Notify the entity that owns the changed state. Notifying a view that happens to
render the model does not invalidate the model or notify its observers. The same
rule matters for an [`Entity::cached`] subtree: reading another entity does not
register a cache dependency, so observe that model and notify the cached view.
[The frame](crate::_documentation::concepts::the_frame#rendering-less) explains
that distinction and the other cache invalidation conditions.

## "cannot update … while it is already being updated"

**Cause.** An `update` has temporarily removed the entity's value from storage.
Reading or updating that entity through another handle cannot retrieve it
until the update returns. Cloning the handle does not create another value.
The panic message reads
`cannot update Counter while it is already being updated`, or `cannot read` for
a read. A common trigger is a handle captured before the update and used again
inside it:

- Inside `counter.update(cx, |this, cx| ...)`, you call `counter.read(cx)` or
  `counter.update(cx, ...)` on the same entity, through a clone of its handle.
- An event handler for entity A updates entity B, and B's handler updates A.

Updates to different entities are allowed; the cycle fails when it returns
to the entity already being updated. The
[ownership concept](crate::_documentation::concepts::ownership) explains these
temporary leases.

**Fix.** Use the `this` argument for the current entity. When a chain of updates
would return to it, defer that work until its lease has ended:

```rust
use gpui::{App, Entity};

fn bump_later(counter: Entity<usize>, cx: &mut App) {
    // Runs after the current update has finished and released its lease.
    cx.defer(move |cx| counter.update(cx, |count, _| *count += 1));
}
```

## A click handler never runs

**Cause.** Work from the target outward: is the handler on the intended
element, does that element occupy any space, and can the click reach it?

- The element has no [`id`][InteractiveElement::id]. [`on_click`][StatefulInteractiveElement::on_click]
  is only available on elements with an id, so the code won't compile without
  one. If you attached the handler to a different element, check that one.
- An occluding element blocks the target. Normal overlapping hitboxes can both
  receive an event, front to back. [`occlude`][InteractiveElement::occlude]
  blocks hitboxes behind it; an earlier listener can also stop propagation.
- The element has no size. A zero-width or zero-height element has nothing to
  hit. Check its width and height with the inspector.

**Fix.** Attach the listener to the element you sized and identified. Give it
`.id(...)` and dimensions through `size`, `w`, `h`, or its children. If another
element covers it, decide which should receive the click: ordinary hitboxes
can share the event, while `occlude` prevents listeners behind the covering
element from receiving it.

## A key binding does nothing

**Cause.** Binding lookup and action dispatch are separate steps. A contextual
shortcut first needs a matching binding, then a reachable action handler:

- The intended element is not on the focus path. Attach its handle with
  `track_focus`, then call `window.focus(&handle, cx)`. App-wide actions have a
  separate listener path, but a keyboard shortcut still needs a binding that
  matches the current context stack; with no focus, that means a context-free
  binding.
- The binding's key context doesn't match. A binding added with a context such as
  `Some("Editor")` applies only when an element on the focus path calls
  `key_context("Editor")`.
- A deeper binding wins. When two bindings match the same keystroke, the one
  whose context is deeper on the focus path wins, and the last one added wins a
  tie. A binding with no context counts as matching at the deepest level, so it
  can override a binding on an ancestor.
- An earlier handler consumed the action. Element action handlers in the bubble phase do so by
  default unless they call `cx.propagate()`. An explicit
  `cx.stop_propagation()` can also end dispatch before the intended listener.

**Fix.** Check focus and the tracked handle, then the key context and binding
precedence, and finally whether a handler stopped propagation. The [input
dispatch concept](crate::_documentation::concepts::input_dispatch) describes
the order in full and includes a working example.

## A scroll container doesn't scroll

**Cause.** Scrolling needs stored offset and overflowing content. Without an
[`id`][InteractiveElement::id], wheel events cannot retain an offset between
frames. Without a height bound, the container grows to fit its children. In a
flex column, the child's minimum height can prevent that bound from shrinking it.

**Fix.** Give the container an id, set `overflow_y_scroll()` (or
`overflow_scroll()` for both axes), and bound its height with `h`, `h_full` or
`max_h` on it or on an ancestor:

```rust
use gpui::{div, prelude::*, px};

let messages = div()
    .id("messages")
    .max_h(px(300.))
    .overflow_y_scroll()
    .children((0..100).map(|index| div().child(format!("message {index}"))));
```

When the container is a flex child inside a column, also add `min_h_0`, as the
[layout example](crate::_documentation::examples::layout) does.

## A flex child overflows instead of shrinking

**Cause.** The parent asks its flex child to shrink, but the child's default
minimum is its content size. A long label can therefore push past the parent
even when the child has a shrink factor.

**Fix.** Allow a smaller minimum with `min_w_0` in a row or `min_h_0` in a
column. Then choose how the child handles its excess content, such as
scrolling or text truncation. In this row, the sidebar keeps its width while
the label may shrink:

```rust
use gpui::{div, prelude::*, px};

let row = div()
    .flex()
    .child(div().w(px(200.)).child("Sidebar"))
    .child(div().flex_1().min_w_0().child("A label that is much longer than the space it has"));
```

## An async task does not run or complete

**Cause.** Check where the returned [`Task`] goes. `cx.spawn(...);` discards
it at the semicolon and cancels the future, possibly before its first poll.
Replacing a stored task cancels the old one too. Cancellation cannot interrupt
synchronous code already running; it takes effect when that code returns
control to the scheduler.

**Fix.** Retain the handle to control cancellation, or await it to obtain the
result. Use `.detach()` for a future that handles its own result or side
effects and may outlive this scope. Detachment gives up the return value; app
or process shutdown can still prevent completion:

```rust
use gpui::App;

fn start(cx: &mut App) {
    cx.background_executor()
        .spawn(async {
            // work that runs to completion
        })
        .detach();
}
```

An `.await` yields to other work only when it returns `Pending`; an already-ready
await continues in the same poll. On native targets the background executor can
use worker threads, but a single-threaded WebAssembly build runs it on the
browser's main thread, so long synchronous work still competes with rendering.
In tests, call [`TestApp::run_until_parked`] to give ready tasks a turn. See the
[testing guide](crate::_documentation::guides::testing). For the polling and
cancellation contract, see [executors and tasks](crate::_documentation::concepts::executors).

## Panic: "Action with name … already registered"

**Cause.** Two crates can declare different Rust types that both register as
`editor::Save`. The action registry uses namespace and type name, which must
be unique across the binary. Creating an app loads that registry and exposes
the collision. A `deprecated_aliases` entry can also collide with another
action's name.

**Fix.** Give each set of actions its own namespace: `actions!(editor, [Save])`
and `actions!(files, [Save])` produce `editor::Save` and `files::Save`. Remove
the alias if its name is already taken. The
[input dispatch concept](crate::_documentation::concepts::input_dispatch) shows
how actions are declared and bound.

## Text is invisible on the web

**Cause.** Desktop GPUI finds system fonts; the web backend starts with an
empty font database. It cannot use the browser's installed fonts or supply a
fallback until the app registers font bytes.

**Fix.** Add the font bytes to the text system before you open a window. Call
`add_fonts` on the text system from [`App::text_system`], with font data you
bundle with the app or fetch before startup.

## The window closes but the app keeps running on macOS

**Cause.** [`QuitMode::Default`] follows macOS convention: closing the last
window leaves the app running, and a menu or another handler must call
[`App::quit`]. This is [`QuitMode::Explicit`] behavior. On Linux and Windows,
the default is [`QuitMode::LastWindowClosed`], which ends the app when its last
window closes.

**Fix.** Choose the mode you want when you create the application:

```rust,no_run
use gpui::QuitMode;

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|_cx| {
            // Open the first window here.
        });
}
```

## Entities leak

**Cause.** A reference cycle keeps every entity in it alive, so none is ever
dropped. One common shape is a parent that holds a strong `Entity` to a child,
while the child holds a strong `Entity` back to the parent. A subscription or an
observer closure stored on `self` has the same effect if its closure captures a
strong handle to `self`.

**Fix.** Make one side of the link weak. Store a [`WeakEntity`] (from
`Entity::downgrade`) for the back-reference, and call `update` on it, which
returns a `Result` that fails if the entity is gone:

```rust
use gpui::{Entity, WeakEntity};

struct Parent {
    child: Entity<Child>,
}

struct Child {
    parent: WeakEntity<Parent>,
}
```

To find leaks in tests, build with the `leak-detection` feature (the
`test-support` feature enables it). Take a snapshot before the code under test
runs, and assert afterward that no new entity is still alive:

```rust
use gpui::TestApp;

let mut app = TestApp::new();
let snapshot = app.read(|cx| cx.leak_detector_snapshot());

let entity = app.new_entity(|_| 0usize);
drop(entity);

app.read(|cx| cx.assert_no_new_leaks(&snapshot));
```

`assert_no_new_leaks` panics and lists each leaked entity with its type. Set
`LEAK_BACKTRACE` to include the backtrace where each one was created.

## "returns a reference to a captured variable which escapes the closure body"

**Cause.** You call a method that returns `impl IntoElement` from inside a
closure, usually one passed to `map` to build a list:

```text
.children(self.items.iter().map(|item| self.render_item(item, cx)))
```

Rust 2024 captures all in-scope generic parameters in a return-position
`impl Trait`, including the argument lifetimes. This can tie the returned
type to the borrowed `cx` even when the concrete element owns its data. The
closure then appears to return a borrow of its captured mutable context.

**Fix.** Exclude the argument lifetimes with a precise capture bound,
`+ use<>`:

```
# use gpui::{Context, IntoElement, div, prelude::*};
# struct List { items: Vec<String> }
# impl List {
fn render_item(&self, item: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    div().child(item.to_string())
}
# }
```

The `Div` returned here owns its text. `use<>` only changes which parameters
the return type captures; it does not turn a borrow into owned data. The
concrete `Element` associated with this return type is `'static`, although an
`IntoElement` wrapper can borrow data before converting to that element. In a
generic method, include the in-scope type and const parameters, such as `use<T>`, as required by
[Rust's precise capture rules](https://doc.rust-lang.org/reference/types/impl-trait.html).

## See also

- [Ownership and entities](crate::_documentation::concepts::ownership) explains
  the leases and weak handles behind re-entrant access and reference cycles.
- [Input and dispatch](crate::_documentation::concepts::input_dispatch) traces
  the focus path, precedence and propagation behind key and click failures.
- [Testing](crate::_documentation::guides::testing) shows how to reproduce
  timing-sensitive symptoms in a headless test.
