# Test your app

Use [`TestApp`] to test views, entities and asynchronous work without opening a real
window. It provides a headless app and a deterministic scheduler. Use the
[`gpui::test`](macro@crate::test) macro with [`TestAppContext`] when a test needs an
async function, several app instances or randomized scheduling.

## Enable the test support

GPUI's test helpers are behind the `test-support` feature. Enable it only for
tests, as a dev-dependency:

```toml
[dev-dependencies]
gpui = { package = "gpui-ce", version = "0.2", features = ["test-support"] }
```

Add `rand` as a dev-dependency too if you write `#[gpui::test]` functions that
take a random number generator, because the macro refers to `rand` by name.

## Test a view with TestApp

[`TestApp::open_window`] builds a root view in a headless window and returns a
[`TestAppWindow`]. Use [`TestAppWindow::update`] and [`TestAppWindow::read`] to
change and inspect the view, and the `simulate_*` methods to send input.

```rust
use gpui::{Context, MouseButton, TestApp, Window, div, point, prelude::*, px};

struct Counter {
    count: usize,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("increment")
            .size(px(100.))
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.count += 1;
                cx.notify();
            }))
            .child(format!("{}", self.count))
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Counter { count: 0 });

// Lay out and paint the view once, so the click has bounds to hit.
window.draw();
window.simulate_click(point(px(10.), px(10.)), MouseButton::Left);

window.read(|counter, _| assert_eq!(counter.count, 1));
```

Each `TestAppWindow` method flushes the effects it causes before returning. The
click invokes `on_click`; `cx.notify()` schedules a redraw, and the final `read`
observes the updated view.

Positions are in window coordinates, so the click above lands inside the 100-pixel square
at the window's origin. Call `draw` before clicking: hit testing uses the last painted
frame.

### Simulate input

| Method | Simulates |
| --- | --- |
| [`simulate_keystrokes`][TestAppWindow::simulate_keystrokes] | Keystrokes separated by spaces, such as `"ctrl-s"` or `"cmd-k cmd-s"`. |
| [`simulate_keystroke`][TestAppWindow::simulate_keystroke] | One keystroke. |
| [`simulate_input`][TestAppWindow::simulate_input] | Typing: one keystroke per character. |
| [`simulate_mouse_move`][TestAppWindow::simulate_mouse_move] | Moving the pointer. |
| [`simulate_mouse_down`][TestAppWindow::simulate_mouse_down] and [`simulate_mouse_up`][TestAppWindow::simulate_mouse_up] | One button press or release. |
| [`simulate_click`][TestAppWindow::simulate_click] | A press and a release at one position. |
| [`simulate_scroll`][TestAppWindow::simulate_scroll] | A scroll wheel delta at a position. |
| [`simulate_resize`][TestAppWindow::simulate_resize] | Resizing the window. |
| [`draw`][TestAppWindow::draw] | A forced frame, so later hit tests see the current layout. |

`simulate_keystrokes` splits on spaces, so you can't use the space key in it.
Use `simulate_keystroke("space")` for that.

Keyboard tests use the same dispatch path as the application. Bind keys with
[`App::bind_keys`], focus the element you want, and send the keystroke:

```standalone_crate
use gpui::{Context, FocusHandle, KeyBinding, TestApp, Window, actions, div, prelude::*};

actions!(testing_guide, [Save]);

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
window.read(|editor, _| assert_eq!(editor.saves, 1));
```

The `actions!` call declares an action by name. Action names must be unique
within a binary, so the example uses its own namespace, `testing_guide`. In your
own project, give each crate that declares actions its own namespace.

Windows in a test start inactive. Keystrokes reach the focused element anyway,
but focus listeners such as [`Context::on_focus`] only run in the active window,
so call [`TestAppWindow::activate`] before testing them.

## Test entities and events

Entities are testable without any window. Use [`TestApp::new_entity`],
[`TestApp::update_entity`] and [`TestApp::read_entity`] to work with them directly.

Event delivery happens after the outermost update returns. Subscribe in one
[`TestApp::update`], emit or notify in another, and assert afterward.
[`App::subscribe`] and [`App::observe`] run their callbacks when the outermost
update finishes, so the assertion must come after the update that emits or
notifies:

