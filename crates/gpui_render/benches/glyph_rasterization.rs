use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gpui::{
    GlyphRenderMode, PlatformTextSystem, PreparedRasterStyle, RenderGlyphParams, TextSystem, font,
    px,
};
use gpui_ce_render::SwashGlyphRasterizer;
use gpui_fonts::IBM_PLEX;
use gpui_parley::ParleyTextSystem;
use std::{borrow::Cow, sync::Arc};

fn raster_case() -> (TextSystem, RenderGlyphParams) {
    let system = ParleyTextSystem::new(SwashGlyphRasterizer::default())
        .with_fallback_families([IBM_PLEX.family]);

    system
        .add_fonts(vec![Cow::Borrowed(IBM_PLEX.data)])
        .unwrap();

    let font_id = system.font_id(&font(IBM_PLEX.family)).unwrap();

    let params = RenderGlyphParams {
        font_id,
        glyph_id: system.glyph_for_char(font_id, 'A').unwrap(),
        font_size: px(16.0),
        subpixel_variant: Default::default(),
        scale_factor: 1.0,
        raster_style: PreparedRasterStyle::independent(GlyphRenderMode::Grayscale),
    };

    (TextSystem::new(Arc::new(system)), params)
}

fn bench_glyph_rasterization(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("swash_glyph_rasterization");

    group.bench_function("first_glyph_rasterization", |bencher| {
        bencher.iter_batched(
            raster_case,
            |(system, params)| system.rasterize_glyph(&params).unwrap(),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("cached_glyph_rasterization", |bencher| {
        let (system, params) = raster_case();
        system.rasterize_glyph(&params).unwrap();
        bencher.iter(|| system.rasterize_glyph(&params).unwrap());
    });

    group.finish();
}

criterion_group!(benches, bench_glyph_rasterization);
criterion_main!(benches);
