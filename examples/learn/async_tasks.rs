//! Async Tasks Example
//!
//! This example demonstrates different async patterns in GPUI:
//!
//! 1. `cx.spawn` - Foreground tasks for UI updates
//! 2. `cx.background_spawn` - Background tasks for heavy computation
//! 3. Task management - Storing, canceling, and detaching tasks
//! 4. Progress updates - Communicating from background to UI

use std::time::Duration;

use gpui::{
    App, Application, Bounds, Context, Entity, Hsla, Render, Task, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};

// ============================================================================
// Example 1: Simple Foreground Task
// ============================================================================
//
// `cx.spawn` runs an async closure on the foreground thread.
// Use it when you need to perform async work that updates UI.

struct ForegroundTaskDemo {
    message: String,
    is_loading: bool,
}

impl ForegroundTaskDemo {
    fn new() -> Self {
        Self {
            message: "Click to start".into(),
            is_loading: false,
        }
    }

    fn start_task(&mut self, cx: &mut Context<Self>) {
        self.is_loading = true;
        self.message = "Loading...".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_secs(1)).await;

            this.update(cx, |this, cx| {
                this.message = "Task completed!".into();
                this.is_loading = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

// ============================================================================
// Example 2: Background Task with Progress
// ============================================================================
//
// `cx.background_spawn` runs work on a background thread pool.
// Results must be sent back to the foreground to update UI.

struct BackgroundTaskDemo {
    progress: u32,
    result: Option<u64>,
    is_computing: bool,
}

impl BackgroundTaskDemo {
    fn new() -> Self {
        Self {
            progress: 0,
            result: None,
            is_computing: false,
        }
    }

    fn start_computation(&mut self, cx: &mut Context<Self>) {
        self.is_computing = true;
        self.progress = 0;
        self.result = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            for i in 0..100 {
                let computation = cx.background_spawn(async move {
                    std::thread::sleep(Duration::from_millis(10));
                    (i + 1) as u64
                });

                let partial_result = computation.await;

                this.update(cx, |this, cx| {
                    this.progress = i as u32 + 1;
                    this.result = Some(partial_result);
                    cx.notify();
                })
                .ok();
            }

            this.update(cx, |this, cx| {
                this.is_computing = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

// ============================================================================
// Example 3: Cancellable Task
// ============================================================================
//
// Store a Task<T> to keep it running. Drop it to cancel.
// Use Option<Task<T>> for optional/cancellable operations.

struct CancellableTaskDemo {
    counter: u32,
    counting_task: Option<Task<()>>,
}

impl CancellableTaskDemo {
    fn new() -> Self {
        Self {
            counter: 0,
            counting_task: None,
        }
    }

    fn is_running(&self) -> bool {
        self.counting_task.is_some()
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.counting_task.is_some() {
            self.counting_task = None;
        } else {
            self.counter = 0;
            cx.notify();

            self.counting_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    smol::Timer::after(Duration::from_millis(100)).await;

                    let should_continue = this
                        .update(cx, |this, cx| {
                            this.counter += 1;
                            cx.notify();
                            this.counter < 100
                        })
                        .unwrap_or(false);

                    if !should_continue {
                        break;
                    }
                }
            }));
        }
    }
}

// ============================================================================
// Example 4: Task with Return Value
// ============================================================================
//
// Tasks can return values that can be awaited.

struct ReturnValueDemo {
    numbers: Vec<u32>,
    sum: Option<u32>,
    is_calculating: bool,
}

impl ReturnValueDemo {
    fn new() -> Self {
        Self {
            numbers: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            sum: None,
            is_calculating: false,
        }
    }

    fn calculate_sum(&mut self, cx: &mut Context<Self>) {
        self.is_calculating = true;
        self.sum = None;
        cx.notify();

        let numbers = self.numbers.clone();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    std::thread::sleep(Duration::from_millis(500));
                    numbers.iter().sum::<u32>()
                })
                .await;

            this.update(cx, |this, cx| {
                this.sum = Some(result);
                this.is_calculating = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn randomize(&mut self, cx: &mut Context<Self>) {
        use rand::Rng;
        let mut rng = rand::rng();
        self.numbers = (0..10).map(|_| rng.random_range(1..100)).collect();
        self.sum = None;
        cx.notify();
    }
}

// ============================================================================
// Main Application View
// ============================================================================

struct AsyncTasksExample {
    foreground_demo: Entity<ForegroundTaskDemo>,
    background_demo: Entity<BackgroundTaskDemo>,
    cancellable_demo: Entity<CancellableTaskDemo>,
    return_demo: Entity<ReturnValueDemo>,
}

impl AsyncTasksExample {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            foreground_demo: cx.new(|_| ForegroundTaskDemo::new()),
            background_demo: cx.new(|_| BackgroundTaskDemo::new()),
            cancellable_demo: cx.new(|_| CancellableTaskDemo::new()),
            return_demo: cx.new(|_| ReturnValueDemo::new()),
        }
    }
}

impl Render for AsyncTasksExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = self.foreground_demo.read(cx);
        let background = self.background_demo.read(cx);
        let cancellable = self.cancellable_demo.read(cx);
        let return_demo = self.return_demo.read(cx);

        div()
            .id("main")
            .size_full()
            .p_6()
            .bg(rgb(0x0f172a))
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .max_w(px(500.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::white())
                                    .child("Async Tasks"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x94a3b8))
                                    .child("Spawning, background work, and task management"),
                            ),
                    )
                    .child(demo_section(
                        "1. Foreground Task (cx.spawn)",
                        "Runs async work on the UI thread. Good for sequential async operations.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(gpui::white())
                                    .child(foreground.message.clone()),
                            )
                            .child(
                                button("foreground-btn", "Start Task", foreground.is_loading)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.foreground_demo.update(cx, |demo, cx| {
                                            demo.start_task(cx);
                                        });
                                    })),
                            ),
                    ))
                    .child(demo_section(
                        "2. Background Task (cx.background_spawn)",
                        "Runs heavy computation off the UI thread with progress updates.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(progress_bar(background.progress))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(gpui::white())
                                    .child(format!(
                                        "Progress: {}% | Result: {}",
                                        background.progress,
                                        background
                                            .result
                                            .map(|r| r.to_string())
                                            .unwrap_or_else(|| "-".into())
                                    )),
                            )
                            .child(
                                button("background-btn", "Compute", background.is_computing)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.background_demo.update(cx, |demo, cx| {
                                            demo.start_computation(cx);
                                        });
                                    })),
                            ),
                    ))
                    .child(demo_section(
                        "3. Cancellable Task",
                        "Store Task in a field to keep it running. Drop to cancel.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::white())
                                    .child(format!("{}", cancellable.counter)),
                            )
                            .child({
                                let is_running = cancellable.is_running();
                                div()
                                    .id("cancel-btn")
                                    .px_3()
                                    .py_1p5()
                                    .rounded_md()
                                    .text_sm()
                                    .text_color(gpui::white())
                                    .cursor_pointer()
                                    .when(is_running, |el| {
                                        el.bg(rgb(0xef4444))
                                            .hover(|style| style.bg(rgb(0xdc2626)))
                                    })
                                    .when(!is_running, |el| {
                                        el.bg(rgb(0x22c55e))
                                            .hover(|style| style.bg(rgb(0x16a34a)))
                                    })
                                    .child(if is_running {
                                        "Stop"
                                    } else {
                                        "Start Counter"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancellable_demo.update(cx, |demo, cx| {
                                            demo.toggle(cx);
                                        });
                                    }))
                            }),
                    ))
                    .child(demo_section(
                        "4. Task with Return Value",
                        "Tasks can return values that can be awaited or used in chained operations.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x94a3b8))
                                    .child(format!("Numbers: {:?}", return_demo.numbers)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(gpui::white())
                                    .child(format!(
                                        "Sum: {}",
                                        if return_demo.is_calculating {
                                            "Calculating...".into()
                                        } else {
                                            return_demo
                                                .sum
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| "Not calculated".into())
                                        }
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        button(
                                            "sum-btn",
                                            "Calculate Sum",
                                            return_demo.is_calculating,
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.return_demo.update(cx, |demo, cx| {
                                                demo.calculate_sum(cx);
                                            });
                                        })),
                                    )
                                    .child(
                                        div()
                                            .id("random-btn")
                                            .px_3()
                                            .py_1p5()
                                            .rounded_md()
                                            .text_sm()
                                            .text_color(gpui::white())
                                            .cursor_pointer()
                                            .bg(rgb(0x6366f1))
                                            .hover(|style| style.bg(rgb(0x4f46e5)))
                                            .child("Randomize")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.return_demo.update(cx, |demo, cx| {
                                                    demo.randomize(cx);
                                                });
                                            })),
                                    ),
                            ),
                    ))
                    .child(
                        div()
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(0x1e293b))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .text_xs()
                                    .text_color(rgb(0x94a3b8))
                                    .child("Key Patterns:")
                                    .child("• cx.spawn(async move |this, cx| ...) - foreground async")
                                    .child("• cx.background_spawn(async { ... }) - off UI thread")
                                    .child("• task.detach() - fire and forget")
                                    .child(
                                        "• task.detach_and_log_err(cx) - fire and forget with error logging",
                                    )
                                    .child("• Store Task<T> in field to keep running, drop to cancel"),
                            ),
                    ),
            )
    }
}

