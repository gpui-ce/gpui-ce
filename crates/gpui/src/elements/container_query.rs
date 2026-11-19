//! An element that builds its contents from its laid-out size.
//!
//! [`container_query`] measures the element first, then calls its closure with that size. The
//! closure's contents cannot change the size that triggered the call; use the surrounding style
//! when the container itself needs a different size.

use refineable::Refineable as _;

use crate::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Size, Style, StyleRefinement, Styled,
    Window, relative,
};

/// Creates a [`ContainerQuery`] whose contents depend on the size it is given.
///
/// Use it when a component should change its layout to fit the space its parent gives it, rather
/// than the window's size. `render` receives the element's size, after layout has placed it, and
/// returns the contents. The contents can't change the element's own size, because they are built
/// after layout.
///
/// By default the element fills its parent, as `.size_full()` does. Use the [`Styled`] methods to
/// size it differently.
///
/// # Example
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{
///     Context, IntoElement, Pixels, Render, TestApp, Window, container_query, div, prelude::*, px,
/// };
///
/// struct Responsive {
///     width: Rc<Cell<Option<Pixels>>>,
/// }
///
/// impl Render for Responsive {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let width = self.width.clone();
///         container_query(move |size, _window, _cx| {
///             width.set(Some(size.width));
///             if size.width < px(240.) {
///                 div().child("Narrow layout")
///             } else {
///                 div().child("Wide layout")
///             }
///         })
///     }
/// }
///
/// let width = Rc::new(Cell::new(None));
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let width = width.clone();
///     move |_window, _cx| Responsive { width }
/// });
///
/// window.draw();
/// assert!(width.get().is_some_and(|width| width > px(0.)));
/// ```
pub fn container_query<E>(
    render: impl 'static + FnOnce(Size<Pixels>, &mut Window, &mut App) -> E,
) -> ContainerQuery
where
    E: IntoElement,
{
    let mut base_style = StyleRefinement::default();
    base_style.size.width = Some(relative(1.).into());
    base_style.size.height = Some(relative(1.).into());

    ContainerQuery {
        render: Some(Box::new(|size, window, cx| {
            render(size, window, cx).into_any_element()
        })),
        style: base_style,
    }
}

/// A container query element, created with [`container_query`].
pub struct ContainerQuery {
    render: Option<Box<dyn FnOnce(Size<Pixels>, &mut Window, &mut App) -> AnyElement>>,
    style: StyleRefinement,
}

impl Element for ContainerQuery {
    type RequestLayoutState = ();
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let render = self.render.take()?;
        let mut child = render(bounds.size, window, cx);
        child.layout_as_root(bounds.size.map(AvailableSpace::Definite), window, cx);
        child.prepaint_at(bounds.origin, window, cx);
        Some(child)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = prepaint {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for ContainerQuery {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for ContainerQuery {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
