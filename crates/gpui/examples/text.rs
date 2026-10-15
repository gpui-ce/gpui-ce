//! Typography: the size scale, weights, decorations, alignment, wrapping and truncation,
//! monospace text, and one paragraph with styled ranges.
//!
//! Text style cascades from a parent to its children, so a line height or color set on a
//! container applies to everything inside it. `StyledText` styles byte ranges within one string,
//! which lets a single paragraph mix weights, colors, and decorations.
//!
//! Run it with `cargo run --example text`.

use std::ops::Range;

use gpui::{
    App, Bounds, Context, Div, FontStyle, FontWeight, HighlightStyle, QuitMode, StrikethroughStyle,
    StyledText, TextTransform, UnderlineStyle, Window, WindowBounds, WindowOptions, colors::Colors,
    div, prelude::*, px, relative, rgb, rgb_to_hsla, size,
};

const ACCENT: u32 = 0x2563eb;

struct Typography;

impl Render for Typography {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id("page")
            .size_full()
            .overflow_y_scroll()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div().flex().justify_center().p_4().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_10()
                        .w_full()
                        .max_w(px(640.))
                        .child(header(&colors))
                        .child(paragraph())
                        .child(section("Size", sizes(&colors)))
                        .child(section("Weight", weights(&colors)))
                        .child(section("Style", styles(&colors)))
                        .child(section("Alignment", alignment(&colors)))
                        .child(section("Wrapping and truncation", overflow(&colors)))
                        .child(section("Monospace", monospace(&colors))),
                ),
            )
    }
}

fn header(colors: &Colors) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(ACCENT))
                .text_transform(TextTransform::Uppercase)
                .letter_spacing(px(1.))
                .child("Typography"),
        )
        .child(
            div()
                .text_3xl()
                .font_weight(FontWeight::BOLD)
                .child("Setting type in GPUI"),
        )
        .child(
            div()
                .text_lg()
                .text_color(colors.disabled)
                .child("A few style methods cover most of what a page of text needs."),
        )
}

fn paragraph() -> impl IntoElement {
    let text = "Styles cascade from parent to child, so one line height on a container covers \
        every paragraph in it. Inside a single paragraph, StyledText changes only the ranges \
        you name: a semibold term, an italic aside, a colored phrase, an underlined link and a \
        struck-through correction.";
    let accent = rgb_to_hsla(rgb(ACCENT));

    let highlights = [
        (
            range_of(text, "a semibold term"),
            HighlightStyle {
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            },
        ),
        (
            range_of(text, "an italic aside"),
            HighlightStyle {
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
        ),
        (
            range_of(text, "a colored phrase"),
            HighlightStyle::color(accent),
        ),
        (
            range_of(text, "an underlined link"),
            HighlightStyle {
                underline: Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(accent),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
        (
            range_of(text, "struck-through correction"),
            HighlightStyle {
                strikethrough: Some(StrikethroughStyle {
                    thickness: px(1.),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ),
    ];

    div()
        .text_base()
        .line_height(relative(1.6))
        .child(StyledText::new(text).with_highlights(highlights))
}

fn range_of(text: &str, phrase: &str) -> Range<usize> {
    let start = text.find(phrase).expect("the phrase is part of the text");
    start..start + phrase.len()
}

fn section(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(body)
}

/// Each sample next to a monospace label that names the method that styled it.
fn specimens(
    colors: &Colors,
    rows: impl IntoIterator<Item = (&'static str, Div)>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(rows.into_iter().map(|(label, sample)| {
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap_x_4()
                .gap_y_1()
                .child(
                    div()
                        .w(px(112.))
                        .flex_none()
                        .font_family("Lilex")
                        .text_xs()
                        .text_color(colors.disabled)
                        .child(label),
                )
                .child(sample)
        }))
}

fn sizes(colors: &Colors) -> impl IntoElement {
    let sample = "Sphinx of black quartz";
    specimens(
        colors,
        [
            ("text_xs", div().text_xs().child(sample)),
            ("text_sm", div().text_sm().child(sample)),
            ("text_base", div().text_base().child(sample)),
            ("text_lg", div().text_lg().child(sample)),
            ("text_xl", div().text_xl().child(sample)),
            ("text_2xl", div().text_2xl().child(sample)),
            ("text_3xl", div().text_3xl().child(sample)),
        ],
    )
}

fn weights(colors: &Colors) -> impl IntoElement {
    let weights = [
        ("LIGHT", FontWeight::LIGHT),
        ("NORMAL", FontWeight::NORMAL),
        ("MEDIUM", FontWeight::MEDIUM),
        ("SEMIBOLD", FontWeight::SEMIBOLD),
        ("BOLD", FontWeight::BOLD),
    ];
    specimens(
        colors,
        weights.map(|(label, weight)| {
            (
                label,
                div()
                    .text_lg()
                    .font_weight(weight)
                    .child("Sphinx of black quartz"),
            )
        }),
    )
}

fn styles(colors: &Colors) -> impl IntoElement {
    specimens(
        colors,
        [
            ("italic", div().italic().child("For asides and titles")),
            ("underline", div().underline().child("For links")),
            ("line_through", div().line_through().child("For removals")),
            (
                "wavy",
                div()
                    .underline()
                    .text_decoration_wavy()
                    .text_decoration_color(rgb(ACCENT))
                    .child("For spelling errors"),
            ),
            (
                "text_color",
                div().text_color(rgb(ACCENT)).child("For emphasis"),
            ),
        ],
    )
}

fn alignment(colors: &Colors) -> impl IntoElement {
    let box_ = || div().w_full().p_2().rounded_md().bg(colors.container);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(box_().text_left().child("Left aligned"))
        .child(box_().text_center().child("Centered"))
        .child(box_().text_right().child("Right aligned"))
}

fn overflow(colors: &Colors) -> impl IntoElement {
    let sentence = "GPUI lays out this sentence in a fixed column, so it wraps onto as many lines as it needs.";
    let column = || div().w(px(240.));
    specimens(
        colors,
        [
            ("wrap", column().child(sentence)),
            ("truncate", column().truncate().child(sentence)),
            (
                "line_clamp(2)",
                column().line_clamp(2).text_ellipsis().child(sentence),
            ),
            (
                "ellipsis_middle",
                column()
                    .whitespace_nowrap()
                    .text_ellipsis_middle()
                    .child("quarterly-report-final-reviewed.pdf"),
            ),
        ],
    )
}

fn monospace(colors: &Colors) -> impl IntoElement {
    div()
        .font_family("Lilex")
        .text_sm()
        .p_3()
        .rounded_md()
        .bg(colors.container)
        .flex()
        .flex_col()
        .gap_1()
        .child(r#"let style = div().font_family("Lilex").text_sm();"#)
        .child("let count = cx.new(|_cx| Counter { count: 0 });")
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(720.), px(860.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Typography),
    )
    .unwrap();
    cx.activate(true);
}
