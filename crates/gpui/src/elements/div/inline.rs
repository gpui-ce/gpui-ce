use super::{ScrollHandle, StackSafe};
use crate::{
    AnyElement, App, AvailableSpace, Bounds, InlineBoxRequest, InlineLayout, InlineLayoutRequest,
    InlineTextMetrics, LayoutId, Pixels, Point, SharedString, Size, Style, TextLayout, TextRun,
    TextStyle, Window, place_inline_layout, point, px, size,
};
use gpui_util::ResultExt;
use smallvec::SmallVec;
use std::{
    cell::{Ref, RefCell},
    rc::Rc,
    sync::Arc,
};

struct InlineDocument {
    text: SharedString,
    runs: Vec<TextRun>,
    boxes: Vec<InlineBoxRequest>,
    text_style: TextStyle,
    font_size: Pixels,
    line_height: Pixels,
    text_metrics: InlineTextMetrics,
}

struct InlineMeasurement {
    wrap_width: Option<Pixels>,
    layout: InlineLayout,
}

pub(super) struct InlineDivFrameState {
    document: Arc<InlineDocument>,
    measurement: Rc<RefCell<Option<InlineMeasurement>>>,
    box_child_indices: SmallVec<[usize; 2]>,
    request_style: Style,
    paint_origin: Option<Point<Pixels>>,
}

type InlineChildPlacement = (usize, Bounds<Pixels>);

impl InlineDocument {
    fn collect(
        children: &mut [StackSafe<AnyElement>],
        window: &mut Window,
        cx: &mut App,
    ) -> (Arc<Self>, SmallVec<[usize; 2]>) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );
        let font_id = window.text_system().resolve_font(&text_style.font());
        let text_metrics = InlineTextMetrics {
            ascent: window.text_system().ascent(font_id, font_size),
            descent: window.text_system().descent(font_id, font_size),
            x_height: window.text_system().x_height(font_id, font_size),
        };
        let (text, runs, boxes, box_child_indices) =
            collect_inline_content(children, &text_style, window, cx);
        (
            Arc::new(Self {
                text: text.into(),
                runs,
                boxes,
                text_style,
                font_size,
                line_height,
                text_metrics,
            }),
            box_child_indices,
        )
    }
}

impl InlineDivFrameState {
    pub(super) fn request_layout(
        style: &Style,
        children: &mut [StackSafe<AnyElement>],
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self) {
        let (document, box_child_indices) = InlineDocument::collect(children, window, cx);
        let (layout_id, measurement) = request_inline_measurement(style, &document, window);
        (
            layout_id,
            Self {
                document,
                measurement,
                box_child_indices,
                request_style: style.clone(),
                paint_origin: None,
            },
        )
    }

    pub(super) fn box_count(&self) -> usize {
        self.box_child_indices.len()
    }

    pub(super) fn prepare_layout(
        &self,
        bounds: Bounds<Pixels>,
        scroll_handle: Option<&ScrollHandle>,
        window: &Window,
    ) -> Size<Pixels> {
        let content_bounds = inline_content_bounds(bounds, &self.request_style, window.rem_size());
        self.ensure_layout(content_bounds.size.width, window);

        let layout = self.layout();
        if let Some(scroll_handle) = scroll_handle {
            scroll_handle.0.borrow_mut().child_bounds = self
                .child_placements(content_bounds.origin, window)
                .into_iter()
                .map(|(_, bounds)| bounds)
                .collect();
        }
        layout.size
    }

    pub(super) fn prepaint_children(
        &mut self,
        children: &mut [StackSafe<AnyElement>],
        bounds: Bounds<Pixels>,
        style: &Style,
        scroll_offset: Point<Pixels>,
        order: Option<&[usize]>,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<Bounds<Pixels>> {
        let content_origin =
            inline_content_bounds(bounds, style, window.rem_size()).origin + scroll_offset;
        self.paint_origin = Some(content_origin);
        let placements = self.child_placements(content_origin, window);
        prepaint_inline_children(children, &placements, order, window, cx);

        placements
            .into_iter()
            .map(|(_, child_bounds)| child_bounds)
            .collect()
    }

    pub(super) fn paint_children(
        &self,
        children: &mut [StackSafe<AnyElement>],
        bounds: Bounds<Pixels>,
        style: &Style,
        window: &mut Window,
        cx: &mut App,
    ) {
        let origin = self
            .paint_origin
            .unwrap_or_else(|| inline_content_bounds(bounds, style, window.rem_size()).origin);
        let layout = self.layout();
        layout.paint_background(origin, window, cx).log_err();
        for child_index in &self.box_child_indices {
            if let Some(child) = children.get_mut(*child_index) {
                child.paint(window, cx);
            }
        }
        layout.paint(origin, window, cx).log_err();
    }

    fn ensure_layout(&self, width: Pixels, window: &Window) {
        let available_space = size(AvailableSpace::Definite(width), AvailableSpace::MaxContent);
        let expected_wrap_width = inline_wrap_width(&self.document.text_style, available_space);
        if self
            .measurement
            .borrow()
            .as_ref()
            .is_some_and(|measurement| measurement.wrap_width == expected_wrap_width)
        {
            return;
        }
        self.measurement
            .borrow_mut()
            .replace(measure_inline_document(
                &self.document,
                available_space,
                window,
            ));
    }

    fn child_placements(
        &self,
        content_origin: Point<Pixels>,
        window: &Window,
    ) -> SmallVec<[InlineChildPlacement; 2]> {
        let layout = self.layout();
        let placement = place_inline_layout(content_origin, layout.alignment_offset, window);
        self.box_child_indices
            .iter()
            .filter_map(|child_index| {
                layout
                    .boxes
                    .iter()
                    .find(|inline_box| inline_box.id == *child_index as u64)
                    .map(|inline_box| {
                        (
                            *child_index,
                            Bounds::new(
                                content_origin + inline_box.bounds.origin + placement.delta,
                                inline_box.bounds.size,
                            ),
                        )
                    })
            })
            .collect()
    }

    fn layout(&self) -> Ref<'_, InlineLayout> {
        Ref::map(self.measurement.borrow(), |measurement| {
            &measurement
                .as_ref()
                .expect("inline layout was not computed")
                .layout
        })
    }
}

