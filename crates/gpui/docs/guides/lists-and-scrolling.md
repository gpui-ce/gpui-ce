# Show long lists and scroll

Choose the scrolling container that matches your item count and row heights:

| Container | Handle | Best for |
| --- | --- | --- |
| `div().id(...).overflow_y_scroll()` | [`ScrollHandle`] | Short lists or arbitrary mixed children laid out every frame |
| [`uniform_list`] | [`UniformListScrollHandle`] | Large lists where every row has the same height |
| [`list`] | [`ListState`] | Large lists where row heights vary, stream in, or anchor to the bottom |

All three keep persistent row data on your view or model, because elements
themselves live for one frame only.

## Scroll a `div` with `ScrollHandle`

Give a [`Div`] a stable [`ElementId`], a bounded height, and
[`overflow_y_scroll`](StatefulInteractiveElement::overflow_y_scroll), then
attach a [`ScrollHandle`] stored on the view:

```rust
# use gpui::*;
struct Log {
    scroll: ScrollHandle,
    lines: Vec<String>,
}

impl Render for Log {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("log")
            .h(px(300.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(self.lines.iter().enumerate().map(|(ix, line)| {
                div().id(ix).h(px(24.)).child(line.clone())
            }))
    }
}
```

[`ScrollHandle`] reads geometry from the most recently drawn frame and queues
programmatic scroll changes for the next frame:

* [`ScrollHandle::offset`] returns the content origin relative to the container:
  `px(0.)` at the top, becoming negative as you scroll down.
* [`ScrollHandle::max_offset`] returns the maximum scrollable distance as
  positive coordinates, and [`ScrollHandle::set_offset`] sets the offset
  directly.
* [`ScrollHandle::scroll_to_bottom`], [`ScrollHandle::scroll_to_item`] (minimum
  movement to reveal a child), and [`ScrollHandle::scroll_to_top_of_item`] take
  effect on the next draw.
* [`ScrollHandle::top_item`] and [`ScrollHandle::bottom_item`] return the
  indices of children at the viewport edges.

```rust
# use gpui::*;
# struct Log {
#     scroll: ScrollHandle,
# }
# impl Render for Log {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
#         div()
#             .id("log")
#             .h(px(300.))
#             .overflow_y_scroll()
#             .track_scroll(&self.scroll)
#             .children((0..200).map(|ix| div().id(ix).h(px(24.)).child(format!("line {ix}"))))
#     }
# }
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Log { scroll: ScrollHandle::new() });
window.update(|log, _window, _cx| log.scroll.scroll_to_bottom());
window.draw();
window.read(|log, _| assert!(log.scroll.offset().y < px(0.)));
```

## Equal-height rows with `uniform_list`

[`uniform_list`] measures a single sample row, computes every row's vertical
offset as `index * row_height`, and builds elements only for the visible index
range. Use [`Context::processor`] when the row builder needs `&mut Self` and
`&mut Context<Self>`:

```rust
# use gpui::*;
# use std::ops::Range;
struct Contacts {
    names: Vec<String>,
    scroll: UniformListScrollHandle,
}

impl Render for Contacts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        uniform_list(
            "contacts",
            self.names.len(),
            cx.processor(|this, range: Range<usize>, _window, _cx| {
                range
                    .map(|ix| div().id(ix).h(px(32.)).child(this.names[ix].clone()))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
    }
}
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Contacts {
#     names: (0..1000).map(|ix| format!("Contact {ix}")).collect(),
#     scroll: UniformListScrollHandle::new(),
# });
# window.update(|contacts, _window, _cx| contacts.scroll.scroll_to_item(500, ScrollStrategy::Top));
# window.draw();
```

Drive keyboard or programmatic scrolling through [`UniformListScrollHandle`]:

* [`ScrollStrategy::Nearest`] moves the viewport only when the target row lies
  outside it, making it the standard choice for arrow-key selection.
* [`ScrollStrategy::Top`], [`ScrollStrategy::Center`], and
  [`ScrollStrategy::Bottom`] align the row with that part of the viewport.
* [`UniformListScrollHandle::scroll_to_item`] leaves already-visible rows in
  place; [`UniformListScrollHandle::scroll_to_item_strict`] applies the
  alignment unconditionally.

## Variable-height rows with `list`

[`list`] pairs with a [`ListState`] stored on your view (cloning `ListState` is
cheap and shares the underlying state). It renders the visible viewport plus an
overdraw margin in pixels on each side to reduce pop-in, caching each row's
measured height as it renders:

```rust
# use gpui::*;
# use std::rc::Rc;
struct Feed {
    state: ListState,
    items: Rc<Vec<String>>,
}

impl Feed {
    fn new() -> Self {
        let items = vec!["Welcome".to_string()];
        Self {
            state: ListState::new(items.len(), ListAlignment::Top, px(200.)),
            items: Rc::new(items),
        }
    }

    fn push(&mut self, text: String, cx: &mut Context<Self>) {
        let old_count = self.items.len();
        Rc::make_mut(&mut self.items).push(text);
        self.state.splice(old_count..old_count, 1);
        cx.notify();
    }
}

impl Render for Feed {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items.clone();
        list(self.state.clone(), move |ix, _window, _cx| {
            div().p_2().child(items[ix].clone()).into_any_element()
        })
        .size_full()
    }
}
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Feed::new());
# window.update(|feed, _window, cx| feed.push("Hello".to_string(), cx));
# window.draw();
# window.read(|feed, _| assert_eq!(feed.state.item_count(), 2));
```

Whenever your backing collection changes count, order, or item heights, update
`ListState` in the same update as the data and call `cx.notify()`:

* [`ListState::splice`] inserts, deletes, or replaces an index range while
  preserving surrounding measurements and scroll position
  (`splice(len..len, 1)` appends one item). Use [`ListState::splice_focusable`]
  when inserted rows can hold keyboard focus so focused rows stay rendered even
  while scrolled out of view.
* [`ListState::reset`] replaces the entire list and jumps back to the start (for
  [`ListAlignment::Top`]) or end (for [`ListAlignment::Bottom`]).
* [`ListState::remeasure_items`] invalidates cached heights for an index range
  whose content grew or shrank in place (for example streaming markdown), and
  [`ListState::remeasure`] invalidates all heights when a container font size
  changes.

Three details matter when tuning variable-height lists:

* **Bottom-anchored timelines**: [`ListAlignment::Bottom`] anchors items to the
  bottom edge like a chat transcript. Pair it with
  [`ListState::set_follow_mode`]`(`[`FollowMode::Tail`]`)` to stay pinned to new
  messages until the user scrolls upward, or call [`ListState::scroll_to_end`] /
  [`ListState::scroll_to_reveal_item`] explicitly.
* **Scrollbar thumb stability**: Unmeasured rows contribute `0px` until
  rendered. Supply an estimated height with
  [`ListState::with_uniform_item_height`] (or
  [`ListState::reset_with_uniform_height`]) so scrollbar proportions stay steady
  before every row has been visited; avoid [`ListState::measure_all`] on large
  lists because it lays out every row up front.
* **Reading scroll position**: [`ListState::logical_scroll_top`] returns the
  current [`ListOffset`] (`item_ix` and pixel `offset_in_item`).

## See also

* [List example](crate::_documentation::examples::list) — a 10,000-row
  `uniform_list` with keyboard selection (`cargo run --example list`).
* [Layout and style](crate::_documentation::concepts::layout_and_style) — how
  bounded container heights interact with flex layouts.
* [Performance](crate::_documentation::guides::performance) — profiling frame
  times on large views.
