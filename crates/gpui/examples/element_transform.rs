//! Element Transform Example
//!
//! A pan and zoom camera over a small node graph. Every card is an ordinary `div` laid out in
//! world coordinates, and a single [`ElementTransform`] on their parent moves and scales the
//! whole subtree. Painting, clipping and hit testing follow the transform, so the cards keep
//! their hover styles, clicks and drags at any zoom level.
//!
//! - Scroll to zoom around the cursor (0.25x to 4x).
//! - Drag the background to pan.
//! - Drag a card to move it, click a card to select it and count the click.

#![cfg_attr(target_family = "wasm", no_main)]

#[path = "example_support/fonts.rs"]
mod example_support;

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, ClickEvent, Context, ElementTransform, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Render, ScrollWheelEvent, Window, WindowBounds, WindowOptions,
    canvas, div, point, prelude::*, px, rgb, rgba, size,
};
use gpui_platform::application;

const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 4.0;
const ZOOM_PER_SCROLLED_PIXEL: f32 = 0.0015;
const SCROLL_LINE_HEIGHT: Pixels = px(20.);

/// How a card is drawn, to cover the cases the transform has to handle.
#[derive(Clone, Copy, PartialEq)]
enum CardKind {
    Filled,
    /// Transparent background with a border only.
    Outlined,
    /// Filled, with a badge that carries its own transform inside the camera transform.
    Badged,
}

struct NodeCard {
    id: &'static str,
    title: &'static str,
    rows: &'static [&'static str],
    kind: CardKind,
    /// Top left corner in world coordinates.
    position: Point<Pixels>,
    click_count: usize,
}

impl NodeCard {
    fn new(
        id: &'static str,
        title: &'static str,
        rows: &'static [&'static str],
        kind: CardKind,
        position: Point<Pixels>,
    ) -> Self {
        Self {
            id,
            title,
            rows,
            kind,
            position,
            click_count: 0,
        }
    }
}

enum Drag {
    Pan {
        last_position: Point<Pixels>,
    },
    Card {
        index: usize,
        last_position: Point<Pixels>,
        moved: bool,
    },
}

struct ElementTransformExample {
    pan: Point<Pixels>,
    zoom: f32,
    cards: Vec<NodeCard>,
    selected: Option<usize>,
    drag: Option<Drag>,
    /// Window position of the viewport, recorded during prepaint. Mouse events report window
    /// coordinates, so zooming around the cursor needs it to find the point under the cursor.
    viewport_origin: Rc<Cell<Point<Pixels>>>,
}

impl ElementTransformExample {
    fn new() -> Self {
        Self {
            pan: Point::default(),
            zoom: 1.0,
            cards: vec![
                NodeCard::new(
                    "card-users",
                    "users",
                    &["id: uuid", "email: text", "created_at: timestamp"],
                    CardKind::Filled,
                    point(px(40.), px(90.)),
                ),
                NodeCard::new(
                    "card-orders",
                    "orders",
                    &["id: uuid", "user_id: uuid", "total: numeric"],
                    CardKind::Filled,
                    point(px(300.), px(110.)),
                ),
                NodeCard::new(
                    "card-order-items",
                    "order_items",
                    &["order_id: uuid", "product_id: uuid", "quantity: int"],
                    CardKind::Badged,
                    point(px(560.), px(90.)),
                ),
                NodeCard::new(
                    "card-products",
                    "products",
                    &["id: uuid", "name: text", "price: numeric"],
                    CardKind::Filled,
                    point(px(560.), px(310.)),
                ),
                NodeCard::new(
                    "card-notes",
                    "notes (outlined)",
                    &["transparent background", "border only"],
                    CardKind::Outlined,
                    point(px(300.), px(330.)),
                ),
                NodeCard::new(
                    "card-sessions",
                    "sessions",
                    &["token: text", "user_id: uuid", "expires_at: timestamp"],
                    CardKind::Filled,
                    point(px(40.), px(310.)),
                ),
                NodeCard::new(
                    "card-audit",
                    "audit_log",
                    &["actor: uuid", "action: text"],
                    CardKind::Outlined,
                    point(px(820.), px(200.)),
                ),
            ],
            selected: None,
            drag: None,
            viewport_origin: Rc::new(Cell::new(Point::default())),
        }
    }

    fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.pan = Point::default();
        self.zoom = 1.0;
        cx.notify();
    }

    fn zoom_around_cursor(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scrolled = f32::from(event.delta.pixel_delta(SCROLL_LINE_HEIGHT).y);
        let zoom =
            (self.zoom * (scrolled * ZOOM_PER_SCROLLED_PIXEL).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
        if zoom == self.zoom {
            return;
        }

        // Keep the world point under the cursor fixed while the zoom changes.
        let cursor = event.position - self.viewport_origin.get();
        let world_under_cursor = (cursor - self.pan).map(|value| value / self.zoom);
        self.pan = cursor - world_under_cursor.map(|value| value * zoom);
        self.zoom = zoom;
        cx.notify();
    }

    fn start_pan(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.drag = Some(Drag::Pan {
            last_position: event.position,
        });
        self.selected = None;
        cx.notify();
    }

    fn start_card_drag(&mut self, index: usize, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.drag = Some(Drag::Card {
            index,
            last_position: event.position,
            moved: false,
        });
        self.selected = Some(index);
        cx.stop_propagation();
        cx.notify();
    }

    fn continue_drag(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }

        match &mut self.drag {
            Some(Drag::Pan { last_position }) => {
                self.pan += event.position - *last_position;
                *last_position = event.position;
                cx.notify();
            }
            Some(Drag::Card {
                index,
                last_position,
                moved,
            }) => {
                // The pointer moves in window space, the card lives in world space.
                let world_delta = (event.position - *last_position).map(|value| value / self.zoom);
                *last_position = event.position;
                *moved = true;

                if let Some(card) = self.cards.get_mut(*index) {
                    card.position += world_delta;
                }
                cx.notify();
            }
            None => {}
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        cx.notify();
    }

    fn count_click(&mut self, index: usize, cx: &mut Context<Self>) {
        let dragged = matches!(self.drag, Some(Drag::Card { moved: true, .. }));
        if dragged {
            return;
        }

        if let Some(card) = self.cards.get_mut(index) {
            card.click_count += 1;
        }
        cx.notify();
    }

    fn render_card(
        &self,
        index: usize,
        card: &NodeCard,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let selected = self.selected == Some(index);
        let border_color = if selected {
            rgb(0x7aa2f7)
        } else {
            rgb(0x3b4261)
        };

        div()
            .id(card.id)
            .absolute()
            .left(card.position.x)
            .top(card.position.y)
            .w(px(200.))
            .p_3()
            .flex()
            .flex_col()
            .gap_1()
            .rounded(px(8.))
            .border_2()
            .border_color(border_color)
            .when(card.kind != CardKind::Outlined, |card_element| {
                card_element.bg(rgb(0x1f2335))
            })
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(0xbb9af7)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.start_card_drag(index, event, cx);
                }),
            )
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.count_click(index, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xc0caf5))
                            .child(card.title),
                    )
                    .when(card.kind == CardKind::Badged, |header| {
                        header.child(
                            div()
                                .px_1()
                                .rounded(px(4.))
                                .bg(rgb(0x9ece6a))
                                .text_xs()
                                .text_color(rgb(0x1a1b26))
                                .transform(
                                    ElementTransform::default()
                                        .scale(size(1.5, 1.5))
                                        .origin(point(0.5, 0.5)),
                                )
                                .child("1.5x"),
                        )
                    }),
            )
            .children(
                card.rows
                    .iter()
                    .map(|row| div().text_sm().text_color(rgb(0x9aa5ce)).child(*row)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x565f89))
                    .child(format!("clicks: {}", card.click_count)),
            )
    }

    fn render_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("camera-overlay")
            .absolute()
            .top_3()
            .left_3()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(6.))
            .bg(rgba(0x16161ee6))
            .border_1()
            .border_color(rgb(0x3b4261))
            .text_sm()
            .text_color(rgb(0xc0caf5))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation();
            })
            .child(format!("zoom {:.2}x", self.zoom))
            .child(format!(
                "pan {:.0}, {:.0}",
                f32::from(self.pan.x),
                f32::from(self.pan.y)
            ))
            .child(
                div()
                    .id("reset-view")
                    .px_2()
                    .rounded(px(4.))
                    .bg(rgb(0x3b4261))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x565f89)))
                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                        this.reset_view(cx);
                    }))
                    .child("Reset"),
            )
    }
}

impl Render for ElementTransformExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let camera = ElementTransform::default()
            .translate(self.pan)
            .scale(size(self.zoom, self.zoom));
        let viewport_origin = self.viewport_origin.clone();

        div().size_full().p_6().bg(rgb(0x16161e)).child(
            div()
                .id("viewport")
                .relative()
                .size_full()
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(0x3b4261))
                .bg(rgb(0x1a1b26))
                .on_scroll_wheel(cx.listener(Self::zoom_around_cursor))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::start_pan))
                .on_mouse_move(cx.listener(Self::continue_drag))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _event, _window, cx| this.end_drag(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _event, _window, cx| this.end_drag(cx)),
                )
                .child(
                    canvas(
                        move |bounds: Bounds<Pixels>, _window, _cx| {
                            viewport_origin.set(bounds.origin);
                        },
                        |_bounds, _state, _window, _cx| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    div()
                        .id("world")
                        .absolute()
                        .top_0()
                        .left_0()
                        .transform(camera)
                        .children(
                            self.cards
                                .iter()
                                .enumerate()
                                .map(|(index, card)| self.render_card(index, card, cx))
                                .collect::<Vec<_>>(),
                        ),
                )
                .child(self.render_overlay(cx)),
        )
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }

        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions::new()
                .window_bounds(Some(WindowBounds::Windowed(bounds)))
                .focus(true),
            |_, cx| cx.new(|_| ElementTransformExample::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    env_logger::init();
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
