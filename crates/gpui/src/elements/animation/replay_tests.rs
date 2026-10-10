use super::*;
use crate::{
    Context, FocusHandle, InteractiveElement, Lerp, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, PlatformInput, Render, TestAppContext, WindowHandle, canvas, div, millis, point,
    prelude::*, px, rgb, size,
};
use std::cell::RefCell;

struct ReplayView {
    event: usize,
    replay_enabled: bool,
    show: bool,
    animations: Vec<Animation>,
    samples: Rc<RefCell<Vec<(usize, f32)>>>,
    renders: Rc<Cell<usize>>,
    mounts: Rc<Cell<usize>>,
    clicks: usize,
    focus: FocusHandle,
}

impl Render for ReplayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let samples = self.samples.clone();
        let mounts = self.mounts.clone();
        div().size_full().when(self.show, |root| {
            let row = div()
                .id("notification-row")
                .size_full()
                .child("Identical notification payload")
                .child(
                    div()
                        .id("action")
                        .absolute()
                        .top(px(30.0))
                        .size(px(40.0))
                        .track_focus(&self.focus)
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.clicks += 1;
                            cx.notify();
                        })),
                )
                .child(
                    canvas(
                        move |_, window, _| {
                            window.with_global_id("retained-child".into(), |id, window| {
                                window.with_element_state(id, |state: Option<()>, _| {
                                    if state.is_none() {
                                        mounts.set(mounts.get() + 1);
                                    }
                                    ((), ())
                                });
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .size(px(1.0)),
                );
            let animator = move |row: crate::Stateful<crate::Div>, index, emphasis| {
                samples.borrow_mut().push((index, emphasis));
                row.bg(rgb(0x202020).lerp(&rgb(0x4080c0), emphasis))
            };
            let animation = if self.animations.len() == 1 {
                row.with_animation(
                    "stable-animation",
                    self.animations[0].clone(),
                    move |row, value| animator(row, 0, value),
                )
            } else {
                row.with_animations("stable-animation", self.animations.clone(), animator)
            };
            root.child(animation.when(self.replay_enabled, |animation| {
                animation.replay_on(self.event)
            }))
        })
    }
}

fn open(cx: &mut TestAppContext, animations: Vec<Animation>) -> WindowHandle<ReplayView> {
    let window = cx.open_window(size(px(120.0), px(120.0)), move |_, cx| ReplayView {
        event: 1,
        replay_enabled: true,
        show: true,
        animations,
        samples: Rc::default(),
        renders: Rc::new(Cell::new(0)),
        mounts: Rc::new(Cell::new(0)),
        clicks: 0,
        focus: cx.focus_handle(),
    });
    cx.run_until_parked();
    draw(&window, cx);
    window
}

