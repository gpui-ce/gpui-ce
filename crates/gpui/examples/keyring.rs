//! Stores, reads and deletes a username and password in the platform credential store.
//!
//! `write_credentials`, `read_credentials` and `delete_credentials` talk to Keychain on
//! macOS, Credential Manager on Windows and the Secret Service on Linux. On Linux, items
//! are grouped under the keyring label set with `set_keyring_label`.
//!
//! Runs on macOS, Linux, FreeBSD and Windows.
//!
//! Run it with `cargo run --example keyring`.

use gpui::{
    App, Bounds, Context, Div, QuitMode, SharedString, Stateful, Window, WindowBounds,
    WindowOptions, colors::Colors, div, prelude::*, px, size,
};

const CREDENTIAL_URL: &str = "https://example.com/keyring-demo";

struct Keyring {
    status: SharedString,
}

impl Keyring {
    fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.set_status("Saving...", cx);
        cx.spawn(async move |this, cx| {
            let task =
                cx.update(|cx| cx.write_credentials(CREDENTIAL_URL, "ada@example.com", b"hunter2"));
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.set_status("Saved credentials for ada@example.com.", cx),
                Err(err) => this.set_status(format!("Failed to save: {err}"), cx),
            })
        })
        .detach();
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.set_status("Loading...", cx);
        cx.spawn(async move |this, cx| {
            let task = cx.update(|cx| cx.read_credentials(CREDENTIAL_URL));
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(Some((username, password))) => this.set_status(
                    format!("Loaded {username} (password is {} bytes).", password.len()),
                    cx,
                ),
                Ok(None) => this.set_status("No credentials stored yet.", cx),
                Err(err) => this.set_status(format!("Failed to load: {err}"), cx),
            })
        })
        .detach();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        self.set_status("Deleting...", cx);
        cx.spawn(async move |this, cx| {
            let task = cx.update(|cx| cx.delete_credentials(CREDENTIAL_URL));
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.set_status("Deleted stored credentials.", cx),
                Err(err) => this.set_status(format!("Failed to delete: {err}"), cx),
            })
        })
        .detach();
    }
}

impl Render for Keyring {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Keyring"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child(format!("Stored under {CREDENTIAL_URL}")),
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
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button(&colors, "save", "Save")
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        button(&colors, "load", "Load")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                    .child(
                        button(&colors, "delete", "Delete")
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    ),
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
    cx.set_keyring_label("gpui-ce-keyring-example");

    let bounds = Bounds::centered(None, size(px(480.), px(260.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| {
            cx.new(|_| Keyring {
                status: "Use the buttons to store, load and delete credentials.".into(),
            })
        },
    )
    .unwrap();
    cx.activate(true);
}
