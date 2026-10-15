# Show long lists and scroll

Start with the cost of building one frame. A short list can render all its
children directly. For many equal-height rows, [`uniform_list`] creates only
the rows in the viewport. For rows whose heights differ or change, [`list`]
uses [`ListState`] to keep geometry and renders the viewport plus configured
overdraw. Measurement and a focused off-screen row can make that range larger.

Ordinary scrolling uses a stable [`ElementId`] and a [`ScrollHandle`] retained
by the view. The handle reads positions from the last frame; changing it
requests work for the next frame. A variable-height list likewise depends on
`ListState` matching the row data.

## Render a simple scrolling container

Give a [`Div`] an ID, set its overflow to scroll, and track it with the view's
[`ScrollHandle`]:

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

Every child is laid out on every frame. This keeps ordinary layout simple and
works well for a short list, but frame cost grows with row count and row render
cost. If profiling identifies full-list construction or layout as the bottleneck,
move to a virtualized element; the useful threshold depends on the content and
device.

## Read a position or request a scroll

[`ScrollHandle`] reports the position computed by the last frame. A setter or a
scroll request queues work for the next frame, so read it after drawing when
you need the resulting offset:

- [`ScrollHandle::offset`] is the distance from the container's top to the top
  of its content. It starts at zero and becomes more negative as you scroll down.
- [`ScrollHandle::max_offset`] is how far the content can scroll, as a positive
  distance.
- [`ScrollHandle::set_offset`] sets the offset directly.
- [`ScrollHandle::scroll_to_bottom`] and [`ScrollHandle::scroll_to_item`] are
  applied when the next frame is prepared. `scroll_to_item` scrolls the least
  amount that makes the child fully visible. [`ScrollHandle::scroll_to_top_of_item`]
  puts the child at the top instead.
- [`ScrollHandle::top_item`] and [`ScrollHandle::bottom_item`] return the indices
  of the children at the edges of the viewport.

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

## Virtualize equal-height rows with `uniform_list`

[`uniform_list`] asks its callback for the visible range and places each row at
`index * measured_height`. It measures one row during layout and prepaint, then
does no layout for rows outside the viewport. This suits a 10,000-row contact
table with 32-pixel rows, provided every returned row has the measured height.
Use [`Context::processor`] when the callback needs to read the view that owns
the data:

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

Keep the [`UniformListScrollHandle`] on the view and pass a [`ScrollStrategy`]
to [`UniformListScrollHandle::scroll_to_item`]. The strategy matters when the
list needs to move:

- [`ScrollStrategy::Nearest`] moves the least amount. Use it for keyboard selection.
- [`ScrollStrategy::Top`], [`ScrollStrategy::Center`] and [`ScrollStrategy::Bottom`]
  put the row at that edge of the viewport, or as close as the list allows.
- [`UniformListScrollHandle::scroll_to_item`] does nothing when the row is
  already fully visible. [`UniformListScrollHandle::scroll_to_item_strict`]
  applies the strategy every time, even then.

```rust
# use gpui::*;
# struct Contacts {
#     scroll: UniformListScrollHandle,
# }
# impl Contacts {
fn select_row(&mut self, ix: usize, cx: &mut Context<Self>) {
    self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
    cx.notify();
}
# }
```

## Virtualize variable-height rows with `list`

[`list`] measures each row as it renders it and stores measurements and scroll
state in [`ListState`]. Its callback receives the viewport and overdraw range.
The state must follow the data mutation: use [`ListState::reset`] for a complete
replacement, [`ListState::splice`] when rows are inserted, removed, or moved,
and [`ListState::remeasure_items`] when existing rows change height. Without the
matching operation, old positions remain in use and the scrollbar can describe
the wrong content:

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

When row count, order, or height changes, update the data and `ListState`
together, then call `cx.notify()`, as `push` does above. Notification schedules
the render; the state update keeps geometry consistent. A content-only change
that preserves row geometry needs notification without remeasurement.

A [`ListState`] is cheap to clone because clones share the same state. Keep one
on the view and pass a clone to [`list`]. The overdraw passed to
[`ListState::new`] adds a band above and below the viewport. Those rows are
laid out and measured before entering view, reducing visible popping; the
callback can therefore receive more rows than the viewport shows.

Choose the state operation by what changed. Each one controls measurements and
the resulting scroll position:

- [`ListState::splice`] replaces a range of items with a number of new ones. Use
  it when items are added, removed or moved. `splice(old_count..old_count, 1)`
  appends one.
- [`ListState::reset`] replaces every item and resets the position to the start
  for [`ListAlignment::Top`], or the end for [`ListAlignment::Bottom`]. It also
  clears pending scroll events. Use `splice` when a replacement should retain
  the current position.
- [`ListState::remeasure_items`] marks a range for re-measurement when its content
  changes height but the items stay, such as streaming text. It keeps the scroll
  position. [`ListState::splice`] changes the row count; it does not remeasure an
  existing row whose height changed, so call both when one operation does both.
- [`ListState::remeasure`] re-measures everything and keeps the position
  proportionally, for when the font size changes.

Other methods:

- [`ListAlignment::Bottom`] anchors the list to the bottom, as a chat log does.
  With [`FollowMode::Tail`], set through [`ListState::set_follow_mode`], the list
  keeps the newest item in view until the user scrolls up.
- [`ListState::scroll_to_reveal_item`] scrolls one item fully into view.
- [`ListState::scroll_to_end`] scrolls past the last item. Use it when that item
  is still growing.
- [`ListState::logical_scroll_top`] returns the current [`ListOffset`].

## Know what virtualization retains

`uniform_list` renders and paints only the viewport range. It measures one row
and derives every other position from that height, so it cannot represent rows
whose heights differ. Rows outside the viewport are not created. Keep row data
on the view because row elements are dropped as they scroll away.

`list` renders and paints the viewport plus the overdraw from `ListState::new`.
It measures each rendered row and uses those measurements to locate later rows.
The extra rows reduce visible popping, but a variable-height list may measure
more rows as content enters the overdraw region.

That flexibility has three consequences:

- [`list`] does not know a row's height until it renders it. An unmeasured row
  contributes zero pixels, so the scrollbar thumb can change as you scroll.
  [`ListState::measure_all`] measures every row on the first frame, which is
  expensive for a large list. [`ListState::with_uniform_item_height`] and
  [`ListState::reset_with_uniform_height`] supply an estimate for rows that have
  not been rendered. As those rows are measured, their actual heights replace
  the estimates, which can change the scrollbar's proportions.
- When a row leaves the rendered range, its element state can be dropped. Keep
  persistent row data in your model; an element represents one frame, not durable
  row storage.
- A row that holds keyboard focus stays rendered while it is off screen, if you
  add it with [`ListState::splice_focusable`].

## See also

- [List example](crate::_documentation::examples::list), a 10,000-row
  `uniform_list` with keyboard selection. Run it with `cargo run --example list`.
- [Layout and style](crate::_documentation::concepts::layout_and_style) for how
  a container's size is decided.
- [The frame](crate::_documentation::concepts::the_frame) for when layout and
  paint run.
- [Performance](crate::_documentation::guides::performance) for finding slow frames.
