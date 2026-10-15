//! A virtualized list of 10,000 contacts, with keyboard selection.
//!
//! `uniform_list` builds rows for the visible range instead of constructing all
//! 10,000 contacts at once. Clicking a row or pressing Up and Down changes the
//! selection, and a `UniformListScrollHandle` scrolls the selected row into view.
//!
//! Run it with `cargo run --example list`.

use gpui::{
    App, Bounds, Context, FocusHandle, FontWeight, KeyBinding, QuitMode, ScrollStrategy,
    UniformListScrollHandle, Window, WindowBounds, WindowOptions, colors::Colors, div, prelude::*,
    px, size, uniform_list,
};

const ROW_COUNT: usize = 10_000;
const CONTEXT: &str = "ListExample";
const FIRST_NAMES: [&str; 8] = [
    "Ada", "Grace", "Linus", "Margaret", "Alan", "Barbara", "Dennis", "Hedy",
];
const LAST_NAMES: [&str; 8] = [
    "Lovelace", "Hopper", "Torvalds", "Hamilton", "Turing", "Liskov", "Ritchie", "Lamarr",
];
const CITIES: [&str; 6] = ["Lisbon", "Oslo", "Kyoto", "Nairobi", "Lima", "Auckland"];

gpui::actions!(list_example, [MoveUp, MoveDown]);

/// Builds the contact at `ix` on demand, so no 10,000-element vector is needed.
fn contact(ix: usize) -> (String, String, &'static str) {
    let first = FIRST_NAMES[ix % FIRST_NAMES.len()];
    let last = LAST_NAMES[(ix / FIRST_NAMES.len()) % LAST_NAMES.len()];
    let city = CITIES[(ix * 7) % CITIES.len()];
    let email = format!(
        "{}.{}{ix}@example.com",
        first.to_lowercase(),
        last.to_lowercase()
    );
    (format!("{first} {last}"), email, city)
}

struct ContactList {
    selected: usize,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl ContactList {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            focus,
        }
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix;
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }
}

impl Render for ContactList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let selected = self.selected;
        let list = cx.weak_entity();

        let rows = uniform_list("contacts", ROW_COUNT, move |range, _window, _cx| {
            range
                .map(|ix| {
                    let list = list.clone();
                    let (name, email, city) = contact(ix);
                    let row = div()
                        .id(ix)
                        .flex()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .cursor_pointer()
                        .on_click(move |_event, _window, cx| {
                            list.update(cx, |list, cx| list.select(ix, cx)).ok();
                        });
                    let (row, muted) = if ix == selected {
                        (
                            row.bg(colors.selected).text_color(colors.selected_text),
                            colors.selected_text,
                        )
                    } else {
                        (
                            row.hover(|style| style.bg(colors.container)),
                            colors.disabled,
                        )
                    };
                    row.child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("{:05}", ix + 1)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(format!("{email} · {city}")),
                            ),
                    )
                })
                .collect()
        })
        .track_scroll(&self.scroll)
        .w_full()
        .flex_1()
        .min_h_0();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|list, _: &MoveUp, _window, cx| {
                list.select(list.selected.saturating_sub(1), cx);
            }))
            .on_action(cx.listener(|list, _: &MoveDown, _window, cx| {
                list.select((list.selected + 1).min(ROW_COUNT - 1), cx);
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .p_4()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{ROW_COUNT} contacts")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child(format!("Selected {}", selected + 1)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mx_4()
                    .mb_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(colors.border)
                    .overflow_hidden()
                    .child(rows),
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
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
    ]);

    let bounds = Bounds::centered(None, size(px(520.), px(640.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |window, cx| cx.new(|cx| ContactList::new(window, cx)),
    )
    .unwrap();
    cx.activate(true);
}
