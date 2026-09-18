use crate::elements::div::{ScrollHandle, StackSafe};
use crate::{
    AnyElement, App, AvailableSpace, Bounds, Display, InlineBoxRequest, InlineLayout,
    InlineLayoutRequest, InlineTextMetrics, InlineTextStyle, LayoutId, Pixels, Point, Position,
    SharedString, Size, Style, TextLayout, TextRun, TextStyle, Window, place_inline_layout, size,
};

use collections::FxHashMap;
use gpui_util::ResultExt;
use smallvec::SmallVec;
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

/// Resolved content published by the element's ordinary layout request. Wrappers that return
/// the same layout ID automatically retain this content and the element's normal lifecycle.
pub(crate) enum InlineContent {
    Text {
        text: SharedString,
        runs: Arc<[TextRun]>,
        font_size: Pixels,
        line_height: Pixels,
    },

    Container {
        children: SmallVec<[LayoutId; 2]>,
    },

    /// Content with its own interaction and text layout, such as InteractiveText.
    Atomic,
}

struct InlineSpan {
    layout_id: LayoutId,
    text_range: Range<usize>,
    box_range: Range<usize>,
}

#[derive(Default)]
struct InlineDocument {
    text: String,
    runs: Vec<TextRun>,
    text_styles: Vec<InlineTextStyle>,
    boxes: Vec<InlineBoxRequest>,
    box_layout_ids: Vec<LayoutId>,
    spans: Vec<InlineSpan>,
}

struct InlineParagraphMeasurement {
    wrap_width: Option<Pixels>,
    layout: InlineLayout,
}

struct InlineParagraph {
    layout_id: LayoutId,
    document: Arc<InlineDocument>,
    measurement: Rc<RefCell<Option<InlineParagraphMeasurement>>>,
    paint_origin: Point<Pixels>,
}

#[derive(Default)]
pub(super) struct InlineDivFrameState {
    paragraphs: Vec<InlineParagraph>,
    /// Text and inline containers whose bounds come from paragraph fragments.
    span_layout_ids: Vec<LayoutId>,
    /// Anonymous paragraphs and separate children, including absolute children.
    flow_child_layout_ids: Vec<LayoutId>,
}

/// Collects text, nested inline spans, and atomic boxes into paragraphs.
/// Block children finish the current paragraph and remain separate flow children.
struct InlineParagraphCollector<'a> {
    frame_state: InlineDivFrameState,
    current_document: InlineDocument,
    open_span_layout_ids: Vec<LayoutId>,
    text_style: TextStyle,
    window: &'a mut Window,
    cx: &'a mut App,
}

