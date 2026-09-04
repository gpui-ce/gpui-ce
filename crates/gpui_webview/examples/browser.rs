use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Bounds, ClickEvent, Context, Entity, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_elements::editable_text::{
    EditableTextState, StringStorage,
    actions::{DEFAULT_INPUT_CONTEXT, Enter, default_bindings},
    text_input,
};
use gpui_webview::{WebViewHandle, webview};

const HOME_URL: &str = "https://example.com";

struct Browser {
    url_state: Option<Entity<EditableTextState>>,
    webview_handle: Rc<RefCell<Option<WebViewHandle>>>,
}

impl Browser {
    fn new() -> Self {
        Self {
            url_state: None,
            webview_handle: Rc::new(RefCell::new(None)),
        }
    }

    /// Navigate the webview to whatever is currently in the URL bar.
    fn go(&self, cx: &mut App) {
        let Some(state) = &self.url_state else {
            return;
        };
        let text = state.read(cx).as_str().trim().to_string();
        if text.is_empty() {
            return;
        }
        let has_scheme = text.split_once("://").is_some_and(|(scheme, _)| {
            scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        });
        let url = if has_scheme {
            text
        } else {
            format!("https://{text}")
        };
        if let Some(handle) = self.webview_handle.borrow().as_ref() {
            handle.load_url(&url);
        }
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Find or lazily create the state entity backing the URL bar.
        let url_state = EditableTextState::use_keyed_init("url-bar", window, cx, |_window, _cx| {
            StringStorage::from(HOME_URL)
        });
        self.url_state = Some(url_state.clone());
        if std::env::var_os("GPUI_TRACE").is_some() {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default();
            eprintln!(
                "TRACE {ms}ms render url_state: {:?}",
                self.url_state.as_ref().unwrap().read(cx).as_str()
            );
        }

        // Keep the webview handle in a shared cell so the `on_create_handle`
        // callback can hand it to us once the native webview exists.
        let handle_cell = self.webview_handle.clone();

        // Weak reference to the URL bar state so the webview can push URL
        // changes back into the text box once it detects internal navigation.
        let url_state_for_webview = url_state.downgrade();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .child(
                // URL bar row: textbox + go button
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h(px(48.0))
                    .px_3()
                    .bg(rgb(0x313244))
                    .child(
                        text_input("url-bar")
                            .state(url_state.downgrade())
                            .caret_blink_interval_500ms()
                            .placeholder("Enter a URL")
                            .flex_1()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x45475a))
                            .text_color(rgb(0xcdd6f4))
                            .whitespace_nowrap()
                            .overflow_x_scroll()
                            .on_action(cx.listener(|this, _: &Enter, _window, cx| {
                                this.go(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("go")
                            .px_4()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0xa6e3a1))
                            .text_color(rgb(0x1e1e2e))
                            .font_weight(gpui::FontWeight::BOLD)
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x94e2d5)))
                            .child("Go")
                            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                this.go(cx);
                            })),
                    ),
            )
            .child(
                // Web content (native webview)
                webview("browser-content")
                    .url(HOME_URL)
                    .on_url_changed(move |url, _window, cx| {
                        let Some(state) = url_state_for_webview.upgrade() else {
                            return;
                        };
                        state.update(cx, |state, cx| {
                            if state.as_str() != url {
                                state.emplace(url, cx);
                            }
                        });
                    })
                    .on_create_handle(move |handle, _window, _cx| {
                        *handle_cell.borrow_mut() = Some(handle);
                    })
                    .flex_1(),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Browser::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}
