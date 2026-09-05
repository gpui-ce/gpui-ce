//! A compact showcase of delayed, counted, and alternating motion.
//!
//! Run with `cargo run -p gpui-ce --example motion_patterns`.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use gpui::{
    App, AppContext, Bounds, Context, Motion, Repeat, Transition, Window, WindowBounds,
    WindowOptions, div, ease_in_out, millis, prelude::*, px, rgb, size,
};

struct MotionPatterns;

fn button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(rgb(0x30394d))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x45516c)))
        .child(label)
}

fn lane(
    title: &'static str,
    description: &'static str,
    value: f32,
    travel: f32,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .w(px(190.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(title)
                .child(div().text_sm().text_color(rgb(0xaeb9d0)).child(description)),
        )
        .child(
            div()
                .relative()
                .h(px(24.))
                .flex_1()
                .rounded(px(12.))
                .bg(rgb(0x101522))
                .child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(px(travel * value))
                        .size(px(16.))
                        .rounded(px(8.))
                        .bg(rgb(0x9bbcff)),
                ),
        )
}

impl Render for MotionPatterns {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let patterns = [
            (
                "Delay",
                "350 ms before one pass",
                Motion::new(millis(900)).with_delay(millis(350)),
            ),
            (
                "Count",
                "Three forward passes",
                Motion::new(millis(900)).with_repeat(Repeat::Count(3)),
            ),
            (
                "Auto-reverse",
                "Two passes return to start",
                Motion::new(millis(900))
                    .with_repeat(Repeat::Count(2))
                    .with_auto_reverse(true),
            ),
            (
                "Forever",
                "Alternates until Reset",
                Motion::new(millis(900))
                    .with_repeat(Repeat::Forever)
                    .with_auto_reverse(true),
            ),
        ];

        let travel = (f32::from(window.viewport_size().width) - 250.).max(0.);
        let mut transitions: Vec<Transition<f32>> = Vec::new();
        let mut lanes = Vec::new();
        for (index, (title, description, motion)) in patterns.into_iter().enumerate() {
            let transition = window.use_keyed_transition(
                ("motion-pattern", index),
                cx,
                motion.with_easing(ease_in_out),
                |_, _| 0.0_f32,
            );

            if cx.reduce_motion() {
                let goal = *transition.read_goal(cx);
                transition.jump_to(goal, cx);
            }

            let value = *transition.evaluate(window, cx);
            transitions.push(transition);
            lanes.push(lane(title, description, value, travel));
        }

        let play = transitions.clone();
        let reset = transitions.clone();
        div()
            .id("motion-patterns")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .bg(rgb(0x141a26))
            .text_color(rgb(0xe5edff))
            .child(div().text_xl().child("Motion patterns"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xaeb9d0))
                    .child("One keyed transition, four Motion configurations."),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(button("play", "Play").on_click(move |_, _, cx| {
                        for transition in &play {
                            transition.update(cx, |value, cx| {
                                *value = 1.;
                                cx.notify();
                            });
                        }
                    }))
                    .child(button("reset", "Reset").on_click(move |_, _, cx| {
                        for transition in &reset {
                            transition.reset(cx);
                        }
                    }))
                    .child(
                        button(
                            "reduce-motion",
                            if cx.reduce_motion() {
                                "Reduced motion: on"
                            } else {
                                "Reduced motion: off"
                            },
                        )
                        .on_click(|_, _, cx| cx.set_reduce_motion(!cx.reduce_motion())),
                    ),
            )
            .children(lanes)
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(700.), px(500.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Motion patterns");
                cx.new(|_| MotionPatterns)
            },
        )
        .expect("Failed to open window");

        example_prelude::init_example(cx, "Motion patterns");
    });
}
