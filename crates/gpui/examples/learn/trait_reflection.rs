//! Run with `cargo run -p gpui-ce --example trait_reflection`.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, Empty, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ParentElement, Pixels, Reflect, StyleRefinement, Styled, Window,
};

#[gpui::reflect_trait]
trait Draggable {}

#[derive(Default, Reflect)]
#[reflect(Draggable)]
struct Card {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Draggable for Card {}

impl IntoElement for Card {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Card {
    type RequestLayoutState = <Empty as Element>::RequestLayoutState;
    type PrepaintState = <Empty as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        <Empty as Element>::request_layout(&mut Empty, global_id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        <Empty as Element>::prepaint(
            &mut Empty,
            global_id,
            inspector_id,
            bounds,
            request_layout,
            window,
            cx,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        <Empty as Element>::paint(
            &mut Empty,
            global_id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        )
    }
}

fn main() {
    let card = Card::default().into_any_element();

    println!("Card traits:");

    for reflected_trait in card.reflected_traits() {
        println!("  {}", reflected_trait.name);
    }

    println!("Styled: {}", card.implements_trait(gpui::Styled));
    println!(
        "ParentElement: {}",
        card.implements_trait(gpui::ParentElement)
    );
    println!("Draggable: {}", card.implements_trait(Draggable));
    println!(
        "InteractiveElement: {}",
        card.implements_trait(gpui::InteractiveElement)
    );

    let empty = Empty.into_any_element();

    println!("Empty Styled: {}", empty.implements_trait(gpui::Styled));
}
