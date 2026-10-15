//! Opens, positions and closes windows, and lists the ones that are open.
//!
//! Each button opens a window with different `WindowOptions`: a small fixed-size window, a window
//! at a specific position, and a window with a minimum size. The list comes from `cx.windows()`.
//! Each window closes itself with `Window::remove_window`, and the list closes the others through
//! their handles.
//!
//! Runs on macOS, Linux and Windows.
//!
//! Run it with `cargo run --example windows`.

use gpui::{
    App, Bounds, Context, Div, ElementId, QuitMode, Stateful, TitlebarOptions, Window,
    WindowBounds, WindowOptions, colors::Colors, div, point, prelude::*, px, size,
};

/// A launcher entry: the window's title, its distinguishing behavior, and its options.
const KINDS: [(&str, &str, fn(&App) -> WindowOptions); 3] = [
    (
        "Small, fixed size",
        "Not resizable and not minimizable.",
        |cx| {
            WindowOptions::default()
                .window_bounds(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(260.), px(160.)),
                    cx,
                )))
                .is_resizable(false)
                .is_minimizable(false)
        },
    ),
    (
        "At a position",
        "Opened at a fixed origin, in screen coordinates.",
        |_cx| {
            let bounds = Bounds::new(point(px(200.), px(160.)), size(px(360.), px(200.)));
            WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds))
        },
    ),
    (
        "With a minimum size",
        "Resize me, but not below 320 by 200 pixels.",
        |cx| {
            WindowOptions::default()
                .window_bounds(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(480.), px(320.)),
                    cx,
                )))
                .window_min_size(size(px(320.), px(200.)))
        },
    ),
];

struct Launcher;

impl Launcher {
    fn new(cx: &mut Context<Self>) -> Self {
        // Re-render the list when any window closes, including ones closed from their own button.
        let launcher = cx.entity().downgrade();
        cx.on_window_closed(move |cx, _| {
            launcher.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        Launcher
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let this_window = window.window_handle();

        let open_buttons =
            KINDS
                .into_iter()
                .enumerate()
                .map(|(index, (title, detail, options))| {
                    button(&colors, ("open", index), title).on_click(move |_event, _window, cx| {
                        let titlebar = TitlebarOptions {
                            title: Some(title.into()),
                            ..Default::default()
                        };
                        cx.open_window(options(cx).titlebar(Some(titlebar)), |_window, cx| {
                            cx.new(|_| Panel(detail))
                        })
                        .unwrap();
                    })
                });

        let rows = cx.windows().into_iter().map(|handle| {
            let id = handle.window_id().as_u64() as usize;
            let row = div()
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .rounded_lg()
                .bg(colors.container)
                .text_sm();
            if handle == this_window {
                row.child(format!("Window {id} (this window)"))
            } else {
                row.child(format!("Window {id}")).child(
                    button(&colors, ("close", id), "Close").on_click(move |_event, _window, cx| {
                        handle
                            .update(cx, |_, window, _| window.remove_window())
                            .ok();
                    }),
                )
            }
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().flex().flex_wrap().gap_2().children(open_buttons))
            .child(
                div()
                    .text_sm()
                    .text_color(colors.disabled)
                    .child(format!("Open windows: {}", cx.windows().len())),
            )
            .child(div().flex().flex_col().gap_2().children(rows))
    }
}

/// A window that says what is special about it, and closes itself.
struct Panel(&'static str);

impl Render for Panel {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_between()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(self.0)
            .child(
                button(&colors, "close", "Close this window")
                    .on_click(|_event, window, _cx| window.remove_window()),
            )
    }
}

fn button(colors: &Colors, id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
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

/// Opens the launcher window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(520.), px(380.)), cx);
    let titlebar = TitlebarOptions {
        title: Some("Windows".into()),
        ..Default::default()
    };
    cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(titlebar)),
        |_window, cx| cx.new(Launcher::new),
    )
    .unwrap();
    cx.activate(true);
}
