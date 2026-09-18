use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gpui::{
    FontFallbacks, GlyphRenderMode, PlatformTextSystem, PreparedRasterStyle, RenderGlyphParams,
    TextLayoutRequest, TextRun, TextSystem, font, px,
};
use gpui_ce_parley::{ParleyTextSystem, SystemFonts};
use std::borrow::Cow;
use std::sync::Arc;

const IBM_PLEX: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
const LILEX: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");
const SOURCE_SERIF: &[u8] =
    include_bytes!("../../../assets/fonts/source-serif-4/SourceSerif4[opsz,wght].ttf");

fn text_system() -> (ParleyTextSystem, TextRun) {
    let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans");
    system
        .add_fonts(vec![
            Cow::Borrowed(IBM_PLEX),
            Cow::Borrowed(LILEX),
            Cow::Borrowed(SOURCE_SERIF),
        ])
        .unwrap();
    let mut descriptor = font("IBM Plex Sans");
    descriptor.fallbacks = Some(FontFallbacks::from_fonts(vec![
        "Source Serif 4".to_string(),
        "Lilex".to_string(),
    ]));
    (
        system,
        TextRun {
            len: 0,
            font: descriptor,
            letter_spacing: None,
            ..Default::default()
        },
    )
}

fn code_text() -> String {
    "fn layout(text: &str, width: Pixels) -> LineLayout { cache.get(text, width) } ".repeat(64)
}

fn raster_case() -> (TextSystem, RenderGlyphParams) {
    let (system, mut run) = text_system();
    let text = "A";
    run.len = text.len();
    let layout = system.layout_text(TextLayoutRequest {
        text,
        font_size: px(16.0),
        runs: &[run],
        wrap_width: None,
        line_clamp: None,
    });
    let fragment = &layout.paint_fragments[0];
    let glyph = &fragment.glyphs[0];
    let params = RenderGlyphParams {
        font_id: fragment.font_id,
        glyph_id: glyph.id,
        font_size: layout.font_size,
        subpixel_variant: Default::default(),
        scale_factor: 1.0,
        raster_style: PreparedRasterStyle::independent(if glyph.is_emoji {
            GlyphRenderMode::Color
        } else {
            GlyphRenderMode::Grayscale
        }),
    };
    (TextSystem::new(Arc::new(system)), params)
}

fn bench_text_pipeline(c: &mut Criterion) {
    let (system, base_run) = text_system();
    let code = code_text();
    let multilingual =
        "office cafe\u{301} العربية אבג 日本語 ไทย 👩🏽‍💻 🇬🇧 one two three four".repeat(16);

    let mut group = c.benchmark_group("parley_text_pipeline");
    group.bench_function("layout_code_warm", |b| {
        let runs = [TextRun {
            len: code.len(),
            ..base_run.clone()
        }];
        b.iter(|| {
            system.layout_text(TextLayoutRequest {
                text: &code,
                font_size: px(14.0),
                runs: &runs,
                wrap_width: None,
                line_clamp: None,
            })
        });
    });
    group.bench_function("layout_multilingual_warm", |b| {
        let runs = [TextRun {
            len: multilingual.len(),
            ..base_run.clone()
        }];
        b.iter(|| {
            system.layout_text(TextLayoutRequest {
                text: &multilingual,
                font_size: px(16.0),
                runs: &runs,
                wrap_width: None,
                line_clamp: None,
            })
        });
    });
    group.bench_function("wrap_multilingual_warm", |b| {
        let runs = [TextRun {
            len: multilingual.len(),
            ..base_run.clone()
        }];
        b.iter(|| {
            system.layout_text(TextLayoutRequest {
                text: &multilingual,
                font_size: px(16.0),
                runs: &runs,
                wrap_width: Some(px(480.0)),
                line_clamp: None,
            })
        });
    });
    group.bench_function("layout_multilingual_cold", |b| {
        b.iter_batched(
            text_system,
            |(system, run)| {
                let runs = [TextRun {
                    len: multilingual.len(),
                    ..run
                }];
                system.layout_text(TextLayoutRequest {
                    text: &multilingual,
                    font_size: px(16.0),
                    runs: &runs,
                    wrap_width: None,
                    line_clamp: None,
                })
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("first_glyph_rasterization", |b| {
        b.iter_batched(
            raster_case,
            |(system, params)| system.rasterize_glyph(&params).unwrap(),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("cached_glyph_rasterization", |b| {
        let (system, params) = raster_case();
        system.rasterize_glyph(&params).unwrap();
        b.iter(|| system.rasterize_glyph(&params).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_text_pipeline);
criterion_main!(benches);
