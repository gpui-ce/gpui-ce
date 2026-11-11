#[cfg(all(not(target_family = "wasm"), not(target_os = "macos")))]
use gpui::HeadlessAppContext;
#[cfg(target_os = "macos")]
use gpui::VisualTestAppContext;
#[cfg(target_family = "wasm")]
use gpui::{App, ApplicationHandle, Bounds, WindowBounds, WindowOptions, size};
use gpui::{
    AppContext as _, Context, Div, FontWeight, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, px, rgb, white,
};
#[cfg(not(target_family = "wasm"))]
use image::{Rgba, RgbaImage};
use std::borrow::Cow;
#[cfg(target_family = "wasm")]
use std::cell::RefCell;

use gpui_fonts::{IBM_PLEX, NOTO_ARABIC, NOTO_COLOR_EMOJI, NOTO_HEBREW, SOURCE_SERIF};

#[cfg(target_os = "macos")]
const EMOJI_FAMILY: &str = "Apple Color Emoji";
#[cfg(target_os = "windows")]
const EMOJI_FAMILY: &str = "Segoe UI Emoji";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const EMOJI_FAMILY: &str = "Noto Color Emoji";

#[cfg(target_family = "wasm")]
thread_local! {
    static APPLICATION: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

const BIDI_SAMPLE: &str =
    "שלום עולם\nمرحبا بالعالم\nabc אבג def\nx (مرحبا) y\nEnglish ثم عربي ثم English";

fn positioned_text(left: f32, top: f32) -> Div {
    div().absolute().left(px(left)).top(px(top))
}

fn fixture_fonts() -> Vec<Cow<'static, [u8]>> {
    vec![
        Cow::Borrowed(IBM_PLEX.data),
        Cow::Borrowed(NOTO_ARABIC.data),
        Cow::Borrowed(NOTO_HEBREW.data),
        Cow::Borrowed(NOTO_COLOR_EMOJI.data),
        Cow::Borrowed(SOURCE_SERIF.data),
    ]
}

#[cfg(not(target_family = "wasm"))]
fn pixels_in(
    image: &RgbaImage,
    scale_factor: f32,
    bounds: (f32, f32, f32, f32),
) -> impl Iterator<Item = &Rgba<u8>> {
    let (left, top, right, bottom) = bounds;
    let left = (left * scale_factor).floor() as u32;
    let right = (right * scale_factor).ceil() as u32;
    let top = (top * scale_factor).floor() as u32;
    let bottom = (bottom * scale_factor).ceil() as u32;

    (top..bottom.min(image.height()))
        .flat_map(move |y| (left..right.min(image.width())).map(move |x| image.get_pixel(x, y)))
}

#[cfg(not(target_family = "wasm"))]
fn count_colored_pixels_in(
    image: &RgbaImage,
    scale_factor: f32,
    bounds: (f32, f32, f32, f32),
) -> usize {
    pixels_in(image, scale_factor, bounds)
        .filter(|pixel| {
            let maximum = pixel[0].max(pixel[1]).max(pixel[2]);
            let minimum = pixel[0].min(pixel[1]).min(pixel[2]);

            maximum - minimum > 32
        })
        .count()
}