fn draw(window: &WindowHandle<ReplayView>, cx: &mut TestAppContext) {
    cx.update_window((*window).into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn advance(window: &WindowHandle<ReplayView>, cx: &mut TestAppContext, elapsed: Duration) {
    cx.executor().advance_clock(elapsed);
    window
        .update(cx, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    draw(window, cx);
}

fn sample(window: &WindowHandle<ReplayView>, cx: &mut TestAppContext) -> (usize, f32) {
    window
        .update(cx, |v, _, _| *v.samples.borrow().last().unwrap())
        .unwrap()
}

fn replay(window: &WindowHandle<ReplayView>, cx: &mut TestAppContext, event: usize) {
    window
        .update(cx, |v, _, cx| {
            v.event = event;
            cx.notify();
        })
        .unwrap();
    draw(window, cx);
}

#[gpui::test]
fn events_replay_identical_payload_without_remounting_controls(cx: &mut TestAppContext) {
    let window = open(
        cx,
        vec![Animation::new(
            Motion::new(millis(100)).iterations(2).alternate(),
        )],
    );
    assert_eq!(sample(&window, cx), (0, 0.0));
    window.update(cx, |v, w, cx| w.focus(&v.focus, cx)).unwrap();
    advance(&window, cx, millis(50));
    assert_eq!(sample(&window, cx), (0, 0.5));
    replay(&window, cx, 1);
    assert_eq!(sample(&window, cx), (0, 0.5), "redraws keep phase");
    replay(&window, cx, 2);
    assert_eq!(sample(&window, cx), (0, 0.0), "active replay restarts");
    advance(&window, cx, millis(100));
    assert_eq!(sample(&window, cx), (0, 1.0));
    window
        .update(cx, |v, w, _| {
            assert!(v.focus.is_focused(w));
            assert_eq!(v.mounts.get(), 1, "child retained state survives replay");
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: point(px(20.0), px(50.0)),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        w.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                position: point(px(20.0), px(50.0)),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    assert_eq!(window.update(cx, |v, _, _| v.clicks).unwrap(), 1);
    advance(&window, cx, millis(100));
    assert_eq!(sample(&window, cx), (0, 0.0));
    assert_eq!(
        window
            .update(cx, |_, w, cx| w.simulate_next_frame(cx))
            .unwrap(),
        0
    );
    replay(&window, cx, 3);
    advance(&window, cx, millis(50));
    assert_eq!(sample(&window, cx), (0, 0.5), "completed replay runs again");
}

#[gpui::test]
fn replay_uses_delay_reverse_and_actual_terminal_sample(cx: &mut TestAppContext) {
    let window = open(
        cx,
        vec![Animation::new(
            Motion::new(millis(100))
                .with_delay(millis(50))
                .direction(crate::Direction::Reverse),
        )],
    );
    advance(&window, cx, millis(100));
    assert_eq!(sample(&window, cx), (0, 0.5));
    replay(&window, cx, 2);
    assert_eq!(sample(&window, cx), (0, 1.0));
    advance(&window, cx, millis(50));
    assert_eq!(sample(&window, cx), (0, 1.0));
    advance(&window, cx, millis(100));
    assert_eq!(sample(&window, cx), (0, 0.0));
    window
        .update(cx, |v, _, _| {
            v.animations = vec![Animation::new(millis(100)).with_easing(|p| 0.2 + p * 0.6)];
        })
        .unwrap();
    replay(&window, cx, 3);
    advance(&window, cx, millis(100));
    assert!((sample(&window, cx).1 - 0.8).abs() < 1e-6);
    draw(&window, cx);
    assert!((sample(&window, cx).1 - 0.8).abs() < 1e-6);
}

#[gpui::test]
fn replay_restarts_the_whole_chain_after_sparse_handoffs(cx: &mut TestAppContext) {
    let window = open(
        cx,
        vec![
            Animation::new(
                Motion::new(millis(100))
                    .with_delay(millis(50))
                    .iterations(2),
            ),
            Animation::new(Motion::new(millis(100)).direction(crate::Direction::Reverse))
                .with_easing(|p| 0.2 + p * 0.6),
            Animation::new(millis(400)),
        ],
    );
    advance(&window, cx, millis(450));
    assert_eq!(sample(&window, cx), (2, 0.25));
    replay(&window, cx, 2);
    assert_eq!(sample(&window, cx), (0, 0.0));
    advance(&window, cx, millis(250));
    let (index, progress) = sample(&window, cx);
    assert_eq!(index, 1, "exact incoming handoff");
    assert!(
        (progress - 0.8).abs() < 1e-6,
        "reverse/custom origin at local zero"
    );
    advance(&window, cx, millis(500));
    assert_eq!(sample(&window, cx), (2, 1.0));
}

#[gpui::test]
fn reduced_replay_is_consumed_until_another_event(cx: &mut TestAppContext) {
    let window = open(
        cx,
        vec![Animation::new(
            Motion::new(millis(100)).iterations(2).alternate(),
        )],
    );
    advance(&window, cx, millis(40));
    cx.update(|cx| cx.set_reduce_motion(true));
    draw(&window, cx);
    assert_eq!(sample(&window, cx), (0, 0.0));
    // A preference change cannot withdraw a frame already queued by the old run.
    window
        .update(cx, |_, w, cx| w.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
    replay(&window, cx, 2);
    cx.update(|cx| cx.set_reduce_motion(false));
    draw(&window, cx);
    assert_eq!(sample(&window, cx), (0, 0.0));
    assert_eq!(
        window
            .update(cx, |_, w, cx| w.simulate_next_frame(cx))
            .unwrap(),
        0
    );
    replay(&window, cx, 3);
    advance(&window, cx, millis(50));
    assert_eq!(sample(&window, cx), (0, 0.5));
}

#[gpui::test]
fn reduced_motion_toggle_between_layouts_consumes_replay(cx: &mut TestAppContext) {
    for max_fps in [None, Some(10.0)] {
        let mut animation = Animation::new(Motion::new(millis(1000)).iterations(2).alternate());
        animation.max_fps = max_fps;
        let window = open(cx, vec![animation]);
        advance(&window, cx, millis(40));
        assert!((sample(&window, cx).1 - 0.04).abs() < 1e-6);

        cx.update(|cx| {
            cx.set_reduce_motion(true);
            cx.set_reduce_motion(false);
        });
        draw(&window, cx);
        assert_eq!(sample(&window, cx), (0, 0.0), "the run is consumed");
        // A frame queued before the preference change may drain once.
        window
            .update(cx, |_, w, cx| w.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        draw(&window, cx);
        let renders = window.update(cx, |v, _, _| v.renders.get()).unwrap();
        cx.executor().advance_clock(millis(2000));
        cx.run_until_parked();
        assert_eq!(
            window.update(cx, |v, _, _| v.renders.get()).unwrap(),
            renders,
            "the consumed run's throttle timer is cancelled"
        );
        assert_eq!(
            window
                .update(cx, |_, w, cx| w.simulate_next_frame(cx))
                .unwrap(),
            0
        );
        replay(&window, cx, 2);
        advance(&window, cx, millis(50));
        assert!((sample(&window, cx).1 - 0.05).abs() < 1e-6);
    }
}

#[gpui::test]
fn replay_preference_history_respects_mount_and_key_boundaries(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_reduce_motion(false);
    });
    let window = open(
        cx,
        vec![Animation::new(
            Motion::new(millis(1000)).iterations(2).alternate(),
        )],
    );
    advance(&window, cx, millis(40));
    assert!(
        (sample(&window, cx).1 - 0.04).abs() < 1e-6,
        "mount ignores prior enables"
    );

    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_reduce_motion(false);
    });
    replay(&window, cx, 2);
    advance(&window, cx, millis(50));
    assert!(
        (sample(&window, cx).1 - 0.05).abs() < 1e-6,
        "a new key supersedes prior enables"
    );

    window
        .update(cx, |v, _, _| v.replay_enabled = false)
        .unwrap();
    draw(&window, cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_reduce_motion(false);
    });
    advance(&window, cx, millis(50));
    assert!(
        (sample(&window, cx).1 - 0.1).abs() < 1e-6,
        "unkeyed animation keeps legacy sampling"
    );

    window
        .update(cx, |v, _, _| v.replay_enabled = true)
        .unwrap();
    draw(&window, cx);
    advance(&window, cx, millis(50));
    assert!((sample(&window, cx).1 - 0.05).abs() < 1e-6);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_reduce_motion(false);
    });
    draw(&window, cx);
    assert_eq!(
        sample(&window, cx),
        (0, 0.0),
        "the observed key consumes subsequent enables"
    );
}

