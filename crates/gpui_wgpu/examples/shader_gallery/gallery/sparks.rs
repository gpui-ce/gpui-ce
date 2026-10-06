//! Instanced geometry drawn through a custom pipeline.

use gpui::{AnyElement, IntoElement, Styled, canvas, shader::Pipeline};
use std::{f32::consts::TAU, sync::LazyLock};

static SPARKS: LazyLock<Pipeline> =
    LazyLock::new(|| Pipeline::wgsl(include_str!("sparks.wgsl")).expect("the sparks pipeline"));

pub(super) fn sparks(time: f32) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = [
                f32::from(bounds.size.width) / 2.0,
                f32::from(bounds.size.height) / 2.0,
            ];
            let instances = (0..64).map(|index| {
                let along = index as f32 / 64.0;
                let angle = along * TAU * 3.0 + time * (0.5 + along);
                let reach = 16.0 + along * 74.0;
                let tint = [1.0 - along * 0.6, 0.4 + along * 0.5, 1.0, 0.9];
                let position = [
                    center[0] + angle.cos() * reach,
                    center[1] + angle.sin() * reach,
                ];
                (position, 4.0 + along * 10.0, tint)
            });
            window.paint_pipeline(&SPARKS, bounds, instances);
        },
    )
    .size_full()
    .into_any_element()
}
