use crate::editable_text::{
    EditableInputActionElement, InitStorage, StateBackedElement, TextAreaState,
};
use gpui::{
    App, Element, ElementId, Entity, Hitbox, InteractiveElement, Interactivity, IntoElement,
    SharedString, StyleRefinement, Styled, TextStyle, Window,
};

pub fn text_area(id: impl Into<ElementId>) -> TextAreaElement {
    let mut this = TextAreaElement {
        id: id.into(),
        placeholder: None,
        interactivity: Interactivity::new(),
        init_storage: InitStorage::default(),
    };
    this = this.key_context(super::DEFAULT_INPUT_CONTEXT);
    this.register_actions();
    this
}

// TODO: Disabled flag/state?
pub struct TextAreaElement {
    id: ElementId,
    placeholder: Option<SharedString>,
    interactivity: Interactivity,
    init_storage: InitStorage,
}

impl InteractiveElement for TextAreaElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Styled for TextAreaElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for TextAreaElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl EditableInputActionElement for TextAreaElement {}
impl super::StateBackedElement for TextAreaElement {
    type State = TextAreaState;
    type InitProps = (ElementId, InitStorage);

    fn init_props(&self) -> Self::InitProps {
        (self.id.clone(), self.init_storage.clone())
    }

    fn get_or_init_state(
        init_props: &Self::InitProps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self::State> {
        // Get the state from the app using the element's id as the key.
        // If it doesnt exist, initialize a new state with the user's desired storage medium.
        window.use_keyed_state(init_props.0.clone(), cx, |_window, cx| {
            TextAreaState::new(init_props.1.exec(cx), cx)
        })
    }
}

pub mod element {
    use super::*;

    #[doc(hidden)]
    pub struct LayoutState {
        pub state: Entity<TextAreaState>,
        pub text_style: TextStyle,
    }

    #[doc(hidden)]
    pub struct PrepaintState {
        pub hitbox: Option<Hitbox>,
    }
}

impl Element for TextAreaElement {
    type RequestLayoutState = element::LayoutState;
    type PrepaintState = element::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut resolved_text_style = None;

        let state = self.get_state(window, cx);

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let mut style = element_style.clone();
                    if let gpui::Length::Auto = style.size.width {
                        style.size.width = gpui::relative(1.).into();
                    }
                    if let gpui::Length::Auto = style.size.height {
                        style.size.height = gpui::relative(1.).into();
                    }
                    window.request_layout(style, None, cx)
                })
            },
        );

        let layout_state = element::LayoutState {
            state,
            text_style: resolved_text_style.unwrap_or_else(|| window.text_style()),
        };
        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        todo!()
    }

    fn paint(
        &mut self,
        id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        todo!()
    }
}
