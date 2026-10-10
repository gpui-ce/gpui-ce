macro_rules! element_impl {
    ($name:ty $(, [$($generics:tt)*])? $(, reflection $reflection:expr)?) => {
        impl $(<$($generics)*>)? gpui::IntoElement for $name {
            type Element = Self;

            fn into_element(self) -> Self::Element {
                self
            }
        }

        impl $(<$($generics)*>)? gpui::Element for $name {
            type RequestLayoutState = ();
            type PrepaintState = ();

            $(fn reflection(&self) -> &'static gpui::reflection::ElementReflection {
                $reflection
            })?

            fn id(&self) -> Option<gpui::ElementId> {
                None
            }

            fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
                None
            }

            fn request_layout(
                &mut self,
                _global_id: Option<&gpui::GlobalElementId>,
                _inspector_id: Option<&gpui::InspectorElementId>,
                _window: &mut gpui::Window,
                _cx: &mut gpui::App,
            ) -> (gpui::LayoutId, Self::RequestLayoutState) {
                unreachable!()
            }

            fn prepaint(
                &mut self,
                _global_id: Option<&gpui::GlobalElementId>,
                _inspector_id: Option<&gpui::InspectorElementId>,
                _bounds: gpui::Bounds<gpui::Pixels>,
                _request_layout_state: &mut Self::RequestLayoutState,
                _window: &mut gpui::Window,
                _cx: &mut gpui::App,
            ) -> Self::PrepaintState {
                unreachable!()
            }

            fn paint(
                &mut self,
                _global_id: Option<&gpui::GlobalElementId>,
                _inspector_id: Option<&gpui::InspectorElementId>,
                _bounds: gpui::Bounds<gpui::Pixels>,
                _request_layout_state: &mut Self::RequestLayoutState,
                _prepaint_state: &mut Self::PrepaintState,
                _window: &mut gpui::Window,
                _cx: &mut gpui::App,
            ) {
                unreachable!()
            }
        }
    };
}
