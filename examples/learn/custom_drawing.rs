//! Custom Drawing Example
//!
//! This example demonstrates custom drawing in GPUI using:
//!
//! 1. `canvas` element - For direct painting control
//! 2. `PathBuilder` - Creating custom vector shapes
//! 3. `window.paint_*` methods - Drawing quads, paths, and more
//! 4. Interactive drawing - Responding to mouse events

use gpui::{
    App, Application, Bounds, Context, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Path, PathBuilder, Pixels, Point, Render, Window, WindowBounds, WindowOptions,
    canvas, div, fill, point, prelude::*, px, rgb, size,
};

// ============================================================================
// Example 1: Basic Canvas Drawing
// ============================================================================
//
// The `canvas` element provides two callbacks:
// - prepaint: Called during layout to prepare drawing state
// - paint: Called during paint to actually draw

fn basic_shapes_canvas() -> impl IntoElement {
    canvas(
        move |_bounds, _window, _cx| {
            // Prepaint callback - prepare any state needed for painting
        },
        move |bounds, _prepaint_state, window, _cx| {
            // Paint callback - do actual drawing

            // Draw a filled rectangle
            window.paint_quad(fill(
                Bounds {
                    origin: point(bounds.origin.x + px(10.), bounds.origin.y + px(10.)),
                    size: size(px(60.), px(40.)),
                },
                rgb(0xef4444), // Red
            ));

            // Draw another rectangle
            window.paint_quad(fill(
                Bounds {
                    origin: point(bounds.origin.x + px(80.), bounds.origin.y + px(10.)),
                    size: size(px(60.), px(40.)),
                },
                rgb(0x22c55e), // Green
            ));

            // Draw a third rectangle
            window.paint_quad(fill(
                Bounds {
                    origin: point(bounds.origin.x + px(150.), bounds.origin.y + px(10.)),
                    size: size(px(60.), px(40.)),
                },
                rgb(0x3b82f6), // Blue
            ));
        },
    )
    .size_full()
}

// ============================================================================
// Example 2: Custom Paths with PathBuilder
// ============================================================================
//
// PathBuilder lets you create complex vector shapes:
// - move_to: Start a new subpath
// - line_to: Draw a straight line
// - curve_to: Draw a bezier curve
// - close: Close the current subpath

fn create_star(center: Point<Pixels>, outer_radius: f32, inner_radius: f32) -> Path<Pixels> {
    let mut builder = PathBuilder::fill();
    let points = 5;

    for i in 0..points * 2 {
        let angle =
            std::f32::consts::PI / 2.0 - (i as f32) * std::f32::consts::PI / (points as f32);
        let radius = if i % 2 == 0 {
            outer_radius
        } else {
            inner_radius
        };

        let x = center.x + px(angle.cos() * radius);
        let y = center.y - px(angle.sin() * radius);

        if i == 0 {
            builder.move_to(point(x, y));
        } else {
            builder.line_to(point(x, y));
        }
    }

    builder.close();
    builder.build().unwrap()
}

fn create_triangle(p1: Point<Pixels>, p2: Point<Pixels>, p3: Point<Pixels>) -> Path<Pixels> {
    let mut builder = PathBuilder::fill();
    builder.move_to(p1);
    builder.line_to(p2);
    builder.line_to(p3);
    builder.close();
    builder.build().unwrap()
}

fn custom_paths_canvas() -> impl IntoElement {
    canvas(
        move |_bounds, _window, _cx| {},
        move |bounds, _, window, _cx| {
            let center_y = bounds.origin.y + bounds.size.height / 2.0;

            // Draw a star
            let star_center = point(bounds.origin.x + px(50.), center_y);
            let star = create_star(star_center, 30., 15.);
            window.paint_path(star, rgb(0xeab308)); // Yellow

            // Draw a triangle
            let tri_base_x = bounds.origin.x + px(120.);
            let triangle = create_triangle(
                point(tri_base_x + px(30.), center_y - px(25.)),
                point(tri_base_x, center_y + px(25.)),
                point(tri_base_x + px(60.), center_y + px(25.)),
            );
            window.paint_path(triangle, rgb(0x8b5cf6)); // Purple

            // Draw a custom shape (arrow)
            let arrow_x = bounds.origin.x + px(200.);
            let mut arrow_builder = PathBuilder::fill();
            arrow_builder.move_to(point(arrow_x, center_y));
            arrow_builder.line_to(point(arrow_x + px(20.), center_y - px(20.)));
            arrow_builder.line_to(point(arrow_x + px(20.), center_y - px(10.)));
            arrow_builder.line_to(point(arrow_x + px(50.), center_y - px(10.)));
            arrow_builder.line_to(point(arrow_x + px(50.), center_y + px(10.)));
            arrow_builder.line_to(point(arrow_x + px(20.), center_y + px(10.)));
            arrow_builder.line_to(point(arrow_x + px(20.), center_y + px(20.)));
            arrow_builder.close();
            let arrow = arrow_builder.build().unwrap();
            window.paint_path(arrow, rgb(0x06b6d4)); // Cyan
        },
    )
    .size_full()
}

// ============================================================================
// Example 3: Interactive Drawing
// ============================================================================
//
// Combine canvas with mouse events for interactive drawing

struct DrawingCanvas {
    lines: Vec<Vec<Point<Pixels>>>,
    current_line: Vec<Point<Pixels>>,
    is_drawing: bool,
    color_index: usize,
}

