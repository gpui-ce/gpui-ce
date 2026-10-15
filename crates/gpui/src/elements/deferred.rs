use crate::{
    AnyElement, App, Bounds, ContentMask, Element, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
};

/// Creates a [`Deferred`] element that draws `child` after the rest of the tree.
///
/// Use it for content that must appear on top of its surroundings, such as a popover, a menu or a
/// drag preview. The child takes part in layout where it is, so its size and position follow the
/// normal rules. Its prepaint and paint are postponed until the rest of the tree has been drawn.
/// The priority defaults to `0`. See [`Deferred::priority`].
///
/// # Example
///
/// ```
/// use gpui::{
///     Context, IntoElement, Render, TestApp, Window, anchored, deferred, div, point, prelude::*,
///     px,
/// };
///
/// struct Overlay;
///
/// impl Render for Overlay {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div().size_full().child(
///             deferred(
///                 anchored()
///                     .position(point(px(10.), px(10.)))
///                     .child(div().w(px(40.)).h(px(40.))),
///             )
///             .priority(1),
///         )
///     }
/// }
///
/// let mut app = TestApp::new();
/// let mut window = app.open_window(|_window, _cx| Overlay);
/// window.draw();
/// ```
pub fn deferred(child: impl IntoElement) -> Deferred {
    Deferred {
        child: Some(child.into_any_element()),
        priority: DeferredPriority::from(0),
        content_mask: None,
    }
}

/// An element that draws its child after the rest of the tree, in priority order.
///
/// The child is laid out in place, so it is part of the element tree. Its prepaint and paint run
/// later, in a separate pass. Each deferred child is drawn after the elements around it, and
/// deferred children are drawn in ascending priority, so a higher priority draws on top. Create
/// one with [`deferred`].
///
/// A deferred child may create more deferred children. GPUI processes these in rounds, and the
/// frame panics if they nest more than ten rounds deep. Keep such nesting shallow.
#[derive(derive_setters::Setters)]
#[setters(generate = false)]
pub struct Deferred {
    child: Option<AnyElement>,
    /// Sets the draw priority. Deferred elements draw in ascending priority, so a higher value
    /// draws on top of the ones below it. Accepts a number, or a [`DeferredPriority`].
    #[setters(generate, into)]
    priority: DeferredPriority,
    /// Clips the deferred element to `mask` while it is prepainted and painted, so the clipped
    /// area also doesn't receive mouse input.
    ///
    /// Without a mask, no extra clipping is applied.
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
    /// Sets the priority to one more than the enclosing deferred element's, or to 1 when no deferred
    /// element encloses this one.
    ///
    /// A deferred element is enclosing when it is being prepainted and creates this one, directly or
    /// through other elements. Use it for nested popups, so that each level draws above the last.
    pub fn priority_auto(mut self) -> Self {
        self.priority = DeferredPriority::Auto;
        self
    }
}

/// How the draw priority of a [`Deferred`] element is chosen.
///
/// A number converts into `Value`, so `.priority(2)` works.
pub enum DeferredPriority {
    /// One more than the priority of the deferred element that created this one, or 1 when no
    /// deferred element did. Use it for nested popups.
    Auto,
    /// An explicit priority. Higher values draw on top.
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
        Bounds, ContentMask, Context, Entity, Modifiers, MouseButton, MouseDownEvent,
        PlatformInput, StyleRefinement, TestAppContext, Window, anchored, deferred, div, point,
        prelude::*, px, size,
    };
    use std::{cell::Cell, rc::Rc};

    struct MaskedOverlay {
        presses: Rc<Cell<usize>>,
    }

    impl Render for MaskedOverlay {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let presses = self.presses.clone();
            deferred(div().id("masked-overlay").size(px(100.)).on_mouse_down(
                MouseButton::Left,
                move |_, _, _| {
                    presses.set(presses.get() + 1);
                },
            ))
            .content_mask(ContentMask {
                bounds: Bounds::new(point(px(0.), px(0.)), size(px(40.), px(40.))),
                fade_out: Default::default(),
            })
        }
    }

    struct MaskedOverlayRoot {
        overlay: Entity<MaskedOverlay>,
    }

    impl Render for MaskedOverlayRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                self.overlay
                    .clone()
                    .cached(StyleRefinement::default().size_full()),
            )
        }
    }

    /// A deferred element's content mask must clip its hitboxes, which are
    /// inserted during prepaint, and not only its painting.
    #[gpui::test]
    fn test_deferred_mask_clips_input(cx: &mut TestAppContext) {
        let presses = Rc::new(Cell::new(0));
        let window = cx.open_window(size(px(200.), px(200.)), |_, cx| {
            let overlay = cx.new(|_| MaskedOverlay {
                presses: presses.clone(),
            });
            MaskedOverlayRoot { overlay }
        });
        cx.run_until_parked();

        // The second pass reuses the cached overlay's prepaint from the first.
        for pass in 0..2 {
            window
                .update(cx, |_, window, cx| {
                    for position in [point(px(60.), px(60.)), point(px(20.), px(20.))] {
                        window.dispatch_event(
                            PlatformInput::MouseDown(MouseDownEvent {
                                button: MouseButton::Left,
                                position,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                                first_mouse: false,
                            }),
                            cx,
                        );
                    }
                })
                .unwrap();
            assert_eq!(
                presses.get(),
                pass + 1,
                "only the visible region accepts input"
            );

            window.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
        }
    }

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

    /// Regression test for a crash with nested deferred draws (e.g. a popover
    /// menu inside a popover hosted by a cached dock panel). Prepaint indices
    /// recorded during the deferred draw rounds must index the same
    /// `deferred_draws` vector that `reuse_prepaint` slices on the next frame;
    /// previously they were measured against a transient per-round vector, so
    /// reusing the panel's subtree grafted the wrong deferred draws and
    /// panicked in the dispatch tree.
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
