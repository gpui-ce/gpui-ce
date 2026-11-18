# Layout and style

When a view renders, it returns a tree of elements. Style methods on [`Styled`]
record optional overrides in an element's [`StyleRefinement`]; during layout,
GPUI merges those refinements over [`Style`]'s defaults, resolves sizes and
positions across the tree with [Taffy](https://github.com/DioxusLabs/taffy),
and passes the resulting bounds in logical pixels to prepaint and paint.

Text properties and writing direction inherit down the element tree: setting
color, font, size, weight, or [`rtl`](Styled::rtl) on a container applies to
every descendant that leaves that property unset. Box properties—padding,
margins, dimensions, backgrounds, and borders—apply only to the element where
you set them.

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

Interaction states reuse the same refinement system. [`hover`](InteractiveElement::hover)
and [`active`](StatefulInteractiveElement::active) take a closure that receives a fresh
`StyleRefinement` and returns the overrides to apply while the pointer hovers or
presses the element:

```rust
# use gpui::{IntoElement, div, prelude::*, rgb};
# fn button() -> impl IntoElement {
div()
    .id("save")
    .bg(rgb(0x2563eb))
    .hover(|style| style.bg(rgb(0x1d4ed8)))
    .active(|style| style.opacity(0.8))
    .child("Save")
# }
```

GPUI evaluates these closures once when building the element and merges the
stored refinement while the state is active. To style a child when its parent
row or card is hovered, mark the ancestor with [`group`](InteractiveElement::group)
and style the descendant with [`group_hover`](InteractiveElement::group_hover).

## Lengths

Length types encode what a measurement scales with, and each style method's
parameter type restricts which units make sense there:

| Constructor | Meaning |
| --- | --- |
| [`px`] | Logical pixels, scaled by the display's scale factor and snapped to the device pixel grid. |
| [`rems`] | Multiples of the window's root font size (`16px` until changed with [`Window::set_rem_size`]). |
| [`relative`] | Fraction of the parent's size on that axis (`relative(0.5)` is `50%`). |
| [`auto`](crate::auto) | Deferred to the layout algorithm. |

Numbered spacing and sizing helpers follow Tailwind's quarter-rem scale: `p_4()`
is `1rem` (`16px` by default), `gap_2()` is `0.5rem` (`8px`), and `0p5` steps
represent `0.125rem` (`2px`):

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
the whole window on the next layout pass; custom elements can temporarily override
it during prepaint or paint with [`Window::with_rem_size`].

## Flexbox and grid

A [`div`] defaults to [`Display::Block`], stacking children vertically in block
flow. Switch layout modes when arranging rows, columns, or two-dimensional
tracks:

* [`flex`](Styled::flex) arranges children in a row, or in a column with
  [`flex_col`](Styled::flex_col). Control cross-axis alignment with `items_*`
  (such as [`items_center`](Styled::items_center)), main-axis distribution with
  `justify_*` (such as [`justify_between`](Styled::justify_between)), and
  spacing between siblings with `gap_*` (such as [`gap_2`](Styled::gap_2)).
* [`grid`](Styled::grid) arranges children in rows and columns configured with
  [`grid_cols`](Styled::grid_cols), [`grid_rows`](Styled::grid_rows), and
  [`col_span`](Styled::col_span).

Flex children start at their content size. [`flex_1`](Styled::flex_1) sets
`flex-grow: 1`, `flex-shrink: 1`, and a zero basis so the child claims leftover
space; [`flex_shrink_0`](Styled::flex_shrink_0) keeps a fixed pane at its
requested width:

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

`min_w_0()` prevents long unbroken content from blowing out the row. By default,
Taffy sets a flex item's automatic minimum width to its `min-content` width—the
widest unbreakable word or child inside it—unless `min_w_0()` or
`overflow_hidden` / scroll overflow lowers that floor to zero. Because Taffy
implements the CSS Flexbox and Grid specifications, CSS reference documentation
applies directly when debugging track sizing or alignment, keeping in mind
GPUI's defaults below.

## Overflow, positioning, and overlays

Both axes default to `Overflow::Visible`, so overflowing children remain visible
outside their parent's box. [`overflow_hidden`](Styled::overflow_hidden) clips
at the padding box. [`overflow_y_scroll`](StatefulInteractiveElement::overflow_y_scroll)
makes a container scrollable once you give it two things: an [`ElementId`] via
[`id`](InteractiveElement::id) so GPUI can retain the scroll offset across
frames, and a bounded height (such as an explicit `.h(...)` or `.flex_1()` in a
bounded column) so the container does not simply grow to fit its children.

[`absolute`](Styled::absolute) removes an element from flow and resolves `top`,
`left`, `right`, and `bottom` against its **direct parent's** box. Unlike CSS,
where default positioning is `static` and absolute offsets search upward for a
positioned ancestor, GPUI's default position is [`Position::Relative`], so every
parent already acts as the containing block.

Elements paint in tree order, with later siblings drawing over earlier ones. To
break out of local paint order and clip masks—for menus, tooltips, and
popovers—combine [`deferred`] and [`anchored`]:

* [`deferred`] lays out its child in place, then postpones prepaint and paint
  until ancestor trees finish drawing. Higher priority values paint on top.
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

## Writing direction

[`rtl`](Styled::rtl) lays out a container and its descendants right to left for
scripts such as Arabic and Hebrew; [`ltr`](Styled::ltr) switches a nested
subtree back. Physical insets (`left`, `right`) keep their physical meaning in
both directions.
