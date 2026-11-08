use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use gpui::{
    CaretPosition, InlineLayoutRequest, Pixels, PlatformTextSystem, TextAlign, TextLayoutOptions,
    TextLayoutRequest, TextRun, hsla, px,
};
use std::hint::black_box;
use support::text_system;

mod support;

fn styled_runs(base_run: &TextRun, text_len: usize, count: usize) -> Vec<TextRun> {
    assert_eq!(text_len % count, 0);

    (0..count)
        .map(|idx| TextRun {
            len: text_len / count,
            color: hsla((idx % 2) as f32 * 0.5, 0.8, 0.4, 1.),
            ..base_run.clone()
        })
        .collect()
}

fn inline_request<'a>(
    text: &'a str,
    runs: &'a [TextRun],
    wrap_width: Option<Pixels>,
) -> InlineLayoutRequest<'a> {
    InlineLayoutRequest {
        text,
        runs,
        text_styles: &[],
        boxes: &[],
        font_size: px(16.),
        line_height: px(24.),
        base_font: &runs[0].font,
        options: TextLayoutOptions {
            wrap_width,
            text_align: TextAlign::Left,
            ..Default::default()
        },
        bidi_scopes: &[],
    }
}

fn bench_inline_geometry(criterion: &mut Criterion) {
    let (system, base_run) = text_system();
    let segment = "office ";
    let text = segment.repeat(256);
    let ranges = (0..256)
        .map(|idx| idx * segment.len()..(idx + 1) * segment.len())
        .collect::<Vec<_>>();

    let mut group = criterion.benchmark_group("parley_inline_geometry");

    for count in [1, 256] {
        let runs = styled_runs(&base_run, text.len(), count);
        let inline = system.layout_inline(inline_request(&text, &runs, None));
        let platform = &inline.layout.platform_layout;

        group.bench_function(BenchmarkId::new("one_range", count), |bencher| {
            bencher.iter(|| platform.inline_geometry(black_box(0..text.len())));
        });
        group.bench_function(BenchmarkId::new("many_ranges", count), |bencher| {
            bencher.iter(|| platform.inline_geometry_for_ranges(black_box(&ranges)));
        });
    }

    group.finish();
}

fn bench_late_row_carets(criterion: &mut Criterion) {
    let (system, base_run) = text_system();
    let mut group = criterion.benchmark_group("parley_late_row_carets");

    for count in [1, 16, 64, 256, 1024] {
        let text = "office ".repeat(count);
        let runs = [TextRun {
            len: text.len(),
            ..base_run.clone()
        }];
        let request = inline_request(&text, &runs, Some(px(70.)));
        let raw = system.layout_text(TextLayoutRequest {
            text: request.text,
            runs: request.runs,
            font_size: request.font_size,
            options: request.options,
        });
        let inline = system.layout_inline(request);
        let native = &inline.layout.platform_layout;
        let caret = CaretPosition::attached_to_previous_cluster(text.len());

        group.bench_function(BenchmarkId::new("raw_bounds", count), |bencher| {
            bencher.iter(|| raw.platform_layout.caret_bounds(black_box(caret), px(24.)));
        });
        group.bench_function(BenchmarkId::new("native_line_index", count), |bencher| {
            bencher.iter(|| native.visual_line_index(black_box(caret)));
        });

        group.bench_function(BenchmarkId::new("native_bounds", count), |bencher| {
            bencher.iter(|| native.caret_bounds(black_box(caret), px(24.)));
        });

        let wrap_idx = raw
            .visual_lines
            .get(1)
            .map_or(0, |line| line.text_range.start);

        let carets = [0, text.len() / 2, wrap_idx, text.len()]
            .into_iter()
            .flat_map(|idx| {
                [
                    CaretPosition::attached_to_previous_cluster(idx),
                    CaretPosition::attached_to_next_cluster(idx),
                ]
            })
            .collect::<Vec<_>>();

        group.bench_function(BenchmarkId::new("raw_bounds_varied", count), |bencher| {
            let mut positions = carets.iter().copied().cycle();

            bencher.iter(|| {
                raw.platform_layout
                    .caret_bounds(black_box(positions.next().unwrap()), px(24.))
            });
        });

        group.bench_function(BenchmarkId::new("native_bounds_varied", count), |bencher| {
            let mut positions = carets.iter().copied().cycle();

            bencher.iter(|| native.caret_bounds(black_box(positions.next().unwrap()), px(24.)));
        });

        group.bench_function(
            BenchmarkId::new("native_line_index_varied", count),
            |bencher| {
                let mut positions = carets.iter().copied().cycle();

                bencher.iter(|| native.visual_line_index(black_box(positions.next().unwrap())));
            },
        );

        if ![16, 1024].contains(&count) {
            continue;
        }

        group.bench_function(BenchmarkId::new("raw_bounds_first", count), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let (system, _run) = text_system();

                    system.layout_text(TextLayoutRequest {
                        text: request.text,
                        runs: request.runs,
                        font_size: request.font_size,
                        options: request.options,
                    })
                },
                |raw| raw.platform_layout.caret_bounds(black_box(caret), px(24.)),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("native_bounds_first", count), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let (system, _run) = text_system();

                    system.layout_inline(request)
                },
                |inline| {
                    inline
                        .layout
                        .platform_layout
                        .caret_bounds(black_box(caret), px(24.))
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(
            BenchmarkId::new("native_line_index_first", count),
            |bencher| {
                bencher.iter_batched_ref(
                    || {
                        let (system, _run) = text_system();

                        system.layout_inline(request)
                    },
                    |inline| {
                        inline
                            .layout
                            .platform_layout
                            .visual_line_index(black_box(caret))
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_inline_layout_cold(criterion: &mut Criterion) {
    let (_system, base_run) = text_system();
    let text = "office ".repeat(256);
    let mut group = criterion.benchmark_group("parley_inline_layout_cold");

    for count in [1, 256] {
        let runs = styled_runs(&base_run, text.len(), count);
        let request = inline_request(&text, &runs, Some(px(480.)));

        group.bench_function(BenchmarkId::new("shared_parent_font", count), |bencher| {
            bencher.iter_batched(
                || text_system().0,
                |system| system.layout_inline(black_box(request)),
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_inline_geometry,
    bench_late_row_carets,
    bench_inline_layout_cold,
);
criterion_main!(benches);
