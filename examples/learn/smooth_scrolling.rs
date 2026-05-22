use gpui::{
    App, Application, Bounds, Context, MouseButton, Render, ScrollStrategy,
    UniformListScrollHandle, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, rgba,
    size, uniform_list,
};

struct SmoothScrollingExample {
    scroll_handle: UniformListScrollHandle,
}

impl Render for SmoothScrollingExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .child(
                uniform_list(
                    "entries",
                    200,
                    cx.processor(|_this, range, _window, _cx| {
                        let mut items = Vec::new();

                        for ix in range {
                            let item = ix + 1;

                            items.push(
                                div()
                                    .id(ix)
                                    .h(px(30.0))
                                    .px_2()
                                    .border_b_1()
                                    .cursor_pointer()
                                    .on_click(move |_event, _window, _cx| {
                                        println!("clicked Item {item}");
                                    })
                                    .child(format!("Item {item}")),
                            );
                        }

                        items
                    }),
                )
                .track_scroll(&self.scroll_handle)
                .h_full(),
            )
            .child(
                div()
                    .id("scroll_to_top")
                    .absolute()
                    .right_4()
                    .top_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_40()
                    .h_12()
                    .bg(rgba(0x0000004D))
                    .border_1()
                    .border_color(rgba(0x00000080))
                    .rounded_lg()
                    .text_color(rgb(0xffffff))
                    .child("Scroll to top")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _cx| {
                            this.scroll_handle.scroll_to_item(0, ScrollStrategy::Top);
                            window.refresh();
                        }),
                    ),
            )
            .child(
                div()
                    .id("scroll_to_bottom")
                    .absolute()
                    .right_4()
                    .bottom_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_40()
                    .h_12()
                    .bg(rgba(0x0000004D))
                    .border_1()
                    .border_color(rgba(0x00000080))
                    .rounded_lg()
                    .text_color(rgb(0xffffff))
                    .child("Scroll to bottom")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _cx| {
                            this.scroll_handle
                                .scroll_to_item(199, ScrollStrategy::Bottom);
                            window.refresh();
                        }),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(300.0), px(400.0)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| {
                cx.new(|_| SmoothScrollingExample {
                    scroll_handle: UniformListScrollHandle::new(),
                })
            },
        )
        .unwrap();
    });
}
