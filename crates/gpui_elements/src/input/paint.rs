use crate::input::{Input, InputColors, InputLayoutData, InputLogicalLine, InputState};
use gpui::{
    Along, App, Axis, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, Focusable, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, LayoutId, Length, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollWheelEvent, SharedString, Style, TextAlign, TextRun,
    TextStyle, Window, fill, point, px, relative, size,
};
use std::ops::Range;

const CURSOR_WIDTH: f32 = 2.0;
const MARKED_TEXT_UNDERLINE_THICKNESS: f32 = 2.0;

pub struct InputLayoutState {
    text_style: TextStyle,
}

pub struct InputPrepaintState {
    hitbox: Option<Hitbox>,
}

impl Element for Input {
    type RequestLayoutState = InputLayoutState;
    type PrepaintState = InputPrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut resolved_text_style = None;

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                let layout = self.input.read(cx).layout_style();
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let mut layout_style = element_style.clone();
                    if matches!(layout, super::InputLayoutStyle::MultiLine) {
                        if let Length::Auto = layout_style.size.width {
                            layout_style.size.width = relative(1.).into();
                        }
                        if let Length::Auto = layout_style.size.height {
                            layout_style.size.height = relative(1.).into();
                        }
                    }
                    window.request_layout(layout_style, None, cx)
                })
            },
        );

        let layout_state = InputLayoutState {
            text_style: resolved_text_style.unwrap_or_else(|| window.text_style()),
        };
        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout_state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let line_height = layout_state
            .text_style
            .line_height_in_pixels(window.rem_size());

        let wrap_width = match self.input.read(cx).layout_style() {
            super::InputLayoutStyle::SingleLine => None,
            super::InputLayoutStyle::MultiLine => Some(bounds.size.width),
        };

        self.input.update(cx, |input, _cx| {
            let dirty = input.layout_data.dirty
                || input.layout_data.wrap_width != wrap_width
                || input.layout_data.text_style != layout_state.text_style;
            let layout_data = InputLayoutData {
                text_style: layout_state.text_style.clone(),
                line_height,
                wrap_width,
                available_size: bounds.size,
                dirty: false,
            };
            input.layout_data = layout_data;
            if dirty {
                input.logical_lines =
                    InputState::build_logical_lines(input.content(), window, &input.layout_data);
                input.scroll_to_cursor();
            }
        });

        let hitbox = self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_style, _point, hitbox, window, _cx| {
                hitbox.or_else(|| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal)))
            },
        );

        InputPrepaintState { hitbox }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout_state: &mut Self::RequestLayoutState,
        prepaint_state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.focus_handle(cx);

        if let Some(hitbox) = &prepaint_state.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        let snapshot = InputStateSnapshot::new(&self.input, cx);
        let placeholder = self.placeholder.clone();
        let text_style = layout_state.text_style.clone();
        let is_focused = focus_handle.is_focused(window);
        let colors = self.colors;

        let is_cursor_visible = self.input.update(cx, |input, cx| {
            input.toggle_cursor_on_focus_change(is_focused, cx)
        });

        let perform_paint = |_style: &Style, window: &mut Window, cx: &mut App| {
            let context = PaintContext {
                snapshot,
                is_focused,
                bounds,
                text_style: &text_style,
                placeholder: placeholder.as_ref(),
                colors: &colors,
                cursor_visible: is_cursor_visible,
            };
            context.process_mouse_events(&self.input, window, cx);
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                context.paint(window, cx);
            });
        };
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            prepaint_state.hitbox.as_ref(),
            window,
            cx,
            perform_paint,
        );
    }
}

