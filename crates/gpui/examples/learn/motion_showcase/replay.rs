//! A notification pulse replayed by successive application events.
use super::{ACCENT, BORDER, MUTED, MotionShowcase, SURFACE, TEXT, button, panel};
use gpui::{prelude::FluentBuilder, *};

pub(super) struct NotificationDemo {
    event: usize,
    actions: usize,
    receive_focus: FocusHandle,
    reduced_focus: FocusHandle,
    action_focus: FocusHandle,
}

impl NotificationDemo {
    pub(super) fn new(cx: &mut Context<MotionShowcase>) -> Self {
        Self {
            event: 0,
            actions: 0,
            receive_focus: cx.focus_handle().tab_stop(true),
            reduced_focus: cx.focus_handle().tab_stop(true),
            action_focus: cx.focus_handle().tab_stop(true),
        }
    }

    pub(super) fn panel(&self, cx: &mut Context<MotionShowcase>) -> impl IntoElement + use<> {
        let reduced = cx.reduce_motion();
        let action = focusable_button(
            "notification-action",
            "Open activity",
            false,
            &self.action_focus,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.event_notification.activate(cx);
        }));
        let notification = div()
            .id("event-notification-row")
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .p_3()
            .rounded_md()
            .border_1()
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.0))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Build succeeded"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child("Your build is ready. Open activity for details."),
                    ),
            )
            .child(action)
            .with_animation(
                "event-notification-animation",
                Animation::new(
                    Motion::new(millis(240))
                        .with_reverse_pass(MotionPass::new(millis(420)))
                        .iterations(2)
                        .alternate()
                        .with_easing(ease_in_out),
                ),
                |row, emphasis| {
                    row.bg(rgb(SURFACE).lerp(&rgb(0x365587), emphasis))
                        .border_color(rgb(BORDER).lerp(&rgb(ACCENT), emphasis))
                },
            )
            .replay_on(self.event);
        panel(
            "Event-triggered notification",
            ".with_animation(stable_id, Animation::new(motion), animator).replay_on(event_id)",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().text_sm().text_color(rgb(MUTED)).child(
                    "Receive the same message again, even during a pulse. Its action stays usable.",
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            focusable_button(
                                "notification-event",
                                "Receive notification",
                                true,
                                &self.receive_focus,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(next) = this.event_notification.event.checked_add(1) {
                                    this.event_notification.event = next;
                                    cx.notify();
                                }
                            })),
                        )
                        .child(
                            focusable_button(
                                "notification-reduced-motion",
                                if reduced {
                                    "Reduced motion: on"
                                } else {
                                    "Reduced motion: off"
                                },
                                reduced,
                                &self.reduced_focus,
                            )
                            .on_click(cx.listener(|_, _, _, cx| {
                                let reduced = !cx.reduce_motion();
                                cx.set_reduce_motion(reduced);
                            })),
                        ),
                )
                .when(self.event == 0, |content| {
                    content.child("Send an event to receive a notification.")
                })
                .when(self.event != 0, |content| content.child(notification))
                .when(self.actions != 0, |content| {
                    content.child("Activity: the latest build succeeded with no errors.")
                })
                .child(div().text_sm().text_color(rgb(MUTED)).child(format!(
                    "Notifications received: {} · Activity opened: {}",
                    self.event, self.actions
                ))),
        )
    }

    fn activate(&mut self, cx: &mut Context<MotionShowcase>) {
        if let Some(next) = self.actions.checked_add(1) {
            self.actions = next;
            cx.notify();
        }
    }
}

fn focusable_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    focus: &FocusHandle,
) -> Stateful<Div> {
    button(id, label, primary)
        .track_focus(focus)
        .tab_index(0)
        .role(Role::Button)
        .focus(|style| style.border_color(rgb(TEXT)).shadow_sm())
}

pub(super) fn navigate_focus(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let modifiers = event.keystroke.modifiers;
    if event.keystroke.key == "tab"
        && !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
        && !modifiers.function
    {
        if modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        cx.stop_propagation();
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::MotionShowcase;
    use gpui::{
        AppContext, KeyDownEvent, KeyUpEvent, Keystroke, PlatformInput, TestAppContext,
        WindowHandle, px, size,
    };

    fn press(window: WindowHandle<MotionShowcase>, cx: &mut TestAppContext, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::KeyDown(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }

    #[gpui::test]
    fn notification_controls_navigate_and_remain_usable_with_reduced_motion(
        cx: &mut TestAppContext,
    ) {
        let window = cx.open_window(size(px(640.0), px(700.0)), |_, cx| MotionShowcase::new(cx));
        window
            .update(cx, |view, window, cx| {
                window.focus(&view.event_notification.receive_focus, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();

        press(window, cx, "enter");
        assert_eq!(
            window
                .update(cx, |view, _, _| view.event_notification.event)
                .unwrap(),
            1
        );
        press(window, cx, "tab");
        window
            .update(cx, |view, window, _| {
                assert!(view.event_notification.reduced_focus.is_focused(window));
            })
            .unwrap();
        press(window, cx, "space");
        assert!(cx.update(|cx| cx.reduce_motion()));
        press(window, cx, "tab");
        window
            .update(cx, |view, window, _| {
                assert!(view.event_notification.action_focus.is_focused(window));
            })
            .unwrap();
        press(window, cx, "enter");
        assert_eq!(
            window
                .update(cx, |view, _, _| view.event_notification.actions)
                .unwrap(),
            1
        );
        press(window, cx, "ctrl-tab");
        window
            .update(cx, |view, window, _| {
                assert!(view.event_notification.action_focus.is_focused(window));
            })
            .unwrap();
        press(window, cx, "shift-tab");
        press(window, cx, "space");
        assert!(!cx.update(|cx| cx.reduce_motion()));
        press(window, cx, "shift-tab");
        press(window, cx, "enter");
        window
            .update(cx, |view, window, _| {
                assert_eq!(view.event_notification.event, 2);
                assert_eq!(view.event_notification.actions, 1);
                assert!(view.event_notification.receive_focus.is_focused(window));
            })
            .unwrap();
    }

    #[gpui::test]
    fn notification_action_activates_once_per_keyboard_press(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(960.0), px(820.0)), |_, cx| MotionShowcase::new(cx));
        window
            .update(cx, |view, window, cx| {
                view.event_notification.event = 1;
                window.focus(&view.event_notification.action_focus, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
        .unwrap();

        for (key, expected_actions) in [("enter", 1), ("space", 2), ("ctrl-enter", 2)] {
            press(window, cx, key);
            assert_eq!(
                window
                    .update(cx, |view, _, _| view.event_notification.actions)
                    .unwrap(),
                expected_actions,
                "keyboard activation for {key}"
            );
        }
    }
}
