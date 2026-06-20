use std::{ops::Range, sync::Arc};

use crate::editable_text::{
    EditableInputActionElement, EditableTextActionHandler, InitStorage, StateBackedElement,
    TextInputLayoutData, TextInputState, TextLayoutWrapping, TextLineSegment,
};
use gpui::{
    Along, App, Axis, Bounds, ContentMask, CursorStyle, DispatchPhase, Display, Element, ElementId,
    ElementInputHandler, Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior, Hsla,
    InteractiveElement, Interactivity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style,
    StyleRefinement, Styled, TextAlign, TextRun, TextStyle, Window, WrappedLine, fill, point, px,
    size,
};
use smallvec::SmallVec;

#[track_caller]
pub fn input(id: impl Into<ElementId>) -> TextInputElement {
    let mut this = TextInputElement {
        id: id.into(),
        placeholder: None,
        interactivity: Interactivity::new(),
        init_storage: InitStorage::default(),
    };
    this = this.key_context(super::DEFAULT_INPUT_CONTEXT);
    this.register_actions();
    this
}

// TODO: Disabled flag/state?
pub struct TextInputElement {
    id: ElementId,
    placeholder: Option<SharedString>,
    interactivity: Interactivity,
    init_storage: InitStorage,
}

impl InteractiveElement for TextInputElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Styled for TextInputElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for TextInputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl EditableInputActionElement for TextInputElement {}
impl super::StateBackedElement for TextInputElement {
    type State = TextInputState;
    type InitProps = (ElementId, InitStorage);

    fn init_props(&self) -> Self::InitProps {
        (self.id.clone(), self.init_storage.clone())
    }

    fn get_or_init_state(
        init_props: &Self::InitProps,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<TextInputState> {
        // Get the state from the app using the element's id as the key.
        // If it doesnt exist, initialize a new state with the user's desired storage medium.
        window.use_keyed_state(init_props.0.clone(), cx, |_window, cx| {
            TextInputState::new(init_props.1.exec(cx), cx)
        })
    }
}

enum PrepaintElement {
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

pub mod element {
    use smallvec::SmallVec;

    use super::*;

    #[doc(hidden)]
    pub struct LayoutState {
        pub state: Entity<TextInputState>,
        pub text_style: TextStyle,
    }

    #[doc(hidden)]
    pub struct PrepaintState {
        pub hitbox: Option<Hitbox>,
        pub focus_handle: FocusHandle,
        pub(super) elements: SmallVec<[PrepaintElement; 3]>,
        pub scroll_offset: Point<Pixels>,
        pub caret_visible: bool,
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = element::LayoutState;
    type PrepaintState = element::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut resolved_text_style = None;

        let state = self.get_state(window, cx);

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let style = element_style.clone();
                    // TODO: Does this need to propagate the line_height as the element's height?
                    window.request_layout(style, None, cx)
                })
            },
        );

        let layout_state = Self::RequestLayoutState {
            state,
            text_style: resolved_text_style.unwrap_or_else(|| window.text_style()),
        };
        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        // TODO: no wrapping in single-line
        let wrap_width = Some(bounds.size.width);

        let wrapping = TextLayoutWrapping::new(request_layout.text_style.clone(), wrap_width);
        let showing_placeholder = request_layout.state.update(cx, |state, _cx| {
            let text_value = state.storage().content_utf8();
            let is_empty = text_value.is_empty();
            let display_text = match is_empty {
                false => text_value,
                true => self
                    .placeholder
                    .as_ref()
                    .map(SharedString::as_str)
                    .unwrap_or_default(),
            };

            state.layout_data_mut().bounds = bounds;
            state.apply_wrapping(wrapping, display_text, window);
            is_empty
        });

        let input = request_layout.state.read(cx);

        let focus_handle = input.focus_handle(cx);
        let caret_pos = input.caret_pos();
        let selection = input.selected_range();
        let ime_range = input.marked_range();
        // TODO: Cursor blinking
        let cursor_visible = true; // input.cursor_visible();

        let text_color = Hsla::white(); // TODO: as an element param
        let placeholder_color = Hsla::black().opacity(0.5); // TODO: as an element param
        let selection_color = Hsla::blue().opacity(0.5); // TODO: as an element param
        let caret_color = Hsla::white(); // TODO: as an element param

        let mut elements = SmallVec::new();

        // TODO: how do we enable scrolling? overflow on interactivity?
        let (hitbox, scroll_offset) = self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_style, scroll_offset, hitbox, window, _cx| {
                let hitbox =
                    hitbox.or_else(|| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal)));
                (hitbox, scroll_offset)
            },
        );

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
        for segment in input.line_segments() {
            let line_distance_from_top = segment.pos_y * line_height;
            let line_y = line_distance_from_top - scroll_offset.y;
            let line_bottom = line_y + line_height * segment.num_visual_lines as f32;
            let line_visible = line_bottom >= Pixels::ZERO && line_y <= bounds.size.height;
            if !line_visible {
                continue;
            }

            // TODO: First render all lines (underlines for IME), then all selections, then cursor if no selection

            if let Some(wrapped) = &segment.wrapped_line {
                let point = bounds.origin + point(Pixels::ZERO, line_y);
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
                            bounds.origin + point(Pixels::ZERO, line_y),
                            bounds.origin + point(EMPTY_LINE_SELECTION_WIDTH, line_y + line_height),
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
                        bounds.origin,
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
                        bounds.origin,
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

        let has_selection = !selection.is_empty() && !showing_placeholder;
        let is_focused = focus_handle.is_focused(window);
        if !has_selection && is_focused && cursor_visible {
            const CURSOR_WIDTH: f32 = 2.0;
            let quad = fill(
                Bounds::new(
                    bounds.origin + carent_point - scroll_offset,
                    size(gpui::px(CURSOR_WIDTH), line_height),
                ),
                caret_color,
            );
            elements.push(PrepaintElement::Quad(quad));
        }

        Self::PrepaintState {
            hitbox,
            focus_handle,
            elements,
            scroll_offset,
            caret_visible: cursor_visible,
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<gpui::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        if let Some(hitbox) = &prepaint.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let perform_paint = |style: &Style, window: &mut Window, cx: &mut App| {
            if style.display == Display::None {
                return;
            }

            // NOTE: Skip when disabled
            let ime_handler = ElementInputHandler::new(bounds, request_layout.state.clone());
            window.handle_input(&prepaint.focus_handle, ime_handler, cx);

            let get_relative_position = {
                let bounds = bounds.clone();
                move |position: Point<Pixels>| {
                    // Converts a screen position to a position relative to the text area origin,
                    // adjusted for scroll offset.
                    let scroll_distance = gpui::px(0.); // TODO: STUB
                    (position - bounds.origin)
                        .apply_along(Axis::Horizontal, |pos| pos + scroll_distance)
                }
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
                    if !bounds.contains(&event.position) {
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

            window.with_content_mask(Some(ContentMask { bounds }), |window| {
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

                // TODO: Render marked IME underlines
            });
        };
        self.interactivity.paint(
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
