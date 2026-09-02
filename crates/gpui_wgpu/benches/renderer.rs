use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use gpui::{
    Bounds, ContentMask, DevicePixels, PlatformHeadlessRenderer, Point, Quad, ScaledPixels, Scene,
    ShaderBool, Size, Underline, solid_background, white,
};
use gpui_ce_wgpu::WgpuHeadlessRenderer;

const TARGET_SIZE: Size<DevicePixels> = Size {
    width: DevicePixels(1280),
    height: DevicePixels(720),
};

fn quad_scene(count: usize) -> Scene {
    let mut scene = unplanned_quad_scene(count);
    scene.finish();
    scene
}

fn unplanned_quad_scene(count: usize) -> Scene {
    let mut scene = Scene::default();
    for index in 0..count {
        let column = index % 32;
        let row = index / 32;
        let bounds = Bounds {
            origin: Point {
                x: ScaledPixels((column * 40) as f32),
                y: ScaledPixels((row * 40) as f32),
            },
            size: Size {
                width: ScaledPixels(36.0),
                height: ScaledPixels(36.0),
            },
        };
        scene.insert_primitive(Quad {
            order: index as u32,
            bounds,
            content_mask: ContentMask { bounds },
            background: solid_background(gpui::rgb(0x336699)),
            ..Default::default()
        });
    }
    scene
}

fn unplanned_mixed_scene(count: usize) -> Scene {
    let mut scene = Scene::default();
    let bounds = Bounds {
        origin: Point {
            x: ScaledPixels(0.0),
            y: ScaledPixels(0.0),
        },
        size: Size {
            width: ScaledPixels(1.0),
            height: ScaledPixels(1.0),
        },
    };
    for index in 0..count {
        scene.insert_primitive(Quad {
            order: (index * 2) as u32,
            bounds,
            content_mask: ContentMask { bounds },
            background: solid_background(gpui::rgb(0x336699)),
            ..Default::default()
        });
        scene.insert_primitive(Underline {
            order: (index * 2 + 1) as u32,
            padding: 0,
            bounds,
            content_mask: ContentMask { bounds },
            color: white().into(),
            thickness: ScaledPixels(1.0),
            wavy: ShaderBool::Disabled,
        });
    }
    scene
}

fn bench_renderer(c: &mut Criterion) {
    let mut renderer = WgpuHeadlessRenderer::new().expect("headless WGPU renderer must initialize");
    let single_quad = quad_scene(1);
    let dense_quads = quad_scene(512);
    let mut mixed_batches = unplanned_mixed_scene(512);
    mixed_batches.finish();

    let mut group = c.benchmark_group("renderer_end_to_end");
    group.bench_function("single_quad", |b| {
        b.iter(|| {
            renderer
                .render_scene_to_image(&single_quad, TARGET_SIZE)
                .expect("single quad render must succeed")
        })
    });
    group.bench_function("512_quads", |b| {
        b.iter(|| {
            renderer
                .render_scene_to_image(&dense_quads, TARGET_SIZE)
                .expect("dense quad render must succeed")
        })
    });
    group.bench_function("single_quad_wait", |b| {
        b.iter(|| {
            renderer
                .render_scene_and_wait(&single_quad, TARGET_SIZE)
                .expect("single quad render must succeed")
        })
    });
    group.bench_function("512_quads_wait", |b| {
        b.iter(|| {
            renderer
                .render_scene_and_wait(&dense_quads, TARGET_SIZE)
                .expect("dense quad render must succeed")
        })
    });
    group.bench_function("1024_mixed_batches_wait", |b| {
        b.iter(|| {
            renderer
                .render_scene_and_wait(&mixed_batches, TARGET_SIZE)
                .expect("mixed batch render must succeed")
        })
    });
    group.finish();

    let mut group = c.benchmark_group("scene_plan");
    group.bench_function("compile_1024_mixed_batches", |b| {
        b.iter_batched(
            || unplanned_mixed_scene(512),
            |mut scene| {
                scene.finish();
                black_box(*scene.render_plan().requirements())
            },
            BatchSize::SmallInput,
        )
    });
    let mut mixed_scene = unplanned_mixed_scene(512);
    mixed_scene.finish();
    group.bench_function("traverse_1024_mixed_batches", |b| {
        b.iter(|| {
            for command in mixed_scene.render_commands() {
                black_box(command);
            }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_renderer);
criterion_main!(benches);
