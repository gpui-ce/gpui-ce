use crate::editable_text::{
    TextInputLayoutData, TextLineSegment,
    actions::{EditableInputActionElement, EditableTextActionHandler},
};
use gpui::{
    Along, App, Axis, Bounds, ContentMask, Context, CursorStyle, DispatchPhase, Display,
    ElementInputHandler, Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior, Hsla,
    InteractiveElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ScrollWheelEvent, SharedString, Size, Style, TextAlign, TextLayout, Window,
    WrappedLine, fill, point, px, size,
};
use smallvec::SmallVec;
use std::{ops::Range, sync::Arc};

pub enum PrepaintElement {
    Line {
        line: Arc<WrappedLine>,
        point: Point<Pixels>,
        align: TextAlign,
    },
    Quad(PaintQuad),
}
impl PrepaintElement {
    fn build_quads(
        offset_corners: Vec<(Point<Pixels>, Point<Pixels>)>,
        origin: Point<Pixels>,
        color: Hsla,
    ) -> impl Iterator<Item = Self> {
        let iter = offset_corners.into_iter();
        iter.map(move |(offset_start, offset_end)| {
            let bounds = Bounds::from_corners(origin + offset_start, origin + offset_end);
            PrepaintElement::Quad(fill(bounds, color))
        })
    }
}

#[doc(hidden)]
pub struct LayoutState<State> {
    pub state: Entity<State>,
}

#[doc(hidden)]
pub struct PrepaintState {
    pub hitbox: Option<Hitbox>,
    pub inner_bounds: Bounds<Pixels>,
    pub focus_handle: FocusHandle,
    pub elements: SmallVec<[PrepaintElement; 3]>,
    pub scroll_offset: Point<Pixels>,
    pub caret_visible: bool,
}

pub trait EditableTextElement: InteractiveElement + EditableInputActionElement {
    fn init_state(&self, cx: &mut Context<Self::State>) -> Self::State;
    fn placeholder(&self) -> &Option<SharedString>;

