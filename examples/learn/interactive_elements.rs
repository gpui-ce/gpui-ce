//! Interactive Elements Example
//!
//! This example demonstrates three different approaches to creating interactive
//! stateful components in GPUI:
//!
//! 1. `use_state` - Hook-like state scoped to an element's lifetime
//! 2. `RenderOnce` - Stateless component that receives state from parent
//! 3. `Render` - Entity-backed view with persistent internal state

use gpui::{
    App, Application, Bounds, Context, Entity, IntoElement, Render, RenderOnce, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};

// ============================================================================
// Approach 1: use_state
// ============================================================================
//
// `use_state` creates element-scoped state that persists across renders.
// It's similar to React's useState hook. The state is automatically tied
// to the element's identity via caller location or a provided key.
//
// Pros:
// - Simple, hook-like API
// - State is scoped to element lifetime
// - No boilerplate for simple state
//
// Cons:
// - Less explicit than Entity-backed state
// - State is tied to call site location

struct UseStateCounter {
    count: i32,
}

fn use_state_counter(window: &mut Window, cx: &mut App) -> impl IntoElement {
    let state: Entity<UseStateCounter> =
        window.use_state(cx, |_window, _cx| UseStateCounter { count: 0 });

    let count = state.read(cx).count;

    div()
        .id("use-state-counter")
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .bg(gpui::rgb(0x1e293b))
        .child(
            div()
                .text_sm()
                .text_color(gpui::rgb(0x94a3b8))
                .child("use_state Counter"),
        )
        .child(
            div()
                .text_2xl()
                .text_color(gpui::white())
                .child(format!("{}", count)),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .id("use-state-decrement")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(gpui::rgb(0xef4444))
                        .text_color(gpui::white())
                        .cursor_pointer()
                        .hover(|style| style.bg(gpui::rgb(0xdc2626)))
                        .active(|style| style.bg(gpui::rgb(0xb91c1c)))
                        .child("−")
                        .on_click({
                            let state = state.clone();
                            move |_, _, cx| {
                                state.update(cx, |state, cx| {
                                    state.count -= 1;
                                    cx.notify();
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .id("use-state-increment")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(gpui::rgb(0x22c55e))
                        .text_color(gpui::white())
                        .cursor_pointer()
                        .hover(|style| style.bg(gpui::rgb(0x16a34a)))
                        .active(|style| style.bg(gpui::rgb(0x15803d)))
                        .child("+")
                        .on_click(move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                state.count += 1;
                                cx.notify();
                            });
                        }),
                ),
        )
}

// ============================================================================
// Approach 2: RenderOnce
// ============================================================================
//
// `RenderOnce` components are stateless and consumed when rendered.
// They receive all data as props and delegate state management to the parent.
// This is the recommended approach for presentational components.
//
// Pros:
// - Clear data flow (props down, events up)
// - Lightweight (no Entity allocation)
// - Easy to test
// - Highly composable
//
// Cons:
// - Cannot maintain internal state
// - Parent must manage all state

#[derive(IntoElement)]
struct RenderOnceCounter {
    count: i32,
    on_increment: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    on_decrement: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl RenderOnceCounter {
    fn new(count: i32) -> Self {
        Self {
            count,
            on_increment: None,
            on_decrement: None,
        }
    }

    fn on_increment(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_increment = Some(Box::new(callback));
        self
    }

    fn on_decrement(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_decrement = Some(Box::new(callback));
        self
    }
}

impl RenderOnce for RenderOnceCounter {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id("render-once-counter")
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(gpui::rgb(0x1e293b))
            .child(
                div()
                    .text_sm()
                    .text_color(gpui::rgb(0x94a3b8))
                    .child("RenderOnce Counter"),
            )
            .child(
                div()
                    .text_2xl()
                    .text_color(gpui::white())
                    .child(format!("{}", self.count)),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("render-once-decrement")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(gpui::rgb(0xef4444))
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .hover(|style| style.bg(gpui::rgb(0xdc2626)))
                            .active(|style| style.bg(gpui::rgb(0xb91c1c)))
                            .child("−")
                            .when_some(self.on_decrement, |element, callback| {
                                element.on_click(move |_, window, cx| callback(window, cx))
                            }),
                    )
                    .child(
                        div()
                            .id("render-once-increment")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(gpui::rgb(0x22c55e))
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .hover(|style| style.bg(gpui::rgb(0x16a34a)))
                            .active(|style| style.bg(gpui::rgb(0x15803d)))
                            .child("+")
                            .when_some(self.on_increment, |element, callback| {
                                element.on_click(move |_, window, cx| callback(window, cx))
                            }),
                    ),
            )
    }
}

