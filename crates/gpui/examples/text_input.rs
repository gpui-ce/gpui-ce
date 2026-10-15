//! Editable text: a single-line field and a multi-line area in a small form.
//!
//! Each field is backed by an `EditableTextState` entity that the form owns. The
//! form subscribes to its `TextChanged` event so the character count stays live,
//! and reads the state when it renders. The key bindings for editing come from
//! `gpui_ce_elements`, so `run` must register them.
//!
//! Run it with `cargo run --example text_input`.

use gpui::{
    App, Bounds, Context, FontWeight, QuitMode, Render, Subscription, Window, WindowBounds,
    WindowOptions, colors::Colors, div, prelude::*, px, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage, TextChanged,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_area, text_input,
};

struct Form {
    name: gpui::Entity<EditableTextState>,
    message: gpui::Entity<EditableTextState>,
    greeting: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Form {
    fn new(cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        let message = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));

        let subscriptions = vec![
            cx.subscribe(&name, |_, _, _: &TextChanged, cx| cx.notify()),
            cx.subscribe(&message, |_, _, _: &TextChanged, cx| cx.notify()),
        ];

        Self {
            name,
            message,
            greeting: None,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let characters = self.message.read(cx).as_str().chars().count();
        let greeting = self
            .greeting
            .clone()
            .unwrap_or_else(|| "Press Greet to say hello.".to_string());

        div()
            .id("form")
            .size_full()
            .flex()
            .justify_center()
            .overflow_y_scroll()
            .bg(colors.background)
            .text_color(colors.text)
            .p_6()
            .child(
                div()
                    .w_full()
                    .max_w(px(420.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Contact form"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("Name"))
                            .child(
                                text_input("name")
                                    .state(self.name.downgrade())
                                    .placeholder("Ada Lovelace")
                                    .placeholder_color(colors.disabled)
                                    .caret_color(gpui::rgb_to_hsla(colors.text))
                                    .caret_blink_interval_500ms()
                                    .w_full()
                                    .min_h_auto()
                                    .max_h_auto()
                                    .whitespace_nowrap()
                                    .px_3()
                                    .py_2()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.container),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("Message"))
                            .child(
                                text_area("message")
                                    .state(self.message.downgrade())
                                    .placeholder("Write a short message")
                                    .placeholder_color(colors.disabled)
                                    .caret_color(gpui::rgb_to_hsla(colors.text))
                                    .caret_blink_interval_500ms()
                                    .w_full()
                                    .h(px(120.))
                                    .whitespace_normal()
                                    .overflow_y_scroll()
                                    .px_3()
                                    .py_2()
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(colors.border)
                                    .bg(colors.container),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(colors.disabled)
                                    .child(format!("{characters} characters")),
                            ),
                    )
                    .child(
                        div()
                            .id("greet")
                            .self_start()
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(colors.selected)
                            .text_color(colors.selected_text)
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .active(|style| style.opacity(0.8))
                            .child("Greet")
                            .on_click(cx.listener(|form, _event, _window, cx| {
                                let name = form.name.read(cx).as_str().trim().to_string();
                                let name = if name.is_empty() {
                                    "stranger".to_string()
                                } else {
                                    name
                                };
                                let characters = form.message.read(cx).as_str().chars().count();
                                form.greeting = Some(format!(
                                    "Hello, {name}! Your message is {characters} characters long."
                                ));
                                cx.notify();
                            })),
                    )
                    .child(div().text_lg().child(greeting)),
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
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));

    let bounds = Bounds::centered(None, size(px(480.), px(560.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(Form::new),
    )
    .unwrap();
    cx.activate(true);
}
