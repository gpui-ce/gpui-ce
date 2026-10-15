# Keep your app fast

An idle window needs no new frames. During a busy interaction, however, GPUI
still has to rebuild views, lay out elements, and paint each requested frame.
Measure that work before choosing an optimization: a cached sidebar avoids
rebuilding its subtree, while a virtualized list avoids laying out offscreen
rows. Neither helps a click handler that blocks the main thread.

## Know where a frame's time goes

Each frame walks the element tree in four phases. See [the frame](crate::_documentation::concepts::the_frame)
for the details.

| Phase | Work it does | What makes it slow |
| --- | --- | --- |
| Render | Calls `render` on each view that needs a new frame | Building large element trees, parsing data, or reading files |
| Request layout | Describes each element's size to the layout engine | Very deep trees, and text that must be measured |
| Prepaint | Computes final bounds and hitboxes | Many elements that take part in mouse handling |
| Paint | Writes primitives into the scene | Many primitives, large images, and paths |

In `render`, build elements from state you already hold. Reading a file or
querying a database there delays the whole frame; prepare those results with
[async work](crate::_documentation::guides::async_work) instead. Avoid redundant
notifications too, while retaining the notifications that observers need.

## Measure before you change anything

Build with the `profiler` feature to see frame times on screen. Enable it in a profiling
build because it adds bookkeeping to every frame:

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.2", features = ["profiler"] }
```

With the feature enabled, each window can show the debug frame overlay. The overlay is a
small panel painted into the scene, so it does not schedule frames by itself. Its modes
are `DebugFrameOverlayMode::Hidden` (the default), `Minimal` and `Full`:

- `Minimal` shows the duration of the last frame, measured around the window's
  draw.
- `Full` shows the last frame, the 1% and 10% percentiles (the 99th and 90th),
  the maximum, and the number of frames drawn since the window opened.

`Window::set_debug_frame_overlay_mode` sets the mode, and
`Window::reset_debug_frame_overlay_stats` clears the samples:

```rust
#[cfg(feature = "profiler")]
fn show_frame_times(window: &mut gpui::Window) {
    use gpui::DebugFrameOverlayMode;

    window.set_debug_frame_overlay_mode(DebugFrameOverlayMode::Full);
    // Clear the samples before a measurement, so the percentiles cover only
    // the interaction you're testing.
    window.reset_debug_frame_overlay_stats();
}
```

GPUI doesn't bind a key to the overlay. Bind an action that calls
`cycle_debug_frame_overlay_mode` if you want to switch modes while the app runs.
`Window::frame_duration_snapshot` returns the same kind of measurements as
histograms, for logging or benchmarks. It has the time from the first change to
the frame being presented, the time spent in `draw`, and the interval between
frames while the window is animating.

## Cache a view that doesn't change

When a view's output depends only on its own state, you can keep its rendered
subtree between frames. [`Entity::cached`] does this. It takes the style that
lays the view out:

```rust
use gpui::{Context, Entity, Render, StyleRefinement, Window, div, prelude::*, px};

struct Sidebar {
    items: Vec<&'static str>,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.items.iter().map(|item| div().child(*item)))
    }
}

struct Root {
    sidebar: Entity<Sidebar>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size(px(800.))
            .child(
                self.sidebar
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
    }
}
```

A cached view is rendered again when one of these happens:

- Its entity is notified with [`Context::notify`]. Calling `cx.notify()` in the
  view itself is the usual case.
- Its bounds, its content mask, its text style, its text direction or its Unicode
  bidirectional text mode change.
- The window is refreshed with [`Window::refresh`], which ignores the cache.

Three things follow:

- Caching needs a definite size. GPUI lays the cached view out from the style you
  pass and doesn't measure its contents. Use a size such as `size_full()`, or a
  fixed width and height, on the cached view.
- The cache doesn't observe arbitrary dependencies. A parent that passes new
  values through a field on the view, or an entity the cached view reads, doesn't
  refresh a cached child by itself. Put the data in the child's own entity, or
  observe the dependency and notify the cached entity when it changes.
- Caching is off while the inspector is choosing an element, so the picked
  element stays live.

The test below shows why a dependency needs an observer. The dependency is read
while `Sidebar` renders, but changing it without notification leaves the cached
subtree untouched. The retained [`Subscription`] turns a later notification into
`Sidebar`'s own [`Context::notify`], which refreshes the cache.

```rust
use std::{cell::Cell, rc::Rc};

