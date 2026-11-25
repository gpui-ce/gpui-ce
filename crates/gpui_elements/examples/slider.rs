//! Run with `cargo run -p gpui_ce_elements --example slider`.

use gpui::{
    App, Axis, Bounds, Context, FocusHandle, KeyBinding, SharedString, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use gpui_ce_elements::slider::{
    BaseSlider, BaseSliderChange, BaseSliderControl, BaseSliderIndicator, BaseSliderLabel,
    BaseSliderThumb, BaseSliderThumbCollisionBehavior, BaseSliderTrack,
};

actions!(slider_example, [FocusNext, FocusPrevious]);

struct SliderExample {
    focus_handle: FocusHandle,
    volume: f32,
    range: [f32; 2],
    brightness: f32,
    last_committed: SharedString,
}

impl Render for SliderExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("slider-example")
            .track_focus(&self.focus_handle)
            .tab_group()
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
            .size_full()
            .flex()
            .flex_col()
            .gap_8()
            .p_6()
            .bg(rgb(0x18181b))
            .text_color(rgb(0xfafafa))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child("Click the track or drag a thumb to change its value.")
                    .child("Tab / Shift+Tab to focus. Arrow keys, Home, End, and Page Up / Down to adjust."),
            )
            .child(
                div()
                    .flex()
                    .gap_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_8()
                            .child(
                                slider("volume", "Volume", &[self.volume], Axis::Horizontal)
                                    .on_change(cx.listener(
                                        |this, change: &BaseSliderChange, _window, cx| {
                                            this.volume = change.value.as_slice()[0];
                                            cx.notify();
                                        },
                                    ))
                                    .on_commit(cx.listener(
                                        |this, change: &BaseSliderChange, _window, cx| {
                                            this.last_committed = format!(
                                                "Volume {:.0}%",
                                                change.value.as_slice()[0]
                                            )
                                            .into();
                                            cx.notify();
                                        },
                                    )),
                            )
                            .child(
                                slider("range", "Selected range", &self.range, Axis::Horizontal)
                                    .step(5.)
                                    .min_steps_between_values(2)
                                    .thumb_collision_behavior(BaseSliderThumbCollisionBehavior::Clamp)
                                    .on_change(cx.listener(
                                        |this, change: &BaseSliderChange, _window, cx| {
                                            this.range.copy_from_slice(change.value.as_slice());
                                            cx.notify();
                                        },
                                    ))
                                    .on_commit(cx.listener(
                                        |this, change: &BaseSliderChange, _window, cx| {
                                            let values = change.value.as_slice();
                                            this.last_committed = format!(
                                                "Range {:.0}% to {:.0}%",
                                                values[0], values[1]
                                            )
                                            .into();
                                            cx.notify();
                                        },
                                    )),
                            ),
                    )
                    .child(
                        slider("brightness", "Brightness", &[self.brightness], Axis::Vertical)
                            .on_change(cx.listener(
                                |this, change: &BaseSliderChange, _window, cx| {
                                    this.brightness = change.value.as_slice()[0];
                                    cx.notify();
                                },
                            ))
                            .on_commit(cx.listener(
                                |this, change: &BaseSliderChange, _window, cx| {
                                    this.last_committed =
                                        format!("Brightness {:.0}%", change.value.as_slice()[0]).into();
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .text_color(rgb(0xa1a1aa))
                    .child(format!("Last committed: {}", self.last_committed)),
            )
    }
}

fn slider(
    element_id: &'static str,
    label: &'static str,
    values: &[f32],
    orientation: Axis,
) -> BaseSlider {
    let thumb_size = px(20.);
    let track_thickness = px(6.);
    let track_inset = (thumb_size - track_thickness) / 2.;

    // Match the fill edges to the centers of the edge-aligned thumbs.
    let start_margin = if values.len() == 1 {
        px(0.)
    } else {
        thumb_size * (0.5 - values[0] / 100.)
    };
    let end_margin = thumb_size * (values[values.len() - 1] / 100. - 0.5);

    let start_radius = if values.len() == 1 {
        track_thickness / 2.
    } else {
        px(0.)
    };

    let indicator = BaseSliderIndicator::new(format!("{element_id}-indicator")).bg(rgb(0x3b82f6));
    let indicator = match orientation {
        Axis::Horizontal => indicator
            .ml(start_margin)
            .mr(end_margin)
            .rounded_l(start_radius),
        Axis::Vertical => indicator
            .mb(start_margin)
            .mt(end_margin)
            .rounded_b(start_radius),
    };
    let track = BaseSliderTrack::new(format!("{element_id}-track"))
        .size_full()
        .rounded_full()
        .bg(rgb(0x3f3f46))
        .indicator(indicator);
    let mut track = match orientation {
        Axis::Horizontal => track.h(track_thickness).top(track_inset),
        Axis::Vertical => track.w(track_thickness).left(track_inset),
    };

    for thumb_idx in 0..values.len() {
        let thumb_label = if values.len() == 1 {
            label.to_string()
        } else if thumb_idx == 0 {
            format!("{label}, lower limit")
        } else {
            format!("{label}, upper limit")
        };

        let thumb = BaseSliderThumb::new(format!("{element_id}-thumb-{thumb_idx}"))
            .aria_label(thumb_label)
            .size(thumb_size)
            .rounded_full()
            .bg(rgb(0x3b82f6))
            .p(px(2.))
            .child(div().size_full().rounded_full().bg(rgb(0xfafafa)))
            .cursor_pointer()
            .focus_visible_style(|style| style.bg(rgb(0xfbbf24)));
        let thumb = match orientation {
            Axis::Horizontal => thumb.mt(-track_inset),
            Axis::Vertical => thumb.ml(-track_inset),
        };

        track = track.thumb(thumb);
    }

    let control = BaseSliderControl::new(format!("{element_id}-control"))
        .cursor_pointer()
        .track(track);
    let control = match orientation {
        Axis::Horizontal => control.w_full().h(thumb_size),
        Axis::Vertical => control.w(thumb_size).h(px(200.)),
    };

    BaseSlider::new(element_id)
        .values(values.iter().copied())
        .orientation(orientation)
        .min(0.)
        .max(100.)
        .step(1.)
        .large_step(10.)
        .value_text_formatter(|value| format!("{value:.0}%"))
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .when(orientation == Axis::Vertical, |this| {
            this.w(px(120.)).flex_none().items_center()
        })
        .disabled_style(|style| style.opacity(0.4))
        .label(BaseSliderLabel::new(format!("{element_id}-label")).text(label))
        .child(
            div().text_color(rgb(0xa1a1aa)).child(
                values
                    .iter()
                    .map(|value| format!("{value:.0}%"))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        )
        .control(control)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", FocusNext, None),
            KeyBinding::new("shift-tab", FocusPrevious, None),
        ]);

        let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);

        cx.open_window(
            WindowOptions::new().window_bounds(Some(WindowBounds::Windowed(bounds))),
            |window, cx| {
                cx.new(|cx| {
                    let focus_handle = cx.focus_handle();
                    focus_handle.focus(window, cx);

                    SliderExample {
                        focus_handle,
                        volume: 35.,
                        range: [25., 75.],
                        brightness: 60.,
                        last_committed: "No changes yet".into(),
                    }
                })
            },
        )
        .expect("Failed to open the slider example window");
        cx.activate(true);
    });
}
