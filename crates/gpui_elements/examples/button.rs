//! Run with `cargo run -p gpui_ce_elements --example button`.

use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, Window, WindowBounds, WindowOptions, actions,
    div, prelude::*, px, rgb, size,
};
use gpui_ce_elements::button::BaseButton;

actions!(button_example, [FocusNext, FocusPrevious]);

struct ButtonExample {
    focus_handle: FocusHandle,
    clicks: usize,
    disabled: bool,
}

impl Render for ButtonExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("button-example")
            .track_focus(&self.focus_handle)
            .tab_group()
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
            .size_full()
            .flex()
            .flex_col()
            .items_start()
            .gap_4()
            .p_6()
            .bg(rgb(0x18181b))
            .text_color(rgb(0xfafafa))
            .child("Tab / Shift+Tab to move focus. Enter or Space to activate.")
            .child(format!("Activations: {}", self.clicks))
            .child(
                button("count", "Count")
                    .disabled(self.disabled)
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })),
            )
            .child(
                button(
                    "toggle-disabled",
                    if self.disabled {
                        "Enable Count"
                    } else {
                        "Disable Count"
                    },
                )
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.disabled = !this.disabled;
                    cx.notify();
                })),
            )
            .child(
                button("disabled-focusable", "Disabled, still focusable")
                    .disabled(true)
                    .focusable_when_disabled(true),
            )
    }
}

fn button(element_id: &'static str, label: &'static str) -> BaseButton {
    BaseButton::new(element_id)
        .aria_label(label)
        .child(label)
        .px_4()
        .py_2()
        .rounded_md()
        .bg(rgb(0x2563eb))
        .border_2()
        .border_color(rgb(0x2563eb))
        .cursor_pointer()
        .focus_visible(|style| style.border_color(rgb(0xfbbf24)))
        .disabled_style(|style| style.opacity(0.4).cursor_not_allowed())
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", FocusNext, None),
            KeyBinding::new("shift-tab", FocusPrevious, None),
        ]);

        let bounds = Bounds::centered(None, size(px(560.), px(360.)), cx);

        cx.open_window(
            WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
            |window, cx| {
                cx.new(|cx| {
                    let focus_handle = cx.focus_handle();
                    focus_handle.focus(window, cx);

                    ButtonExample {
                        focus_handle,
                        clicks: 0,
                        disabled: false,
                    }
                })
            },
        )
        .expect("Failed to open the button example window");
        cx.activate(true);
    });
}