fn main() {
    #[cfg(not(target_family = "wasm"))]
    if std::env::var_os("GPUI_RUN_RENDERING_TESTS").is_none() {
        return;
    }

    struct ParleyRenderingFixture;

    impl Render for ParleyRenderingFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let fixture = div()
                .size_full()
                .relative()
                .bg(rgb(0x101418))
                .text_color(white())
                .text_size(px(28.0))
                .font_family("IBM Plex Sans")
                .p(px(32.0))
                .child(positioned_text(-2.5, 31.25).child("Parley office cafe\u{301} العربية אבג"))
                .child(
                    positioned_text(32.0, 80.0)
                        .font_family(EMOJI_FAMILY)
                        .child("😀 🎉 🚀 💡 🔥 ✨"),
                )
                .child(positioned_text(400.0, 128.0).child("日本語 ไทย"))
                .child(
                    positioned_text(32.0, 128.0)
                        .w(px(260.0))
                        .child("wrapped text one two three four five six seven eight"),
                )
                .child(
                    positioned_text(32.0, 300.0)
                        .w(px(600.0))
                        .line_height(px(40.0))
                        .child(BIDI_SAMPLE),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(32.0))
                        .bottom(px(-20.0))
                        .h(px(40.0))
                        .line_height(px(40.0))
                        .child("MMMM clipped at the bottom edge"),
                )
                .child(
                    positioned_text(700.0, 32.0)
                        .font_family("Source Serif 4")
                        .font_weight(FontWeight(340.0))
                        .text_size(px(12.0))
                        .child("Hamburgefontsiv"),
                )
                .child(
                    positioned_text(700.0, 80.0)
                        .font_family("Source Serif 4")
                        .font_weight(FontWeight(340.0))
                        .text_size(px(48.0))
                        .child("Hamburgefontsiv"),
                )
                .child(
                    // Segoe UI Emoji has no color country flags. The bundled Noto row covers flags.
                    positioned_text(700.0, 220.0)
                        .font_family(EMOJI_FAMILY)
                        .child("👩🏽‍💻 👍🏽 1️⃣"),
                );

            fixture.children(
                ["👩‍💻", "👩🏽", "🇬🇧", "1️⃣"]
                    .into_iter()
                    .enumerate()
                    .map(|(idx, text)| {
                        positioned_text(700.0 + idx as f32 * 80.0, 300.0)
                            .font_family("Noto Color Emoji")
                            .child(text)
                    }),
            )
        }
    }

    #[cfg(target_family = "wasm")]
    {
        gpui_ce_platform::web_init();

        let application = gpui_ce_platform::application().run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(fixture_fonts())
                .expect("failed to load rendering fixture fonts");

            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| ParleyRenderingFixture),
            )
            .expect("failed to open browser text window");
            cx.activate(true);
        });

        APPLICATION.set(Some(application));

        return;
    }

    #[cfg(not(target_family = "wasm"))]
    {
        #[cfg(target_os = "macos")]
        let mut cx = VisualTestAppContext::new(gpui_ce_platform::current_platform(false));

        #[cfg(not(target_os = "macos"))]
        let mut cx = {
            let platform = gpui_ce_platform::current_platform(true);
            let text_system = platform.text_system();

            HeadlessAppContext::with_platform(text_system, std::sync::Arc::new(()), || {
                Some(Box::new(
                    gpui_wgpu::WgpuHeadlessRenderer::new()
                        .expect("failed to initialize the headless WGPU renderer"),
                ) as Box<dyn gpui::PlatformHeadlessRenderer>)
            })
        };

        cx.update(|cx| cx.text_system().add_fonts(fixture_fonts()))
            .expect("failed to load rendering fixture fonts");

        #[cfg(target_os = "macos")]
        let window = cx
            .open_offscreen_window_default(|_, cx| cx.new(|_| ParleyRenderingFixture))
            .expect("failed to create offscreen text window");

        #[cfg(not(target_os = "macos"))]
        let window = cx
            .open_window(gpui::size(px(1280.0), px(800.0)), |_, cx| {
                cx.new(|_| ParleyRenderingFixture)
            })
            .expect("failed to create the headless text window");

        let window = window.into();
        cx.run_until_parked();

        let (primitive_counts, scale_factor) = cx
            .update_window(window, |_, window, _| {
                (window.rendered_primitive_counts(), window.scale_factor())
            })
            .expect("failed to inspect rendered text");

        let (_, monochrome, subpixel, polychrome) = primitive_counts;
        assert!(
            monochrome + subpixel > 0,
            "Parley scene contained no text glyph sprites"
        );
        let minimum_color_glyphs = 13;

        assert!(
            polychrome >= minimum_color_glyphs,
            "Parley scene contained {polychrome} color emoji sprites, expected at least {minimum_color_glyphs}"
        );

        let image = cx
            .capture_screenshot(window)
            .expect("failed to capture rendered text");

        if let Some(output) = std::env::var_os("GPUI_RENDERING_TEST_OUTPUT") {
            image.save(output).expect("failed to save rendered text");
        }

        #[cfg(target_os = "macos")]
        {
            let background = *image.get_pixel(0, 0);

            // AppKit can shrink the requested window to fit the CI display. Glyph positions
            // remain in logical pixels, so only the display scale converts screenshot bounds.
            let viewport_height = image.height() as f32 / scale_factor;

            for (name, bounds, minimum_ink) in [
                ("multiscript text", (24.0, 24.0, 1200.0, 72.0), 100),
                ("emoji text", (24.0, 72.0, 600.0, 120.0), 50),
                ("wrapped text", (24.0, 120.0, 320.0, 260.0), 100),
                ("bidi paragraphs", (24.0, 292.0, 650.0, 510.0), 100),
                (
                    "bottom-clipped text",
                    (24.0, viewport_height - 20.0, 700.0, viewport_height),
                    20,
                ),
                ("small optical text", (690.0, 24.0, 1270.0, 64.0), 30),
                ("large optical text", (690.0, 72.0, 1270.0, 150.0), 100),
                ("Apple emoji sequences", (690.0, 200.0, 1000.0, 280.0), 50),
            ] {
                let changed_pixels = pixels_in(&image, scale_factor, bounds)
                    .filter(|pixel| **pixel != background)
                    .count();

                assert!(
                    changed_pixels >= minimum_ink,
                    "{name} painted {changed_pixels} non-background pixels, expected at least {minimum_ink}; primitive counts were {primitive_counts:?}"
                );
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let background = *image.get_pixel(0, 0);
            let changed_emoji_pixels = pixels_in(&image, scale_factor, (24.0, 72.0, 600.0, 120.0))
                .filter(|pixel| **pixel != background)
                .count();

            assert!(
                changed_emoji_pixels >= 50,
                "emoji region painted {changed_emoji_pixels} non-background pixels; primitive counts were {primitive_counts:?}"
            );
        }

        for (idx, name) in ["joined emoji", "skin-tone modifier", "flag", "keycap"]
            .into_iter()
            .enumerate()
        {
            let colored_pixels = count_colored_pixels_in(
                &image,
                scale_factor,
                (
                    700.0 + idx as f32 * 80.0,
                    300.0,
                    772.0 + idx as f32 * 80.0,
                    350.0,
                ),
            );

            assert!(
                colored_pixels >= 20,
                "CBDT {name} painted only {colored_pixels} colored pixels"
            );
        }
    }
}
