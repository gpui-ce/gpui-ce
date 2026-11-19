use smallvec::SmallVec;

use crate::{
    Anchor, AnyElement, App, Axis, Bounds, Display, Edges, Element, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Point, Position, Size, Style,
    Window, point, px,
};

/// The layout state of an [`Anchored`] element: the layout ids of its children.
///
/// It is passed from `request_layout` to `prepaint`, which uses the children's bounds to place them.
pub struct AnchoredState {
    child_layout_ids: SmallVec<[LayoutId; 4]>,
}

/// An element that places its children at an anchor point, and keeps them inside the window.
///
/// Create one with [`anchored`]. Its children keep their own layout, and the anchored element
/// moves the group so that it sits at the anchor position. It suits popovers, menus and tooltips.
#[derive(derive_setters::Setters)]
#[setters(generate = false)]
pub struct Anchored {
    children: SmallVec<[AnyElement; 2]>,
    /// Sets which corner of the element sits at the anchor position. The default is the top left.
    #[setters(generate)]
    anchor: Anchor,
    fit_mode: AnchoredFitMode,
    /// Sets the anchor position. Its meaning depends on [`Anchored::position_mode`]: window
    /// coordinates by default, or an offset from the parent's origin for [`AnchoredPositionMode::Local`].
    ///
    /// Without this, the anchor is the element's own position.
    #[setters(generate, rename = "position", strip_option)]
    anchor_position: Option<Point<Pixels>>,
    /// Sets how [`Anchored::position`] is interpreted. The default is [`AnchoredPositionMode::Window`].
    ///
    /// [`AnchoredPositionMode::Window`] treats the position as window coordinates.
    /// [`AnchoredPositionMode::Local`] treats it as relative to the element's parent.
    #[setters(generate)]
    position_mode: AnchoredPositionMode,
    /// Moves the placed element by `offset`, after the anchor position is applied.
    ///
    /// Use it to sit beside an anchor rather than on it, such as a popover that's offset from the
    /// button that opens it.
    #[setters(generate, strip_option)]
    offset: Option<Point<Pixels>>,
}

