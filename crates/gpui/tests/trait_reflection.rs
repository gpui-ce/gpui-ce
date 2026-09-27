use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    Interactivity, IntoElement, LayoutId, Pixels, StyleRefinement, Window,
};

#[gpui::reflect_trait]
trait Draggable {}

mod other {
    #[gpui::reflect_trait]
    pub trait Draggable {}
}

#[derive(gpui::Reflect)]
struct Card {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl gpui::Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl gpui::ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

#[derive(gpui::Reflect)]
#[reflect(Draggable)]
struct Control {
    interactivity: Interactivity,
}

impl gpui::InteractiveElement for Control {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl gpui::StatefulInteractiveElement for Control {}

impl Draggable for Control {}

impl other::Draggable for Control {}

macro_rules! element_impl {
    ($name:ty) => {
        impl IntoElement for $name {
            type Element = Self;

            fn into_element(self) -> Self::Element {
                self
            }
        }

        impl Element for $name {
            type RequestLayoutState = ();
            type PrepaintState = ();

            fn id(&self) -> Option<ElementId> {
                None
            }

            fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
                None
            }

            fn request_layout(
                &mut self,
                _global_id: Option<&GlobalElementId>,
                _inspector_id: Option<&InspectorElementId>,
                _window: &mut Window,
                _cx: &mut App,
            ) -> (LayoutId, Self::RequestLayoutState) {
                unreachable!()
            }

            fn prepaint(
                &mut self,
                _global_id: Option<&GlobalElementId>,
                _inspector_id: Option<&InspectorElementId>,
                _bounds: Bounds<Pixels>,
                _request_layout_state: &mut Self::RequestLayoutState,
                _window: &mut Window,
                _cx: &mut App,
            ) -> Self::PrepaintState {
                unreachable!()
            }

            fn paint(
                &mut self,
                _global_id: Option<&GlobalElementId>,
                _inspector_id: Option<&InspectorElementId>,
                _bounds: Bounds<Pixels>,
                _request_layout_state: &mut Self::RequestLayoutState,
                _prepaint_state: &mut Self::PrepaintState,
                _window: &mut Window,
                _cx: &mut App,
            ) {
                unreachable!()
            }
        }
    };
}

element_impl!(Card);

element_impl!(Control);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflects_preset_and_custom_traits() {
        fn require_other_draggable<Type: other::Draggable>() {}

        require_other_draggable::<Control>();

        let card = Card {
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
        .into_any_element();

        assert!(card.implements_trait(gpui::Styled));
        assert!(card.implements_trait(gpui::ParentElement));
        assert!(!card.implements_trait(gpui::InteractiveElement));
        assert!(!card.implements_trait(Draggable));

        let card = card.into_any_element();

        assert!(card.implements_trait(gpui::Styled));

        let control = Control {
            interactivity: Interactivity::default(),
        }
        .into_any_element();

        assert!(control.implements_trait(gpui::InteractiveElement));
        assert!(control.implements_trait(gpui::StatefulInteractiveElement));
        assert!(control.implements_trait(Draggable));
        assert!(!control.implements_trait(other::Draggable));
        assert!(!control.implements_trait(gpui::Styled));

        let empty = gpui::Empty.into_any_element();

        assert!(empty.reflected_traits().is_empty());
    }
}
