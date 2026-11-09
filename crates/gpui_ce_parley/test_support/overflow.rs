use gpui::{
    AnyWindowHandle, App, AppContext as _, AvailableSpace, Bounds, Context, Div, DivFrameState,
    Element, ElementId, Font, GlobalElementId, HeadlessAppContext, Hsla, InlineLayout,
    InspectorElementId, IntoElement, LayoutId, Pixels, Render, ScaledPixels, SharedString, Styled,
    TruncateFrom, Window, WindowHandle, div, font, hsla, prelude::*, px, size,
};
use gpui_fonts::IBM_PLEX;
use std::{cell::RefCell, panic::Location, rc::Rc, sync::Arc};
use unicode_segmentation::UnicodeSegmentation as _;

pub(crate) const SCALE_FACTOR: f32 = 1.5;

pub(crate) fn first_group_color() -> Hsla {
    hsla(0.02, 0.75, 0.55, 1.0)
}

pub(crate) fn second_group_color() -> Hsla {
    hsla(0.58, 0.75, 0.55, 1.0)
}

pub(crate) fn logical_bounds(bounds: Bounds<ScaledPixels>) -> Bounds<Pixels> {
    bounds.map(|value| px(value.as_f32() / SCALE_FACTOR))
}

pub(crate) fn assert_close(actual: Pixels, expected: Pixels, context: &str) {
    assert!(
        (actual - expected).abs() < px(0.01),
        "{context}: expected {actual:?} to equal {expected:?}"
    );
}

