use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gpui::{PlatformTextSystem, TextAlign, TextLayoutOptions, TextLayoutRequest, TextRun, px};
use support::text_system;

mod support;

fn code_text() -> String {
    "fn layout(text: &str, width: Pixels) -> LineLayout { cache.get(text, width) } ".repeat(64)
}

fn bench_text_pipeline(criterion: &mut Criterion) {
    let (system, base_run) = text_system();
    let code = code_text();
    let multilingual =
        "office cafe\u{301} العربية אבג 日本語 ไทย 👩🏽‍💻 🇬🇧 one two three four".repeat(16);

    let mut group = criterion.benchmark_group("parley_text_pipeline");
    for (name, text, font_size, wrap_width) in [
        ("layout_code_warm", code.as_str(), 14.0, None),
        (
            "layout_multilingual_warm",
            multilingual.as_str(),
            16.0,
            None,
        ),
        (
            "wrap_multilingual_warm",
            multilingual.as_str(),
            16.0,
            Some(px(480.0)),
        ),
    ] {
        let runs = [TextRun {
            len: text.len(),
            ..base_run.clone()
        }];

        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                system.layout_text(TextLayoutRequest {
                    text,
                    font_size: px(font_size),
                    runs: &runs,
                    options: TextLayoutOptions {
                        wrap_width,
                        text_align: TextAlign::Left,
                        ..Default::default()
                    },
                })
            });
        });
    }

    group.bench_function("layout_multilingual_cold", |bencher| {
        bencher.iter_batched(
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
                    options: TextLayoutOptions {
                        text_align: TextAlign::Left,
                        ..Default::default()
                    },
                })
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_text_pipeline);
criterion_main!(benches);
