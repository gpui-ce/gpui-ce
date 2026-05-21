use gpui::{
    App, Application, Bounds, Context, Render, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size, uniform_list,
};

struct UniformListExample {
    scroll_handle: UniformListScrollHandle,
}

impl Render for UniformListExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0xffffff)).child(
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
                cx.new(|_| UniformListExample {
                    scroll_handle: UniformListScrollHandle::new(),
                })
            },
        )
        .unwrap();
    });
}
