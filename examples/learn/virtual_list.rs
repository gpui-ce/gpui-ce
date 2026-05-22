use std::rc::Rc;

use gpui::{
    App, Application, Bounds, Context, MouseButton, Render, ScrollHandle,
    VirtualListScrollController, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb,
    rgba, size, vlist,
};

struct VirtualListExample {
    scroll_handle: ScrollHandle,
    controller: VirtualListScrollController,
    heights: Rc<Vec<gpui::Pixels>>,
}

impl Render for VirtualListExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .child(vlist(
                cx.entity(),
                "entries",
                self.heights.clone(),
                self.scroll_handle.clone(),
                self.controller.clone(),
                |_this, range, _window, _cx| {
                    let mut items = Vec::new();

                    for ix in range {
                        let item = ix + 1;

                        let height = if ix % 5 == 0 {
                            px(60.0)
                        } else if ix % 3 == 0 {
                            px(45.0)
                        } else {
                            px(30.0)
                        };

                        items.push(
                            div()
                                .id(ix)
                                .h(height)
                                .px_2()
                                .border_b_1()
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .on_click(move |_event, _window, _cx| {
                                    println!("clicked Item {item}");
                                })
                                .child(format!("Item {item} • height {:.0}px", height.to_f64())),
                        );
                    }

                    items
                },
            ))
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
                            this.controller.scroll_to_item(0);
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
                            this.controller.scroll_to_item(199);
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
                cx.new(|_| {
                    let heights = (0..200)
                        .map(|ix| {
                            if ix % 5 == 0 {
                                px(60.0)
                            } else if ix % 3 == 0 {
                                px(45.0)
                            } else {
                                px(30.0)
                            }
                        })
                        .collect();

                    VirtualListExample {
                        scroll_handle: ScrollHandle::new(),
                        controller: VirtualListScrollController::new(),
                        heights: Rc::new(heights),
                    }
                })
            },
        )
        .unwrap();
    });
}
