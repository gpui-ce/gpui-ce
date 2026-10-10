//! Run with `cargo run -p gpui-ce --example element_selectors`.
//!
//! `Button::root_class` tags its rendered root; `.class()` tags its view wrapper.
//! Use `descendants()` to cross component and animation boundaries. Annotations
//! preserve common builder traits; call inherent or custom consuming methods first.

#[path = "../shared/prelude.rs"]
mod example_prelude;

#[path = "trait_reflection.rs"]
#[allow(dead_code)]
mod component_example;

use component_example::{CardElement, Draggable};
use example_prelude::init_example;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, ParentElement, Render, RenderOnce, Select,
    SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px, reflection::trait_set,
    rgb, rgb_to_hsla, size,
};

#[derive(IntoElement)]
struct Button {
    root_class: SharedString,
    children: Vec<AnyElement>,
}

impl Button {
    fn new(root_class: impl Into<SharedString>) -> Self {
        Self {
            root_class: root_class.into(),
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
            .class(self.root_class)
            .id("favorite-button")
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .rounded_md()
            .text_color(rgb(0xffffff))
            .children(self.children)
            .select(
                Select::children()
                    .reflects(trait_set![gpui::Styled, gpui::ParentElement])
                    .class((any(["icon", "badge"]), not("muted")))
                    .nth(0),
                |mut element| {
                    element.text_style().color = Some(rgb_to_hsla(rgb(0xfacc15)));

                    element.text_xl().child(div())
                },
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
                Button::new("button")
                    .class("button-component")
                    .child(div().class(["icon", "accent"]).child("★"))
                    .child("Favorite"),
            )
            .child(CardElement::new("card").class("card").select(
                Select::this().class("card").reflects(trait_set![
                    component_example::CardRole,
                    component_example::Draggable,
                    gpui::Styled,
                    gpui::ParentElement,
                    gpui::StatefulInteractiveElement,
                ]),
                |mut element| {
                    *element.drag_payload() = Some("card-data".into());

                    element
                        .px_4()
                        .py_2()
                        .bg(rgb(0x334155))
                        .text_color(rgb(0xffffff))
                        .role(accesskit::Role::Button)
                        .aria_label("Draggable card")
                        .on_click(|_event, _window, _cx| println!("Card clicked"))
                        .child("Card with reflected capabilities")
                },
            ))
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
                    )
                    .bg(rgb(0x202020)),
            )
            .select(
                Select::descendants().class("button").reflects(gpui::Styled),
                |element| element.bg(rgb(0x2563eb)),
            )
    }
}

#[allow(unused_variables)]
fn build_view(window: &mut Window, cx: &mut App) -> Entity<ElementSelectorsExample> {
    cx.new(|cx| ElementSelectorsExample)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(360.), px(280.)), cx);
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
