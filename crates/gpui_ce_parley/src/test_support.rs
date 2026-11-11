//! Shared paragraph fixtures for headless overflow tests.

use gpui::{
    AnyWindowHandle, App, AppContext as _, AvailableSpace, Bounds, Context, Div, DivFrameState,
    Element, ElementId, Font, GlobalElementId, HeadlessAppContext, Hsla, InlineLayout,
    InspectorElementId, IntoElement, LayoutId, Pixels, Render, ScaledPixels, SharedString, Styled,
    TruncateFrom, Window, WindowHandle, div, font, hsla, prelude::*, px, size,
};
use gpui_fonts::IBM_PLEX;
use std::{cell::RefCell, panic::Location, rc::Rc, sync::Arc};

pub const SCALE_FACTOR: f32 = 1.5;

pub const OVERFLOW_CASES: [(TruncateFrom, Option<usize>, f32, f32); 4] = [
    (TruncateFrom::End, None, 180., SCALE_FACTOR),
    (TruncateFrom::Start, None, 180., SCALE_FACTOR),
    (TruncateFrom::Middle, None, 180., SCALE_FACTOR),
    (TruncateFrom::End, Some(2), 140., 1.0),
];

pub fn first_group_color() -> Hsla {
    hsla(0.02, 0.75, 0.55, 1.0)
}

pub fn second_group_color() -> Hsla {
    hsla(0.58, 0.75, 0.55, 1.0)
}

pub fn logical_bounds(bounds: Bounds<ScaledPixels>) -> Bounds<Pixels> {
    bounds.map(|value| px(value.as_f32() / SCALE_FACTOR))
}

pub fn update_view<View: Render>(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<View>,
    edit: impl FnOnce(&mut View),
) {
    window
        .update(cx, |view, _window, cx| {
            edit(view);
            cx.notify();
        })
        .unwrap();

    cx.run_until_parked();
}

pub type CapturedParagraphs = Rc<RefCell<Vec<(SharedString, Arc<InlineLayout>)>>>;

struct ParagraphProbe {
    element: Div,
    painted: CapturedParagraphs,
    measurements: CapturedParagraphs,
    remeasure: bool,
}

impl IntoElement for ParagraphProbe {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ParagraphProbe {
    type RequestLayoutState = DivFrameState;
    type PrepaintState = <Div as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (layout_id, state) = self
            .element
            .request_layout(global_id, inspector_id, window, cx);

        if self.remeasure {
            self.measurements.borrow_mut().clear();

            for width in [80., 200., 2000., 80.] {
                window.compute_layout(
                    layout_id,
                    size(
                        AvailableSpace::Definite(px(width)),
                        AvailableSpace::MaxContent,
                    ),
                    cx,
                );

                self.measurements
                    .borrow_mut()
                    .extend(state.measured_inline_paragraphs());
            }
        }

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.element
            .prepaint(global_id, inspector_id, bounds, state, window, cx)
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        *self.painted.borrow_mut() = state.measured_inline_paragraphs();

        self.element
            .paint(global_id, inspector_id, bounds, state, prepaint, window, cx);
    }
}

pub const OVERFLOW_TEXT: &str =
    "Begin café e\u{301} 👩‍👩‍👧‍👦 and several words that need room before the ending";
const OVERFLOW_MIDDLE: &str = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW";

pub struct OverflowParagraph {
    pub text: &'static str,
    pub font: Font,
    pub width: f32,
    direction: TruncateFrom,
    pub max_lines: Option<usize>,
    pub styled: bool,
    pub marker: bool,
    pub remeasure: bool,
    painted: CapturedParagraphs,
    pub measurements: CapturedParagraphs,
}

impl OverflowParagraph {
    pub fn new(direction: TruncateFrom) -> Self {
        Self {
            text: OVERFLOW_TEXT,
            font: font(IBM_PLEX.family),
            width: 180.,
            direction,
            max_lines: None,
            styled: false,
            marker: true,
            remeasure: false,
            painted: Rc::default(),
            measurements: Rc::default(),
        }
    }
}

impl Render for OverflowParagraph {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut paragraph = div()
            .block()
            .w_full()
            .min_w_0()
            .font(self.font.clone())
            .text_size(px(14.))
            .line_height(px(24.))
            .text_color(first_group_color());

        if self.marker {
            paragraph = match self.direction {
                TruncateFrom::End => paragraph.text_ellipsis(),
                TruncateFrom::Start => paragraph.text_ellipsis_start(),
                TruncateFrom::Middle => paragraph.text_ellipsis_middle(),
            };
        }

        paragraph = if let Some(count) = self.max_lines {
            paragraph.line_clamp(count)
        } else {
            paragraph.whitespace_nowrap()
        };

        paragraph = if self.styled {
            paragraph.child(
                div()
                    .inline()
                    .bg(first_group_color())
                    .child("begin ")
                    .child(
                        div()
                            .inline()
                            .text_size(px(30.))
                            .line_height(px(46.))
                            .text_color(second_group_color())
                            .bg(second_group_color())
                            .child(OVERFLOW_MIDDLE),
                    )
                    .child(" end"),
            )
        } else {
            paragraph.child(self.text)
        };

        div()
            .size_full()
            .items_start()
            .child(div().w(px(self.width)).child(ParagraphProbe {
                element: paragraph,
                painted: self.painted.clone(),
                measurements: self.measurements.clone(),
                remeasure: self.remeasure,
            }))
    }
}

pub fn open_overflow_paragraph(
    cx: &mut HeadlessAppContext,
    view: OverflowParagraph,
    viewport_width: f32,
    scale: f32,
) -> (WindowHandle<OverflowParagraph>, CapturedParagraphs) {
    let painted = view.painted.clone();
    let window = cx
        .open_window(size(px(viewport_width), px(180.)), |window, cx| {
            window.set_scale_factor(scale);

            cx.new(|_cx| view)
        })
        .unwrap();

    cx.run_until_parked();

    (window, painted)
}

pub fn quads(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    color: Hsla,
) -> Vec<Bounds<Pixels>> {
    let mut bounds: Vec<_> = cx
        .solid_quad_bounds(window, color)
        .unwrap()
        .into_iter()
        .map(logical_bounds)
        .collect();

    bounds.sort_by(|left, right| {
        left.origin
            .y
            .partial_cmp(&right.origin.y)
            .unwrap()
            .then_with(|| left.origin.x.partial_cmp(&right.origin.x).unwrap())
    });

    bounds
}