fn request_inline_measurement(
    style: &Style,
    document: &Arc<InlineDocument>,
    window: &mut Window,
) -> (LayoutId, Rc<RefCell<Option<InlineMeasurement>>>) {
    let measurement = Rc::new(RefCell::new(None));
    let measured_document = document.clone();
    let measured_state = measurement.clone();
    let layout_id = window.request_measured_layout(
        style.clone(),
        move |_known_dimensions, available_space, window, _cx| {
            let measurement = measure_inline_document(&measured_document, available_space, window);
            let size = measurement.layout.size;
            measured_state.borrow_mut().replace(measurement);
            size
        },
    );
    (layout_id, measurement)
}

fn collect_inline_content(
    children: &mut [StackSafe<AnyElement>],
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> (
    String,
    Vec<TextRun>,
    Vec<InlineBoxRequest>,
    SmallVec<[usize; 2]>,
) {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut boxes = Vec::new();
    let mut box_child_indices = SmallVec::new();

    for (child_index, child) in children.iter_mut().enumerate() {
        if let Some(content) = child.take_inline_text(text_style) {
            text.push_str(&content.text);
            runs.extend(content.runs);
        } else {
            let measurement = child.layout_as_inline_box(
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                cx,
            );
            boxes.push(InlineBoxRequest {
                id: child_index as u64,
                index: text.len(),
                size: measurement.size,
                vertical_align: measurement.vertical_align,
            });
            box_child_indices.push(child_index);
        }
    }

    (text, runs, boxes, box_child_indices)
}

fn measure_inline_document(
    document: &InlineDocument,
    available_space: Size<AvailableSpace>,
    window: &Window,
) -> InlineMeasurement {
    let wrap_width = inline_wrap_width(&document.text_style, available_space);
    InlineMeasurement {
        wrap_width,
        layout: window.text_system().layout_inline(InlineLayoutRequest {
            text: &document.text,
            runs: &document.runs,
            boxes: &document.boxes,
            font_size: document.font_size,
            line_height: document.line_height,
            text_metrics: document.text_metrics,
            wrap_width,
            line_clamp: document.text_style.line_clamp,
            text_align: document.text_style.text_align,
        }),
    }
}

fn inline_wrap_width(
    text_style: &TextStyle,
    available_space: Size<AvailableSpace>,
) -> Option<Pixels> {
    TextLayout::evaluate_wrap_width(&text_style.white_space, Size::default(), available_space)
}

fn inline_content_bounds(
    bounds: Bounds<Pixels>,
    style: &Style,
    rem_size: Pixels,
) -> Bounds<Pixels> {
    let padding = style.padding.to_pixels(bounds.size.into(), rem_size);
    let border = style.border_widths.to_pixels(rem_size);
    let horizontal = border.left + padding.left + padding.right + border.right;
    let vertical = border.top + padding.top + padding.bottom + border.bottom;
    Bounds::new(
        bounds.origin + point(border.left + padding.left, border.top + padding.top),
        size(
            (bounds.size.width - horizontal).max(px(0.)),
            (bounds.size.height - vertical).max(px(0.)),
        ),
    )
}

fn prepaint_inline_children(
    children: &mut [StackSafe<AnyElement>],
    placements: &[InlineChildPlacement],
    order: Option<&[usize]>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(order) = order {
        for child_index in order {
            if let Some((_, bounds)) = placements.iter().find(|(index, _)| index == child_index)
                && let Some(child) = children.get_mut(*child_index)
            {
                child.prepaint_at(bounds.origin, window, cx);
            }
        }
    } else {
        for (child_index, bounds) in placements {
            if let Some(child) = children.get_mut(*child_index) {
                child.prepaint_at(bounds.origin, window, cx);
            }
        }
    }
}
