# Draw custom graphics

Use [`canvas`] when [`div`], [`svg`], and [`img`] cannot express the geometry you
need — for example a sparkline, waveform, custom meter, or debug overlay. It
gives you an element's laid-out [`Bounds`] in window coordinates and direct
access to [`Window`]'s painting methods without defining a custom [`Element`]
type.

`canvas` takes two `FnOnce` closures that run in the frame that builds it:

1. **Prepaint** runs once layout has resolved the element's bounds. Compute
   geometry, insert hitboxes, and return any state needed for painting.
2. **Paint** receives the same bounds as its first argument and prepaint's
   return value as its second argument, then submits primitives to the scene.

Because a canvas has no children to give it an intrinsic size, size it explicitly
with [`Styled`] methods such as `.size_full()` or `.w_full().h(px(12.))`. Its
own style paints around your callback: drop shadows first, then background and
inset shadows, then your `paint` closure, and finally borders.

```rust
# use gpui::*;
# struct Meter;
# impl Render for Meter {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
let meter = canvas(
    |bounds, _window, _cx| bounds.size.width * 0.6,
    |bounds, filled_width, window, _cx| {
        window.paint_quad(
            PaintQuad::new(bounds)
                .border_widths(px(1.))
                .border_color(rgb(0x9ca3af)),
        );
        let filled = Bounds::new(bounds.origin, size(filled_width, bounds.size.height));
        window.paint_quad(PaintQuad::new(filled).background(rgb(0x2563eb)));
    },
)
.w_full()
.h(px(12.));

div().p_4().child(meter)
#     }
# }
# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Meter);
# window.draw();
```

For a longer example that draws a chart from paths and quads, run the
[canvas example](crate::_documentation::examples::canvas)
(`cargo run --example canvas`).

## Paint rectangles with `paint_quad`

[`Window::paint_quad`] adds a [`PaintQuad`] during the paint phase (debug builds
assert the phase). Build one with [`PaintQuad::new`], then chain setters for the
parts you need:

* [`PaintQuad::background`] fills the rectangle; no background leaves it
  transparent.
* [`PaintQuad::corner_radii`] rounds the corners, and
  [`PaintQuad::corner_smoothing`] controls how circular those corners are.
* [`PaintQuad::border_widths`], [`PaintQuad::border_color`], and
  [`PaintQuad::border_style`] draw borders. [`PaintQuad::border_dashed_length`]
  and [`PaintQuad::border_dashed_gap`] set dash lengths as multiples of the
  border width.

```rust
# use gpui::*;
# let bounds = Bounds::new(point(px(0.), px(0.)), size(px(120.), px(48.)));
# fn paint(bounds: Bounds<Pixels>, window: &mut Window) {
let card = PaintQuad::new(bounds)
    .background(rgb(0xf9fafb))
    .corner_radii(px(8.))
    .border_widths(px(1.))
    .border_color(rgb(0xd1d5db));
window.paint_quad(card);
# }
```

The older free functions [`fill`], [`outline`], and [`quad`] are deprecated; they
exist only to build the same `PaintQuad`.

`paint_quad` does not clamp oversized corner radii automatically; if a radius
can exceed half the shorter side, clamp it first with
[`Corners::clamp_radii_for_quad_size`] so corner arcs do not overlap.

## Draw lines and curves with `PathBuilder`

[`PathBuilder`] tessellates lines, quadratic curves ([`PathBuilder::curve_to`]),
cubic Béziers ([`PathBuilder::cubic_bezier_to`]), arcs, and polygons into GPU
triangles:

* [`PathBuilder::stroke`] takes a line width in logical pixels and supports
  dashed strokes via [`PathBuilder::dash_array`] (alternating dash and gap
  lengths in logical pixels; odd-length slices repeat to form an even cycle).
* [`PathBuilder::fill`] fills the interior using the even-odd rule, closing open
  sub-paths automatically.

To draw a path, compute its points in window coordinates from the canvas
`bounds`, call [`PathBuilder::build`] to tessellate a [`Path`], and pass the
result to [`Window::paint_path`]:

```rust
# use gpui::*;
canvas(
    |_, _, _| (),
    |bounds, _, window, _cx| {
        let top = point(bounds.center().x, bounds.origin.y);
        let left = point(bounds.origin.x, bounds.origin.y + bounds.size.height);
        let right = point(bounds.origin.x + bounds.size.width, left.y);

        let mut triangle = PathBuilder::stroke(px(2.));
        triangle.move_to(left);
        triangle.line_to(top);
        triangle.line_to(right);
        triangle.close();

        if let Ok(path) = triangle.build() {
            window.paint_path(path, rgb(0x2563eb));
        }
    },
)
.size_full();
```

[`PathBuilder::build`] returns an error for invalid geometry; its documentation
lists the conditions. Tessellation happens in `build`, so cache and `.clone()`
complex [`Path`]s whose geometry does not change between frames rather than
rebuilding them every paint.

## Paint shadows, images, text, and clipping

Primitives are painted in the order you submit them.

### Shadows

Call [`Window::paint_drop_shadows`] before painting the shape's fill, and
[`Window::paint_inset_shadows`] after painting the fill. Both take bounds, corner
radii, corner smoothing (`0.0..=1.0`), and a `&[BoxShadow]` slice, and filter for
shadows whose `inset` flag matches the call:

