//! Native Window Background Showcase
//!
//! Demonstrates WGPUI native window background modes by opening:
//! - one opaque backdrop window with loud colors
//! - one transparent floating window
//! - one blurred floating window
//!
//! Run with:
//!   cargo run --example window_background_showcase

#[path = "../prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{
    point, px, rgb, rgba, size, transparent_black, App, Application, Bounds, Context, FontWeight,
    Render, Rgba, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div,
    prelude::*,
};

struct BackdropWindow;

struct MaterialWindow {
    title: &'static str,
    subtitle: &'static str,
    accent: u32,
    mode_label: &'static str,
}

fn palette_band(label: &'static str, colors: &[u32]) -> impl IntoElement {
    let mut band = div()
        .flex()
        .gap_2()
        .child(
            div()
                .w_32()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xf3f4f6))
                .child(label),
        );

    for (index, color) in colors.iter().copied().enumerate() {
        band = band.child(
            div()
                .id((label, index))
                .h_16()
                .flex_1()
                .rounded_md()
                .bg(rgb(color))
                .border_1()
                .border_color(rgba(0xffffff1f)),
        );
    }

    band
}

fn frosted_panel(
    id: &'static str,
    title: &'static str,
    body: &'static str,
    accent: u32,
    blur_radius: f32,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_xl()
        .bg(Rgba { r: (((accent >> 16) & 0xff) as f32) / 255.0, g: (((accent >> 8) & 0xff) as f32) / 255.0, b: ((accent & 0xff) as f32) / 255.0, a: 0.18 })
        .backdrop_blur(blur_radius)
        .border_1()
        .border_color(rgba(0xffffff33))
        .shadow_lg()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(0xf8fafc))
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgba(0xe2e8f0eb))
                .child(body),
        )
}

impl Render for BackdropWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0b1020))
            .text_color(rgb(0xf8fafc))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child("Backdrop Window"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xcbd5e1))
                            .child("Keep this window visible behind the floating windows to verify transparency and blur."),
                    ),
            )
            .child(palette_band(
                "Neon",
                &[0xff5d73, 0xffc857, 0x56cfe1, 0x80ed99, 0x6d77ff],
            ))
            .child(palette_band(
                "Candy",
                &[0xff8fab, 0xf9bec7, 0xfdc5f5, 0xa0c4ff, 0xb9fbc0],
            ))
            .child(palette_band(
                "Heat",
                &[0x8b1e3f, 0xdc2f02, 0xf48c06, 0xffba08, 0x9d0208],
            ))
            .child(
                div()
                    .flex_1()
                    .rounded_xl()
                    .bg(rgb(0x111827))
                    .border_1()
                    .border_color(rgba(0xffffff14))
                    .p_5()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x93c5fd))
                            .child("Tip: move the smaller windows around. The left one should show plain transparency; the right one should blur this colorful content."),
                    ),
            )
    }
}

impl Render for MaterialWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_5()
            .bg(transparent_black())
            .text_color(rgb(0xf8fafc))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(Rgba { r: (((self.accent >> 16) & 0xff) as f32) / 255.0, g: (((self.accent >> 8) & 0xff) as f32) / 255.0, b: ((self.accent & 0xff) as f32) / 255.0, a: 0.95 })
                                    .child(self.mode_label),
                            )
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .child(self.title),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgba(0xe2e8f0eb))
                                    .child(self.subtitle),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(frosted_panel(
                                "panel-primary",
                                "Semi-transparent surface",
                                "This panel is translucent. You should still perceive the backdrop window behind it.",
                                self.accent,
                                0.0,
                            ))
                            .child(frosted_panel(
                                "panel-blur",
                                "Element backdrop blur",
                                "This inner panel also uses element-level backdrop blur so you can compare window blur and component blur together.",
                                self.accent,
                                16.0,
                            ))
                            .child(
                                div()
                                    .rounded_xl()
                                    .bg(rgba(0x0206174d))
                                    .border_1()
                                    .border_color(rgba(0xffffff29))
                                    .p_3()
                                    .text_xs()
                                    .text_color(rgba(0xf8fafce0))
                                    .child("Expected result: the desktop-facing window background is transparent, while these panels provide only partial tinting.")
                            ),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        init_example(cx, "Window Background Showcase");

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(80.), px(80.)),
                    size(px(980.), px(720.)),
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Backdrop Window".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            },
            |_, cx| cx.new(|_| BackdropWindow),
        )
        .expect("open backdrop window");

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(150.), px(140.)),
                    size(px(440.), px(460.)),
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Transparent Window".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                kind: WindowKind::Floating,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| MaterialWindow {
                    title: "Transparent Window",
                    subtitle: "Shows plain alpha transparency without compositor blur.",
                    accent: 0x67e8f9,
                    mode_label: "window_background: Transparent",
                })
            },
        )
        .expect("open transparent window");

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(620.), px(180.)),
                    size(px(440.), px(460.)),
                ))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Blurred Window".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                kind: WindowKind::Floating,
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| MaterialWindow {
                    title: "Blurred Window",
                    subtitle: "Uses compositor blur behind the window while keeping the content mostly transparent.",
                    accent: 0xf9a8d4,
                    mode_label: "window_background: Blurred",
                })
            },
        )
        .expect("open blurred window");
    });
}