/// Creates an [`Anchored`] element that keeps its children inside the window.
///
/// By default the element's top-left corner sits at its anchor position. If the children would
/// overflow the window, the element first switches to the opposite corner, when that fits. Use
/// [`Anchored::snap_to_window`] to push the children inside the window instead.
///
/// The anchor position is in window coordinates unless you change it with
/// [`Anchored::position_mode`]. Without [`Anchored::position`], the anchor is the element's own
/// position.
///
/// # Example
///
/// The menu's children are drawn at the anchor, `(50, 50)`. The `canvas` reports where it was
/// placed:
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{
///     Context, IntoElement, Pixels, Point, Render, TestApp, Window, anchored, canvas, deferred,
///     div, point, prelude::*, px,
/// };
///
/// struct Menu {
///     origin: Rc<Cell<Option<Point<Pixels>>>>,
/// }
///
/// impl Render for Menu {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let origin = self.origin.clone();
///         deferred(
///             anchored()
///                 .position(point(px(50.), px(50.)))
///                 .snap_to_window()
///                 .child(
///                     div().w(px(200.)).h(px(300.)).child(
///                         canvas(
///                             move |bounds, _window, _cx| origin.set(Some(bounds.origin)),
///                             |_bounds, _prepaint_result, _window, _cx| {},
///                         )
///                         .size_full(),
///                     ),
///                 ),
///         )
///     }
/// }
///
/// let origin = Rc::new(Cell::new(None));
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let origin = origin.clone();
///     move |_window, _cx| Menu { origin }
/// });
///
/// window.draw();
/// assert_eq!(origin.get(), Some(point(px(50.), px(50.))));
/// ```
pub fn anchored() -> Anchored {
    Anchored {
        children: SmallVec::new(),
        anchor: Anchor::TopLeft,
        fit_mode: AnchoredFitMode::SwitchAnchor,
        anchor_position: None,
        position_mode: AnchoredPositionMode::Window,
        offset: None,
    }
}

impl Anchored {
    /// Snaps the element to the window's edge when it would overflow, rather than switching
    /// corners.
    pub fn snap_to_window(mut self) -> Self {
        self.fit_mode = AnchoredFitMode::SnapToWindow;
        self
    }

    /// Snaps the element inside the window's edges, keeping `edges` as margin from each edge.
    ///
    /// The margin is added to the window's client inset, when the platform sets one.
    pub fn snap_to_window_with_margin(mut self, edges: impl Into<Edges<Pixels>>) -> Self {
        self.fit_mode = AnchoredFitMode::SnapToWindowWithMargin(edges.into());
        self
    }
}

impl ParentElement for Anchored {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl Element for Anchored {
    type RequestLayoutState = AnchoredState;
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
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let child_layout_ids = self
            .children
            .iter_mut()
            .map(|child| child.request_layout(window, cx))
            .collect::<SmallVec<_>>();

        let anchored_style = Style {
            position: Position::Absolute,
            display: Display::Flex,
            ..Style::default()
        };

        let layout_id = window.request_layout(anchored_style, child_layout_ids.iter().copied(), cx);

        (layout_id, AnchoredState { child_layout_ids })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if request_layout.child_layout_ids.is_empty() {
            return;
        }

        let children_bounds = request_layout
            .child_layout_ids
            .iter()
            .map(|id| window.layout_bounds(*id))
            .reduce(|acc, bounds| acc.union(&bounds))
            .unwrap();

        let (origin, mut desired) = self.position_mode.get_position_and_bounds(
            self.anchor_position,
            self.anchor,
            children_bounds.size,
            bounds,
            self.offset,
        );

        let limits = Bounds {
            origin: Point::default(),
            size: window.viewport_size(),
        };

        if self.fit_mode == AnchoredFitMode::SwitchAnchor {
            let mut anchor = self.anchor;

            if desired.left() < limits.left() || desired.right() > limits.right() {
                let switched = Bounds::from_anchor_and_size(
                    anchor.other_side_along(Axis::Horizontal),
                    origin,
                    children_bounds.size,
                );
                if !(switched.left() < limits.left() || switched.right() > limits.right()) {
                    anchor = anchor.other_side_along(Axis::Horizontal);
                    desired = switched
                }
            }

            if desired.top() < limits.top() || desired.bottom() > limits.bottom() {
                let switched = Bounds::from_anchor_and_size(
                    anchor.other_side_along(Axis::Vertical),
                    origin,
                    children_bounds.size,
                );
                if !(switched.top() < limits.top() || switched.bottom() > limits.bottom()) {
                    desired = switched;
                }
            }
        }

        let client_inset = window.client_inset.unwrap_or(px(0.));
        let edges = match self.fit_mode {
            AnchoredFitMode::SnapToWindowWithMargin(edges) => edges,
            _ => Edges::default(),
        }
        .map(|edge| *edge + client_inset);

        // Snap the horizontal edges of the anchored element to the horizontal edges of the window if
        // its horizontal bounds overflow, aligning to the left if it is wider than the limits.
        if desired.right() > limits.right() {
            desired.origin.x -= desired.right() - limits.right() + edges.right;
        }
        if desired.left() < limits.left() {
            desired.origin.x = limits.origin.x + edges.left;
        }

        // Snap the vertical edges of the anchored element to the vertical edges of the window if
        // its vertical bounds overflow, aligning to the top if it is taller than the limits.
        if desired.bottom() > limits.bottom() {
            desired.origin.y -= desired.bottom() - limits.bottom() + edges.bottom;
        }
        if desired.top() < limits.top() {
            desired.origin.y = limits.origin.y + edges.top;
        }

        let offset = desired.origin - bounds.origin;
        let offset = point(offset.x.round(), offset.y.round());

        window.with_element_offset(offset, |window| {
            for child in &mut self.children {
                child.prepaint(window, cx);
            }
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: crate::Bounds<crate::Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for Anchored {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// How an [`Anchored`] element keeps its children inside the window.
#[derive(Copy, Clone, PartialEq)]
pub enum AnchoredFitMode {
    /// Moves the element to the window edge when it would overflow.
    SnapToWindow,
    /// Moves the element to the window edge, keeping the given margins from each edge.
    SnapToWindowWithMargin(Edges<Pixels>),
    /// Switches to the opposite corner when it would overflow and that corner fits. This is the
    /// default.
    SwitchAnchor,
}

/// The coordinate space of an [`Anchored`] element's anchor position.
#[derive(Copy, Clone, PartialEq)]
pub enum AnchoredPositionMode {
    /// The anchor position is in window coordinates. This is the default.
    Window,
    /// The anchor position is relative to the element's parent.
    Local,
}

impl AnchoredPositionMode {
    fn get_position_and_bounds(
        &self,
        anchor_position: Option<Point<Pixels>>,
        anchor: Anchor,
        size: Size<Pixels>,
        bounds: Bounds<Pixels>,
        offset: Option<Point<Pixels>>,
    ) -> (Point<Pixels>, Bounds<Pixels>) {
        let offset = offset.unwrap_or_default();

        match self {
            AnchoredPositionMode::Window => {
                let anchor_position = anchor_position.unwrap_or(bounds.origin) + offset;
                let bounds = Bounds::from_anchor_and_size(anchor, anchor_position, size);
                (anchor_position, bounds)
            }
            AnchoredPositionMode::Local => {
                let anchor_position = bounds.origin + anchor_position.unwrap_or_default() + offset;
                let bounds = Bounds::from_anchor_and_size(anchor, anchor_position, size);
                (anchor_position, bounds)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Anchor, Bounds, Context, Pixels, PlatformInput, Point, TestAppContext, Window, deferred,
        div, point, prelude::*, px, size,
    };

    /// The point returned for switching anchors must be the one the initial placement used.
    #[test]
    fn anchor_switch_point_includes_offset_and_parent_origin() {
        let layout_bounds = Bounds::new(point(px(500.), px(100.)), size(px(200.), px(100.)));
        for (mode, expected) in [
            (
                super::AnchoredPositionMode::Window,
                point(px(125.), px(25.)),
            ),
            (
                super::AnchoredPositionMode::Local,
                point(px(625.), px(125.)),
            ),
        ] {
            let (origin, bounds) = mode.get_position_and_bounds(
                Some(point(px(100.), px(20.))),
                Anchor::TopLeft,
                size(px(200.), px(300.)),
                layout_bounds,
                Some(point(px(25.), px(5.))),
            );
            assert_eq!(origin, expected);
            assert_eq!(bounds.origin, expected);
        }
    }

    struct LocalAnchoredTestView;

    impl Render for LocalAnchoredTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .absolute()
                    .left(px(500.))
                    .top(px(100.))
                    .child(deferred(
                        super::anchored()
                            .position_mode(super::AnchoredPositionMode::Local)
                            .position(point(px(100.), px(20.)))
                            .offset(point(px(25.), px(5.)))
                            .child(
                                div()
                                    .debug_selector(|| "LOCAL_MENU".into())
                                    .w(px(200.))
                                    .h(px(300.)),
                            ),
                    )),
            )
        }
    }

    /// The menu would overflow the right edge at (625, 125), so it switches to the right anchor
    /// at that same point.
    #[gpui::test]
    fn test_anchored_local_switch_anchor(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| LocalAnchoredTestView);
        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window
                    .rendered_frame
                    .debug_bounds
                    .get("LOCAL_MENU")
                    .copied()
            })
            .unwrap()
            .expect("LOCAL_MENU debug bounds not found");

        assert_eq!(menu_bounds.origin, point(px(425.), px(125.)));
        assert_eq!(menu_bounds.size, size(px(200.), px(300.)));
    }

    struct AnchoredTestView {
        position: Point<Pixels>,
    }

    impl Render for AnchoredTestView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .id("scroll-container")
                    .overflow_y_scroll()
                    .size_full()
                    .child(div().h(px(2000.)).w_full())
                    .child(
                        deferred(
                            super::anchored()
                                .snap_to_window()
                                .position(self.position)
                                .child(
                                    div()
                                        .id("menu")
                                        .debug_selector(|| "MENU".into())
                                        .w(px(200.))
                                        .h(px(300.)),
                                ),
                        )
                        .priority(1),
                    ),
            )
        }
    }

    #[gpui::test]
    fn test_anchored_position_without_scroll(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| AnchoredTestView {
            position: point(px(100.), px(100.)),
        });

        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window.rendered_frame.debug_bounds.get("MENU").copied()
            })
            .unwrap()
            .expect("MENU debug bounds not found");

        assert_eq!(menu_bounds.origin, point(px(100.), px(100.)));
        assert_eq!(menu_bounds.size, size(px(200.), px(300.)));
    }

    #[gpui::test]
    fn test_anchored_position_when_scrolled(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| AnchoredTestView {
            position: point(px(100.), px(100.)),
        });

        cx.run_until_parked();

        window
            .update(cx, |_, window, cx| {
                let event = gpui::ScrollWheelEvent {
                    position: point(px(400.), px(300.)),
                    delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1000.))),
                    ..Default::default()
                };
                window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
            })
            .unwrap();

        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window.rendered_frame.debug_bounds.get("MENU").copied()
            })
            .unwrap()
            .expect("MENU debug bounds not found");

        assert_eq!(menu_bounds.origin, point(px(100.), px(100.)));
        assert_eq!(menu_bounds.size, size(px(200.), px(300.)));
    }

    #[gpui::test]
    fn test_anchored_snaps_to_window(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| AnchoredTestView {
            position: point(px(100.), px(500.)),
        });

        cx.run_until_parked();

        let menu_bounds = window
            .update(cx, |_, window, _| {
                window.rendered_frame.debug_bounds.get("MENU").copied()
            })
            .unwrap()
            .expect("MENU debug bounds not found");

        assert_eq!(menu_bounds.origin, point(px(100.), px(300.)));
        assert_eq!(menu_bounds.size, size(px(200.), px(300.)));
    }
}
