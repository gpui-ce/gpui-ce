//! Run with `cargo run -p gpui-ce --example element_selectors`.

#[path = "../shared/prelude.rs"]
mod example_prelude;

use example_prelude::init_example;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, ParentElement, Render, RenderOnce, Select, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, reflection::trait_set, rgb, size,
};

#[derive(IntoElement)]
struct Button {
    children: Vec<AnyElement>,
}

impl Button {
    fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Button {
    #[allow(unused_variables)]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .id("favorite-button")
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .rounded_md()
            .bg(rgb(0x2563eb))
            .text_color(rgb(0xffffff))
            .children(self.children)
            .select(
                Select::children()
                    .reflects(trait_set![gpui::Styled, gpui::ParentElement])
                    .class((any(["icon", "badge"]), not("muted")))
                    .nth(0),
                |element| element.text_xl().text_color(rgb(0xfacc15)).child(div()),
            )
    }
}

struct ElementSelectorsExample;

impl Render for ElementSelectorsExample {
    #[allow(unused_variables)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x111827))
            .flex_col()
            .gap_4()
            .child(
                Button::new()
                    .child(div().child("★").class(["icon", "accent"]))
                    .child("Favorite"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children((0_usize..5).map(|idx| {
                        div()
                            .id(("badge", idx))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .child(format!("{idx}"))
                            .class("badge")
                    }))
                    .select(
                        Select::children()
                            .reflects(gpui::Styled)
                            .class("badge")
                            .id(not(("badge", 4_usize)))
                            .every(2),
                        |element| element.bg(rgb(0x2563eb)).text_color(rgb(0xffffff)),
                    ),
            )
    }
}

#[allow(unused_variables)]
fn build_view(window: &mut Window, cx: &mut App) -> Entity<ElementSelectorsExample> {
    cx.new(|cx| ElementSelectorsExample)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(320.), px(200.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            build_view,
        )
        .expect("Failed to open window");

        init_example(cx, "Element Selectors");
    });
}