impl InlineParagraphCollector<'_> {
    fn collect_element(&mut self, layout_id: LayoutId) {
        let (display, position) = self.window.layout_display_and_position(layout_id);

        if display == Display::None {
            return;
        }

        if position == Position::Absolute {
            self.frame_state.flow_child_layout_ids.push(layout_id);
            return;
        }

        let content = self.window.inline_content(layout_id);

        match content.as_deref() {
            Some(InlineContent::Text {
                text,
                runs,
                font_size,
                line_height,
            }) => {
                self.frame_state.span_layout_ids.push(layout_id);

                let text_start = self.current_document.text.len();
                let box_start = self.current_document.boxes.len();

                self.current_document.text.push_str(text);
                self.current_document.runs.extend(runs.iter().cloned());
                self.current_document.text_styles.push(InlineTextStyle {
                    range: text_start..self.current_document.text.len(),
                    font_size: *font_size,
                    line_height: *line_height,
                });

                self.record_span_ranges(layout_id, text_start, box_start);
                self.record_open_span_ranges(text_start, box_start);
            }

            Some(InlineContent::Container { children }) if display == Display::Inline => {
                self.frame_state.span_layout_ids.push(layout_id);
                self.open_span_layout_ids.push(layout_id);

                for child in children {
                    self.collect_element(*child);
                }

                self.open_span_layout_ids.pop();
            }

            _ if !matches!(display, Display::Inline | Display::InlineFlex) => {
                self.finish_paragraph();
                self.frame_state.flow_child_layout_ids.push(layout_id);
            }

            _ => {
                // Measure atomic contents before Taffy enters a measurement callback. The
                // layout engine is temporarily absent from Window during those callbacks.
                self.window.compute_layout(
                    layout_id,
                    size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                    self.cx,
                );

                let bounds = self.window.layout_bounds(layout_id);
                let box_start = self.current_document.boxes.len();
                let text_start = self.current_document.text.len();

                self.current_document.boxes.push(InlineBoxRequest {
                    id: box_start as u64,
                    index: text_start,
                    size: bounds.size,
                    vertical_align: self.window.layout_vertical_align(layout_id),
                });

                self.current_document.box_layout_ids.push(layout_id);
                self.record_open_span_ranges(text_start, box_start);
            }
        }
    }

    fn record_open_span_ranges(&mut self, text_start: usize, box_start: usize) {
        for idx in 0..self.open_span_layout_ids.len() {
            self.record_span_ranges(self.open_span_layout_ids[idx], text_start, box_start);
        }
    }

    fn record_span_ranges(&mut self, layout_id: LayoutId, text_start: usize, box_start: usize) {
        let text_end = self.current_document.text.len();
        let box_end = self.current_document.boxes.len();

        if let Some(span) = self
            .current_document
            .spans
            .iter_mut()
            .find(|span| span.layout_id == layout_id)
        {
            span.text_range.end = text_end;
            span.box_range.end = box_end;
        } else {
            self.current_document.spans.push(InlineSpan {
                layout_id,
                text_range: text_start..text_end,
                box_range: box_start..box_end,
            });
        }
    }

    fn finish_paragraph(&mut self) {
        if self.current_document.text.is_empty() && self.current_document.boxes.is_empty() {
            return;
        }

        let document = Arc::new(std::mem::take(&mut self.current_document));
        let measurement = Rc::new(RefCell::new(None));

        let text_style = self.text_style.clone();
        let font_size = text_style.font_size.to_pixels(self.window.rem_size());
        let line_height = self.window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), self.window.rem_size()),
        );

        let font_id = self.window.text_system().resolve_font(&text_style.font());

        let text_metrics = InlineTextMetrics {
            ascent: self.window.text_system().ascent(font_id, font_size),
            descent: self.window.text_system().descent(font_id, font_size),
            x_height: self.window.text_system().x_height(font_id, font_size),
        };

        let measured_document = document.clone();
        let measurement_cache = measurement.clone();

        let layout_id = self.window.request_measured_layout(
            Style {
                display: Display::Block,
                ..Style::default()
            },
            move |known_dimensions, available_space, window, _context| {
                let wrap_width = TextLayout::evaluate_wrap_width(
                    &text_style.white_space,
                    known_dimensions,
                    available_space,
                );

                if let Some(measurement) =
                    measurement_cache.borrow().as_ref() as Option<&InlineParagraphMeasurement>
                    && measurement.wrap_width == wrap_width
                {
                    return measurement.layout.size;
                }

                let layout = window.text_system().layout_inline(InlineLayoutRequest {
                    text: &measured_document.text,
                    runs: &measured_document.runs,
                    text_styles: &measured_document.text_styles,
                    boxes: &measured_document.boxes,
                    font_size,
                    line_height,
                    text_metrics,
                    wrap_width,
                    line_clamp: text_style.line_clamp,
                    text_align: text_style.text_align,
                });

                let size = layout.size;
                measurement_cache
                    .borrow_mut()
                    .replace(InlineParagraphMeasurement { wrap_width, layout });

                size
            },
        );

        self.frame_state.flow_child_layout_ids.push(layout_id);
        self.frame_state.paragraphs.push(InlineParagraph {
            layout_id,
            document,
            measurement,
            paint_origin: Point::default(),
        });
    }
}

impl InlineDivFrameState {
    pub(super) fn request_layout(
        style: &Style,
        children: &[LayoutId],
        window: &mut Window,
        context: &mut App,
    ) -> (LayoutId, Self) {
        let mut paragraph_collector = InlineParagraphCollector {
            frame_state: Self::default(),
            current_document: InlineDocument::default(),
            open_span_layout_ids: Vec::new(),
            text_style: window.text_style(),
            window,
            cx: context,
        };

        for child in children {
            paragraph_collector.collect_element(*child);
        }

        paragraph_collector.finish_paragraph();

        let node_id = paragraph_collector.window.request_layout(
            style.clone(),
            paragraph_collector
                .frame_state
                .flow_child_layout_ids
                .iter()
                .copied(),
            paragraph_collector.cx,
        );

        (node_id, paragraph_collector.frame_state)
    }