// ============================================================================
// UI Components
// ============================================================================

fn surface_color() -> Hsla {
    let base: Hsla = rgb(0x1e293b).into();
    base.opacity(0.5)
}

fn demo_section(
    title: &'static str,
    description: &'static str,
    content: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(surface_color())
        .border_1()
        .border_color(rgb(0x334155))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(gpui::white())
                        .child(title),
                )
                .child(div().text_xs().text_color(rgb(0x94a3b8)).child(description)),
        )
        .child(content)
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    disabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1p5()
        .rounded_md()
        .text_sm()
        .text_color(gpui::white())
        .when(disabled, |el| {
            el.bg(rgb(0x475569)).cursor_not_allowed().opacity(0.6)
        })
        .when(!disabled, |el| {
            el.bg(rgb(0x3b82f6))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x2563eb)))
                .active(|style| style.bg(rgb(0x1d4ed8)))
        })
        .child(label)
}

fn progress_bar(progress: u32) -> impl IntoElement {
    let clamped = progress.min(100);
    div()
        .h_2()
        .w_full()
        .rounded_full()
        .bg(rgb(0x334155))
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(0x22c55e))
                .w(gpui::relative(clamped as f32 / 100.0)),
        )
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(550.), px(850.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| AsyncTasksExample::new(cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
    });
}
