use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels,
    RenderOnce, Stateful, StatefulInteractiveElement, Styled, Window, div, px, reflection::Reflect,
};
use std::panic::Location;

#[gpui::reflection::reflect_trait]
pub(crate) trait TestValue {
    fn value(&mut self) -> &mut usize;
}

#[derive(
    IntoElement, Styled, InteractiveElement, StatefulInteractiveElement, ParentElement, Reflect,
)]
#[into_element(self)]
#[reflect(TestValue)]
pub(crate) struct TestElement {
    #[style(delegate)]
    #[interactivity(delegate)]
    #[children(delegate)]
    inner: Stateful<Div>,
    pub(crate) value: usize,
}

impl TestElement {
    #[track_caller]
    pub(crate) fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            inner: div().id(element_id),
            value: 0,
        }
    }
}

impl TestValue for TestElement {
    fn value(&mut self) -> &mut usize {
        &mut self.value
    }
}

pub(crate) fn set_value(element: &mut AnyElement, value: usize) {
    let mut implementations = Vec::new();
    TestValue.__register::<TestElement>(&mut implementations);

    let methods = implementations.pop().unwrap().methods.unwrap();
    let methods = methods.downcast::<__GpuiReflectTestValueMethods>().unwrap();
    let concrete = element.downcast_mut::<TestElement>().unwrap();
    *(methods.value)(concrete) = value;
}

impl Element for TestElement {
    type RequestLayoutState = <Div as Element>::RequestLayoutState;
    type PrepaintState = <Div as Element>::PrepaintState;

    fn reflection(&self) -> &'static gpui::reflection::ElementReflection {
        <Self as Reflect>::reflection()
    }

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.inner
            .request_layout(global_id, inspector_id, window, cx)
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
        self.inner
            .prepaint(global_id, inspector_id, bounds, request_layout, window, cx)
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
        self.inner.paint(
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
        self.inner.a11y_role()
    }

    fn is_a11y_hidden(&self) -> bool {
        self.inner.is_a11y_hidden()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.inner.write_a11y_info(node);
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
#[reflect(TestValue)]
pub(crate) struct TestComponent {
    value: usize,
}

impl TestValue for TestComponent {
    fn value(&mut self) -> &mut usize {
        &mut self.value
    }
}

impl RenderOnce for TestComponent {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().size(px(24.))
    }
}
