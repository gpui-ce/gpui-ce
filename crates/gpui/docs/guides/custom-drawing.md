# Draw custom graphics

Use [`canvas`] when the built-in elements do not express the geometry you need. A canvas
takes two closures: prepaint computes data that depends on final bounds, and paint turns
that data into primitives through a [`Window`]. Both closures receive the canvas's
[`Bounds`] in window coordinates, so the value returned by prepaint connects layout to
painting without storing state in the canvas.

## Draw into a canvas

The value that prepaint returns is handed to paint. Use it to pass computed
geometry from one phase to the other:

```rust
# use gpui::*;
# struct Meter;
# impl Render for Meter {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
let meter = canvas(
    |bounds, _window, _cx| bounds.size.width * 0.6,
    |bounds, filled_width, window, _cx| {
        window.paint_quad(outline(bounds, rgb(0x9ca3af), BorderStyle::Solid));
        let filled = Bounds::new(bounds.origin, size(filled_width, bounds.size.height));
        window.paint_quad(fill(filled, rgb(0x2563eb)));
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

Both closures run once per frame. A canvas has no ID and no state of its own; keep the
data it needs on the view and move a copy into the closures.

## Paint rectangles with `paint_quad`

[`Window::paint_quad`] draws a [`PaintQuad`], and three functions build one:

- [`fill`] paints a background with no border.
- [`outline`] paints a 1px border with no background.
- [`quad`] takes every part: corner radii, background, border widths, border
  color and [`BorderStyle`].

A quad's corners can be rounded and its border dashed. Use builder methods
on [`PaintQuad`] to adjust a quad from [`fill`] or [`outline`]:

```rust
# use gpui::*;
# let bounds = Bounds::new(point(px(0.), px(0.)), size(px(120.), px(48.)));
# fn paint(bounds: Bounds<Pixels>, window: &mut Window) {
let card = fill(bounds, rgb(0xf9fafb))
    .corner_radii(px(8.))
    .border_widths(px(1.))
    .border_color(rgb(0xd1d5db));
window.paint_quad(card);
# }
```

Corner radii larger than half the quad's size produce sharp corners where the
arcs meet. Use `Corners::clamp_radii_for_quad_size` when the radii must fit the quad.

## Draw lines and curves with `PathBuilder`

[`PathBuilder`] builds a [`Path`] from commands. Use [`PathBuilder::stroke`] for
lines, which needs a width, and [`PathBuilder::fill`] for closed shapes. Commands
are `move_to`, `line_to`, [`PathBuilder::curve_to`] for a quadratic curve,
[`PathBuilder::cubic_bezier_to`] for a cubic one, and `close`. Call
[`PathBuilder::build`], which returns a `Result`, and then
[`Window::paint_path`] with a color:

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

`build` can fail for a degenerate path or invalid dash pattern, so check the result. A
dash pattern goes on the builder with
[`PathBuilder::dash_array`]. A stroke is tessellated into triangles in `build`,
so build a path once and reuse it while its shape stays the same, rather than
rebuilding every frame.

## Draw shadows

[`Window::paint_drop_shadows`] paints one or more [`BoxShadow`]s behind a shape,
and [`Window::paint_inset_shadows`] paints them inside it. Each shadow has a
color, an offset, a blur radius, a spread radius and an `inset` flag. The drop
call ignores shadows marked `inset`.

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
window.paint_drop_shadows(bounds, Corners::all(px(8.)), &[shadow]);
window.paint_quad(fill(bounds, rgb(0xffffff)).corner_radii(px(8.)));
# }
```

Paint the shadow before its shape: primitives are composited in call order, so the later
shape covers the shadow where they overlap.

## Draw images and text

[`Window::paint_image`] draws a decoded [`RenderImage`] into `image_bounds`. Only
the part inside `bounds` is visible, and `corner_radii` applies to `bounds`. It
returns a `Result`, and it takes the frame index for animated images. Get the image with
[`Window::use_asset`] and [`ImgResourceLoader`], the same loader [`img`] uses,
so the image is loaded once and shared:

```rust
# use gpui::*;
# use std::path::PathBuf;
fn paint_thumbnail(path: PathBuf, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if let Some(Ok(image)) = window.use_asset::<ImgResourceLoader>(&Resource::from(path), cx) {
        let _ = window.paint_image(bounds, bounds, Corners::all(px(6.)), image, 0, false);
    }
}
```

`use_asset` returns `None` while the image loads, and asks for a redraw when it
is ready. Nothing is drawn until then.

To draw text, shape it with [`WindowTextSystem::shape_line`] and paint the
[`ShapedLine`] at an origin. The [`text`](mod@crate::_documentation::guides::text)
guide covers shaping and measuring in detail:

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

## Clip what you draw

[`Window::with_content_mask`] limits everything painted inside its closure to a
rectangle. The mask is intersected with the mask already in effect, so a clip
can only shrink the visible area. Pass `None` to keep the current mask:

```rust
# use gpui::*;
# fn paint(bounds: Bounds<Pixels>, window: &mut Window) {
let inner = Bounds::new(
    point(bounds.origin.x + px(8.), bounds.origin.y + px(8.)),
    size(bounds.size.width - px(16.), bounds.size.height - px(16.)),
);

window.with_content_mask(Some(ContentMask { bounds: inner, ..Default::default() }), |window| {
    window.paint_quad(fill(bounds, rgb(0xef4444)));
});
# }
```

Anything outside `inner` is not drawn.

## Make a canvas interactive

A canvas has no ID, so it cannot use `on_click` or `on_hover` like a [`Div`]. Connect it
to input in two steps:

1. In prepaint, insert a hitbox with [`Window::insert_hitbox`] and return it.
   Hitboxes are how the window decides which element is under the pointer.
2. In paint, register a listener with [`Window::on_mouse_event`]. Listeners are
   removed at the start of the next frame, so register them on every paint.
   Set a cursor with [`Window::set_cursor_style`] while you are there.

A listener is called in the capture and bubble phases. Check the
[`DispatchPhase`], and check [`Hitbox::is_hovered`] so that a click on an
overlapping element above the canvas is ignored. To change view state, keep a
[`WeakEntity`] for the view and update it from the listener:

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
                    if phase == DispatchPhase::Bubble
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

The same pattern works for dragging. Listen for [`MouseMoveEvent`] and
[`MouseUpEvent`] as well, and record the press in your view state so that a drag
that leaves the canvas still ends correctly.

## Implement `Element` instead

A canvas is convenient, but it is a closure pair with no state of its own. Write
a type that implements [`Element`] when you need one of these:

- Child elements that the element lays out itself.
- State that persists between frames in the element, not in the view.
- Full control over the three phases, or your own [`ElementId`] for
  accessibility.

An element goes through three phases. [`Element::request_layout`] asks the layout
engine for a size, and returns a [`LayoutId`] and any state it needs later.
[`Element::prepaint`] runs once the size is known. It inserts hitboxes and computes
anything that depends on the bounds. [`Element::paint`] draws, sets the cursor and
registers listeners. The [elements concept page](crate::_documentation::concepts::elements)
explains where each type fits, and the [custom element example](crate::_documentation::examples::custom_element)
builds a slider from these phases.

## See also

- [Canvas example](crate::_documentation::examples::canvas), a chart drawn from
  paths and quads. Run it with `cargo run --example canvas`.
- [Custom element example](crate::_documentation::examples::custom_element), a
  slider written against the `Element` trait.
- [The frame](crate::_documentation::concepts::the_frame) for when prepaint and
  paint run relative to layout and input.
- [Images and assets](crate::_documentation::guides::images_and_assets) for
  loading images that you paint yourself.