use gpui::{Context, Entity, Render, StyleRefinement, Subscription, TestApp, Window, div, prelude::*, px};

struct Sidebar {
    dependency: Entity<usize>,
    dependency_subscription: Subscription,
    renders: Rc<Cell<usize>>,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = *self.dependency.read(cx);
        self.renders.set(self.renders.get() + 1);
        div().size_full().child(value.to_string())
    }
}

struct Root {
    sidebar: Entity<Sidebar>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(800.)).child(
            self.sidebar
                .clone()
                .cached(StyleRefinement::default().size_full()),
        )
    }
}

let renders = Rc::new(Cell::new(0));
let mut app = TestApp::new();
let dependency = app.update(|cx| cx.new(|_| 0_usize));
let mut window = app.open_window({
    let dependency_for_sidebar = dependency.clone();
    let renders = renders.clone();
    move |_, cx| {
        let sidebar = cx.new(|cx| {
            let dependency_subscription =
                cx.observe(&dependency_for_sidebar, |_, _, cx| cx.notify());
            Sidebar {
                dependency: dependency_for_sidebar,
                dependency_subscription,
                renders,
            }
        });
        Root { sidebar }
    }
});
window.draw();
let after_first_frame = renders.get();

window.draw();
assert_eq!(renders.get(), after_first_frame, "an unchanged cache is reused");

app.update(|cx| dependency.update(cx, |value, _| *value = 1));
window.draw();
assert_eq!(renders.get(), after_first_frame, "mutation without notification leaves the cache intact");

app.update(|cx| dependency.update(cx, |value, cx| {
    *value = 2;
    cx.notify();
}));
window.draw();
assert_eq!(renders.get(), after_first_frame + 1, "the observer notifies the cached view");
```

Cache a view when it is expensive and rarely changes, such as a sidebar, document preview
or chart. A view that you notify every frame gains nothing from caching.

## Render only the rows on screen

A long list that renders every row costs time in every frame, even when only a few rows
are visible. Choose a virtualized list based on the row heights:

**[`uniform_list`]** is for rows that all have the same height. It renders only
the range of rows that is visible. You provide the item count and a function
that renders a range:

```rust
use std::sync::Arc;

use gpui::{Context, Render, SharedString, Window, div, prelude::*, px, uniform_list};

struct Contacts {
    names: Arc<Vec<SharedString>>,
}

impl Render for Contacts {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Cloning the `Arc` is cheap. The closure below must own its data,
        // because the list calls it later, during layout.
        let names = self.names.clone();
        div().h(px(400.)).overflow_y_hidden().child(
            uniform_list("contacts", names.len(), move |range, _window, _cx| {
                range
                    .map(|index| div().h(px(24.)).child(names[index].clone()))
                    .collect::<Vec<_>>()
            })
            .size_full(),
        )
    }
}
```

Put the list in a container with a fixed or maximum height and
`overflow_y_hidden`, as in the example. The list calls your function with the
range of visible rows each frame, so the work done per frame depends on the
number of visible rows, not on the number of items.

**[`list`]** is for rows of different heights. It needs a [`ListState`] that
records each row's height as the rows are measured. Tell the state when the
rows change: [`ListState::reset`] when the whole list is replaced, and
[`ListState::splice`] when rows are inserted or removed. For content changes that
can alter existing row heights, call [`ListState::remeasure_items`].
Otherwise the list draws with stale heights and scrolls to the wrong place.

Both lists render only the rows on screen, plus the overdraw margin you give them. A plain
`div` containing every row pays the cost of every row on every frame.

## Avoid redundant notifications

[`Context::notify`] announces an entity's change to its observers and marks each
window whose last frame read the entity as needing a redraw. Several notifies
before the next frame cost one frame.

Notify once after a batch of changes. An unchanged value needs no announcement,
as the zero increment below shows. A change can still need notification when no
window displays it: an observer may derive other state from the new value.

```rust
use gpui::Context;

struct Counter {
    count: usize,
    history: Vec<usize>,
}

