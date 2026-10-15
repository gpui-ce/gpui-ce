//! Custom drawing with `canvas`: a small chart built from paths and quads.
//!
//! `canvas` gives you the bounds of an element and the `Window` to paint into.
//! The area and the line use `PathBuilder` and `paint_path`; the grid lines and
//! the round data points use `paint_quad`. All positions are computed from the
//! bounds, so the chart scales with its container.
//!
//! Run it with `cargo run --example canvas`.

use gpui::{
    App, Bounds, Context, PathBuilder, Pixels, Point, QuitMode, Window, WindowBounds,
    WindowOptions, canvas, colors::Colors, div, fill, point, prelude::*, px, rgb, rgba, size,
};

const SERIES: [(&str, &[f32]); 2] = [
    (
        "Signups",
        &[12., 18., 15., 26., 22., 31., 28., 40., 36., 44., 41., 52.],
    ),
    (
        "Churn",
        &[30., 27., 33., 24., 29., 21., 25., 18., 22., 16., 19., 12.],
    ),
];

struct Chart {
    series: usize,
}

impl Render for Chart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let (name, values) = SERIES[self.series];
        let other = SERIES[1 - self.series].0;

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().text_lg().child(name))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors.disabled)
                                    .child("Last 12 months"),
                            ),
                    )
                    .child(
                        div()
                            .id("switch")
                            .px_3()
                            .py_1p5()
                            .rounded_lg()
                            .text_sm()
                            .bg(colors.selected)
                            .text_color(colors.selected_text)
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .child(format!("Show {other}"))
                            .on_click(cx.listener(|chart, _event, _window, cx| {
                                chart.series = 1 - chart.series;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            paint_chart(bounds, values, &colors, window);
                        },
                    )
                    .size_full(),
                ),
            )
    }
}

fn paint_chart(bounds: Bounds<Pixels>, values: &[f32], colors: &Colors, window: &mut Window) {
    let accent = rgb(0x3b82f6);
    let plot = bounds.inset(px(12.));

    for i in 0..=4 {
        let y = plot.origin.y + plot.size.height * (i as f32 / 4.);
        window.paint_quad(fill(
            Bounds::new(point(plot.origin.x, y), size(plot.size.width, px(1.))),
            colors.separator,
        ));
    }

    let max = values.iter().copied().fold(0., f32::max) * 1.1;
    let step = plot.size.width / (values.len() - 1) as f32;
    let points: Vec<Point<Pixels>> = values
        .iter()
        .copied()
        .enumerate()
        .map(|(i, value)| {
            point(
                plot.origin.x + step * i as f32,
                plot.origin.y + plot.size.height * (1. - value / max),
            )
        })
        .collect();
    let (first, last) = (points[0], points[points.len() - 1]);
    let baseline = plot.origin.y + plot.size.height;

    let mut area = PathBuilder::fill();
    area.move_to(point(first.x, baseline));
    for p in &points {
        area.line_to(*p);
    }
    area.line_to(point(last.x, baseline));
    area.close();
    if let Ok(path) = area.build() {
        window.paint_path(path, rgba(0x3b82f626));
    }

    let mut line = PathBuilder::stroke(px(2.));
    line.move_to(first);
    for p in &points[1..] {
        line.line_to(*p);
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, accent);
    }

    let radius = px(4.);
    for p in &points {
        window.paint_quad(
            fill(
                Bounds::centered_at(*p, size(radius * 2., radius * 2.)),
                accent,
            )
            .corner_radii(radius)
            .border_widths(px(2.))
            .border_color(colors.background),
        );
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(720.), px(480.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Chart { series: 0 }),
    )
    .unwrap();
    cx.activate(true);
}
