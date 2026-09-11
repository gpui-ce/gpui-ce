//! Mixing upstream color types with the `palette` extension.
//!
//! gpui-ce treats the upstream color types as canonical: `Hsla` and `Rgba` are
//! re-exported at the `gpui` root. The `palette` crate stays available as an
//! extension (`gpui::palette`), and the `IntoHsla` facade lets values from either
//! family be passed to color-taking style APIs.
//!
//! The two families differ in one important way: upstream `Hsla.h` is a `0..1`
//! fraction, while a palette hue is in degrees.

use gpui::{
    App, Bounds, Context, IntoHsla, Render, Window, WindowBounds, WindowOptions, div, palette,
    point, prelude::*, px, rgb, rgba, size,
};

struct PaletteColorsExample;

/// A labeled swatch. `fill` is anything that can become the canonical [`Hsla`](gpui::Hsla),
/// so both upstream and palette colors can be passed straight in.
fn swatch(label: &'static str, fill: impl IntoHsla) -> impl IntoElement {
    let color = fill.into_hsla();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .items_center()
        .child(
            div()
                .w(px(120.))
                .h(px(72.))
                .rounded_lg()
                .bg(color)
                .border_1()
                .border_color(color.opacity(0.4)),
        )
        .child(div().text_sm().child(label))
}

impl Render for PaletteColorsExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Upstream types: `Hsla` (0..1 hue) and `Rgba`.
        let upstream_hsla = gpui::hsla(0.55, 0.7, 0.5, 1.0);
        let upstream_rgba = rgb(0x2a63d9);

        // palette types: `palette::Hsla` (degrees) and `palette::rgb::Rgba`.
        let palette_hsla = palette::Hsla::new(198.0, 0.7, 0.5, 1.0);
        let palette_rgba = palette::rgb::Rgba::new(0.16, 0.68, 0.52, 1.0);

        div()
            .flex()
            .flex_col()
            .gap_6()
            .p_8()
            .bg(rgba(0x111827ff))
            .text_color(rgb(0xe5e7eb))
            .child(div().text_2xl().child("Upstream + palette colors"))
            .child(
                div()
                    .flex()
                    .gap_6()
                    // Upstream values are accepted directly.
                    .child(swatch("upstream Hsla", upstream_hsla))
                    .child(swatch("upstream Rgba", upstream_rgba))
                    // palette values are accepted through `IntoHsla`.
                    .child(swatch("palette Hsla", palette_hsla))
                    .child(swatch("palette Rgba", palette_rgba))
                    // `bg` takes `Into<Fill>` rather than `IntoHsla`, so a palette
                    // color needs an explicit conversion first.
                    .child(
                        div()
                            .w(px(120.))
                            .h(px(72.))
                            .rounded_lg()
                            .bg(palette_rgba.into_hsla())
                            .border_1()
                            .border_color(palette_hsla),
                    ),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.activate(true);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds {
            origin: point(px(100.), px(100.)),
            size: size(px(760.), px(360.)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| PaletteColorsExample),
        )
        .expect("failed to open window");
    });
}