```rust
use std::{cell::Cell, rc::Rc};

use gpui::{Context, EventEmitter, TestApp, prelude::*};

struct Counter {
    count: usize,
}

struct CountChanged(usize);

impl EventEmitter<CountChanged> for Counter {}

impl Counter {
    fn increment(&mut self, cx: &mut Context<Self>) {
        self.count += 1;
        cx.emit(CountChanged(self.count));
        cx.notify();
    }
}

let mut app = TestApp::new();
let counter = app.new_entity(|_| Counter { count: 0 });

let last_seen = Rc::new(Cell::new(0));
let seen = last_seen.clone();
// Keep the subscription alive for as long as you want to receive events.
let _subscription = app.update(|cx| {
    cx.subscribe(&counter, move |_, event: &CountChanged, _| seen.set(event.0))
});

app.update_entity(&counter, |counter, cx| counter.increment(cx));

assert_eq!(last_seen.get(), 1);
app.read_entity(&counter, |counter, _| assert_eq!(counter.count, 1));
```

Drop the subscription to stop receiving events. A subscription that is not
stored is dropped at the end of its statement, so the callback never runs.

### Globals, clipboard and prompts

[`TestApp`] also simulates the platform services that an app uses:

```rust
use gpui::{ClipboardItem, Global, TestApp};

struct Theme(&'static str);
impl Global for Theme {}

let mut app = TestApp::new();
app.set_global(Theme("dark"));
app.read_global::<Theme, _>(|theme, _| assert_eq!(theme.0, "dark"));

app.write_to_clipboard(ClipboardItem::new_string("copied".into()));
let pasted = app.read_from_clipboard().and_then(|item| item.text());
assert_eq!(pasted.as_deref(), Some("copied"));
```

Prompts work the same way. A call to [`Window::prompt`] records the prompt and
returns a receiver. The test answers it by the label of a button:

```rust
use gpui::{PromptLevel, TestApp};

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| gpui::Empty);

let mut answer = window.update(|_, window, cx| {
    window.prompt(PromptLevel::Warning, "Discard changes?", None, &["Discard", "Cancel"], cx)
});

assert!(app.has_pending_prompt());
app.simulate_prompt_answer("Discard");
assert_eq!(answer.try_recv().unwrap(), Some(0));
```

`simulate_prompt_answer` takes the label, not the index, and panics if no label
matches. [`TestApp::opened_url`] reports a URL that the app asked the platform to
open, so you can assert on links without launching a browser.

## Test async code

A spawned task runs only when the test scheduler gives it a turn. Advance that
scheduler explicitly:

- [`TestApp::run_until_parked`] runs every task that can make progress now.
  Timers that haven't expired don't count as progress.
- [`TestApp::advance_clock`] moves the simulated clock forward and fires the
  timers that have expired.

```rust
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::TestApp;

let mut app = TestApp::new();
let finished = Rc::new(Cell::new(false));

let flag = finished.clone();
app.spawn(|cx| async move {
    cx.background_executor().timer(Duration::from_secs(1)).await;
    flag.set(true);
})
.detach();

app.run_until_parked();
assert!(!finished.get(), "the timer hasn't expired yet");

app.advance_clock(Duration::from_secs(1));
app.run_until_parked();
assert!(finished.get());
```

[`TestApp::spawn`] runs a future on the foreground executor, with an [`AsyncApp`]
handle. [`TestApp::background_spawn`] runs one on the background executor. Both
return a [`Task`], which you must await, store or detach, as in a real app.
See [async work](crate::_documentation::guides::async_work) for the rules.

## Use the test macro

[`gpui::test`](macro@crate::test) replaces `#[test]` for tests that need GPUI. It creates a
[`TestAppContext`], a test [`App`] with extra helpers, and passes it to your function.
The function may be `async`:

```rust
use gpui::{AppContext, TestAppContext};

#[gpui::test]
async fn test_counts_are_positive(cx: &mut TestAppContext) {
    let count = cx.new(|_| 1usize);
    cx.run_until_parked();
    cx.read(|cx| assert!(*count.read(cx) > 0));
}
```

The macro also accepts `&TestAppContext` parameters, and [`BackgroundExecutor`]
to spawn tasks directly.

### Test a window with VisualTestContext

[`TestAppContext::add_window_view`] opens a window with a root view. It returns
the view's entity and a [`VisualTestContext`], which you use as a window and an
app. Inputs go to that window, and `read_with` works on the view:

```rust
use gpui::{Context, MouseButton, Modifiers, TestAppContext, Window, div, point, prelude::*, px};

struct Counter {
    count: usize,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size(px(100.))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.count += 1;
                cx.notify();
            }))
    }
}

#[gpui::test]
fn clicking_counts(cx: &mut TestAppContext) {
    let (counter, cx) = cx.add_window_view(|_, _| Counter { count: 0 });
    cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());
    counter.read_with(cx, |counter, _| assert_eq!(counter.count, 1));
}
```

Use `add_empty_window` when you want a window with no root view, and
[`VisualTestContext::draw`] to paint an element by itself.
[`VisualTestContext::debug_bounds`] returns where an element was painted, if you
gave it a selector with [`InteractiveElement::debug_selector`]. Use it to click an
element without guessing its position.

### Use several apps in one test

Each `&TestAppContext` parameter gets its own app. The apps share one scheduler,
so their tasks interleave deterministically. Use [`TestAppContext::new_app`] to add an app
inside a test that already has one.

```rust
use gpui::{Global, TestAppContext};

struct Name(&'static str);
impl Global for Name {}

#[gpui::test]
fn apps_are_independent(cx_a: &mut TestAppContext, cx_b: &mut TestAppContext) {
    cx_a.update(|cx| cx.set_global(Name("alice")));
    cx_b.update(|cx| cx.set_global(Name("bob")));

    cx_a.read_global::<Name, _>(|name, _| assert_eq!(name.0, "alice"));
    cx_b.read_global::<Name, _>(|name, _| assert_eq!(name.0, "bob"));
}
```

### Randomize scheduling

By default, a test runs once with seed `0`. The seed chooses the order in which ready
tasks run, so each seed gives a different, repeatable interleaving. Request more
iterations to exercise more orderings:

```rust
use gpui::TestAppContext;

#[gpui::test(iterations = 50)]
async fn test_both_tasks_finish(cx: &mut TestAppContext) {
    // Each iteration uses a different seed, so the two tasks may finish in
    // either order. The result must not depend on that order.
    let first = cx.executor().spawn(async { 1 });
    let second = cx.executor().spawn(async { 2 });
    assert_eq!(first.await + second.await, 3);
}
```

The test process controls these runs through environment variables:

- `ITERATIONS=n` overrides the `iterations` count.
- `SEED=n` runs only seed `n`, or, with more than one iteration, the seeds
  `n` to `n + iterations - 1`. Use it to replay a failure. When the test runs
  more than one seed, the failing seed is printed.

Other arguments to `#[gpui::test]` are `seed = n` or `seeds(1, 2, 3)` to always
run given seeds, `retries = n` to rerun a failing seed up to `n` times, and
`on_failure = "path::to::fn"` to call a function after the last failure, for
example to dump state.

### Property tests

[`gpui::property_test`] runs a test over generated inputs and shrinks a failure to the
smallest input that still fails. It uses proptest and the generated code names that
crate, so add it to your dev-dependencies:

```toml
[dev-dependencies]
proptest = { version = "1.10", features = ["attr-macro"] }
```

```rust
#[gpui::property_test]
fn sorting_is_idempotent(values: Vec<i32>) {
    let mut once = values;
    once.sort();
    let mut twice = once.clone();
    twice.sort();
    assert_eq!(once, twice);
}
# fn main() {}
```

Arguments of type `&mut TestAppContext` and [`BackgroundExecutor`] work as they
do with `#[gpui::test]`. Don't take a `StdRng` argument: it stops shrinking
from working. Give a parameter a custom generator with `#[strategy = ...]`, or
set the number of cases with `config = ...`:

```rust
use proptest::prelude::ProptestConfig;

#[gpui::property_test(config = ProptestConfig { cases: 32, ..ProptestConfig::default() })]
fn small_inputs(#[strategy = 0..10usize] len: usize) {
    assert!(len < 10);
}
# fn main() {}
```

Property tests use `SEED`, which defaults to `0`; leave it unchanged for a
repeatable run or set it when reproducing a failure.

## See also

- The [keyboard guide](crate::_documentation::guides::keyboard) and the
  [input dispatch concept](crate::_documentation::concepts::input_dispatch)
  explain the keystrokes and actions you simulate here.
- [Async work](crate::_documentation::guides::async_work) describes how tasks
  are owned and cancelled.
- The [tutorial's testing chapter](crate::_documentation::tutorial::ch8_testing)
  builds a small app with tests from start to finish.
