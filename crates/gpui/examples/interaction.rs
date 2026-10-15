//! Mouse input: hover and press styles, click counts, cursors, a tooltip, and drag-and-drop.
//!
//! The example puts an `.id(..)` on each interactive surface, then lets the same view declaration
//! describe hover and press feedback. A click handler separates single from double clicks, while
//! `on_drag` carries a chip value into the view that follows the pointer. Drop zones use
//! `on_drag_over` for a preview and `on_drop` to accept the value.
//!
//! Run it with `cargo run --example interaction`.

use gpui::{
    App, Bounds, ClickEvent, Context, CursorStyle, Div, FontWeight, QuitMode, Rgba, SharedString,
    Window, WindowBounds, WindowOptions, colors::Colors, div, prelude::*, px, rgb, size,
};

/// A colored label used both as a drag payload and as a view.
///
/// The same type also supplies the tooltip content, showing that a rendered view and a drag value
/// can share application data without sharing their stateful entity.
#[derive(Clone)]
struct Chip {
    label: SharedString,
    color: Rgba,
}

impl Chip {
    fn new(label: &str, color: u32) -> Self {
        Self {
            label: SharedString::new(label),
            color: rgb(color),
        }
    }

    fn element(&self) -> Div {
        div()
            .px_3()
            .py_1()
            .rounded_full()
            .text_sm()
            .text_color(gpui::white())
            .bg(self.color)
            .child(self.label.clone())
    }
}

impl Render for Chip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.element()
    }
}

struct Interaction {
    clicks: usize,
    double_clicks: usize,
    chips: Vec<Chip>,
    zones: [(&'static str, Vec<SharedString>); 2],
}

impl Render for Interaction {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let section = |title: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().text_sm().text_color(colors.disabled).child(title))
        };
        let tile = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_4()
                .py_3()
                .rounded_lg()
                .border_1()
                .border_color(colors.border)
                .bg(colors.container)
                .hover(|style| style.bg(colors.separator))
                .active(|style| style.bg(colors.selected).text_color(colors.selected_text))
                .child(label)
        };

        let click_me = tile("click-me", "Click or double-click me")
            .cursor_pointer()
            .tooltip(|_window, cx| {
                cx.new(|_| Chip::new("Tooltips appear after a pause", 0x111827))
                    .into()
            })
            .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                if event.click_count() >= 2 {
                    this.double_clicks += 1;
                } else {
                    this.clicks += 1;
                }
                cx.notify();
            }));

        let chips = self.chips.iter().enumerate().map(|(index, chip)| {
            chip.element()
                .id(("chip", index))
                .cursor_grab()
                .on_drag(chip.clone(), |chip, _offset, _window, cx| {
                    cx.new(|_| chip.clone())
                })
        });

        let zones = self
            .zones
            .iter()
            .enumerate()
            .map(|(index, (title, dropped))| {
                let accent = colors.selected;
                div()
                    .id(("zone", index))
                    .flex_1()
                    .min_h(px(112.))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .rounded_lg()
                    .border_2()
                    .border_color(colors.border)
                    .on_drag_over(move |style, _: &Chip, _window, _cx| style.border_color(accent))
                    .on_drop(cx.listener(move |this, chip: &Chip, _window, cx| {
                        this.zones[index].1.push(chip.label.clone());
                        cx.notify();
                    }))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(*title))
                    .when(dropped.is_empty(), |zone| {
                        zone.child(
                            div()
                                .text_sm()
                                .text_color(colors.disabled)
                                .child("Drop chips here"),
                        )
                    })
                    .children(
                        dropped
                            .iter()
                            .map(|label| div().text_sm().child(label.clone())),
                    )
            });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_6()
            .p_6()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                section("HOVER, PRESS AND CLICK").child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(click_me)
                        .child(format!(
                            "{} clicks, {} double clicks",
                            self.clicks, self.double_clicks
                        )),
                ),
            )
            .child(
                section("CURSOR STYLES").child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_3()
                        .child(tile("pointer", "Pointer").cursor_pointer())
                        .child(tile("crosshair", "Crosshair").cursor(CursorStyle::Crosshair))
                        .child(
                            tile("not-allowed", "Not allowed")
                                .cursor(CursorStyle::OperationNotAllowed),
                        ),
                ),
            )
            .child(
                section("DRAG CHIPS ONTO A ZONE")
                    .child(div().flex().flex_wrap().gap_2().children(chips))
                    .child(div().flex().gap_3().children(zones)),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(720.), px(560.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |_window, cx| {
            cx.new(|_cx| Interaction {
                clicks: 0,
                double_clicks: 0,
                chips: vec![
                    Chip::new("Invoice", 0x2563eb),
                    Chip::new("Receipt", 0x16a34a),
                    Chip::new("Newsletter", 0xdb2777),
                    Chip::new("Draft", 0xd97706),
                ],
                zones: [("Inbox", Vec::new()), ("Archive", Vec::new())],
            })
        },
    )
    .unwrap();
    cx.activate(true);
}
