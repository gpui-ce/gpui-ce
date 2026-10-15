//! Four kinds of motion: a one-shot entrance, a repeating pulse, a spring-driven switch, and a style transition.
//!
//! `AnimationExt::with_animation` runs a timed `Animation` on an element. Changing its element id
//! starts it again. `with_spring` drives a value with a physical spring, and `transitions` eases
//! style changes such as a hover color. Animations respect `App::reduce_motion`: when it is set,
//! they render in their static state instead of moving.
//!
//! Run it with `cargo run --example animation`.

use gpui::{
    Animation, AnimationExt, App, Bounds, Context, Div, QuitMode, SpringAnimation, Window,
    WindowBounds, WindowOptions, colors::Colors, div, ease_out_quint, millis, prelude::*,
    pulsating_between, px, rgb, size, spring,
};

#[derive(Default)]
struct Animations {
    entrance_generation: usize,
    switch_on: bool,
}

fn section(colors: &Colors, title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
}

impl Render for Animations {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        // Changing the id restarts the entrance animation.
        let entrance = div()
            .p_4()
            .rounded_lg()
            .bg(colors.container)
            .border_1()
            .border_color(colors.border)
            .child("Slides in and fades up")
            .with_animation(
                ("entrance", self.entrance_generation),
                Animation::new(millis(600)).with_easing(ease_out_quint()),
                |card, delta| card.opacity(delta).mt(px(16. * (1. - delta))),
            );

        let pulse = div()
            .size_3()
            .rounded_full()
            .bg(rgb(0x22c55e))
            .with_animation(
                "pulse",
                Animation::new(millis(1600))
                    .repeat()
                    .with_easing(pulsating_between(0.3, 1.0)),
                |dot, delta| dot.opacity(delta),
            );

        // The knob's position comes from a spring. Keeping the id stable lets the spring keep its
        // velocity when the target changes.
        let target = if self.switch_on { 1. } else { 0. };
        let knob = div()
            .absolute()
            .top(px(3.))
            .size(px(24.))
            .rounded_full()
            .bg(gpui::white())
            .with_spring(
                "switch-knob",
                SpringAnimation::new(spring(320., 24., 1.)).to(target),
                |knob, position| knob.left(px(3. + 22. * position)),
            );
        let track = if self.switch_on {
            colors.selected
        } else {
            colors.separator
        };
        let switch = div()
            .id("switch")
            .relative()
            .w(px(52.))
            .h(px(30.))
            .rounded_full()
            .cursor_pointer()
            .bg(track)
            .child(knob)
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.switch_on = !this.switch_on;
                cx.notify();
            }));

        div()
            .id("content")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                section(&colors, "Entrance").child(entrance).child(
                    div()
                        .id("replay")
                        .self_start()
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .text_sm()
                        .bg(colors.selected)
                        .text_color(colors.selected_text)
                        .cursor_pointer()
                        .child("Replay")
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.entrance_generation += 1;
                            cx.notify();
                        })),
                ),
            )
            .child(
                section(&colors, "Repeat")
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(pulse)
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child("Pulsing while idle"),
                    ),
            )
            .child(
                section(&colors, "Spring")
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(switch)
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child(if self.switch_on { "On" } else { "Off" }),
                    ),
            )
            .child(
                section(&colors, "Style transition").child(
                    div()
                        .id("hover-button")
                        .self_start()
                        .px_4()
                        .py_2()
                        .rounded_lg()
                        .bg(colors.container)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x2563eb)).text_color(gpui::white()))
                        .transitions(|transitions| {
                            transitions.bg(millis(250)).text_color(millis(250))
                        })
                        .child("Hover me"),
                ),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(520.), px(560.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Animations::default()),
    )
    .unwrap();
    cx.activate(true);
}
