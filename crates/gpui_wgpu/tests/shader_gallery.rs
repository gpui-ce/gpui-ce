//! The shader gallery example, rendered headlessly so it keeps compiling and
//! drawing. Set `GPUI_SHADER_GALLERY_PNG` to a path to save the frame.
#![cfg(feature = "test-support")]

mod support;

#[path = "../examples/shader_gallery/gallery/mod.rs"]
#[allow(dead_code)]
mod gallery;

use std::sync::Arc;

use gpui::{AppContext, HeadlessAppContext, PrimitiveBatch, RenderCommand, px};
use gpui_ce_wgpu::WgpuHeadlessRenderer;

#[test]
fn the_gallery_renders() {
    let mut cx = HeadlessAppContext::with_platform(support::text_system(), Arc::new(()), || {
        Some(Box::new(
            WgpuHeadlessRenderer::new().expect("a GPU adapter"),
        ))
    });
    let window = cx
        .open_window(gpui::size(px(912.0), px(824.0)), |_, cx| {
            cx.new(|_| gallery::Gallery::frozen(1.5))
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        assert!(window.supports_shader_paint());
        let commands = window.rendered_scene_commands();
        let shader_quads: usize = commands
            .iter()
            .filter_map(|command| match command {
                RenderCommand::Batch(PrimitiveBatch::ShaderQuads { range, .. }) => {
                    Some(range.len())
                }
                _ => None,
            })
            .sum();
        assert!(
            shader_quads >= 12,
            "gallery emitted {shader_quads} shader quads"
        );
        assert!(
            commands.iter().any(|command| matches!(
                command,
                RenderCommand::Batch(PrimitiveBatch::Pipelines(_))
            ))
        );
    })
    .unwrap();
    let image = cx.capture_screenshot(window.into()).unwrap();
    if let Ok(path) = std::env::var("GPUI_SHADER_GALLERY_PNG") {
        image.save(path).unwrap();
    }

    // Interior samples must show each card, including the imported shaders.
    let scale = image.width() as f32 / 912.0;
    let page = image.get_pixel(4, 4).0;
    for (row, column) in (0..2).flat_map(|row| (0..4).map(move |column| (row, column))) {
        let x = 24.0 + column as f32 * 216.0 + 100.0;
        let y = 24.0 + 52.0 + row as f32 * 242.0 + 100.0;
        let pixel = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
        assert_ne!(pixel, page, "card ({row}, {column}) painted nothing");
    }
    // A varying paint must shade the interior, not merely outline the card.
    let samples: std::collections::HashSet<_> = [44.0, 80.0, 120.0, 160.0, 204.0]
        .map(|x| {
            image
                .get_pixel((x * scale) as u32, (176.0 * scale) as u32)
                .0
        })
        .into_iter()
        .collect();
    assert!(
        samples.len() >= 4,
        "wave card has only {} colors",
        samples.len()
    );
}