impl Counter {
    fn add(&mut self, amount: usize, cx: &mut Context<Self>) {
        if amount == 0 {
            return; // nothing changed, so there's nothing to draw
        }
        self.count += amount;
        self.history.push(self.count);
        cx.notify(); // one notify for the whole change
    }
}
```

Do not call `notify` from `render`; it requests another frame immediately. If only the
hovered item changes, keep that state in the element that displays it, or notify only
when the hovered item changes instead of on every mouse move.

Notifying also triggers observers, so an observer that does heavy work runs on
every notify. Keep observers light, and move expensive work into a background
task, as described below.

## Move slow work off the main thread

GPUI runs view code on one foreground thread. Work that takes more than a few
milliseconds there—parsing a large file, searching an index or compressing an image—
delays the next frame. Run it on the background executor and update the entity when the
result is ready.

[`Context::spawn`] runs an async function on the foreground thread with a weak
handle to the entity and an [`AsyncApp`]. Inside it, [`AsyncApp::background_spawn`]
runs the slow part on a worker thread. Await it, and then update the entity:

```rust
use gpui::{AppContext, Context};

struct Index {
    results: Vec<String>,
    searching: bool,
}

impl Index {
    fn search(&mut self, query: String, cx: &mut Context<Self>) {
        self.searching = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let results = cx
                .background_spawn(async move { expensive_search(&query) })
                .await;

            this.update(cx, |this, cx| {
                this.results = results;
                this.searching = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn expensive_search(query: &str) -> Vec<String> {
    vec![format!("result for {query}")]
}
```

The boundary between the executors imposes three rules:

- The future passed to [`AppContext::background_spawn`] must be `Send`, so it
  can't capture entities or anything else tied to the foreground thread. Move
  plain data in, and send plain data back.
- A [`Task`] that isn't stored or detached is cancelled when it's dropped. The
  `detach()` above keeps the task running.
- Show progress while the work runs, as the `searching` flag does, instead of
  leaving the old state on screen.

## Avoid copying large values

Cloning is cheap for the types GPUI uses for shared state, and expensive for the
ones you own:

- [`Entity`] is a handle, so cloning it doesn't copy the state it refers to.
  Store entities in views and pass clones to closures.
- [`SharedString`] is a string that is cheap to clone. Use it for labels and
  text that are passed around and rendered many times.
- For large data that several places read, wrap it in `Arc`. Clone the `Arc`
  instead of the data. Clone the data itself only when you need to change it,
  for example with `Arc::make_mut`.

```rust
use std::sync::Arc;

use gpui::SharedString;

struct Row {
    label: SharedString,
    pixels: Arc<Vec<u8>>,
}

let label: SharedString = "Inbox".into();
let pixels = Arc::new(vec![0u8; 4 * 1024 * 1024]);

// Each row shares the same buffer. Only the handles are copied.
let rows: Vec<Row> = (0..3)
    .map(|_| Row { label: label.clone(), pixels: pixels.clone() })
    .collect();
assert_eq!(Arc::strong_count(&pixels), 4);
assert_eq!(&*rows[0].label, "Inbox");
```

Avoid building a new `String` or `Vec` inside `render` for every frame. Build it
once in your state when its input changes, and render from there.

## Inspect the element tree

When a view looks wrong, or you want to know which element a pixel belongs to,
use the inspector. You pick an element on screen and it shows that element's
details.

The inspector is built in debug builds and in builds with the `inspector` feature. Open
it with `Window::toggle_inspector`:

```rust
use gpui::{App, Window};

fn toggle_inspector(window: &mut Window, cx: &mut App) {
    window.toggle_inspector(cx);
}
```

GPUI doesn't bind a key to the inspector, so add an action that calls
`toggle_inspector` and bind it in your app. While the inspector is picking an
element, [`Window::is_inspector_picking`] returns `true`. Use it when your own
code should act differently during picking.

## See also

- [The frame](crate::_documentation::concepts::the_frame) explains when frames
  run and what each phase does.
- [Async work](crate::_documentation::guides::async_work) covers tasks and their
  ownership.
- [Lists and scrolling](crate::_documentation::guides::lists_and_scrolling)
  covers scroll containers and list state in more depth.
- The [list example](crate::_documentation::examples::list) renders a virtualized
  list you can run.
