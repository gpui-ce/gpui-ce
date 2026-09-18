#[cfg(any(target_os = "macos", target_os = "windows"))]
fn main() {
    use gpui::{
        AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
        VisualTestAppContext, Window, div, px, rgb, white,
    };

    if std::env::var_os("GPUI_RUN_RENDERING_TESTS").is_none() {
        return;
    }

    struct ParleyRenderingFixture;

    impl Render for ParleyRenderingFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .relative()
                .bg(rgb(0x101418))
                .text_color(white())
                .text_size(px(28.0))
                .p(px(32.0))
                .child(
                    div()
                        .absolute()
                        .left(px(32.0))
                        .top(px(32.0))
                        .child("Parley office cafe\u{301} العربية אבג"),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(32.0))
                        .top(px(80.0))
                        .child("日本語 ไทย 👩🏽‍💻 🇬🇧 1️⃣"),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(32.0))
                        .top(px(128.0))
                        .w(px(260.0))
                        .child("wrapped text one two three four five six seven eight"),
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
        }
    }

    let mut cx = VisualTestAppContext::new(gpui_ce_platform::current_platform(false));
    let window = cx
        .open_offscreen_window_default(|_, cx| cx.new(|_| ParleyRenderingFixture))
        .expect("failed to create offscreen text window");
    let window = window.into();
    cx.run_until_parked();

    let primitive_counts = cx
        .update_window(window, |_, window, _| window.rendered_primitive_counts())
        .expect("failed to inspect rendered text");
    let (_, monochrome, subpixel, polychrome) = primitive_counts;
    assert!(
        monochrome + subpixel > 0,
        "Parley scene contained no text glyph sprites"
    );
    assert!(
        polychrome > 0,
        "Parley scene contained no color emoji sprites"
    );

    #[cfg(target_os = "macos")]
    {
        let image = cx
            .capture_screenshot(window)
            .expect("failed to capture rendered text");
        let background = *image.get_pixel(0, 0);
        let scale_x = image.width() as f32 / 1280.0;
        let scale_y = image.height() as f32 / 800.0;
        let changed_pixels_in = |left: f32, top: f32, right: f32, bottom: f32| {
            let left = (left * scale_x).floor() as u32;
            let top = (top * scale_y).floor() as u32;
            let right = ((right * scale_x).ceil() as u32).min(image.width());
            let bottom = ((bottom * scale_y).ceil() as u32).min(image.height());
            (top..bottom)
                .flat_map(|y| (left..right).map(move |x| (x, y)))
                .filter(|(x, y)| *image.get_pixel(*x, *y) != background)
                .count()
        };

        for (name, bounds, minimum_ink) in [
            ("multiscript text", (24.0, 24.0, 1200.0, 72.0), 100),
            ("emoji text", (24.0, 72.0, 1200.0, 120.0), 50),
            ("wrapped text", (24.0, 120.0, 320.0, 260.0), 100),
            ("bottom-clipped text", (24.0, 780.0, 700.0, 800.0), 20),
        ] {
            let (left, top, right, bottom) = bounds;
            let changed_pixels = changed_pixels_in(left, top, right, bottom);
            assert!(
                changed_pixels >= minimum_ink,
                "{name} painted {changed_pixels} non-background pixels, expected at least {minimum_ink}; primitive counts were {primitive_counts:?}"
            );
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {}