/// A minimal copy of InputState that is used during paint operations without needing to read from the entity in App multiple times in a single paint.
/// Ideally this struct is quite small.
struct InputStateSnapshot {
    layout_axis: Axis,
    should_center_placeholder: bool,
    show_placeholder: bool,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    cursor_position: usize,
    logical_lines: Vec<InputLogicalLine>,
    scroll_offset: Pixels,
    line_height: Pixels,
}
impl InputStateSnapshot {
    fn new(entity: &Entity<InputState>, cx: &App) -> Self {
        let input_state = entity.read(cx);
        let selected_range = input_state.selected_range().clone();
        let marked_range = input_state.marked_range().cloned();
        let cursor_position = input_state.cursor_position();
        let logical_lines = input_state.logical_lines.clone();
        let scroll_offset = input_state.scroll_offset;
        let line_height = input_state.line_height();
        let layout_axis = input_state.layout_style().axis();
        let should_center_placeholder = matches!(
            input_state.layout_style(),
            super::InputLayoutStyle::SingleLine
        );
        Self {
            layout_axis,
            should_center_placeholder,
            show_placeholder: input_state.content().is_empty(),
            selected_range,
            marked_range,
            cursor_position,
            logical_lines,
            scroll_offset,
            line_height,
        }
    }
}

struct PaintContext<'app> {
    snapshot: InputStateSnapshot,
    is_focused: bool,
    bounds: Bounds<Pixels>,
    text_style: &'app TextStyle,
    placeholder: Option<&'app SharedString>,
    colors: &'app InputColors,
    cursor_visible: bool,
}