impl DrawingCanvas {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_line: Vec::new(),
            is_drawing: false,
            color_index: 0,
        }
    }

    fn colors() -> &'static [u32] {
        &[0xef4444, 0x22c55e, 0x3b82f6, 0xeab308, 0x8b5cf6, 0x06b6d4]
    }

    fn current_color(&self) -> u32 {
        Self::colors()[self.color_index % Self::colors().len()]
    }

    fn next_color(&mut self) {
        self.color_index += 1;
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Left {
            self.is_drawing = true;
            self.current_line = vec![event.position];
            cx.notify();
        }
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_drawing {
            self.current_line.push(event.position);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_drawing && self.current_line.len() > 1 {
            self.lines.push(std::mem::take(&mut self.current_line));
            self.next_color();
        }
        self.is_drawing = false;
        self.current_line.clear();
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.lines.clear();
        self.current_line.clear();
        self.color_index = 0;
        cx.notify();
    }

    fn draw_line(window: &mut Window, points: &[Point<Pixels>], color: u32) {
        if points.len() < 2 {
            return;
        }

        for pair in points.windows(2) {
            let start = pair[0];
            let end = pair[1];

            // Draw line segment as a thin quad
            let dx = end.x - start.x;
            let dy = end.y - start.y;

            // Convert to f32 for math operations
            let dx_f = f32::from(dx);
            let dy_f = f32::from(dy);
            let len = (dx_f * dx_f + dy_f * dy_f).sqrt();

            if len < 0.1 {
                continue;
            }

            // Perpendicular offset for line thickness
            let thickness = 3.0_f32;
            let px_offset = px(-dy_f / len * thickness / 2.0);
            let py_offset = px(dx_f / len * thickness / 2.0);

            let mut builder = PathBuilder::fill();
            builder.move_to(point(start.x + px_offset, start.y + py_offset));
            builder.line_to(point(end.x + px_offset, end.y + py_offset));
            builder.line_to(point(end.x - px_offset, end.y - py_offset));
            builder.line_to(point(start.x - px_offset, start.y - py_offset));
            builder.close();

            if let Ok(path) = builder.build() {
                window.paint_path(path, rgb(color));
            }
        }
    }
}

impl Render for DrawingCanvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.lines.clone();
        let current_line = self.current_line.clone();
        let current_color = self.current_color();
        let colors = Self::colors().to_vec();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("drawing-area")
                    .h_48()
                    .rounded_lg()
                    .bg(rgb(0x1e293b))
                    .border_1()
                    .border_color(rgb(0x334155))
                    .cursor_crosshair()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .child(
                        canvas(
                            move |_, _, _| {},
                            move |_bounds, _, window, _cx| {
                                // Draw completed lines
                                for (i, line) in lines.iter().enumerate() {
                                    let color = colors[i % colors.len()];
                                    DrawingCanvas::draw_line(window, line, color);
                                }

                                // Draw current line being drawn
                                if !current_line.is_empty() {
                                    DrawingCanvas::draw_line(window, &current_line, current_color);
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("clear-btn")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0xef4444))
                            .text_sm()
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xdc2626)))
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear(cx);
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x94a3b8))
                            .child("Click and drag to draw"),
                    ),
            )
    }
}

// ============================================================================
// Main Application View
// ============================================================================

struct CustomDrawingExample {
    drawing_canvas: gpui::Entity<DrawingCanvas>,
}

impl CustomDrawingExample {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            drawing_canvas: cx.new(|_| DrawingCanvas::new()),
        }
    }
}

impl Render for CustomDrawingExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("main")
            .size_full()
            .p_6()
            .bg(rgb(0x0f172a))
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .max_w(px(500.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::white())
                                    .child("Custom Drawing"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x94a3b8))
                                    .child("Canvas element, paths, and interactive painting"),
                            ),
                    )
                    .child(section(
                        "1. Basic Shapes (paint_quad)",
                        "Use window.paint_quad() to draw filled rectangles",
                        basic_shapes_canvas(),
                        px(70.),
                    ))
                    .child(section(
                        "2. Custom Paths (PathBuilder)",
                        "Create complex shapes with PathBuilder and paint_path()",
                        custom_paths_canvas(),
                        px(80.),
                    ))
                    .child(section(
                        "3. Interactive Drawing",
                        "Combine canvas with mouse events for drawing",
                        self.drawing_canvas.clone(),
                        px(240.),
                    ))
                    .child(
                        div().p_3().rounded_lg().bg(rgb(0x1e293b)).child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .child("Key APIs:")
                                .child("• canvas(prepaint, paint) - Custom drawing element")
                                .child("• PathBuilder::fill() / stroke() - Create vector paths")
                                .child("• window.paint_quad(fill(...)) - Draw rectangles")
                                .child("• window.paint_path(path, color) - Draw custom paths"),
                        ),
                    ),
            )
    }
}

fn section(
    title: &'static str,
    description: &'static str,
    content: impl IntoElement,
    height: Pixels,
) -> impl IntoElement {
    let surface: Hsla = rgb(0x1e293b).into();

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(surface.opacity(0.5))
        .border_1()
        .border_color(rgb(0x334155))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(gpui::white())
                        .child(title),
                )
                .child(div().text_xs().text_color(rgb(0x94a3b8)).child(description)),
        )
        .child(div().h(height).child(content))
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(550.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| CustomDrawingExample::new(cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
    });
}
