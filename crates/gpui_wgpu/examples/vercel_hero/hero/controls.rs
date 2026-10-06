//! Navigation and button styles used by the hero.

use gpui::{
    FontWeight, IntoElement, ParentElement, Styled, black, div, px, rgb,
    shader::{color, paint, shape, vec2},
    white,
};

/// The mark in the navigation bar: a small solid triangle.
fn mark() -> impl IntoElement {
    div().size(px(22.0)).bg(paint(|px| {
        // A triangle's center sits a third of the way up; lower it to look centered.
        color(white()).clip(shape::triangle(px.centered() - vec2(0.0, 2.6), 10.5))
    }))
}

fn button(label: &'static str, primary: bool) -> impl IntoElement {
    let button = div()
        .px_3()
        .py_1p5()
        .rounded(px(8.0))
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .child(label);
    if primary {
        button.bg(white()).text_color(black())
    } else {
        button
            .border_1()
            .border_color(rgb(0x2e2e2e))
            .bg(rgb(0x0a0a0a))
            .text_color(white())
    }
}

pub(super) fn pill(label: &'static str, primary: bool) -> impl IntoElement {
    let pill = div()
        .px_5()
        .py_3()
        .rounded_full()
        .text_base()
        .font_weight(FontWeight::MEDIUM)
        .child(label);
    if primary {
        pill.bg(rgb(0xededed)).text_color(black())
    } else {
        pill.border_1()
            .border_color(rgb(0x333333))
            .text_color(white())
    }
}

pub(super) fn navigation() -> impl IntoElement {
    let link = |label: &'static str| div().text_sm().text_color(rgb(0xa1a1a1)).child(label);
    div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(64.0))
        .px_6()
        .child(
            div().flex().items_center().gap_8().child(mark()).child(
                div()
                    .flex()
                    .gap_6()
                    .child(link("Products"))
                    .child(link("Resources"))
                    .child(link("Enterprise"))
                    .child(link("Pricing")),
            ),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(button("Get a Demo", false))
                .child(button("Log In", false))
                .child(button("Sign Up", true)),
        )
}
