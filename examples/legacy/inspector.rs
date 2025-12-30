//! Example demonstrating the GPUI inspector dev-tools.
//!
//! Run with:
//! ```sh
//! cargo run --example inspector --features dev-tools
//! ```
//!
//! Press Cmd+Shift+I (macOS) or Ctrl+Shift+I (other platforms) to toggle the inspector.
//! Click the "🔍 Pick" button to enter element picking mode, then click on elements to inspect them.
//! Scroll while hovering to select parent/child elements in the hierarchy.

use gpui::{
    App, Application, Bounds, Context, KeyBinding, Window, WindowBounds, WindowOptions, actions,
    div, prelude::*, px, rgb, size,
};

#[cfg(feature = "dev-tools")]
use gpui::dev_tools;

actions!(inspector_example, [Quit]);

struct InspectorExample {
    counter: u32,
}

impl InspectorExample {
    fn increment(&mut self, cx: &mut Context<Self>) {
        self.counter += 1;
        cx.notify();
    }
}

impl Render for InspectorExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counter = self.counter;

        div()
            .id("root")
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .bg(rgb(0x1e1e2e))
            .size_full()
            .text_color(rgb(0xcdd6f4))
            .child(
                div()
                    .id("header")
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Inspector Example"),
            )
            .child(
                div()
                    .id("instructions")
                    .text_sm()
                    .text_color(rgb(0xa6adc8))
                    .child("Press Cmd+Shift+I (macOS) or Ctrl+Shift+I to toggle the inspector."),
            )
            .child(
                div()
                    .id("content")
                    .flex()
                    .flex_row()
                    .gap_4()
                    .child(
                        div()
                            .id("card-1")
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_4()
                            .bg(rgb(0x313244))
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(0x45475a))
                            .child(div().text_base().child("Card 1"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xa6adc8))
                                    .child("This is a card with some content."),
                            ),
                    )
                    .child(
                        div()
                            .id("card-2")
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_4()
                            .bg(rgb(0x313244))
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(0x45475a))
                            .child(div().text_base().child("Card 2"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        div().id("box-red").size_8().bg(rgb(0xf38ba8)).rounded_md(),
                                    )
                                    .child(
                                        div()
                                            .id("box-green")
                                            .size_8()
                                            .bg(rgb(0xa6e3a1))
                                            .rounded_md(),
                                    )
                                    .child(
                                        div()
                                            .id("box-blue")
                                            .size_8()
                                            .bg(rgb(0x89b4fa))
                                            .rounded_md(),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .id("counter-section")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .bg(rgb(0x313244))
                    .rounded_lg()
                    .child(div().text_base().child(format!("Counter: {}", counter)))
                    .child(
                        div()
                            .id("increment-button")
                            .px_4()
                            .py_2()
                            .bg(rgb(0x89b4fa))
                            .text_color(rgb(0x1e1e2e))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xb4befe)))
                            .child("Increment")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.increment(cx);
                            })),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        // Initialize default colors for the inspector UI
        cx.init_colors();

        // Initialize the dev-tools (inspector)
        #[cfg(feature = "dev-tools")]
        dev_tools::init(cx);

        // Bind keyboard shortcut to toggle the inspector
        #[cfg(feature = "dev-tools")]
        {
            #[cfg(target_os = "macos")]
            let toggle_key = "cmd-shift-i";
            #[cfg(not(target_os = "macos"))]
            let toggle_key = "ctrl-shift-i";

            cx.bind_keys([KeyBinding::new(
                toggle_key,
                dev_tools::ToggleInspector,
                None,
            )]);
        }

        // Bind quit shortcut
        cx.on_action(|_: &Quit, cx| cx.quit());

        #[cfg(target_os = "macos")]
        let quit_key = "cmd-q";
        #[cfg(not(target_os = "macos"))]
        let quit_key = "ctrl-q";

        cx.bind_keys([KeyBinding::new(quit_key, Quit, None)]);

        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| InspectorExample { counter: 0 }),
        )
        .unwrap();

        cx.activate(true);
    });
}
