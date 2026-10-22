use crate::{
    AnyElement, App, Bounds, ContentMask, Element, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
};

/// Builds a `Deferred` element, which delays the layout and paint of its child.
pub fn deferred(child: impl IntoElement) -> Deferred {
    Deferred {
        child: Some(child.into_any_element()),
        priority: DeferredPriority::from(0),
        content_mask: None,
    }
}

/// Delays painting a child until after its ancestors while keeping its layout in the tree.
///
/// Limit cases where deferred elements produce more deferred elements.
/// [`Window::prepaint_deferred_draws`] stops processing after a depth limit to avoid infinite loops.
#[derive(derive_setters::Setters)]
#[setters(generate = false)]
pub struct Deferred {
    child: Option<AnyElement>,
    /// Sets a priority for the element. A higher priority means painting the element
    /// on top of deferred draws with a lower priority, closer to the viewer.
    #[setters(generate, into)]
    priority: DeferredPriority,
    /// When a content mask is provided, the deferred element will be clipped to that region during
    /// both prepaint and paint.
    #[setters(generate, strip_option)]
    content_mask: Option<ContentMask<Pixels>>,
}

impl Element for Deferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout_id = self.child.as_mut().unwrap().request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = self.child.take().unwrap();
        let element_offset = window.element_offset();
        let priority = self.priority.evaluate(cx);
        window.defer_draw(child, element_offset, priority, self.content_mask);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for Deferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Deferred {
    /// Sets priority one above the deferred element that caused this one, even indirectly.
    /// Defaults to 1 for elements caused by non-deferred elements.
    pub fn priority_auto(mut self) -> Self {
        self.priority = DeferredPriority::Auto;
        self
    }
}

/// Describes how the priority for a [`Deferred`] element is calculated.
pub enum DeferredPriority {
    /// Returns priority 1 for the first deferred element in a rendering stack, or the
    /// previous deferred priority plus 1, so nested deferred elements render in front.
    Auto,
    /// Takes an explicit priority value for the deferred element.
    Value(usize),
}
impl From<usize> for DeferredPriority {
    fn from(value: usize) -> Self {
        Self::Value(value)
    }
}
impl DeferredPriority {
    /// Calculates the new priority based on the current value in the [`DeferredPriorityStackCache`].
    /// Should only be called during prepaint of a [`Deferred`] element.
    fn evaluate(&self, cx: &App) -> usize {
        match self {
            DeferredPriority::Value(value) => *value,
            DeferredPriority::Auto => match DeferredPriorityStackCache::current_depth(cx) {
                None => 1, // NOTE: functionality change. If using auto, we start at a priority of 1 instead of 0.
                Some(prev_deferred_priority) => prev_deferred_priority.saturating_add(1),
            },
        }
    }
}

/// Internal global to track the depth of deferred renders during prepaint.
#[derive(Default)]
pub(crate) struct DeferredPriorityStackCache(Vec<usize>);
impl crate::Global for DeferredPriorityStackCache {}
impl DeferredPriorityStackCache {
    pub(crate) fn push(priority: usize, cx: &mut App) {
        cx.default_global::<Self>().0.push(priority);
    }

    pub(crate) fn pop(cx: &mut App) {
        cx.default_global::<Self>().0.pop();
    }

    fn current_depth(cx: &App) -> Option<usize> {
        let cache = cx.try_global::<Self>()?;
        cache.0.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Context, Entity, StyleRefinement, TestAppContext, Window, anchored, deferred, div, point,
        prelude::*, px, size,
    };

    /// A stand-in for a dock panel hosting a popover (deferred draw) whose
    /// content opens another popover (a deferred draw created while
    /// prepainting the first one's content).
    struct PanelView;

    impl Render for PanelView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Panel").size_full().child(
                deferred(
                    anchored().position(point(px(10.), px(10.))).child(
                        div().key_context("Popover").w(px(200.)).h(px(200.)).child(
                            deferred(
                                anchored().position(point(px(30.), px(30.))).child(
                                    div()
                                        .key_context("NestedMenu")
                                        .debug_selector(|| "NESTED_MENU".into())
                                        .w(px(50.))
                                        .h(px(50.)),
                                ),
                            )
                            .priority(2),
                        ),
                    ),
                )
                .priority(1),
            )
        }
    }

    struct RootView {
        panel: Entity<PanelView>,
    }

    impl Render for RootView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().key_context("Root").size_full().child(
                self.panel
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        }
    }

    /// Nested deferred draws in cached panels must retain indices into the same
    /// `deferred_draws` vector that `reuse_prepaint` slices next frame. Indices into a
    /// transient per-round vector previously grafted wrong draws and panicked.
    #[gpui::test]
    fn test_nested_deferred_draws_with_reused_views(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, cx| {
            let panel = cx.new(|_| PanelView);
            RootView { panel }
        });
        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window
                    .rendered_frame
                    .debug_bounds
                    .get("NESTED_MENU")
                    .copied()
            })
            .unwrap()
            .expect("NESTED_MENU debug bounds not found");
        assert_eq!(menu_bounds.size, size(px(50.), px(50.)));

        // Re-render only the root view; the panel is cached, so its subtree -
        // including both deferred draw records - is reused from the previous
        // frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Reuse the subtree a second time, exercising ranges that were
        // themselves recorded during a reused frame.
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();

        // Re-render the panel itself again to prove the popovers still draw.
        window
            .update(cx, |root, _, cx| {
                root.panel.update(cx, |_, cx| cx.notify());
            })
            .unwrap();
        cx.run_until_parked();

        window
            .update(cx, |_, window, _| {
                assert_eq!(window.rendered_frame.deferred_draws.len(), 2);
                assert!(
                    window
                        .rendered_frame
                        .debug_bounds
                        .contains_key("NESTED_MENU")
                );
            })
            .unwrap();
    }
}