pub(crate) fn update_view<View: Render>(
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

pub(crate) type CapturedParagraphs = Rc<RefCell<Vec<(SharedString, Arc<InlineLayout>)>>>;

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

pub(crate) const OVERFLOW_TEXT: &str =
    "Begin café e\u{301} 👩‍👩‍👧‍👦 and several words that need room before the ending";
const OVERFLOW_MIDDLE: &str = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW";

pub(crate) struct OverflowParagraph {
    pub(crate) text: &'static str,
    pub(crate) font: Font,
    pub(crate) width: f32,
    direction: TruncateFrom,
    pub(crate) max_lines: Option<usize>,
    styled: bool,
    marker: bool,
    pub(crate) remeasure: bool,
    painted: CapturedParagraphs,
    pub(crate) measurements: CapturedParagraphs,
}

impl OverflowParagraph {
    pub(crate) fn new(direction: TruncateFrom) -> Self {
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

pub(crate) fn assert_paragraph_fits(
    text: &str,
    layout: &InlineLayout,
    width: f32,
    max_lines: usize,
) {
    assert_eq!(layout.layout.len, text.len());

    assert!(
        layout.lines.len() <= max_lines,
        "{text:?}: {:?}",
        layout.lines
    );

    assert!(
        layout.layout.platform_layout.size().width <= px(width + 0.01),
        "{text:?}: {:?}",
        layout.layout.visual_lines
    );
}

pub(crate) fn open_overflow_paragraph(
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

pub(crate) fn check_overflow(cx: &mut HeadlessAppContext, check_pixels: bool) {
    let boundaries = OVERFLOW_TEXT
        .grapheme_indices(true)
        .map(|(idx, _grapheme)| idx)
        .chain([OVERFLOW_TEXT.len()])
        .collect::<Vec<_>>();

    for (direction, max_lines, width, scale) in [
        (TruncateFrom::End, None, 180., SCALE_FACTOR),
        (TruncateFrom::Start, None, 180., SCALE_FACTOR),
        (TruncateFrom::Middle, None, 180., SCALE_FACTOR),
        (TruncateFrom::End, Some(2), 140., 1.0),
    ] {
        let mut view = OverflowParagraph::new(direction);
        view.max_lines = max_lines;
        view.width = width;
        let (window, painted) = open_overflow_paragraph(cx, view, 360., scale);

        {
            let captured = painted.borrow();
            assert_eq!(captured.len(), 1);
            let (text, layout) = &captured[0];
            let expected_lines = max_lines.unwrap_or(1);
            assert_paragraph_fits(text, layout, width, expected_lines);
            assert_eq!(layout.lines.len(), expected_lines);

            let (prefix, suffix) = text
                .split_once('…')
                .expect("paragraph must paint the marker");

            assert!(OVERFLOW_TEXT.starts_with(prefix) && OVERFLOW_TEXT.ends_with(suffix));
            assert!(boundaries.contains(&prefix.len()));
            assert!(boundaries.contains(&(OVERFLOW_TEXT.len() - suffix.len())));

            match direction {
                TruncateFrom::End => assert!(suffix.is_empty() && !prefix.is_empty()),
                TruncateFrom::Start => assert!(prefix.is_empty() && !suffix.is_empty()),
                TruncateFrom::Middle => assert!(!prefix.is_empty() && !suffix.is_empty()),
            }

            if max_lines == Some(2) {
                let marker = text.len() - '…'.len_utf8()..text.len();

                assert!(
                    layout.layout.visual_lines[1]
                        .text_range
                        .contains(&marker.start)
                );

                let geometry = layout
                    .layout
                    .platform_layout
                    .inline_geometry_for_ranges(&[marker]);

                assert!(!geometry[0].is_empty());

                assert!(
                    geometry[0]
                        .iter()
                        .all(|geometry| geometry.visual_line_index == 1)
                );
            }
        }

        if check_pixels {
            assert!(
                !cx.glyph_bounds(window.into(), first_group_color())
                    .unwrap()
                    .is_empty(),
                "{direction:?}, max_lines={max_lines:?}, width={width}, scale={scale}"
            );
        }

        update_view(cx, window, |view| view.marker = false);
        let captured = painted.borrow();
        assert_eq!(captured[0].0.as_ref(), OVERFLOW_TEXT);
        assert_eq!(captured[0].1.lines.len(), max_lines.unwrap_or(1));
    }
}

pub(crate) fn check_styled_ellipsis(cx: &mut HeadlessAppContext, check_pixels: bool) {
    let mut view = OverflowParagraph::new(TruncateFrom::Middle);
    view.styled = true;
    view.width = 260.;
    let (window, painted) = open_overflow_paragraph(cx, view, 360., SCALE_FACTOR);

    for width in [260., 100., 260.] {
        update_view(cx, window, |view| view.width = width);

        let captured = painted.borrow();
        let (text, layout) = &captured[0];
        assert_paragraph_fits(text, layout, width, 1);
        assert!(text.contains('…') && text.ends_with("end"));
        let spans = quads(cx, window.into(), first_group_color());
        let union = spans
            .iter()
            .copied()
            .reduce(|left, right| left.union(&right))
            .unwrap();

        assert_close(
            union.size.width,
            px((f32::from(layout.size.width) * SCALE_FACTOR).round() / SCALE_FACTOR),
            "outer span covers the displayed prefix and suffix",
        );

        let geometry = layout
            .layout
            .platform_layout
            .inline_geometry(0..text.len())
            .unwrap();

        for fragment in geometry {
            assert!(
                spans
                    .iter()
                    .any(|bounds| bounds.contains(&fragment.bounds.center()))
            );
        }

        let backgrounds = quads(cx, window.into(), second_group_color());

        if check_pixels {
            let glyphs = cx
                .glyph_bounds(window.into(), second_group_color())
                .unwrap();

            assert_eq!(glyphs.is_empty(), width == 100., "width={width}");

            for glyph in glyphs {
                assert!(
                    backgrounds
                        .iter()
                        .any(|bounds| bounds.contains(&logical_bounds(glyph).center())),
                    "width={width}: glyph {glyph:?} outside {backgrounds:?}"
                );
            }
        }

        if width == 100. {
            assert!(!text.contains('W'));
            assert!(backgrounds.is_empty());
            assert!(layout.size.height < px(46.));
        } else {
            assert!(layout.size.height >= px(46.));

            for font_size in [px(30.), px(14.)] {
                assert!(
                    layout
                        .layout
                        .paint_fragments
                        .iter()
                        .any(|fragment| fragment.font_size == font_size),
                    "width={width}: missing {font_size:?} span"
                );
            }

            assert!(!backgrounds.is_empty());
        }
    }
}

pub(crate) fn quads(
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
