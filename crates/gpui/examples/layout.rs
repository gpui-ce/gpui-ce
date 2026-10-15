//! An application shell built with flexbox and grid: header, sidebar, card grid, and status bar.
//!
//! The sidebar has a fixed width and never shrinks. The content area takes the remaining space with
//! `flex_1` and `min_w_0`. As the window narrows, the card grid loses columns and the sidebar hides.
//! Pay particular attention to `grid_cols`, `overflow_y_scroll`, and `relative`.
//!
//! Run it with `cargo run --example layout`.

use gpui::{
    App, Bounds, Context, FontWeight, QuitMode, Window, WindowBounds, WindowOptions,
    colors::Colors, div, prelude::*, px, relative, rems, rgb, size,
};

const NAV_ITEMS: [&str; 4] = ["Overview", "Projects", "Calendar", "Settings"];

/// A project card: its title, a status line and an accent color.
const CARDS: [(&str, &str, u32); 9] = [
    (
        "Website redesign",
        "Homepage and pricing pages are in review.",
        0x2457ca,
    ),
    (
        "Mobile release",
        "Build 1.4 is waiting for store approval.",
        0x3a9d5d,
    ),
    (
        "Onboarding flow",
        "Five steps, down from eight after testing.",
        0xd08a2e,
    ),
    (
        "API v2",
        "Auth endpoints are done. Rate limits next.",
        0x8a4fd1,
    ),
    ("Docs portal", "Search indexing is half finished.", 0x2a9bb0),
    (
        "Billing sync",
        "Blocked on the payment provider sandbox.",
        0xc7463f,
    ),
    (
        "Design system",
        "Forty components migrated to tokens.",
        0x2457ca,
    ),
    (
        "Analytics",
        "Weekly report now goes out on Mondays.",
        0x3a9d5d,
    ),
    ("Hiring", "Two engineering roles open this month.", 0xd08a2e),
];

struct AppShell {
    selected: usize,
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let width = window.viewport_size().width;
        let narrow = width < px(640.);
        let columns: u16 = if narrow {
            1
        } else if width < px(960.) {
            2
        } else {
            3
        };

        let header = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .h(rems(3.5))
            .px(rems(1.))
            .border_b_1()
            .border_color(colors.separator)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(rems(1.5)).rounded_md().bg(rgb(0x2457ca)))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Atlas")),
            );

        let sidebar = div()
            .w(px(200.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .border_r_1()
            .border_color(colors.separator)
            .children(NAV_ITEMS.iter().enumerate().map(|(ix, label)| {
                let selected = ix == self.selected;
                div()
                    .id(("nav", ix))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |item| {
                        item.bg(colors.selected).text_color(colors.selected_text)
                    })
                    .when(!selected, |item| {
                        item.hover(|style| style.bg(colors.container))
                    })
                    .child(*label)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected = ix;
                        cx.notify();
                    }))
            }))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(colors.container)
                    .text_xs()
                    .text_color(colors.disabled)
                    .child("Storage, 70% used")
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded_full()
                            .bg(colors.background)
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(0.7))
                                    .rounded_full()
                                    .bg(rgb(0x2457ca)),
                            ),
                    ),
            );

        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .px(rems(1.))
            .py(rems(0.75))
            .child(
                div()
                    .min_w_0()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Projects"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(colors.disabled)
                            .child("9 active, 2 archived"),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .bg(colors.selected)
                    .text_color(colors.selected_text)
                    .child("New project"),
            );

        let cards = div()
            .grid()
            .grid_cols(columns)
            .gap_4()
            .children(CARDS.map(|card| card_view(card, &colors)));

        let content = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .id("cards")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(rems(1.))
                    .child(cards),
            );

        let status_bar = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .h(rems(2.))
            .px(rems(1.))
            .gap_3()
            .text_xs()
            .text_color(colors.disabled)
            .bg(colors.container)
            .border_t_1()
            .border_color(colors.separator)
            .child("Synced 2 minutes ago")
            .child(format!("{} items", CARDS.len()));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .text_color(colors.text)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .when(!narrow, |body| body.child(sidebar))
                    .child(content),
            )
            .child(status_bar)
    }
}

fn card_view(
    (title, detail, accent): (&'static str, &'static str, u32),
    colors: &Colors,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(colors.separator)
        .bg(colors.container)
        .child(div().h(px(6.)).w(rems(3.)).rounded_full().bg(rgb(accent)))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().text_sm().text_color(colors.disabled).child(detail))
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(960.), px(640.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| AppShell { selected: 0 }),
    )
    .unwrap();
    cx.activate(true);
}
