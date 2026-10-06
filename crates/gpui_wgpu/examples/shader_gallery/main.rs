//! Shader paints: expressions, signed distances, noise, remapping, a `.wgsl`
//! file, a `#[wgsl]` module, border paints, backdrop glass, and a custom
//! pipeline, all animated.
//!
//! Paints need a renderer that runs shaders: WGPU on Linux and the web, and
//! on macOS with the `macos-wgpu` feature. Elsewhere they show fallbacks.
//!
//! ```sh
//! cargo run -p gpui_ce_wgpu --example shader_gallery --features gpui_platform/macos-wgpu
//! ```

mod gallery;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_platform::application;

fn main() {
    env_logger::init();
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions::new()
                .titlebar(Some(TitlebarOptions {
                    title: Some("Shader paints".into()),
                    ..Default::default()
                }))
                .window_bounds(Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(912.0), px(824.0)),
                    cx,
                )))),
            |_, cx| cx.new(|_| gallery::Gallery::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}