    pub(super) fn prepare_layout(
        &self,
        bounds: Bounds<Pixels>,
        scroll_handle: Option<&ScrollHandle>,
        children: &[LayoutId],
        window: &mut Window,
    ) -> Size<Pixels> {
        let mut fragments: FxHashMap<LayoutId, Vec<Bounds<Pixels>>> = self
            .span_layout_ids
            .iter()
            .map(|node_id| (*node_id, Vec::new()))
            .collect();

        for paragraph in &self.paragraphs {
            let origin = window.layout_bounds(paragraph.layout_id).origin;
            let measurement = paragraph.measurement.borrow();
            let layout = &measurement
                .as_ref()
                .expect("paragraph was not measured")
                .layout;

            let placement = place_inline_layout(origin, layout.alignment_offset, window);
            let origin = origin + placement.delta;

            for inline_box in &layout.boxes {
                window.place_inline(
                    paragraph.document.box_layout_ids[inline_box.id as usize],
                    Bounds::new(
                        window.pixel_snap_point(origin + inline_box.bounds.origin),
                        inline_box.bounds.size,
                    ),
                    None,
                );
            }

            for span in &paragraph.document.spans {
                let regions = fragments.get_mut(&span.layout_id).unwrap();

                for (native, line_idx) in layout
                    .layout
                    .platform_layout
                    .inline_geometry(span.text_range.clone())
                {
                    let Some(line) = layout.lines.get(line_idx) else {
                        continue;
                    };

                    // Selection geometry can include boxes attached to a neighboring cluster.
                    // Remove every box first, then add exactly the boxes owned by this span.
                    let mut ranges = vec![native.origin.x..native.right()];

                    for inline_box in layout
                        .boxes
                        .iter()
                        .filter(|right| right.line_index == line_idx)
                    {
                        let left = inline_box.bounds.origin.x;
                        let right = inline_box.bounds.right();

                        ranges = ranges
                            .into_iter()
                            .flat_map(|range| {
                                let mut pieces = Vec::new();

                                if range.start < left {
                                    pieces.push(range.start..range.end.min(left));
                                }

                                if range.end > right {
                                    pieces.push(range.start.max(right)..range.end);
                                }

                                pieces
                            })
                            .collect();
                    }

                    for range in ranges {
                        if range.end > range.start {
                            regions.push(Bounds::new(
                                origin + crate::point(range.start, line.origin.y),
                                size(range.end - range.start, line.size.height),
                            ));
                        }
                    }
                }

                for inline_box in &layout.boxes {
                    if span.box_range.contains(&(inline_box.id as usize)) {
                        regions.push(Bounds::new(
                            origin + inline_box.bounds.origin,
                            inline_box.bounds.size,
                        ));
                    }
                }
            }
        }

        for (node_id, mut regions) in fragments {
            for region in &mut regions {
                *region = Bounds::from_corners(
                    window.pixel_snap_point(region.origin),
                    window.pixel_snap_point(region.bottom_right()),
                );
            }

            merge_fragments(&mut regions);

            let union = regions
                .iter()
                .copied()
                .reduce(|left, right| left.union(&right))
                .unwrap_or_else(|| Bounds::new(bounds.origin, Size::default()));

            window.place_inline(node_id, union, Some(regions));
        }

        if let Some(scroll_handle) = scroll_handle {
            scroll_handle.0.borrow_mut().child_bounds = children
                .iter()
                .map(|node_id| window.layout_bounds(*node_id))
                .collect();
        }

        self.flow_child_layout_ids
            .iter()
            .map(|node_id| window.layout_bounds(*node_id))
            .reduce(|left, right| left.union(&right))
            .map_or(Size::default(), |right| right.size)
    }

    pub(super) fn prepaint_children(
        &mut self,
        children: &mut [StackSafe<AnyElement>],
        child_ids: &[LayoutId],
        scroll_offset: Point<Pixels>,
        order: Option<&[usize]>,
        window: &mut Window,
        context: &mut App,
    ) -> Vec<Bounds<Pixels>> {
        window.with_element_offset(scroll_offset, |window| {
            for paragraph in &mut self.paragraphs {
                paragraph.paint_origin = window.layout_bounds(paragraph.layout_id).origin;
            }

            if let Some(order) = order {
                for idx in order {
                    if let Some(child) = children.get_mut(*idx) {
                        child.prepaint(window, context);
                    }
                }
            } else {
                for child in children {
                    child.prepaint(window, context);
                }
            }

            child_ids
                .iter()
                .map(|node_id| window.layout_bounds(*node_id))
                .collect()
        })
    }

    pub(super) fn paint_children(
        &self,
        children: &mut [StackSafe<AnyElement>],
        window: &mut Window,
        context: &mut App,
    ) {
        for child in children {
            child.paint(window, context);
        }

        for paragraph in &self.paragraphs {
            let measurement = paragraph.measurement.borrow();
            let layout = &measurement
                .as_ref()
                .expect("paragraph was not measured")
                .layout;

            layout
                .paint_background(paragraph.paint_origin, window, context)
                .log_err();

            layout
                .paint(paragraph.paint_origin, window, context)
                .log_err();
        }
    }
}

fn merge_fragments(regions: &mut Vec<Bounds<Pixels>>) {
    regions.sort_by(|left, right| {
        left.origin
            .y
            .partial_cmp(&right.origin.y)
            .unwrap()
            .then_with(|| left.origin.x.partial_cmp(&right.origin.x).unwrap())
    });

    let mut merged: Vec<Bounds<Pixels>> = Vec::with_capacity(regions.len());

    for region in regions.drain(..) {
        if let Some(last) = merged.last_mut()
            && last.origin.y == region.origin.y
            && last.size.height == region.size.height
            && region.origin.x <= last.right()
        {
            *last = last.union(&region);
        } else {
            merged.push(region);
        }
    }

    *regions = merged;
}
