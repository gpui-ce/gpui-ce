//! Pick an element in a window, then inspect its details and state.
//!
//! The inspector is available with the `inspector` feature and in debug builds
//! (`debug_assertions`). In other builds, [`InspectorElementId`] remains available without
//! fields, but `Inspector`, its renderer and reflection types are omitted.
//! [`Window::is_inspector_picking`] always returns `false` in those builds.

/// Identifies one element so the inspector can select it and describe its state.
///
/// The `path` and `instance_id` fields exist only in builds with the `inspector` feature or with
/// `debug_assertions`. In other builds the type has no identifying data, so every two IDs compare
/// equal; do not use it there as a stable key.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct InspectorElementId {
    /// The element's stable path. It is reference counted, so cloning the ID does not copy it.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub path: std::rc::Rc<InspectorElementPath>,
    /// Distinguishes elements that share a `path`, such as two rows built at the same call site.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub instance_id: usize,
}

impl Into<InspectorElementId> for &InspectorElementId {
    fn into(self) -> InspectorElementId {
        self.clone()
    }
}

#[cfg(any(feature = "inspector", debug_assertions))]
pub use conditional::*;

#[cfg(any(feature = "inspector", debug_assertions))]
mod conditional {
    use super::*;
    use crate::{AnyElement, App, Context, Empty, IntoElement, Render, Window};
    use collections::{FxHashMap, TypeIdHashMap};
    use std::any::{Any, TypeId};

    /// A [`GlobalElementId`](crate::GlobalElementId) plus the source location where the element was built.
    ///
    /// Two elements built at different call sites get different paths even when they
    /// have the same ID, so the inspector can tell them apart.
    #[derive(Debug, Eq, PartialEq, Hash)]
    pub struct InspectorElementPath {
        /// The global ID of this element: the chain of `ElementId`s from the root down to it,
        /// including the element itself when it has an `ElementId`.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub global_id: crate::GlobalElementId,
        /// Where the element was constructed in source code.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub source_location: &'static std::panic::Location<'static>,
    }

    impl Clone for InspectorElementPath {
        fn clone(&self) -> Self {
            Self {
                global_id: self.global_id.clone(),
                source_location: self.source_location,
            }
        }
    }

    impl Into<InspectorElementPath> for &InspectorElementPath {
        fn into(self) -> InspectorElementPath {
            self.clone()
        }
    }

    /// The function that builds the inspector panel. Install it with
    /// [`App::set_inspector_renderer`].
    ///
    /// GPUI passes the [`Inspector`], window and inspector context to the renderer. It returns
    /// the panel's element, and GPUI calls it on each draw of the inspector view.
    pub type InspectorRenderer =
        Box<dyn Fn(&mut Inspector, &mut Window, &mut Context<Inspector>) -> AnyElement>;

    /// Tracks the element selected or hovered in the inspector, and whether picking is active.
    ///
    /// Each window owns one while its inspector is open. Use [`Window::toggle_inspector`] to open
    /// or close it, and [`Inspector::is_picking`] to query the mode.
    pub struct Inspector {
        active_element: Option<InspectedElement>,
        pub(crate) pick_depth: Option<f32>,
    }

    struct InspectedElement {
        id: InspectorElementId,
        states: TypeIdHashMap<Box<dyn Any>>,
    }

    impl InspectedElement {
        fn new(id: InspectorElementId) -> Self {
            InspectedElement {
                id,
                states: Default::default(),
            }
        }
    }

    impl Inspector {
        pub(crate) fn new() -> Self {
            Self {
                active_element: None,
                pick_depth: Some(0.0),
            }
        }

        pub(crate) fn select(&mut self, id: InspectorElementId, window: &mut Window) {
            self.set_active_element_id(id, window);
            self.pick_depth = None;
        }

        pub(crate) fn hover(&mut self, id: InspectorElementId, window: &mut Window) {
            if self.is_picking() {
                let changed = self.set_active_element_id(id, window);
                if changed {
                    self.pick_depth = Some(0.0);
                }
            }
        }

        pub(crate) fn set_active_element_id(
            &mut self,
            id: InspectorElementId,
            window: &mut Window,
        ) -> bool {
            let changed = Some(&id) != self.active_element_id();
            if changed {
                self.active_element = Some(InspectedElement::new(id));
                window.refresh();
            }
            changed
        }

        /// Returns the ID of the element that is hovered while picking, or the element
        /// that was selected, or `None` if no element is active.
        pub fn active_element_id(&self) -> Option<&InspectorElementId> {
            self.active_element.as_ref().map(|e| &e.id)
        }