// ============================================================================
// Approach 3: Render (Entity-backed)
// ============================================================================
//
// `Render` components are backed by an `Entity<T>` and maintain their own
// internal state. This is the recommended approach for complex components
// that need to manage their own state, subscribe to events, or spawn tasks.
//
// Pros:
// - Full control over internal state
// - Can subscribe to events and observe other entities
// - Can spawn async tasks
// - Has identity (can be passed around as Entity<T>)
//
// Cons:
// - More boilerplate
// - Higher memory overhead
// - More complex lifecycle

struct RenderCounter {
    count: i32,
}

impl RenderCounter {
    fn new() -> Self {
        Self { count: 0 }
    }

    fn increment(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.count += 1;
        cx.notify();
    }

    fn decrement(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.count -= 1;
        cx.notify();
    }
}

impl Render for RenderCounter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("render-counter")
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .bg(gpui::rgb(0x1e293b))
            .child(
                div()
                    .text_sm()
                    .text_color(gpui::rgb(0x94a3b8))
                    .child("Render Counter"),
            )
            .child(
                div()
                    .text_2xl()
                    .text_color(gpui::white())
                    .child(format!("{}", self.count)),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("render-decrement")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(gpui::rgb(0xef4444))
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .hover(|style| style.bg(gpui::rgb(0xdc2626)))
                            .active(|style| style.bg(gpui::rgb(0xb91c1c)))
                            .child("−")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.decrement(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("render-increment")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(gpui::rgb(0x22c55e))
                            .text_color(gpui::white())
                            .cursor_pointer()
                            .hover(|style| style.bg(gpui::rgb(0x16a34a)))
                            .active(|style| style.bg(gpui::rgb(0x15803d)))
                            .child("+")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.increment(window, cx);
                            })),
                    ),
            )
    }
}

// ============================================================================
// Main Application View
// ============================================================================

struct InteractiveElementsExample {
    render_counter: Entity<RenderCounter>,
    render_once_count: i32,
}

impl InteractiveElementsExample {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            render_counter: cx.new(|_| RenderCounter::new()),
            render_once_count: 0,
        }
    }
}

impl Render for InteractiveElementsExample {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_once_count = self.render_once_count;
        let handle = cx.entity().downgrade();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_6()
            .p_8()
            .bg(gpui::rgb(0x0f172a))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(gpui::white())
                            .child("Interactive Elements"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(gpui::rgb(0x94a3b8))
                            .child("Three approaches to stateful components in GPUI"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .child(use_state_counter(window, cx))
                    .child(
                        RenderOnceCounter::new(render_once_count)
                            .on_increment({
                                let handle = handle.clone();
                                move |_window, cx| {
                                    handle
                                        .update(cx, |this, cx| {
                                            this.render_once_count += 1;
                                            cx.notify();
                                        })
                                        .ok();
                                }
                            })
                            .on_decrement(move |_window, cx| {
                                handle
                                    .update(cx, |this, cx| {
                                        this.render_once_count -= 1;
                                        cx.notify();
                                    })
                                    .ok();
                            }),
                    )
                    .child(self.render_counter.clone()),
            )
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .rounded_lg()
                    .bg(gpui::rgb(0x1e293b))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .text_sm()
                            .text_color(gpui::rgb(0x94a3b8))
                            .child("• use_state: Hook-like state scoped to element lifetime")
                            .child("• RenderOnce: Stateless component, parent manages state")
                            .child("• Render: Entity-backed view with internal state"),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(600.), px(400.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| InteractiveElementsExample::new(cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
    });
}
