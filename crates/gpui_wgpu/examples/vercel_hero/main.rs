//! A hero in the style of vercel.com: a black prism with light spilling
//! around its edges. Move the pointer to sweep the light around the rim;
//! press to flare it. The prism is one shader paint in a `shader::layer`,
//! under ordinary GPUI text and buttons.
//!
//! ```sh
//! cargo run -p gpui_ce_wgpu --example vercel_hero --features gpui_platform/macos-wgpu
//! ```

mod hero;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;

fn main() {
    env_logger::init();
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions::new()
                .titlebar(Some(TitlebarOptions {
                    title: Some("Hero".into()),
                    ..Default::default()
                }))
                .window_bounds(Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.0), px(800.0)),
                    cx,
                )))),
            |_, cx| cx.new(|_| hero::Hero),
        )
        .unwrap();
        cx.activate(true);
    });
}