        pub(crate) fn with_active_element_state<T: 'static, R>(
            &mut self,
            window: &mut Window,
            f: impl FnOnce(&mut Option<T>, &mut Window) -> R,
        ) -> R {
            let Some(active_element) = &mut self.active_element else {
                return f(&mut None, window);
            };

            let type_id = TypeId::of::<T>();
            let mut inspector_state = active_element
                .states
                .remove(&type_id)
                .map(|state| *state.downcast().unwrap());

            let result = f(&mut inspector_state, window);

            if let Some(inspector_state) = inspector_state {
                active_element
                    .states
                    .insert(type_id, Box::new(inspector_state));
            }

            result
        }

        /// Enters picking mode. While picking, moving the pointer over an element makes it
        /// the active element, and a click selects it and leaves picking mode.
        pub fn start_picking(&mut self) {
            self.pick_depth = Some(0.0);
        }

        /// Returns `true` while the inspector is in picking mode.
        pub fn is_picking(&self) -> bool {
            self.pick_depth.is_some()
        }

        /// Returns one element for each state type held by the active element when that type has
        /// a renderer registered with [`App::register_inspector_element`]. Returns an empty list
        /// when no element is active.
        ///
        /// Call this from your inspector renderer to show per-element details.
        pub fn render_inspector_states(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Vec<AnyElement> {
            let mut elements = Vec::new();
            if let Some(active_element) = self.active_element.take() {
                for (type_id, state) in &active_element.states {
                    if let Some(render_inspector) = cx
                        .inspector_element_registry
                        .renderers_by_type_id
                        .remove(type_id)
                    {
                        let mut element = (render_inspector)(
                            active_element.id.clone(),
                            state.as_ref(),
                            window,
                            cx,
                        );
                        elements.push(element);
                        cx.inspector_element_registry
                            .renderers_by_type_id
                            .insert(*type_id, render_inspector);
                    }
                }

                self.active_element = Some(active_element);
            }

            elements
        }
    }

    impl Render for Inspector {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Some(inspector_renderer) = cx.inspector_renderer.take() {
                let result = inspector_renderer(self, window, cx);
                cx.inspector_renderer = Some(inspector_renderer);
                result
            } else {
                Empty.into_any_element()
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct InspectorElementRegistry {
        renderers_by_type_id: FxHashMap<
            TypeId,
            Box<dyn Fn(InspectorElementId, &dyn Any, &mut Window, &mut App) -> AnyElement>,
        >,
    }

    impl InspectorElementRegistry {
        /// Stores the renderer for state type `T`, replacing any earlier one.
        pub fn register<T: 'static, R: IntoElement>(
            &mut self,
            f: impl 'static + Fn(InspectorElementId, &T, &mut Window, &mut App) -> R,
        ) {
            self.renderers_by_type_id.insert(
                TypeId::of::<T>(),
                Box::new(move |id, value, window, cx| {
                    let value = value.downcast_ref().unwrap();
                    f(id, value, window, cx).into_any_element()
                }),
            );
        }
    }
}

/// Runtime descriptions of trait methods, generated by `#[derive_inspector_reflection]`.
///
/// Available only with the `inspector` feature or in debug builds.
#[cfg(any(feature = "inspector", debug_assertions))]
pub mod inspector_reflection {
    use std::any::Any;

    /// A method with the signature `fn some_fn(T) -> T`, described at runtime.
    ///
    /// Gives you the method's name and doc comment, and a way to call it. The
    /// `derive_inspector_reflection` attribute on a trait generates one for each method
    /// of the form `fn method(self) -> Self`, so tools can list and call those methods by
    /// name.
    #[derive(Clone, Copy)]
    pub struct FunctionReflection<T> {
        /// The method's name.
        pub name: &'static str,
        /// Calls the method. Takes and returns a boxed `T`.
        pub function: fn(Box<dyn Any>) -> Box<dyn Any>,
        /// The method's doc comment, if it has one.
        pub documentation: Option<&'static str>,
        /// Ties the reflection to `T`. Not used at runtime.
        pub _type: std::marker::PhantomData<T>,
    }

    impl<T: 'static> FunctionReflection<T> {
        /// Calls the method on `value` and returns its result.
        ///
        /// # Panics
        ///
        /// Panics if the method returns a value that isn't a `T`. Generated reflections
        /// always return `T`, so this indicates a bug in the generated code.
        pub fn invoke(&self, value: T) -> T {
            let boxed = Box::new(value) as Box<dyn Any>;
            let result = (self.function)(boxed);
            *result
                .downcast::<T>()
                .expect("Type mismatch in reflection invoke")
        }
    }
}
