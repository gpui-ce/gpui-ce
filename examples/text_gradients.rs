use gpui::*;

/// Simple gradient test - static gradient from red to blue
struct GradientTest;

impl Render for GradientTest {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(rgb(0x1a1a1a))
            .gap_8()
            .child(
                div()
                    .text_size(px(72.0))
                    .font_weight(FontWeight::BOLD)
                    .text_gradient_horizontal(
                        linear_color_stop(rgb(0xFF0000), 0.0),
                        linear_color_stop(rgb(0x0000FF), 1.0),
                    )
                    .child("GRADIENT TEST"),
            )
            .child(
                div()
                    .text_size(px(48.0))
                    .text_color(rgb(0x00FF00))
                    .child("Solid Green (for comparison)"),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.0), px(400.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| GradientTest),
        )
        .expect("Failed to open window");
    });
}
