//! Hero layout with a shader layer behind its content.

mod controls;
pub mod prism;

use controls::{navigation, pill};
use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, Window, black, div, px, rgb, shader, white,
};
use prism::prism;

pub struct Hero;

impl Render for Hero {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let headline = div()
            .flex()
            .flex_col()
            .gap_8()
            .w(px(420.0))
            .child(
                div()
                    .text_size(px(64.0))
                    .line_height(px(66.0))
                    .text_color(white())
                    .child("Agentic Infrastructure"),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(pill("Deploy now", true))
                    .child(pill("Talk to sales", false)),
            );
        let tagline = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(240.0))
            .text_color(rgb(0xededed))
            .child("For coding agents")
            .child("To ship apps and agents")
            .child("Automated by agents");

        div()
            .size_full()
            .relative()
            .bg(black())
            .text_color(white())
            .child(
                // The prism follows the pointer anywhere in the window, even
                // over the content painted on top of it.
                shader::layer("prism", prism)
                    .animate()
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(navigation())
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .gap_6()
                            .py_2()
                            .text_sm()
                            .child(
                                div()
                                    .text_color(rgb(0xa1a1a1))
                                    .child("Ship 26 is coming to SF"),
                            )
                            .child("Get your ticket  ›"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(48.0))
                            .pb(px(64.0))
                            .child(headline)
                            .child(tagline),
                    ),
            )
    }
}