```rust
# use gpui::*;
# fn paint(bounds: Bounds<Pixels>, window: &mut Window) {
let shadow = BoxShadow {
    color: rgba(0x00000033).into(),
    offset: point(px(0.), px(4.)),
    blur_radius: px(12.),
    spread_radius: px(0.),
    inset: false,
};
window.paint_drop_shadows(bounds, Corners::all(px(8.)), 0.0, &[shadow]);
window.paint_quad(
    PaintQuad::new(bounds)
        .background(rgb(0xffffff))
        .corner_radii(px(8.)),
);
# }
```

### Images

Load a [`RenderImage`] with [`Window::use_asset`] and
[`ImgResourceLoader`], then draw it with [`Window::paint_image`]. `use_asset`
returns `None` while loading in the background and notifies the view for a
later frame once ready; pass a valid `frame_index` (`0..image.frame_count()`):

```rust
# use gpui::*;
# use std::path::PathBuf;
fn paint_thumbnail(path: PathBuf, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if let Some(Ok(image)) = window.use_asset::<ImgResourceLoader>(&Resource::from(path), cx) {
        let _ = window.paint_image(bounds, bounds, Corners::all(px(6.)), 0.0, image, 0, false);
    }
}
```

### Text

Shape single-line text with [`WindowTextSystem::shape_line`] (or
multi-line text with [`WindowTextSystem::shape_text`]) and call `paint`. If a
run carries a background color, call `paint_background` before `paint`. See
the [text guide](mod@crate::_documentation::guides::text) for run formatting
and measurement:

```rust
# use gpui::*;
# fn paint(bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
let text = "Peak 42 ms";
let style = window.text_style();
let font_size = style.font_size.to_pixels(window.rem_size());
let line_height = window.line_height();
let runs = [style.to_run(text.len())];
let line = window.text_system().shape_line(text.into(), font_size, &runs);
let _ = line.paint(bounds.origin, line_height, TextAlign::Left, None, window, cx);
# }
```

### Clipping

[`Window::with_content_mask`] intersects a [`ContentMask`] with
the active clip rectangle for the duration of a closure, clipping both painted
primitives and hitboxes inserted inside it (`fade_out` fades edges instead of
hard-clipping):

```rust
# use gpui::*;
# fn paint(bounds: Bounds<Pixels>, window: &mut Window) {
let inner = Bounds::new(
    point(bounds.origin.x + px(8.), bounds.origin.y + px(8.)),
    size(bounds.size.width - px(16.), bounds.size.height - px(16.)),
);

window.with_content_mask(Some(ContentMask { bounds: inner, ..Default::default() }), |window| {
    window.paint_quad(PaintQuad::new(bounds).background(rgb(0xef4444)));
});
# }
```

## Make a canvas interactive

`canvas` does not wrap [`Interactivity`], so pointer input uses two low-level
window calls:

1. In **prepaint**, call [`Window::insert_hitbox`] and return the [`Hitbox`].
2. In **paint**, call [`Window::on_mouse_event`] (and optionally
   [`Window::set_cursor_style`]).

Window mouse listeners receive every event of their type anywhere in the window
across both [`DispatchPhase::Capture`] and [`DispatchPhase::Bubble`]. Filter on
both the phase and [`Hitbox::is_hovered`] so clicks outside the canvas, occluded
by an overlay, or in the second phase do not fire your handler:

```rust
# use gpui::*;
struct Pad {
    clicks: usize,
}

impl Render for Pad {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pad = cx.weak_entity();
        canvas(
            |bounds, window, _cx| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_bounds, hitbox, window, _cx| {
                window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
                let pad = pad.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase.bubble()
                        && event.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        pad.update(cx, |pad, cx| {
                            pad.clicks += 1;
                            cx.notify();
                        })
                        .ok();
                    }
                });
            },
        )
        .size_full()
    }
}

# let mut app = TestApp::new();
# let mut window = app.open_window(|_, _| Pad { clicks: 0 });
# window.draw();
# window.simulate_click(point(px(100.), px(100.)), MouseButton::Left);
# window.read(|pad, _| assert_eq!(pad.clicks, 1));
```

For drags, record the press in view state when `hitbox.is_hovered(window)` is
true, then handle [`MouseMoveEvent`] and [`MouseUpEvent`] *without* requiring
`is_hovered`, so dragging continues smoothly when the pointer leaves the bounds
and releasing outside still ends the drag.

When custom drawing also needs custom layout measurement, child elements, an
[`ElementId`] for retained state, or an accessibility node, implement
[`Element`] directly instead; see [elements, components and views](crate::_documentation::concepts::elements#custom-elements)
and the [`custom_element` example](crate::_documentation::examples::custom_element).

## See also

* [Canvas example](crate::_documentation::examples::canvas) — a chart built from
  paths and quads (`cargo run --example canvas`).
* [Custom element example](crate::_documentation::examples::custom_element) — a
  slider implementing [`Element`] across layout, prepaint, and paint.
* [The frame](crate::_documentation::concepts::the_frame) — how layout,
  prepaint, and paint fit into a frame.
