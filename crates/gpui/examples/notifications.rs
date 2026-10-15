//! Posts, replaces and dismisses system notifications, and reacts to their responses.
//!
//! Posting again with the same tag replaces the notification. Clicking the body or
//! an action button sends a response to `on_system_notification_response`. macOS only
//! delivers notifications when the app runs from an app bundle.
//!
//! Runs on macOS, Linux and Windows.
//!
//! Run it with `cargo run --example notifications`.

use gpui::{
    App, Bounds, Context, Div, QuitMode, SharedString, Stateful, SystemNotification,
    SystemNotificationAction, SystemNotificationResponse, TitlebarOptions, Window, WindowBounds,
    WindowOptions, colors::Colors, div, prelude::*, px, size,
};

const NOTIFICATION_TAG: &str = "gpui-system-notification-example";

struct Notifications {
    revision: usize,
    status: SharedString,
}

impl Notifications {
    fn show(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        cx.show_system_notification(SystemNotification {
            tag: NOTIFICATION_TAG.into(),
            title: format!("Example notification {revision}").into(),
            body: "This notification was posted by the GPUI example.".into(),
            actions: vec![
                SystemNotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                },
                SystemNotificationAction {
                    id: "snooze".into(),
                    label: "Snooze".into(),
                },
            ],
        });
        self.status = format!("Posted notification revision {revision}").into();
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.dismiss_system_notification(NOTIFICATION_TAG);
        self.status = "Dismissed the notification".into();
        cx.notify();
    }
}

impl Render for Notifications {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xl().font_weight(gpui::FontWeight::SEMIBOLD).child("System notifications"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child("Post again to replace the notification. Click it, or one of its buttons, to respond."),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button(&colors, "show", "Show or replace")
                            .on_click(cx.listener(|this, _, _, cx| this.show(cx))),
                    )
                    .child(
                        button(&colors, "dismiss", "Dismiss")
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                    ),
            )
            .child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(colors.container)
                    .border_1()
                    .border_color(colors.border)
                    .text_sm()
                    .child(self.status.clone()),
            )
    }
}

fn button(colors: &Colors, id: &'static str, label: &'static str) -> Stateful<Div> {
    let hover = colors.border;
    div()
        .id(id)
        .px_3()
        .py_1p5()
        .rounded_md()
        .text_sm()
        .text_color(colors.selected_text)
        .bg(colors.selected)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label)
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    cx.set_app_identity("dev.zed.gpui.notifications", "GPUI Notifications");

    let view = cx.new(|_| Notifications {
        revision: 0,
        status: "No notification posted yet".into(),
    });
    cx.on_system_notification_response({
        let view = view.clone();
        move |SystemNotificationResponse { tag, action_id }, cx| {
            view.update(cx, |this, cx| {
                this.status = match action_id {
                    Some(action_id) => format!("Received action '{action_id}' for tag '{tag}'"),
                    None => format!("Notification body clicked for tag '{tag}'"),
                }
                .into();
                cx.notify();
            });
        }
    });

    let bounds = Bounds::centered(None, size(px(480.), px(260.)), cx);
    cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(TitlebarOptions {
                title: Some("System Notifications".into()),
                ..Default::default()
            })),
        move |_, _| view,
    )
    .unwrap();
    cx.activate(true);
}
