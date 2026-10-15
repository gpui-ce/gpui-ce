//! Keyboard input: actions, key bindings, key contexts and focus.
//!
//! Actions are types declared with `actions!`. `KeyBinding::new` maps a
//! keystroke to an action inside a key context, and an ancestor element
//! handles the action with `on_action`. Tab and Shift-Tab move focus between
//! `track_focus` handles, and `focus_visible` draws a ring only when focus
//! arrived from the keyboard. The shortcut next to each label comes from the
//! keymap, so it always matches the binding.
//!
//! Run it with `cargo run --example keyboard`.

use gpui::{
    Action, App, Bounds, Context, FocusHandle, KeyBinding, QuitMode, Window, WindowBounds,
    WindowOptions, actions, colors::Colors, div, prelude::*, px, size,
};

const CONTEXT: &str = "KeyboardExample";

actions!(
    keyboard_example,
    [FocusNext, FocusPrevious, Save, Duplicate, ClearLog]
);

struct Keyboard {
    focus: [FocusHandle; 3],
    log: Vec<&'static str>,
}

impl Keyboard {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = [0, 1, 2].map(|index| cx.focus_handle().tab_stop(true).tab_index(index));
        window.focus(&focus[0], cx);
        Self {
            focus,
            log: Vec::new(),
        }
    }

    fn record(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.log.push(name);
        if self.log.len() > 5 {
            self.log.remove(0);
        }
        cx.notify();
    }
}

fn action_row<A: Action + Clone>(
    index: usize,
    label: &'static str,
    action: A,
    focus: &FocusHandle,
    colors: &Colors,
    cx: &App,
) -> impl IntoElement {
    let shortcut = cx
        .key_bindings()
        .borrow()
        .bindings_for_action(&action)
        .last()
        .map(|binding| {
            binding
                .keystrokes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    div()
        .id(("row", index))
        .track_focus(focus)
        .focus_visible(|style| style.border_color(colors.selected))
        .flex()
        .justify_between()
        .items_center()
        .px_4()
        .py_3()
        .rounded_lg()
        .border_2()
        .border_color(colors.container)
        .bg(colors.container)
        .cursor_pointer()
        .hover(|style| style.bg(colors.separator))
        .on_click(move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx))
        .child(label)
        .child(
            div()
                .font_family("Lilex")
                .text_sm()
                .text_color(colors.disabled)
                .child(shortcut),
        )
}

impl Render for Keyboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &FocusNext, window, cx| {
                window.focus_next(cx);
                this.record("FocusNext", cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPrevious, window, cx| {
                window.focus_prev(cx);
                this.record("FocusPrevious", cx);
            }))
            .on_action(cx.listener(|this, _: &Save, _, cx| this.record("Save", cx)))
            .on_action(cx.listener(|this, _: &Duplicate, _, cx| this.record("Duplicate", cx)))
            .on_action(cx.listener(|this, _: &ClearLog, _, cx| {
                this.log.clear();
                cx.notify();
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Keyboard"),
                    )
                    .child(div().text_sm().text_color(colors.disabled).child(
                        "Tab and Shift-Tab move focus. Enter or Space runs the focused row.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(action_row(0, "Save", Save, &self.focus[0], &colors, cx))
                    .child(action_row(
                        1,
                        "Duplicate",
                        Duplicate,
                        &self.focus[1],
                        &colors,
                        cx,
                    ))
                    .child(action_row(
                        2,
                        "Clear log",
                        ClearLog,
                        &self.focus[2],
                        &colors,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child("RECENT ACTIONS"),
                    )
                    .child(
                        div()
                            .font_family("Lilex")
                            .text_sm()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(self.log.iter().map(|name| div().child(*name)))
                            .when(self.log.is_empty(), |list| {
                                list.text_color(colors.disabled).child("No actions yet")
                            }),
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
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(CONTEXT)),
        KeyBinding::new("secondary-s", Save, Some(CONTEXT)),
        KeyBinding::new("secondary-d", Duplicate, Some(CONTEXT)),
        KeyBinding::new("secondary-backspace", ClearLog, Some(CONTEXT)),
    ]);

    let bounds = Bounds::centered(None, size(px(520.), px(420.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |window, cx| cx.new(|cx| Keyboard::new(window, cx)),
    )
    .unwrap();
    cx.activate(true);
}