    fn shared_request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, LayoutState<Self::State>) {
        // Fetches or initializes the internal state of the field
        let state = match &self.interactivity().element_id {
            None => unimplemented!("all input elements must be assigned an id"),
            Some(element_id) => {
                let state = window
                    .use_keyed_state(element_id.clone(), cx, |_window, cx| self.init_state(cx));
                // store a reference to the entity owned by the element for access in action handlers
                *self.state_entity_rc().borrow_mut() = state.downgrade();
                state
            }
        };

        let focus_handle;
        let show_placeholder;
        let storage_version;
        {
            let state = state.read(cx);
            focus_handle = state.focus_handle(cx);
            show_placeholder = state.storage().content_utf8().is_empty();
            storage_version = state.storage().version();
        }

        // TODO: This required a gpui api change in order to sync the focus handle between Interactivity and TextInputStateBase
        self.interactivity().track_focus(focus_handle);

        let placeholder = self.placeholder().clone();
        let placeholder_color = Hsla::white().opacity(0.5); // TODO: as an element param
        let layout_id = self.interactivity().request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| {
                let state = state.clone();
                window.with_text_style(style.text_style().cloned(), |window| {
                    // NOTE: Loosely mirrors TextLayout::layout
                    let text_layout_id = window.request_measured_layout(Default::default(), {
                        let text_style = window.text_style();
                        let font_size = text_style.font_size.to_pixels(window.rem_size());
                        let line_height = window.pixel_snap(
                            text_style
                                .line_height
                                .to_pixels(font_size.into(), window.rem_size()),
                        );
                        move |known_dimensions, available_space, window, cx| {
                            let text: SharedString;
                            let color: Hsla;
                            let prev_wrap_width: Option<Pixels>;
                            let prev_size: Option<Size<Pixels>>;
                            let last_seen_storage_version: u16;

                            {
                                let state = state.read(cx);
                                match show_placeholder {
                                    false => {
                                        text = SharedString::from(state.storage().content_utf8());
                                        color = text_style.color;
                                    }
                                    true => {
                                        text = placeholder.clone().unwrap_or_default();
                                        color = placeholder_color;
                                    }
                                }
                                prev_wrap_width = state.layout_data.wrap_width;
                                prev_size = state.layout_data.size;
                                last_seen_storage_version =
                                    state.layout_data.last_seen_storage_version;
                            }

                            let runs = vec![gpui::TextRun {
                                len: text.len(),
                                font: text_style.font(),
                                color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }];

                            let wrap_width = TextLayout::evaluate_wrap_width(
                                &text_style.white_space,
                                known_dimensions,
                                available_space,
                            );

                            let truncation = TextLayout::evaluate_overflow(
                                &text_style,
                                known_dimensions,
                                available_space,
                            );

                            if let Some(size) = prev_size
                                && (wrap_width.is_none() || wrap_width == prev_wrap_width)
                                && truncation.width.is_none()
                                && storage_version == last_seen_storage_version
                            {
                                return size;
                            }

                            let (text, runs) = TextLayout::apply_truncation(
                                text,
                                &text_style,
                                font_size,
                                wrap_width,
                                &truncation,
                                &runs,
                                cx,
                            );

                            let wrapped_lines = window
                                .text_system()
                                .shape_text(
                                    text,
                                    font_size,
                                    &runs,
                                    wrap_width,
                                    text_style.line_clamp,
                                )
                                .unwrap_or_default();

                            // Build the size of the text and convert the wrapped_lines into
                            // lines that will be cached in state and painted.
                            let mut size: Size<Pixels> = Size::default();
                            let mut pos_y = 0;
                            let mut line_start = 0;
                            let mut lines = Vec::with_capacity(wrapped_lines.len());
                            for line in wrapped_lines {
                                let line_size = line.size(line_height);
                                size.height += line_size.height;
                                size.width = size.width.max(line_size.width).ceil();

                                let num_visual_lines = line.wrap_boundaries().len() + 1;
                                let line_len = line.len();
                                lines.push(TextLineSegment {
                                    text_range: line_start..line_start + line_len,
                                    wrapped_line: Some(Arc::new(line)),
                                    pos_y,
                                    num_visual_lines,
                                });
                                line_start += line_len;
                                pos_y += num_visual_lines;
                            }

                            let layout_data = TextInputLayoutData {
                                wrap_width,
                                size: Some(size),
                                last_seen_storage_version,
                                lines,
                                lines_represent_placeholder: show_placeholder,
                            };

                            // Update the state for use in prepaint, paint, and action handlers.
                            // request_measured_layout caches this scope for processing later
                            // between layout and prepaint, so we cant just copy/move these values to the outer scope.
                            state.update(cx, move |state, _cx| {
                                state.layout_data = layout_data;
                            });

                            size
                        }
                    });

                    window.request_layout(style.clone(), Some(text_layout_id), cx)
                })
            },
        );

        let layout_state = LayoutState { state };
        (layout_id, layout_state)
    }

    fn shared_prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut LayoutState<Self::State>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> PrepaintState {
        struct InteractivityPrepaint {
            hitbox: Option<Hitbox>,
            scroll_offset: Point<Pixels>,
            padding: gpui::Edges<Pixels>,
        }
        // TODO: how do we enable scrolling? overflow on interactivity?
        let prepaint = self.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, _cx| {
                let hitbox =
                    hitbox.or_else(|| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal)));
                let padding = style
                    .padding
                    .to_pixels(bounds.size.into(), window.rem_size());
                InteractivityPrepaint {
                    hitbox,
                    scroll_offset,
                    padding,
                }
            },
        );
        let InteractivityPrepaint {
            hitbox,
            scroll_offset,
            padding,
        } = prepaint;
        let inner_bounds = {
            let mut bounds = bounds;
            bounds.origin += point(padding.left, padding.top);
            bounds.size.width -= padding.left + padding.right;
            bounds.size.height -= padding.top + padding.bottom;
            bounds
        };

        let selection_color = Hsla::blue().opacity(0.5); // TODO: as an element param
        let caret_color = Hsla::white(); // TODO: as an element param

        let state = request_layout.state.read(cx);

        let focus_handle = state.focus_handle(cx);
        let caret_pos = state.caret_pos();
        let selection = state.selected_range();
        let ime_range = state.marked_range();
        // TODO: Cursor blinking
        let cursor_visible = true; // input.cursor_visible();

        let mut elements = SmallVec::new();

        let line_height = window.line_height();
        let is_range_contained_by_range =
            |text_range: &Range<usize>, containing_range: &Range<usize>| {
                if text_range.is_empty() {
                    containing_range.start <= text_range.start
                        && containing_range.end > text_range.start
                } else {
                    containing_range.end > text_range.start
                        && containing_range.start < text_range.end
                }
            };
        let mut carent_point = Point::default();
        for segment in &state.layout_data.lines {
            let line_distance_from_top = segment.pos_y * line_height;
            let line_y = line_distance_from_top - scroll_offset.y;
            let line_bottom = line_y + line_height * segment.num_visual_lines as f32;
            let line_visible = line_bottom >= Pixels::ZERO && line_y <= inner_bounds.size.height;
            if !line_visible {
                continue;
            }

            // TODO: First render all lines (underlines for IME), then all selections, then cursor if no selection

            if let Some(wrapped) = &segment.wrapped_line {
                let point = inner_bounds.origin + point(Pixels::ZERO, line_y);
                elements.push(PrepaintElement::Line {
                    line: wrapped.clone(),
                    point,
                    align: TextAlign::Left,
                });
            }

            let segment_is_empty = segment.text_range.is_empty();

            if is_range_contained_by_range(&segment.text_range, &selection) {
                if segment_is_empty {
                    const EMPTY_LINE_SELECTION_WIDTH: Pixels = px(6.);
                    elements.push(PrepaintElement::Quad(fill(
                        Bounds::from_corners(
                            inner_bounds.origin + point(Pixels::ZERO, line_y),
                            inner_bounds.origin
                                + point(EMPTY_LINE_SELECTION_WIDTH, line_y + line_height),
                        ),
                        selection_color,
                    )));
                } else {
                    let offset_corners = build_quad_over_text(
                        &selection,
                        segment,
                        line_y,
                        line_height,
                        Pixels::ZERO,
                    );
                    elements.extend(PrepaintElement::build_quads(
                        offset_corners,
                        inner_bounds.origin,
                        selection_color,
                    ));
                }
            }

            if !segment_is_empty && let Some(ime_range) = &ime_range {
                if !ime_range.is_empty()
                    && is_range_contained_by_range(&segment.text_range, &ime_range)
                {
                    const MARKED_TEXT_UNDERLINE_THICKNESS: f32 = 2.0;
                    let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
                    let underline_offset = line_height - underline_thickness;

                    let offset_corners = build_quad_over_text(
                        &ime_range,
                        segment,
                        line_y,
                        line_height,
                        underline_offset,
                    );
                    elements.extend(PrepaintElement::build_quads(
                        offset_corners,
                        inner_bounds.origin,
                        selection_color,
                    ));
                }
            }

            let is_cursor_in_line = if segment_is_empty {
                caret_pos == segment.text_range.start
            } else {
                segment.text_range.contains(&caret_pos) || caret_pos == segment.text_range.end
            };
            if is_cursor_in_line && let Some(wrapped) = &segment.wrapped_line {
                let local_offset = caret_pos.saturating_sub(segment.text_range.start);
                let caret_px = wrapped
                    .position_for_index(local_offset, line_height)
                    .unwrap_or_default();
                carent_point = caret_px + point(Pixels::ZERO, line_y);
            }
        }

        let is_focused = focus_handle.is_focused(window);
        if is_focused && cursor_visible {
            const CURSOR_WIDTH: f32 = 2.0;
            let quad = fill(
                Bounds::new(
                    inner_bounds.origin + carent_point - scroll_offset,
                    size(gpui::px(CURSOR_WIDTH), line_height),
                ),
                caret_color,
            );
            elements.push(PrepaintElement::Quad(quad));
        }

        PrepaintState {
            hitbox,
            inner_bounds,
            focus_handle,
            elements,
            scroll_offset,
            caret_visible: cursor_visible,
        }
    }

    fn shared_paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut LayoutState<Self::State>,
        prepaint: &mut PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) where
        Self::State:
            for<'app> EditableTextActionHandler<'app, Context = Context<'app, Self::State>>,
    {
        if let Some(hitbox) = &prepaint.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let inner_bounds = prepaint.inner_bounds;
        let bounds_origin = bounds.origin;
        let perform_paint = |style: &Style, window: &mut Window, cx: &mut App| {
            if style.display == Display::None {
                return;
            }

            // NOTE: Skip when disabled
            let ime_handler = ElementInputHandler::new(inner_bounds, request_layout.state.clone());
            window.handle_input(&prepaint.focus_handle, ime_handler, cx);

            let get_relative_position = move |position: Point<Pixels>| {
                // Converts a screen position to a position relative to the text area origin,
                // adjusted for scroll offset.
                let scroll_distance = gpui::px(0.); // TODO: STUB
                (position - bounds_origin)
                    .apply_along(Axis::Horizontal, |pos| pos + scroll_distance)
            };
            window.on_mouse_event({
                let state = request_layout.state.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if !bounds.contains(&event.position) {
                        return;
                    }
                    if event.button != MouseButton::Left {
                        return;
                    }

                    let text_position = get_relative_position(event.position);
                    state.update(cx, |state, cx| {
                        state.on_mouse_down(event, text_position, window, cx);
                    });
                }
            });
            window.on_mouse_event({
                let state = request_layout.state.clone();
                move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if event.button != MouseButton::Left {
                        return;
                    }

                    state.update(cx, |state, cx| {
                        state.on_mouse_up(event, window, cx);
                    });
                }
            });
            window.on_mouse_event({
                let state = request_layout.state.clone();
                move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }

                    let text_position = get_relative_position(event.position);
                    state.update(cx, |state, cx| {
                        state.on_mouse_move(event, text_position, window, cx);
                    });
                }
            });
            window.on_mouse_event({
                let state = request_layout.state.clone();
                /*
                let content_size = match axis {
                    gpui::Axis::Horizontal => {
                        let state = input.read(cx);
                        let line = state.lines().first();
                        let line = line.and_then(|l| l.wrapped_line.as_ref());
                        line.map(|w| w.width()).unwrap_or(px(0.))
                    }
                    gpui::Axis::Vertical => input.read(cx).total_content_height(),
                };
                let max_scroll = (content_size - bounds.size.along(axis)).max(px(0.));
                */
                // TODO: Scroll mouse wheel
                move |event: &ScrollWheelEvent, phase, _window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    if !inner_bounds.contains(&event.position) {
                        return;
                    }

                    // use shift to alter horizontal scroll on text area
                    //event.modifiers.shift;

                    /*
                    let pixel_delta = event.delta.pixel_delta(px(20.));
                    state.update(cx, |state, cx| {
                        let delta = match axis {
                            gpui::Axis::Horizontal => pixel_delta.y,
                            gpui::Axis::Vertical => {
                                if pixel_delta.x.abs() > pixel_delta.y.abs() {
                                    pixel_delta.x
                                } else {
                                    pixel_delta.y
                                }
                            }
                        };
                        state.apply_scroll_delta(delta, max_scroll);
                        cx.notify();
                    });
                    */
                }
            });

            let inner_bounds_mask = Some(ContentMask {
                bounds: inner_bounds,
            });
            window.with_content_mask(inner_bounds_mask, |window| {
                let line_h = window.line_height();
                let mut lines = Vec::with_capacity(prepaint.elements.len());
                for element in prepaint.elements.drain(..) {
                    match element {
                        PrepaintElement::Line { line, point, align } => {
                            let _ = line.paint(point, line_h, align, Some(bounds), window, cx);
                            lines.push(line);
                        }
                        PrepaintElement::Quad(quad) => window.paint_quad(quad),
                    }
                }
            });
        };
        self.interactivity().paint(
            global_id,
            inspector_id,
            bounds.clone(),
            prepaint.hitbox.as_ref(),
            window,
            cx,
            perform_paint,
        );
    }
}

