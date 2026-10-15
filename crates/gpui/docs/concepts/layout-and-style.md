# Layout and style

When a GPUI view renders, it produces a tree of boxes. GPUI first requests a
layout for that tree, computes sizes from the available space, then prepaints
and paints the resulting bounds. A style method contributes to the request;
calling it does not measure the element immediately. The final size comes from
the whole ancestor chain, not one method in isolation. Text properties are the
only properties that flow from a parent to its descendants.

## Styles are method calls

Every element that implements [`Styled`] exposes methods named after the
corresponding [Tailwind CSS](https://tailwindcss.com) classes:

```
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

Each call records an optional property in the element's [`StyleRefinement`].
During the layout request, GPUI applies those overrides over [`Style`]'s
defaults, so an omitted property still has a defined value. The later prepaint
and paint phases use the bounds produced by that request. Interaction states
use the same refinement:
[`hover`](InteractiveElement::hover) takes a closure that returns the
properties to change while the pointer is over the element:

```
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

Text properties such as color, size, font and weight are inherited. Setting
[`text_color`](Styled::text_color) on a container therefore colors every child
that does not set a value of its own; padding, width and other box properties do
not work this way.

## Lengths

Before choosing a layout rule, decide whether the value should respond to the
display scale, root font size, parent, or layout algorithm. GPUI's length types
encode that choice, and a method's parameter type tells you which kind it accepts:

| Function | Meaning |
| --- | --- |
| [`px`] | Logical pixels. GPUI multiplies by the display's scale factor for you. |
| [`rems`] | Multiples of the window's root font size, 16 pixels unless you change it with [`Window::set_rem_size`]. |
| [`relative`] | A fraction of the parent's size: `relative(0.5)` is half. |
| `auto()` | Let layout decide. |

Numbered helpers follow Tailwind's scale: each step is a quarter rem. Thus
`p_4()` adds one rem of padding (16 pixels by default), while `gap_2()` adds
half a rem:

```
use gpui::{px, rems};

let root_font_size = px(16.);
assert_eq!(rems(1.).to_pixels(root_font_size), px(16.));
assert_eq!(rems(0.25).to_pixels(root_font_size), px(4.));
```

Use rems when spacing or text should scale with the root font size. Use logical
pixels for dimensions tied to a control's physical affordance, and `relative`
when the parent—not the window—should determine the result.

## Flexbox and grid

A `div` lays out its children in normal block flow until you choose another
layout. Choose the model that answers the question your component has:

* [`flex`](Styled::flex) arranges children in a row, or in a column with
  [`flex_col`](Styled::flex_col). Align them across the axis with
  [`items_center`](Styled::items_center) and friends, distribute them along it
  with [`justify_between`](Styled::justify_between) and friends, and separate
  them with [`gap_*`](Styled::gap_2).
* [`grid`](Styled::grid) places children in rows and columns. Choose the number
  of columns with [`grid_cols`](Styled::grid_cols), and span cells with
  [`col_span`](Styled::col_span).

Children start at their content size unless a constraint changes that default.
[`flex_1`](Styled::flex_1) lets a child take remaining space, while
[`flex_shrink_0`](Styled::flex_shrink_0) prevents it from shrinking below its
natural size. A common layout gives a fixed sidebar the second behavior and
the content pane the first:

```
# use gpui::{IntoElement, div, prelude::*, px};
# fn shell() -> impl IntoElement {
div()
    .size_full()
    .flex()
    .child(div().w(px(220.)).flex_shrink_0().child("Sidebar"))
    .child(div().flex_1().min_w_0().child("Content"))
# }
```

The `min_w_0` is the part that makes this shell robust. A flex child refuses to
shrink below its content's width by default. Without it, a long line can force
the window wider instead of wrapping. Setting the minimum to zero lets the
content pane accept the width its parent has available; the sidebar remains at
220 pixels because its shrink is disabled.

GPUI computes layout with [Taffy](https://github.com/DioxusLabs/taffy), which
implements the CSS flexbox and grid algorithms. When a layout surprises you,
the CSS documentation for the corresponding property is a useful reference;
the surrounding constraints still come from the GPUI element tree.

## Overflow, scrolling and stacking

Once sizes are resolved, overflow determines what happens to content that does
not fit. It spills out of its box by default, so a child can remain visible
outside the parent's bounds.
[`overflow_hidden`](Styled::overflow_hidden) clips it.
[`overflow_y_scroll`](StatefulInteractiveElement::overflow_y_scroll) makes the
box scroll. Give a scrolling element an [`id`](InteractiveElement::id): GPUI
uses that stable identity to associate its scroll position with the element
between frames. A scrolling box also needs a bounded height: a `flex_1` child
of a column, or an explicit height.

[`absolute`](Styled::absolute) takes an element out of the flow and positions
it relative to its nearest positioned ancestor, with `top`, `left` and friends.
Elements paint in tree order, so later siblings draw on top of earlier ones.
That ordering is local to the element tree. For menus and popovers that must
draw above everything and stay inside the window, use
[`anchored`] and [`deferred`] instead.

## Responsive layout

GPUI has no media queries. A view that needs a window-level breakpoint can read
[`Window::viewport_size`] in `render` and choose a layout; a resize causes GPUI
to render the view again. If a reusable component should respond to the space
its parent gives it, wrap it in a [`container_query`], which renders its contents
using that container's available size.

## Writing direction

Layout follows the writing direction. Call [`rtl`](Styled::rtl) on a container
to lay out its contents right to left, for languages such as Arabic and Hebrew.