impl<'app> PaintContext<'app> {
    pub fn process_mouse_events(
        &self,
        entity: &Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let axis = self.snapshot.layout_axis;
        let bounds = self.bounds;
        window.on_mouse_event({
            let input = entity.clone();
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

                input.update(cx, |input, cx| {
                    // Converts a screen position to a position relative to the text area origin, adjusted for scroll offset.
                    let text_position = (event.position - bounds.origin)
                        .apply_along(axis, |pos| pos + input.scroll_offset);
                    input.on_mouse_down(
                        text_position,
                        event.click_count,
                        event.modifiers.shift,
                        window,
                        cx,
                    );
                });
            }
        });
        window.on_mouse_event({
            let input = entity.clone();
            move |event: &MouseUpEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if event.button != MouseButton::Left {
                    return;
                }

                input.update(cx, |input, cx| {
                    input.on_mouse_up(cx);
                });
            }
        });
        window.on_mouse_event({
            let input = entity.clone();
            move |event: &MouseMoveEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }

                input.update(cx, |input, cx| {
                    // Converts a screen position to a position relative to the text area origin, adjusted for scroll offset.
                    let text_position = (event.position - bounds.origin)
                        .apply_along(axis, |pos| pos + input.scroll_offset);
                    input.on_mouse_move(text_position, cx);
                });
            }
        });
        window.on_mouse_event({
            let input = entity.clone();
            let content_size = match axis {
                gpui::Axis::Horizontal => {
                    let state = input.read(cx);
                    let line = state.logical_lines.first();
                    let line = line.and_then(|l| l.wrapped_line.as_ref());
                    line.map(|w| w.width()).unwrap_or(px(0.))
                }
                gpui::Axis::Vertical => input.read(cx).total_content_height(),
            };
            let max_scroll = (content_size - bounds.size.along(axis)).max(px(0.));
            move |event: &ScrollWheelEvent, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !bounds.contains(&event.position) {
                    return;
                }

                let pixel_delta = event.delta.pixel_delta(px(20.));
                input.update(cx, |input, cx| {
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
                    input.scroll_offset = (input.scroll_offset - delta).clamp(px(0.), max_scroll);
                    cx.notify();
                });
            }
        });
    }

    fn paint_bounds_quad(
        &self,
        window: &mut Window,
        color: Hsla,
        offset_start: Point<Pixels>,
        offset_end: Point<Pixels>,
    ) {
        let top_left = point(self.bounds.left(), self.bounds.top());
        window.paint_quad(fill(
            Bounds::from_corners(top_left + offset_start, top_left + offset_end),
            color,
        ));
    }

    pub fn paint(&self, window: &mut Window, cx: &mut App) {
        if !self.snapshot.selected_range.is_empty() {
            self.paint_selection(window);
        }

        if self.snapshot.show_placeholder {
            self.paint_placeholder(window, cx);
        } else {
            self.paint_text(window, cx);
        }

        self.paint_marked_underline(window);

        if self.is_focused && self.snapshot.selected_range.is_empty() && self.cursor_visible {
            self.paint_cursor(window);
        }
    }

    fn paint_selection(&self, window: &mut Window) {
        let one_line = self.snapshot.logical_lines.len() == 1;
        for line in &self.snapshot.logical_lines {
            let line_y = line.y_offset - self.snapshot.scroll_offset;

            if !one_line {
                if !self.is_line_visible(line) {
                    continue;
                }

                if !line_intersects_range(&line.text_range, &self.snapshot.selected_range) {
                    continue;
                }
            }

            if line.text_range.is_empty() {
                const EMPTY_LINE_SELECTION_WIDTH: Pixels = px(6.);
                self.paint_bounds_quad(
                    window,
                    self.colors.selection,
                    point(px(0.), line_y),
                    point(
                        EMPTY_LINE_SELECTION_WIDTH,
                        line_y + self.snapshot.line_height,
                    ),
                );
            } else {
                self.paint_line_range(
                    window,
                    line,
                    &self.snapshot.selected_range,
                    self.colors.selection,
                    px(0.),
                );
            }
        }
    }

    fn paint_placeholder(&self, window: &mut Window, cx: &mut App) {
        let Some(placeholder) = self.placeholder else {
            return;
        };
        if placeholder.is_empty() {
            return;
        }

        let run = TextRun {
            len: placeholder.len(),
            font: self.text_style.font(),
            color: self.colors.placeholder,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let font_size = self.text_style.font_size.to_pixels(window.rem_size());
        let shaped_line =
            window
                .text_system()
                .shape_line(placeholder.clone(), font_size, &[run], None);
        let line_height = self.text_style.line_height_in_pixels(window.rem_size());

        let mut paint_origin = self.bounds.origin;
        if self.snapshot.should_center_placeholder {
            let y_offset = (self.bounds.size.height - line_height).max(px(0.)) / 2.0;
            paint_origin.y += y_offset;
        }

        let _ = shaped_line.paint(paint_origin, line_height, TextAlign::Left, None, window, cx);
    }

    fn paint_text(&self, window: &mut Window, cx: &mut App) {
        for line_layout in &self.snapshot.logical_lines {
            let line_y = line_layout.y_offset - self.snapshot.scroll_offset;

            if !self.is_line_visible(line_layout) {
                continue;
            }

            let Some(wrapped) = &line_layout.wrapped_line else {
                continue;
            };

            let paint_pos = point(self.bounds.left(), self.bounds.top() + line_y);
            let _ = wrapped.paint(
                paint_pos,
                self.snapshot.line_height,
                TextAlign::Left,
                Some(self.bounds),
                window,
                cx,
            );
        }
    }

    fn paint_marked_underline(&self, window: &mut Window) {
        let Some(marked_range) = &self.snapshot.marked_range else {
            return;
        };
        if marked_range.is_empty() {
            return;
        }

        let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
        let underline_offset = self.snapshot.line_height - underline_thickness;
        for line in &self.snapshot.logical_lines {
            if !self.is_line_visible(line) {
                continue;
            }

            if !line_intersects_range(&line.text_range, marked_range) {
                continue;
            }

            if line.text_range.is_empty() {
                continue;
            }

            self.paint_line_range(
                window,
                line,
                marked_range,
                self.colors.marked,
                underline_offset,
            );
        }
    }

    fn find_cursor_position_in_layouts(&self) -> Point<Pixels> {
        for line in &self.snapshot.logical_lines {
            let line_y = line.y_offset - self.snapshot.scroll_offset;

            if !self.is_line_visible(line) {
                continue;
            }

            // Since range is non-inclusive of the end value we need to check for it explicitly
            let is_cursor_in_line = if line.text_range.is_empty() {
                self.snapshot.cursor_position == line.text_range.start
            } else {
                line.text_range.contains(&self.snapshot.cursor_position)
                    || self.snapshot.cursor_position == line.text_range.end
            };

            if !is_cursor_in_line {
                continue;
            }

            let Some(wrapped) = &line.wrapped_line else {
                return Point::default();
            };
            let local_offset = self
                .snapshot
                .cursor_position
                .saturating_sub(line.text_range.start);
            let cursor_pos = wrapped
                .position_for_index(local_offset, self.snapshot.line_height)
                .unwrap_or_default();
            return cursor_pos + point(px(0.), line_y);
        }
        Point::default()
    }

    fn paint_cursor(&self, window: &mut Window) {
        let cursor_pos = self.find_cursor_position_in_layouts();
        window.paint_quad(fill(
            Bounds::new(
                point(self.bounds.left(), self.bounds.top()) + cursor_pos,
                size(px(CURSOR_WIDTH), self.snapshot.line_height),
            ),
            self.colors.cursor,
        ));
    }

    fn is_line_visible(&self, line: &InputLogicalLine) -> bool {
        let line_y = line.y_offset - self.snapshot.scroll_offset;
        let line_bottom = line_y + self.snapshot.line_height * line.visual_line_count as f32;
        line_bottom >= px(0.) && line_y <= self.bounds.size.height
    }

    fn compute_visual_line_index(&self, y: Pixels) -> usize {
        (y / self.snapshot.line_height).floor() as usize
    }

    fn paint_line_range(
        &self,
        window: &mut Window,
        line: &InputLogicalLine,
        subrange: &Range<usize>,
        color: Hsla,
        quad_offset_y: Pixels,
    ) {
        let Some(wrapped) = &line.wrapped_line else {
            return;
        };

        let line_y = line.y_offset - self.snapshot.scroll_offset;

        let line_start = line.text_range.start;
        let line_end = line.text_range.end;

        let subrange_start = subrange.start.max(line_start) - line_start;
        let subrange_end = subrange.end.min(line_end) - line_start;

        let start_pos = wrapped
            .position_for_index(subrange_start, self.snapshot.line_height)
            .unwrap_or_default();
        let end_pos = wrapped
            .position_for_index(subrange_end, self.snapshot.line_height)
            .unwrap_or_else(|| {
                let last_line_y = self.snapshot.line_height * (line.visual_line_count - 1) as f32;
                point(wrapped.width(), last_line_y)
            });

        let start_visual_line = self.compute_visual_line_index(start_pos.y);
        let end_visual_line = self.compute_visual_line_index(end_pos.y);

        if start_visual_line == end_visual_line {
            self.paint_bounds_quad(
                window,
                color,
                point(start_pos.x, line_y + start_pos.y + quad_offset_y),
                point(end_pos.x, line_y + start_pos.y + self.snapshot.line_height),
            );
        } else {
            let line_width = wrapped.width();

            // First visual line
            self.paint_bounds_quad(
                window,
                color,
                point(start_pos.x, line_y + start_pos.y + quad_offset_y),
                point(line_width, line_y + start_pos.y + self.snapshot.line_height),
            );

            // Middle visual lines
            for visual_line in (start_visual_line + 1)..end_visual_line {
                let y = self.snapshot.line_height * visual_line as f32;
                self.paint_bounds_quad(
                    window,
                    color,
                    point(px(0.), line_y + y + quad_offset_y),
                    point(line_width, line_y + y + self.snapshot.line_height),
                );
            }

            // Last visual line
            self.paint_bounds_quad(
                window,
                color,
                point(px(0.), line_y + end_pos.y + quad_offset_y),
                point(end_pos.x, line_y + end_pos.y + self.snapshot.line_height),
            );
        }
    }
}

fn line_intersects_range(
    text_range: &std::ops::Range<usize>,
    selected_range: &std::ops::Range<usize>,
) -> bool {
    if text_range.is_empty() {
        selected_range.start <= text_range.start && selected_range.end > text_range.start
    } else {
        selected_range.end > text_range.start && selected_range.start < text_range.end
    }
}