fn build_quad_over_text(
    containing_range: &Range<usize>,
    segment: &TextLineSegment,
    line_y: Pixels,
    line_height: Pixels,
    offset_y: Pixels,
) -> Vec<(Point<Pixels>, Point<Pixels>)> {
    let Some(wrapped) = &segment.wrapped_line else {
        return vec![];
    };

    let line_start = segment.text_range.start;
    let line_end = segment.text_range.end;

    let subrange_start = containing_range.start.max(line_start) - line_start;
    let subrange_end = containing_range.end.min(line_end) - line_start;

    let start_pos = wrapped
        .position_for_index(subrange_start, line_height)
        .unwrap_or_default();
    let end_pos = wrapped
        .position_for_index(subrange_end, line_height)
        .unwrap_or_else(|| {
            let last_line_y = line_height * (segment.num_visual_lines - 1) as f32;
            point(wrapped.width(), last_line_y)
        });

    let start_visual_line = (start_pos.y / line_height).floor() as usize;
    let end_visual_line = (end_pos.y / line_height).floor() as usize;

    if start_visual_line == end_visual_line {
        vec![(
            point(start_pos.x, line_y + start_pos.y + offset_y),
            point(end_pos.x, line_y + start_pos.y + line_height),
        )]
    } else {
        let line_width = wrapped.width();
        let middle_lines = (start_visual_line + 1)..end_visual_line;
        let mut quad_corners = Vec::with_capacity(middle_lines.end - middle_lines.start + 2);

        quad_corners.push((
            point(start_pos.x, line_y + start_pos.y + offset_y),
            point(line_width, line_y + start_pos.y + line_height),
        ));

        // Middle visual lines
        for visual_line in (start_visual_line + 1)..end_visual_line {
            let y = line_height * visual_line as f32;
            quad_corners.push((
                point(Pixels::ZERO, line_y + y + offset_y),
                point(line_width, line_y + y + line_height),
            ));
        }

        // Last visual line
        quad_corners.push((
            point(Pixels::ZERO, line_y + end_pos.y + offset_y),
            point(end_pos.x, line_y + end_pos.y + line_height),
        ));

        quad_corners
    }
}
