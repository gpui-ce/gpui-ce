//! Style Selectors Example
//!
//! This example shows how a component can style part of another component without
//! adding presentation-specific properties to that component. The button selects
//! the icon's `button-icon` class and supplies its size.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{
    App, Bounds, Context, IntoElement, PathBuilder, Render, RenderOnce, Rgba, Window, WindowBounds,
    WindowOptions, canvas, div, point, prelude::*, px, rgb, selectors::class, size,
};

struct StyleSelectorsExample;

impl Render for StyleSelectorsExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x0f172a))
            .child(Button { label: "Continue" })
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| StyleSelectorsExample),
        )
        .expect("failed to open window");

        init_example(cx, "Style Selectors");
    });
}

#[derive(IntoElement)]
struct Button {
    label: &'static str,
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id("selector-button")
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .rounded_lg()
            .bg(rgb(0x2563eb))
            .text_color(rgb(0xffffff))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x1d4ed8)))
            .active(|style| style.bg(rgb(0x1e40af)))
            .select_children(class("icon"), |style| style.size(px(20.)))
            .child(Icon {
                color: rgb(0xffffff),
            })
            .child(self.label)
    }
}

#[derive(IntoElement)]
struct Icon {
    color: Rgba,
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center_y = bounds.origin.y + bounds.size.height / 2.;
                let left = bounds.origin.x + bounds.size.width * 0.15;
                let shoulder = bounds.origin.x + bounds.size.width * 0.55;
                let tip = bounds.origin.x + bounds.size.width * 0.85;
                let half_height = bounds.size.height * 0.3;
                let shaft_half_height = bounds.size.height * 0.1;

                let mut path = PathBuilder::fill();
                path.move_to(point(left, center_y - shaft_half_height));
                path.line_to(point(shoulder, center_y - shaft_half_height));
                path.line_to(point(shoulder, center_y - half_height));
                path.line_to(point(tip, center_y));
                path.line_to(point(shoulder, center_y + half_height));
                path.line_to(point(shoulder, center_y + shaft_half_height));
                path.line_to(point(left, center_y + shaft_half_height));
                path.close();

                if let Ok(path) = path.build() {
                    window.paint_path(path, self.color);
                }
            },
        )
        .class("icon")
    }
}
