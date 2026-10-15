//! Raster and vector images from bytes embedded in the binary.
//!
//! `img` takes an `Arc<Image>` built with `Image::from_bytes`, so it needs no file or asset source.
//! It renders SVG bytes in full color as well. `svg().data(..)` also takes bytes, but paints a
//! single-color mask tinted by the text color, which suits icons.
//!
//! Run it with `cargo run --example images`.

use std::sync::Arc;

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ObjectFit, QuitMode, Window, WindowBounds,
    WindowOptions, colors::Colors, div, img, prelude::*, px, rgb, size, svg,
};

const ACCENT: u32 = 0x2563eb;

struct Images {
    icon: Arc<Image>,
    color: Arc<Image>,
    dragon: Arc<Image>,
}

impl Render for Images {
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
                        .child(section("Object fit", self.object_fit(&colors)))
                        .child(section("Shape", self.shapes(&colors)))
                        .child(section("Vector", self.vectors(&colors))),
                ),
            )
    }
}

impl Images {
    fn new() -> Self {
        Self {
            icon: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("assets/app-icon.png").to_vec(),
            )),
            color: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("assets/color.svg").to_vec(),
            )),
            dragon: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("assets/dragon.svg").to_vec(),
            )),
        }
    }

    fn object_fit(&self, colors: &Colors) -> impl IntoElement {
        let modes = [
            ("Fill", ObjectFit::Fill),
            ("Contain", ObjectFit::Contain),
            ("Cover", ObjectFit::Cover),
            ("ScaleDown", ObjectFit::ScaleDown),
            ("None", ObjectFit::None),
        ];

        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(modes.map(|(label, fit)| {
                tile(
                    label,
                    colors,
                    img(self.icon.clone()).size_full().object_fit(fit),
                )
            }))
    }

    fn shapes(&self, colors: &Colors) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_6()
            .child(captioned(
                "rounded_xl",
                colors,
                img(self.icon.clone()).size(px(96.)).rounded_xl(),
            ))
            .child(captioned(
                "clipped circle",
                colors,
                div().size(px(96.)).rounded_full().overflow_hidden().child(
                    img(self.icon.clone())
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                ),
            ))
            .child(captioned(
                "grayscale",
                colors,
                img(self.icon.clone())
                    .size(px(96.))
                    .rounded_xl()
                    .grayscale(true),
            ))
    }

    fn vectors(&self, colors: &Colors) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_6()
            .child(captioned(
                "color.svg",
                colors,
                img(self.color.clone()).size(px(120.)),
            ))
            .child(captioned(
                "dragon.svg",
                colors,
                img(self.dragon.clone()).size(px(120.)),
            ))
            .child(captioned(
                "svg() tinted",
                colors,
                svg()
                    .data(include_bytes!("assets/dragon.svg"))
                    .size(px(120.))
                    .text_color(rgb(ACCENT)),
            ))
    }
}

fn section(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(body)
}

/// A framed box holding one image, with its mode named underneath.
fn tile(label: &'static str, colors: &Colors, content: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .w(px(160.))
                .h(px(100.))
                .overflow_hidden()
                .rounded_md()
                .border_1()
                .border_color(colors.border)
                .bg(colors.container)
                .child(content),
        )
        .child(div().text_xs().text_color(colors.disabled).child(label))
}

fn captioned(label: &'static str, colors: &Colors, content: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(content)
        .child(div().text_xs().text_color(colors.disabled).child(label))
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(720.), px(760.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Images::new()),
    )
    .unwrap();
    cx.activate(true);
}
