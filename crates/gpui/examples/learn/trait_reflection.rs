//! Run with `cargo run -p gpui-ce --example trait_reflection`.
//!
//! Reflection follows the concrete element that survives conversion. The default
//! `IntoElement` derive stores a `ViewElement`, independently of its rendered
//! root. Animation wrappers also keep their own receiver. `Stateful<Div>` converts
//! to `Div`, preserving its ID and state without granting reflected stateful methods.
//! `CardElement` uses `#[into_element(self)]` to retain its concrete receiver. Its
//! trait derives delegate to `Stateful<Div>`, and its nongeneric `Reflect` derive detects
//! the builtin traits and registers `Draggable` explicitly.

use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels,
    RenderOnce, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div, px,
    reflection::{self, Reflect},
    rgb,
};
use std::panic::Location;

#[reflection::reflect_trait]
pub(crate) trait Draggable {
    fn drag_payload(&mut self) -> &mut Option<SharedString>;
}

#[derive(
    IntoElement, Styled, InteractiveElement, StatefulInteractiveElement, ParentElement, Reflect,
)]
#[into_element(self)]
#[reflect(Draggable)]
pub(crate) struct CardElement {
    #[style(delegate)]
    #[interactivity(delegate)]
    #[children(delegate)]
    inner: Stateful<Div>,
    pub(crate) drag_payload: Option<SharedString>,
}

impl CardElement {
    #[track_caller]
    pub(crate) fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            inner: div().id(element_id),
            drag_payload: None,
        }
    }
}

impl Draggable for CardElement {
    fn drag_payload(&mut self) -> &mut Option<SharedString> {
        &mut self.drag_payload
    }
}

impl Element for CardElement {
    type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Div> as Element>::PrepaintState;

    fn reflection(&self) -> &'static reflection::ElementReflection {
        <Self as Reflect>::reflection()
    }

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        Element::source_location(&self.inner)
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        Element::request_layout(&mut self.inner, global_id, inspector_id, window, cx)
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
        Element::prepaint(
            &mut self.inner,
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
        Element::paint(
            &mut self.inner,
            global_id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Element::a11y_role(&self.inner)
    }

    fn is_a11y_hidden(&self) -> bool {
        Element::is_a11y_hidden(&self.inner)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        Element::write_a11y_info(&self.inner, node);
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        Element::a11y_synthetic_children(&mut self.inner, prepaint, builder);
    }
}

#[derive(IntoElement, Reflect, Default)]
#[reflect(Draggable)]
pub(crate) struct Icon {
    drag_payload: Option<SharedString>,
}

impl Draggable for Icon {
    fn drag_payload(&mut self) -> &mut Option<SharedString> {
        &mut self.drag_payload
    }
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().size(px(24.)).bg(rgb(0x4a90d9))
    }
}

pub(crate) fn set_drag_payload(element: &mut AnyElement, payload: impl Into<SharedString>) {
    assert!(element.implements_trait(Draggable));

    // Invoke the generated table directly; the receiver must be CardElement.
    let methods = <CardElement as Reflect>::build_reflection()
        .into_iter()
        .find_map(|implementation| {
            implementation
                .methods?
                .downcast::<__GpuiReflectDraggableMethods>()
                .ok()
        })
        .expect("Draggable has a callable table");
    let receiver = element
        .downcast_mut::<CardElement>()
        .expect("the table targets CardElement");
    *(methods.drag_payload)(receiver) = Some(payload.into());
}

pub(crate) fn main() {
    let mut card = CardElement::new("card")
        .w(px(240.))
        .h(px(80.))
        .role(accesskit::Role::Button)
        .aria_label("Draggable card")
        .child(div().size(px(24.)))
        .into_any_element();
    set_drag_payload(&mut card, "card-data");

    println!("CardElement traits:");

    for reflected_trait in card.reflected_traits() {
        println!("  {}", reflected_trait.name);
    }

    println!(
        "Payload after reflected mutation: {:?}",
        card.downcast_mut::<CardElement>().unwrap().drag_payload
    );

    let icon = Icon::default().into_any_element();
    let stateful_div = div().id("ordinary-card").into_any_element();

    println!(
        "ViewElement<Icon>: Draggable={}, Styled={}\nDiv: StatefulInteractiveElement={}",
        icon.implements_trait(Draggable),
        icon.implements_trait(gpui::Styled),
        stateful_div.implements_trait(gpui::StatefulInteractiveElement)
    );
}
