# Layout and style

Styles describe how an element should look and where it should sit; layout turns
those requests into positions and sizes. GPUI separates the two so that a render
pass can describe a tree cheaply and resolve geometry once:

* [`Style`] holds the resolved defaults for an element. Text properties live in
  its [`TextStyle`], which descendants can inherit.
* [`StyleRefinement`] holds optional overrides. Its fields are `Option`s, so an
  unset field means "keep the inherited or default value".
* [`Styled`] is the trait that exposes the builder methods. Every element that
  contains styling implements it, storing a `StyleRefinement` — for interactive
  elements, usually inside their [`Interactivity`].
* Rendering a view produces a tree of elements. GPUI merges each element's
  refinement over [`Style`]'s defaults, resolves sizes and positions across the
  tree with [Taffy](https://github.com/DioxusLabs/taffy), and passes the
  resulting bounds in logical pixels (screen-space coordinates) to prepaint and
  paint.

The [`Styled`] trait documentation is the source of truth for individual
methods; this page explains the model they share.

## Styles are method calls

Every type that implements [`Styled`] exposes builder methods named after
[Tailwind CSS](https://tailwindcss.com) utilities. GPUI does not run a CSS
engine; each call writes a field on the element's [`StyleRefinement`]:

```rust
# use gpui::{IntoElement, div, prelude::*, rgb};
# fn card() -> impl IntoElement {
div()
    .flex()
    .flex_col()
    .gap_2()
    .p_4()
    .rounded_lg()
    .bg(rgb(0xffffff))
    .shadow_md()
# }
```

Setting the same property twice replaces the earlier value. Additive methods say
so in their docs: [`Styled::blur`] appends to the filter list, whereas a later
[`Styled::filter`] replaces the list wholesale.

Interaction states reuse the same refinement system. [`hover_style`](InteractiveElement::hover_style)
and [`active_style`](StatefulInteractiveElement::active_style) take a closure that receives a fresh
`StyleRefinement` and returns the overrides to apply while the pointer hovers or
presses the element:

```rust
# use gpui::{IntoElement, div, prelude::*, rgb};
# fn button() -> impl IntoElement {
div()
    .bg(rgb(0x2563eb))
    .hover_style(|style| style.bg(rgb(0x1d4ed8)))
    .child("Save")
# }
```

GPUI evaluates these closures once when building the element and merges the
stored refinement while the state is active.

Groups change who a state style applies to, not what it does. Mark an element
with [`group`](InteractiveElement::group) and style any member of the same group
with methods such as [`group_hover_style`](InteractiveElement::group_hover_style)
or [`group_active_style`](StatefulInteractiveElement::group_active_style). The
connection is bidirectional: an element can style another member whether that
member is a descendant, an ancestor, or a sibling. Group names let several
independent groups coexist in one subtree.

## Inheritance and scope

Text properties are hierarchical. Setting color, font, size, weight, line height,
or [`rtl`](Styled::rtl) on a container applies to every descendant that leaves
that property unset. Box properties — padding, margins, dimensions, backgrounds,
and borders — are self-scoped: they apply only to the element where you set them.

Each setter's documentation states its scope when the distinction matters. When a
CSS property you know is inherited, look for the method on [`TextStyle`] (or the
matching `Styled` method) rather than on [`Style`] directly.

## Lengths

Length types encode what a measurement scales with, and each style method's
parameter type restricts which units make sense there. They all convert into
[`Length`]:

| Constructor | Meaning |
| --- | --- |
| [`px`] | Logical pixels, scaled by the display's scale factor and snapped to the device pixel grid. |
| [`rems`] | Multiples of the window's root font size, which starts at [`DEFAULT_REM_SIZE`] and changes with [`Window::set_rem_size`]. |
| [`relative`] | Fraction of the parent's size on that axis (`relative(0.5)` is `50%`). |
| [`auto`](crate::auto) | Deferred to the layout algorithm. |

Numbered spacing and sizing helpers follow Tailwind's quarter-rem scale: `p_4()`
is `1rem`, `gap_2()` is `0.5rem`, and `0p5` steps represent `0.125rem`. The
[`Styled`] trait documents the full naming scale, so you can read a method's
suffix as a multiple of the root font size rather than memorising pixel values:

```rust
use gpui::{px, rems};

let root_font_size = px(16.);
assert_eq!(rems(1.).to_pixels(root_font_size), px(16.));
assert_eq!(rems(0.25).to_pixels(root_font_size), px(4.));
```

Use [`rems`] for typography, spacing, and padding that should scale with a font
size preference. Use [`px`] for fixed chrome dimensions such as a `220px`
sidebar or a `1px` border, and [`relative`] when an element should take a
proportion of its parent. [`Window::set_rem_size`] updates the root scale for
the whole window on the next layout pass; custom elements (types that implement
[`Element`]) can temporarily override it during prepaint or paint with
[`Window::with_rem_size`].

## Flexbox and grid

A [`div`] defaults to [`Display::Block`], stacking children vertically in block
flow. Switch layout modes when arranging rows, columns, or two-dimensional
tracks:

* [`flex`](Styled::flex) arranges children in a row, or in a column with
  [`flex_col`](Styled::flex_col).
  * Cross-axis alignment: `items_*`, such as [`items_center`](Styled::items_center).
  * Main-axis distribution: `justify_*`, such as [`justify_between`](Styled::justify_between).
  * Spacing between siblings: `gap_*`, such as [`gap_2`](Styled::gap_2).
  * Wrapping: [`flex_wrap`](Styled::flex_wrap).
  * Per-child sizing: flex factors, `w_*`, `min_w_*`, and `max_w_*`.
* [`grid`](Styled::grid) arranges children in rows and columns configured with
  [`grid_cols`](Styled::grid_cols), [`grid_rows`](Styled::grid_rows), and
  [`col_span`](Styled::col_span).

The individual flex and grid setters document what they do, including the exact
factor each shorthand writes. Taffy implements the CSS Flexbox and Grid
specifications, so CSS reference documentation applies when debugging track
sizing or alignment, keeping in mind GPUI's defaults below.

### Choose the layout from the behavior you want

Start from the behavior, then pick the container:

| Behavior | Use |
| --- | --- |
| Children flow inline, wrap as a group, and each child sizes to its content | block flow with [`inline`](Styled::inline) children, or `flex` with `flex_wrap` and `flex_none` children |
| Children share leftover space equally | `flex` with [`flex_1`](Styled::flex_1) on the flexible children |
| One pane stays fixed while the other fills the rest | `flex` with a fixed `w_*` and [`flex_shrink_0`](Styled::flex_shrink_0) pane plus a `flex_1` pane |
| Rows and columns align like a table | `grid` with `grid_cols` and [`col_span`](Styled::col_span) |

Two small patterns cover most layouts. A row with a fixed sidebar and a flexible
main pane:

```rust
# use gpui::{IntoElement, div, prelude::*, px};
# fn shell() -> impl IntoElement {
div()
    .size_full()
    .flex()
    .child(div().w(px(220.)).flex_shrink_0().child("Sidebar"))
    .child(div().flex_1().min_w_0().child("Content"))
# }
```

A grid table where the header spans every column:

```rust
# use gpui::{IntoElement, div, prelude::*, px};
# fn table() -> impl IntoElement {
div()
    .grid()
    .grid_cols(3)
    .gap_2()
    .child(div().col_span(3).child("Header"))
    .child("Left")
    .child("Middle")
    .child("Right")
# }
```

## Overflow and absolute positioning

Both axes default to `Overflow::Visible`, so children that overflow their
parent's bounds (that is, they are painted outside the box assigned to the
parent) remain visible. [`overflow_hidden`](Styled::overflow_hidden) clips at the
padding box. Scrolling needs its own container and is covered by the
[lists and scrolling guide](crate::_documentation::guides::lists_and_scrolling),
including how a bounded height and an [`ElementId`] keep the scroll offset across
frames.

[`absolute`](Styled::absolute) removes an element from flow and resolves `top`,
`left`, `right`, and `bottom` against its **direct parent's** box. Unlike CSS,
where default positioning is `static` and absolute offsets search upward for a
positioned ancestor, GPUI's default position is [`Position::Relative`], so every
parent already acts as the containing block.

### Why a flex row overflows its container

This is the CSS pitfall that surprises most people. A flex item's automatic
minimum size is its `min-content` size — the width of its widest unbreakable
word or child. By default Taffy enforces that floor, so a `flex_1` child holding a
long URL or an unwrapped `nowrap` line can be wider than the space it was given
and overflow the row instead of shrinking.

Lower that floor when the content should shrink and wrap or scroll:

```rust
# use gpui::{IntoElement, div, prelude::*, px};
# fn row() -> impl IntoElement {
div()
    .flex()
    .w(px(400.))
    .child(div().w(px(200.)).flex_shrink_0().child("Fixed"))
    // Without min_w_0(), long content keeps this pane at its min-content width.
    .child(div().flex_1().min_w_0().child("A very long unbroken value that should wrap").whitespace_normal())
# }
```

[`min_w_0`](Styled::min_w_0) sets the floor to zero; `overflow_hidden` and
scroll overflow lower it as well. If a row still overflows, check for a child
whose minimum size is larger than the track: a fixed `w_*`, an unbreakable
string, or a nested flex container with the same problem.

## Overlays and paint order

Elements paint in tree order, with later siblings drawing over earlier ones. To
break out of local paint order and clip masks — for menus, tooltips, and
popovers — combine [`deferred`] and [`anchored`]:

* [`deferred`] lays out its child in place, then postpones prepaint and paint
  until ancestor trees finish drawing. Set [`Deferred::priority`] with a
  [`DeferredPriority`] to order it against other deferred elements; higher
  priorities paint on top. [`Deferred::priority_auto`] derives the priority from
  the deferred element that created this one.
* [`anchored`] places its children at a window or parent-relative coordinate and
  keeps them inside the viewport, flipping to the opposite corner on overflow by
  default or clamping to the edge with [`Anchored::snap_to_window`].

## Responsive layout

GPUI does not use CSS media queries. For window-level breakpoints, read
[`Window::viewport_size`] (the content area in logical pixels, excluding native
decorations) inside `render` and branch in Rust; resizing the window triggers a
fresh render automatically.

For components that adapt to the space their parent gives them rather than the
whole window, use [`container_query`]. It fills its parent by default, resolves
its own layout first, and then invokes your closure during prepaint with the
measured `Size<Pixels>`. Because the closure runs after the container's size is
fixed, the children it returns cannot resize the container itself.

```rust
# use gpui::{IntoElement, container_query, div, prelude::*, px};
# fn details() -> impl IntoElement {
container_query(|size, _window, _cx| {
    if size.width < px(320.) {
        div().flex_col().gap_2().child("Title").child("Body")
    } else {
        div().flex().gap_2().child("Title").child("Body")
    }
})
# }
```

The [`container_query`] documentation has a complete example; if you need the
measured size outside the closure, store it in your view and redraw as the
example does.

## Writing direction

[`rtl`](Styled::rtl) lays out a container and its descendants right to left for
scripts such as Arabic and Hebrew; [`ltr`](Styled::ltr) switches a nested
subtree back. Physical insets (`left`, `right`) keep their physical meaning in
both directions.
