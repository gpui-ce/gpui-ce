//! A custom element: a slider written by hand against the `Element` trait.
//!
//! `request_layout` asks Taffy for a box with a fixed height that grows to fill
//! its parent. `prepaint` inserts a hitbox, and `paint` draws the track, the fill
//! and the knob, sets the cursor and registers mouse listeners. The listeners
//! report new values through `on_change`, and the parent view owns the value.
//!
//! Run it with `cargo run --example custom_element`.

use std::{cell::Cell, panic::Location, rc::Rc};

use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, QuitMode, Render, Style, Window, WindowBounds,
    WindowOptions, colors::Colors, div, fill, point, prelude::*, px, rgb, size,
};

struct Slider {
    id: ElementId,
    value: f32,
    on_change: Rc<dyn Fn(&f32, &mut Window, &mut App)>,
}

fn slider(
    id: impl Into<ElementId>,
    value: f32,
    on_change: impl Fn(&f32, &mut Window, &mut App) + 'static,
) -> Slider {
    Slider {
        id: id.into(),
        value,
        on_change: Rc::new(on_change),
    }
}

impl IntoElement for Slider {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Maps a window x coordinate to a value between 0 and 1 across the track.
fn value_at(bounds: Bounds<Pixels>, x: Pixels) -> f32 {
    ((x - bounds.origin.x) / bounds.size.width).clamp(0., 1.)
}

impl Element for Slider {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    // The id is needed to keep the drag state across frames.
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.flex_grow = 1.;
        style.size.height = px(24.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let Some(id) = id else { return };
        let dragging = window.with_element_state::<Rc<Cell<bool>>, _>(id, |state, _| {
            let state = state.unwrap_or_default();
            (state.clone(), state)
        });

        let colors = Colors::for_appearance(window);
        let accent = rgb(0x3b82f6);
        let knob_size = px(16.);
        let track_height = px(6.);
        let track = Bounds::new(
            point(
                bounds.origin.x,
                bounds.origin.y + (bounds.size.height - track_height) / 2.,
            ),
            size(bounds.size.width, track_height),
        );
        let x = track.origin.x + track.size.width * self.value.clamp(0., 1.);

        let radius = track_height / 2.;
        let filled = Bounds::new(track.origin, size(x - track.origin.x, track_height));
        let knob = Bounds::centered_at(point(x, track.center().y), size(knob_size, knob_size));
        window.paint_quad(fill(track, colors.separator).corner_radii(radius));
        window.paint_quad(fill(filled, accent).corner_radii(radius));
        window.paint_quad(
            fill(knob, colors.background)
                .corner_radii(knob.size.width / 2.)
                .border_widths(px(2.))
                .border_color(accent),
        );

        window.set_cursor_style(CursorStyle::PointingHand, hitbox);

        let on_change = self.on_change.clone();
        window.on_mouse_event({
            let hitbox = hitbox.clone();
            let dragging = dragging.clone();
            let on_change = on_change.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    dragging.set(true);
                    let value = value_at(bounds, event.position.x);
                    on_change(&value, window, cx);
                }
            }
        });
        window.on_mouse_event({
            let dragging = dragging.clone();
            let on_change = on_change.clone();
            move |event: &MouseMoveEvent, _phase, window, cx| {
                if dragging.get() && event.pressed_button == Some(MouseButton::Left) {
                    let value = value_at(bounds, event.position.x);
                    on_change(&value, window, cx);
                }
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, _phase, _window, _cx| {
            if event.button == MouseButton::Left {
                dragging.set(false);
            }
        });
    }
}

struct SliderDemo {
    volume: f32,
}

impl Render for SliderDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap_3()
            .px_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_sm().text_color(colors.disabled).child("Volume"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(slider(
                        "volume",
                        self.volume,
                        cx.listener(|demo, value: &f32, _window, cx| {
                            demo.volume = *value;
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .w(px(48.))
                            .text_right()
                            .child(format!("{:.0}%", self.volume * 100.)),
                    ),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(520.), px(200.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| SliderDemo { volume: 0.6 }),
    )
    .unwrap();
    cx.activate(true);
}
