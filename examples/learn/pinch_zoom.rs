//! Pinch Zoom Example
//!
//! This example demonstrates how to handle pinch-to-zoom gestures in GPUI using:
//!
//! 1. `PinchEvent` - Gesture event data including delta and position
//! 2. `.on_pinch()` - Handle pinch gestures during bubble phase
//! 3. `TouchPhase` - Track gesture lifecycle (Started, Moved, Ended)
//!
//! The example shows a rectangle that can be zoomed in/out using pinch gestures
//! on trackpads (macOS/Linux) or touchscreens.

use gpui::{
    actions, div, prelude::*, px, rgb, rgba, size, Application, Bounds, Context,
    EventEmitter, InteractiveElement, IntoElement, ParentElement, PinchEvent, Render,
    Styled, TouchPhase, Window, WindowBounds, WindowOptions,
};

#[path = "../prelude.rs"]
mod example_prelude;

actions!(pinch_zoom, [ZoomIn, ZoomOut, Reset]);

fn main() {
    Application::new().run(move |cx| {
        cx.activate(true);
        let bounds = Bounds::centered(None, size(px(800.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| PinchZoomExample::new(cx)),
        )
        .unwrap();
    })
}

struct PinchZoomExample {
    scale: f32,
    min_scale: f32,
    max_scale: f32,
}

impl PinchZoomExample {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            scale: 1.0,
            min_scale: 0.1,
            max_scale: 5.0,
        }
    }

    fn apply_zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        // The delta from pinch events is typically small (e.g., 0.1 = 10% zoom)
        // We multiply by current scale for proportional zooming
        let scale_delta = delta * self.scale;

        // Apply the zoom and clamp to min/max bounds
        self.scale = (self.scale + scale_delta).max(self.min_scale).min(self.max_scale);

        cx.notify();
    }
}

impl EventEmitter<()> for PinchZoomExample {}

impl Render for PinchZoomExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(0x1a1a1a))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("Pinch to Zoom Example"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgba(0xffffff80))
                    .child("Use pinch gestures on your trackpad or touchscreen to zoom"),
            )
            .child(
                // The zoomable element
                div()
                    .id("zoomable-rect")
                    .w(px(400.0 * self.scale))
                    .h(px(300.0 * self.scale))
                    .border_1()
                    .border_color(rgba(0x4a90d9ff))
                    .rounded_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x2a2a2a))
                    .on_pinch(cx.listener(|this, event: &PinchEvent, _window, cx| {
                        // Apply zoom based on gesture phase
                        match event.phase {
                            TouchPhase::Started => {
                                // Gesture just started - we could track initial state here
                            }
                            TouchPhase::Moved => {
                                // Gesture is updating - apply zoom delta
                                this.apply_zoom(event.delta, cx);
                            }
                            TouchPhase::Ended => {
                                // Gesture completed
                            }
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgba(0x4a90d9ff))
                                    .child(format!("{:.1}x", self.scale)),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .text_color(rgba(0xffffff99))
                                    .child(if self.scale >= 2.0 {
                                        "Zoomed In"
                                    } else if self.scale <= 0.5 {
                                        "Zoomed Out"
                                    } else {
                                        "Normal Scale"
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgba(0xffffff66))
                                    .child(format!(
                                        "Scale: {:.2} (min: {:.1}, max: {:.1})",
                                        self.scale, self.min_scale, self.max_scale
                                    )),
                            ),
                    )
                    .when(
                        self.scale != 1.0,
                        |this| {
                            // Visual feedback when zoomed
                            this.border_2().border_color(rgba(0x4a90d9cc))
                        },
                    ),
            )
            .child(
                div()
                    .mt_4()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgba(0xffffff66))
                            .child("Tip: Pinch in/out on trackpad to zoom"),
                    ),
            )
    }
}
