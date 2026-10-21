//! Example of pointer locking.

#[path = "shared/prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{App, Context, Pixels, Point, Render, Window, WindowOptions, div, prelude::*, white};

#[derive(Default)]
struct PointerLockingExample {
    pointer_locked: bool,
    pointer_delta: Point<Pixels>,
}

impl Render for PointerLockingExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .text_color(white())
            .child(div().child(format!(
                "{}\n (delta {})",
                if self.pointer_locked {
                    "Click to release pointer"
                } else {
                    "Click to capture pointer"
                },
                self.pointer_delta
            )))
            .id("main")
            .on_mouse_move(
                cx.listener(|state, mouse_move: &gpui::MouseMoveEvent, window, _| {
                    state.pointer_delta = mouse_move.delta;
                    window.refresh();
                }),
            )
            .on_click(cx.listener(|state, _, window, _| {
                state.pointer_locked = !state.pointer_locked;
                window.set_pointer_locked(state.pointer_locked);
            }))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| PointerLockingExample::default())
        })
        .unwrap();

        init_example(cx, "Pointer locking");
    });
}
