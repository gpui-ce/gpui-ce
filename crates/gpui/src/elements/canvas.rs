use refineable::Refineable as _;

use crate::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, Pixels,
    Style, StyleRefinement, Styled, Window,
};

/// Creates a [`Canvas`] that draws with the painting methods on [`Window`].
///
/// Use it for custom drawing that doesn't need its own element type, such as a chart or a debug
/// overlay. `prepaint` runs once the canvas's bounds are known, and returns a value. `paint` runs
/// later in the same frame, with the same bounds and that value. Draw in `paint`, with calls such
/// as [`Window::paint_quad`]. Use `prepaint` for work that needs the bounds, such as computing
/// points from them.
///
/// The canvas has no content to size it, so give it a size with a [`Styled`] method, such as
/// `.size_full()`. For a reusable drawing with its own state, implement [`Element`] instead.
///
/// # Example
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{
///     Context, IntoElement, PaintQuad, Point, Pixels, Render, TestApp, Window, canvas, point, prelude::*,
///     px, rgb,
/// };
///
/// struct Swatch {
///     origin: Rc<Cell<Option<Point<Pixels>>>>,
///     painted: Rc<Cell<bool>>,
/// }
///
/// impl Render for Swatch {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let origin = self.origin.clone();
///         let painted = self.painted.clone();
///         canvas(
///             move |bounds, _window, _cx| origin.set(Some(bounds.origin)),
///             move |bounds, _prepaint_result, window, _cx| {
///                 window.paint_quad(PaintQuad::new(bounds).background(rgb(0x2563eb)));
///                 painted.set(true);
///             },
///         )
///         .size_full()
///     }
/// }
///
/// let origin = Rc::new(Cell::new(None));
/// let painted = Rc::new(Cell::new(false));
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let origin = origin.clone();
///     let painted = painted.clone();
///     move |_window, _cx| Swatch { origin, painted }
/// });
///
/// window.draw();
/// assert_eq!(origin.get(), Some(point(px(0.), px(0.))));
/// assert!(painted.get());
/// ```
pub fn canvas<T>(
    prepaint: impl 'static + FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T,
    paint: impl 'static + FnOnce(Bounds<Pixels>, T, &mut Window, &mut App),
) -> Canvas<T> {
    Canvas {
        prepaint: Some(Box::new(prepaint)),
        paint: Some(Box::new(paint)),
        style: StyleRefinement::default(),
    }
}

/// An element that runs custom drawing code, created with [`canvas`].
///
/// Its two callbacks run once each, in the `prepaint` and `paint` phases of the frame that
/// created it. Use a canvas when a callback is enough. Implement [`Element`] when the drawing needs
/// its own state across frames.
pub struct Canvas<T> {
    prepaint: Option<Box<dyn FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> T>>,
    paint: Option<Box<dyn FnOnce(Bounds<Pixels>, T, &mut Window, &mut App)>>,
    style: StyleRefinement,
}

impl<T: 'static> IntoElement for Canvas<T> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: 'static> Element for Canvas<T> {
    type RequestLayoutState = Style;
    type PrepaintState = Option<T>;

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
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<T> {
        Some(self.prepaint.take().unwrap()(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let prepaint = prepaint.take().unwrap();
        style.paint(bounds, window, cx, |window, cx| {
            (self.paint.take().unwrap())(bounds, prepaint, window, cx)
        });
    }
}

impl<T> Styled for Canvas<T> {
    fn style(&mut self) -> &mut crate::StyleRefinement {
        &mut self.style
    }
}
