//! Plays haptic feedback when the pointer moves onto a button or onto a step of a scale.
//!
//! `supports_haptic_feedback` reports whether the machine can play haptics, and
//! `play_haptic_feedback` plays one in a `HapticFeedbackStyle`. On macOS, GPUI plays them through
//! `NSHapticFeedbackManager` on a Force Touch trackpad. Other platforms accept the calls and do
//! nothing.
//!
//! Run it with `cargo run --example haptics`.

use gpui::{
    App, Bounds, Context, HapticFeedbackStyle, QuitMode, Window, WindowBounds, WindowOptions,
    colors::Colors, div, prelude::*, px, size,
};

const STYLES: [(&str, HapticFeedbackStyle); 3] = [
    ("Generic", HapticFeedbackStyle::Generic),
    ("Alignment", HapticFeedbackStyle::Alignment),
    ("Level change", HapticFeedbackStyle::LevelChange),
];

struct Haptics {
    supported: bool,
    level: usize,
}

impl Render for Haptics {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        let buttons = STYLES.into_iter().map(|(label, style)| {
            div()
                .id(label)
                .px_4()
                .py_3()
                .rounded_md()
                .bg(colors.selected)
                .text_color(colors.selected_text)
                .hover(|button| button.opacity(0.85))
                .child(label)
                .on_hover(move |hovered, _window, cx| {
                    if *hovered {
                        cx.play_haptic_feedback(style);
                    }
                })
        });

        // Each step plays `LevelChange` as the pointer moves onto it, like the detents of a slider.
        let steps = (0..=10).map(|level| {
            div()
                .id(("level", level))
                .flex_1()
                .h_6()
                .rounded_sm()
                .bg(if level <= self.level {
                    colors.selected
                } else {
                    colors.container
                })
                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                    if *hovered && this.level != level {
                        this.level = level;
                        cx.play_haptic_feedback(HapticFeedbackStyle::LevelChange);
                        cx.notify();
                    }
                }))
        });

        let hint = |text: String| div().text_sm().text_color(colors.disabled).child(text);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(if self.supported {
                "This machine supports haptic feedback."
            } else {
                "This machine doesn't support haptic feedback, so the calls do nothing."
            })
            .child(hint(
                "Move the pointer onto a button to feel its pattern.".into(),
            ))
            .child(div().flex().gap_3().children(buttons))
            .child(hint(format!(
                "Move the pointer along the scale. Level {}",
                self.level
            )))
            .child(div().flex().gap_1().children(steps))
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(520.), px(320.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| {
            cx.new(|cx| Haptics {
                supported: cx.supports_haptic_feedback(),
                level: 0,
            })
        },
    )
    .unwrap();
    cx.activate(true);
}