#[gpui::test]
fn replay_cancels_previous_throttle_and_disposal_stops_work(cx: &mut TestAppContext) {
    let window = open(cx, vec![Animation::new(millis(1000)).with_max_fps(10.0)]);
    cx.executor().advance_clock(millis(40));
    replay(&window, cx, 2);
    let renders = window.update(cx, |v, _, _| v.renders.get()).unwrap();
    cx.executor().advance_clock(millis(65));
    cx.run_until_parked();
    assert_eq!(
        window.update(cx, |v, _, _| v.renders.get()).unwrap(),
        renders,
        "the previous run's 100ms timer was cancelled"
    );
    cx.executor().advance_clock(millis(40));
    cx.run_until_parked();
    assert!(sample(&window, cx).1 > 0.09);
    for event in 3..23 {
        replay(&window, cx, event);
    }
    window.update(cx, |v, _, _| v.show = false).unwrap();
    draw(&window, cx);
    draw(&window, cx);
    let renders = window.update(cx, |v, _, _| v.renders.get()).unwrap();
    cx.executor().advance_clock(millis(2000));
    cx.run_until_parked();
    assert_eq!(
        window.update(cx, |v, _, _| v.renders.get()).unwrap(),
        renders
    );
}

#[gpui::test]
fn changing_throttle_replaces_pending_timer_without_replaying(cx: &mut TestAppContext) {
    let window = open(cx, vec![Animation::new(millis(1000)).with_max_fps(0.5)]);
    cx.executor().advance_clock(millis(40));
    window
        .update(cx, |view, _, _| view.animations[0].max_fps = Some(20.0))
        .unwrap();
    draw(&window, cx);
    assert!((sample(&window, cx).1 - 0.04).abs() < 1e-6);
    let renders = window.update(cx, |view, _, _| view.renders.get()).unwrap();
    cx.executor().advance_clock(millis(55));
    cx.run_until_parked();
    assert_eq!(
        window.update(cx, |view, _, _| view.renders.get()).unwrap(),
        renders + 1,
        "the new 50ms throttle must replace the old 2s timer"
    );
    assert!((sample(&window, cx).1 - 0.09).abs() < 1e-6);

    window
        .update(cx, |view, _, _| view.animations[0].max_fps = None)
        .unwrap();
    draw(&window, cx);
    assert_eq!(
        window
            .update(cx, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap(),
        1,
        "removing the cap returns to ordinary animation frames"
    );
    draw(&window, cx);
    let renders = window.update(cx, |view, _, _| view.renders.get()).unwrap();
    cx.executor().advance_clock(millis(60));
    cx.run_until_parked();
    assert_eq!(
        window.update(cx, |view, _, _| view.renders.get()).unwrap(),
        renders,
        "the superseded 50ms timer must be cancelled"
    );
}

#[gpui::test]
fn replay_does_not_reset_the_synchronized_app_epoch(cx: &mut TestAppContext) {
    cx.executor().advance_clock(millis(300));
    let window = open(cx, vec![Animation::new(millis(1000)).repeat_synced()]);
    assert!((sample(&window, cx).1 - 0.3).abs() < 1e-6);
    advance(&window, cx, millis(100));
    replay(&window, cx, 2);
    assert!((sample(&window, cx).1 - 0.4).abs() < 1e-6);
}

#[gpui::test]
fn empty_and_zero_duration_replays_do_not_schedule_work(cx: &mut TestAppContext) {
    for animations in [vec![], vec![Animation::new(Duration::ZERO); 2]] {
        let window = open(cx, animations);
        replay(&window, cx, 2);
        assert_eq!(
            window
                .update(cx, |_, w, cx| w.simulate_next_frame(cx))
                .unwrap(),
            0
        );
        window
            .update(cx, |v, _, _| {
                if v.animations.is_empty() {
                    assert!(v.samples.borrow().is_empty());
                } else {
                    assert_eq!(*v.samples.borrow().last().unwrap(), (1, 1.0));
                }
            })
            .unwrap();
    }
}

#[gpui::test]
fn removing_replay_control_returns_to_legacy_sampling(cx: &mut TestAppContext) {
    let window = open(cx, vec![Animation::new(millis(100))]);
    advance(&window, cx, millis(40));
    cx.update(|cx| cx.set_reduce_motion(true));
    draw(&window, cx);
    assert_eq!(sample(&window, cx), (0, 1.0));
    cx.update(|cx| cx.set_reduce_motion(false));
    draw(&window, cx);
    assert_eq!(sample(&window, cx), (0, 1.0), "keyed run is consumed");
    window
        .update(cx, |v, _, _| v.replay_enabled = false)
        .unwrap();
    draw(&window, cx);
    assert!((sample(&window, cx).1 - 0.4).abs() < 1e-6);
}
