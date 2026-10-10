//! Run with `RUST_LOG=info cargo run -p gpui-ce --example app_activation`.
//! Queue a request, then switch to another application before the five-second delay expires.
//! The event log records requests and observed focus changes separately.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, rgb, size,
};
use gpui_platform::application;

#[derive(Clone, Copy)]
enum ActivationTarget {
    App,
    Window,
}

impl ActivationTarget {
    fn label(self) -> &'static str {
        match self {
            Self::App => "App::activate(true)",
            Self::Window => "Window::activate()",
        }
    }
}

struct ActivationExample {
    started: Instant,
    events: VecDeque<String>,
    pending: bool,
    seconds_remaining: u64,
}

impl ActivationExample {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_activation(window, |this, window, cx| {
            this.record(format!("FOCUS active={}", window.is_window_active()));
            cx.notify();
        })
        .detach();

        let mut this = Self {
            started: Instant::now(),
            events: VecDeque::new(),
            pending: false,
            seconds_remaining: 0,
        };
        this.record(format!("Backend: {}", cx.compositor_name()));
        this
    }

    fn record(&mut self, message: String) {
        let event = format!("{:>6.1}s  {message}", self.started.elapsed().as_secs_f32());
        eprintln!("ACTIVATION {event}");
        if self.events.len() == 7 {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    fn schedule(
        &mut self,
        target: ActivationTarget,
        minimize: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.seconds_remaining = 5;
        self.record(format!("QUEUED {} (minimize={minimize})", target.label()));
        cx.notify();
        let handle = window.window_handle().downcast::<Self>().unwrap();
        if minimize {
            window.minimize_window();
        }

        cx.spawn(async move |this, cx| {
            for remaining in (0..5).rev() {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |this, cx| {
                        this.seconds_remaining = remaining;
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }

            if handle
                .update(cx, |this, window, cx| {
                    this.record(format!(
                        "REQUEST {} (active before={})",
                        target.label(),
                        window.is_window_active()
                    ));
                    match target {
                        ActivationTarget::App => cx.activate(true),
                        ActivationTarget::Window => window.activate(),
                    }
                    cx.notify();
                })
                .is_err()
            {
                return;
            }

            cx.background_executor().timer(Duration::from_secs(1)).await;
            handle
                .update(cx, |this, window, cx| {
                    this.pending = false;
                    this.record(format!(
                        "OBSERVED after 1s: active={}",
                        window.is_window_active()
                    ));
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }
}

impl Render for ActivationExample {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("app-activation")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_6()
            .gap_3()
            .bg(rgb(0xf5f5f5))
            .text_color(rgb(0x202020))
            .child(div().text_xl().child("Application activation test"))
            .child(div().text_sm().child(
                "Click a button, then switch to another app. The request runs after 5 seconds.",
            ))
            .child(format!("Window focused: {}", window.is_window_active()))
            .child(if self.pending {
                if self.seconds_remaining > 0 {
                    format!("Request in {} seconds", self.seconds_remaining)
                } else {
                    "Observing focus after the request...".into()
                }
            } else {
                "Ready".into()
            })
            .children(
                [
                    ("Activate app after 5s", ActivationTarget::App, false),
                    ("Activate window after 5s", ActivationTarget::Window, false),
                    ("Minimize, then activate app", ActivationTarget::App, true),
                    (
                        "Minimize, then activate window",
                        ActivationTarget::Window,
                        true,
                    ),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (label, target, minimize))| {
                    div()
                        .id(index)
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(0x2563eb))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .when(self.pending, |this| this.opacity(0.5))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.schedule(target, minimize, window, cx);
                        }))
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_sm()
                    .children(self.events.iter().cloned().map(|event| div().child(event))),
            )
    }
}

fn main() {
    env_logger::init();
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(620.), px(620.)), cx);
        cx.open_window(
            WindowOptions {
                app_id: Some("dev.gpui.activation-test".into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("GPUI application activation".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                focus: true,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| ActivationExample::new(window, cx)),
        )
        .unwrap();
    });
}
