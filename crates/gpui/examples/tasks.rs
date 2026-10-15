//! Async work: a stopwatch driven by a timer, and a prime search that runs in the
//! background and can be cancelled.
//!
//! Both start a `Task` and keep it in a field. Dropping the task stops the work. The
//! stopwatch awaits a timer in a loop inside `Context::spawn`; the search awaits
//! `background_spawn` one chunk at a time and reports progress after each chunk.
//!
//! Run it with `cargo run --example tasks`.

use std::time::Duration;

use gpui::{
    App, Bounds, Context, Div, FontWeight, QuitMode, Stateful, Task, Window, WindowBounds,
    WindowOptions, colors::Colors, div, prelude::*, px, relative, size,
};

const TICK: Duration = Duration::from_millis(100);
const LIMIT: u64 = 300_000;
const CHUNK: u64 = 5_000;

#[derive(Default)]
struct Tasks {
    elapsed: Duration,
    stopwatch: Option<Task<()>>,
    checked: u64,
    found: usize,
    search: Option<Task<()>>,
}

impl Tasks {
    fn start_stopwatch(&mut self, cx: &mut Context<Self>) {
        self.stopwatch = Some(cx.spawn(async move |tasks, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                // The update fails once the view is gone, which ends the loop.
                let ticked = tasks.update(cx, |tasks, cx| {
                    tasks.elapsed += TICK;
                    cx.notify();
                });
                if ticked.is_err() {
                    break;
                }
            }
        }));
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        self.checked = 0;
        self.found = 0;
        self.search = Some(cx.spawn(async move |tasks, cx| {
            let mut next = 0;
            while next < LIMIT {
                let range = next..(next + CHUNK).min(LIMIT);
                next = range.end;
                // Runs on a worker thread natively, and between frames on the web.
                let found = cx
                    .background_spawn(async move { range.filter(|&n| is_prime(n)).count() })
                    .await;
                let updated = tasks.update(cx, |tasks, cx| {
                    tasks.checked = next;
                    tasks.found += found;
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            tasks.update(cx, |tasks, _| tasks.search = None).ok();
        }));
    }
}

fn is_prime(n: u64) -> bool {
    n >= 2
        && (2..)
            .take_while(|d| d * d <= n)
            .all(|d| !n.is_multiple_of(d))
}

impl Render for Tasks {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let panel = |title: &'static str| {
            div()
                .flex_1()
                .min_w(px(260.))
                .flex()
                .flex_col()
                .gap_4()
                .p_5()
                .rounded_lg()
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
        };
        let button = |id: &'static str, label: &'static str| -> Stateful<Div> {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_md()
                .bg(colors.selected)
                .text_color(colors.selected_text)
                .cursor_pointer()
                .child(label)
        };

        let stopwatch = panel("Stopwatch")
            .child(
                div()
                    .text_3xl()
                    .font_family("Lilex")
                    .child(format!("{:.1} s", self.elapsed.as_secs_f32())),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(if self.stopwatch.is_some() {
                        button("stop", "Stop").on_click(cx.listener(|tasks, _, _, cx| {
                            tasks.stopwatch = None;
                            cx.notify();
                        }))
                    } else {
                        button("start", "Start").on_click(cx.listener(|tasks, _, _, cx| {
                            tasks.start_stopwatch(cx);
                            cx.notify();
                        }))
                    })
                    .child(
                        button("reset", "Reset").on_click(cx.listener(|tasks, _, _, cx| {
                            tasks.elapsed = Duration::ZERO;
                            cx.notify();
                        })),
                    ),
            );

        let search = panel("Prime search")
            .child(
                div().h_2().rounded_full().bg(colors.container).child(
                    div()
                        .h_full()
                        .w(relative(self.checked as f32 / LIMIT as f32))
                        .rounded_full()
                        .bg(colors.selected),
                ),
            )
            .child(div().text_sm().child(format!(
                "{} primes among the first {} numbers",
                self.found, self.checked
            )))
            .child(if self.search.is_some() {
                button("cancel", "Cancel").on_click(cx.listener(|tasks, _, _, cx| {
                    tasks.search = None;
                    cx.notify();
                }))
            } else {
                button("search", "Search").on_click(cx.listener(|tasks, _, _, cx| {
                    tasks.start_search(cx);
                    cx.notify();
                }))
            });

        div()
            .size_full()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_4()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(stopwatch)
            .child(search)
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(680.), px(320.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| cx.new(|_cx| Tasks::default()),
    )
    .unwrap();
    cx.activate(true);
}
