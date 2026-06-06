use crate::input::{Input, InputLineLayout, InputState, PaintColors};
use gpui::{
    Along, App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, Focusable, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, LayoutId, Length, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollWheelEvent, SharedString, Style, TextAlign, TextRun,
    TextStyle, Window, WrappedLine, fill, point, px, relative, size,
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
                let layout = self.input.read(cx).get_layout();
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let mut layout_style = element_style.clone();
                    if matches!(layout, super::InputLayout::MultiLine) {
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

        (
            layout_id,
            InputLayoutState {
                text_style: resolved_text_style.unwrap_or_else(|| window.text_style()),
            },
        )
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

        let wrap_width = match self.input.read(cx).get_layout() {
            super::InputLayout::SingleLine => px(100000.),
            super::InputLayout::MultiLine => bounds.size.width,
        };

        self.input.update(cx, |input, _cx| {
            input.available_height = bounds.size.height;
            input.available_width = bounds.size.width;
            input.update_line_layouts(wrap_width, line_height, &layout_state.text_style, window);
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

        // TODO: refactor cursor_visible so it is clear that it is called on_paint
        let cursor_visible = self
            .input
            .update(cx, |input, cx| input.cursor_visible(is_focused, cx));

        let perform_paint = |_style: &Style, window: &mut Window, cx: &mut App| {
            let precomputed_first_line = match (snapshot.layout, snapshot.line_layouts.first()) {
                (
                    super::InputLayout::SingleLine,
                    Some(InputLineLayout {
                        wrapped_line: Some(wrapped_line),
                        ..
                    }),
                ) => Some(PrecomputedLinePosition::new(
                    &snapshot.content,
                    &**wrapped_line,
                    snapshot.line_height,
                )),
                _ => None,
            };
            let context = PaintContext {
                snapshot,
                is_focused,
                bounds,
                text_style: &text_style,
                placeholder: placeholder.as_ref(),
                colors: &colors,
                cursor_visible,
                precomputed_first_line,
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

struct InputStateSnapshot {
    layout: super::InputLayout,
    content: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    cursor_offset: usize,
    line_layouts: Vec<InputLineLayout>,
    scroll_offset: Pixels,
    line_height: Pixels,
}
impl InputStateSnapshot {
    fn new(entity: &Entity<InputState>, cx: &App) -> Self {
        let input_state = entity.read(cx);
        let selected_range = input_state.selected_range().clone();
        let marked_range = input_state.marked_range().cloned();
        let cursor_offset = input_state.cursor_offset();
        let line_layouts = input_state.line_layouts.clone();
        let scroll_offset = input_state.scroll_offset;
        let line_height = input_state.line_height;
        Self {
            layout: input_state.get_layout(),
            content: input_state.content().clone(),
            selected_range,
            marked_range,
            cursor_offset,
            line_layouts,
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
    colors: &'app PaintColors,
    cursor_visible: bool,
    precomputed_first_line: Option<PrecomputedLinePosition>,
}

impl<'app> PaintContext<'app> {
    pub fn process_mouse_events(
        &self,
        entity: &Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let axis = self.snapshot.layout.axis();
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
                    let line = state.line_layouts.first();
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

    pub fn paint(&self, window: &mut Window, cx: &mut App) {
        if !self.snapshot.selected_range.is_empty() {
            self.paint_selection(window);
        }

        if self.snapshot.content.is_empty() {
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
        match self.snapshot.layout {
            super::InputLayout::MultiLine => {
                for line in &self.snapshot.line_layouts {
                    let line_y = line.y_offset - self.snapshot.scroll_offset;

                    if !is_line_visible(
                        line_y,
                        self.snapshot.line_height,
                        line.visual_line_count,
                        self.bounds.size.height,
                    ) {
                        continue;
                    }

                    if !line_intersects_range(&line.text_range, &self.snapshot.selected_range) {
                        continue;
                    }

                    if line.text_range.is_empty() {
                        const EMPTY_LINE_SELECTION_WIDTH: Pixels = px(6.);
                        paint_selection_quad(
                            window,
                            self.colors.selection,
                            &self.bounds,
                            point(px(0.), line_y),
                            point(
                                EMPTY_LINE_SELECTION_WIDTH,
                                line_y + self.snapshot.line_height,
                            ),
                        );
                    } else if let Some(wrapped) = &line.wrapped_line {
                        let line_start = line.text_range.start;
                        let line_end = line.text_range.end;

                        let sel_start =
                            self.snapshot.selected_range.start.max(line_start) - line_start;
                        let sel_end = self.snapshot.selected_range.end.min(line_end) - line_start;

                        let start_pos = wrapped
                            .position_for_index(sel_start, self.snapshot.line_height)
                            .unwrap_or(point(px(0.), px(0.)));
                        let end_pos = wrapped
                            .position_for_index(sel_end, self.snapshot.line_height)
                            .unwrap_or_else(|| {
                                let last_line_y =
                                    self.snapshot.line_height * (line.visual_line_count - 1) as f32;
                                point(wrapped.width(), last_line_y)
                            });

                        let start_visual_line =
                            compute_visual_line_index(start_pos.y, self.snapshot.line_height);
                        let end_visual_line =
                            compute_visual_line_index(end_pos.y, self.snapshot.line_height);

                        if start_visual_line == end_visual_line {
                            paint_selection_quad(
                                window,
                                self.colors.selection,
                                &self.bounds,
                                point(start_pos.x, line_y + start_pos.y),
                                point(end_pos.x, line_y + start_pos.y + self.snapshot.line_height),
                            );
                        } else {
                            let line_width = wrapped.width();

                            // First visual line
                            paint_selection_quad(
                                window,
                                self.colors.selection,
                                &self.bounds,
                                point(start_pos.x, line_y + start_pos.y),
                                point(line_width, line_y + start_pos.y + self.snapshot.line_height),
                            );

                            // Middle visual lines
                            for visual_line in (start_visual_line + 1)..end_visual_line {
                                let y = self.snapshot.line_height * visual_line as f32;
                                paint_selection_quad(
                                    window,
                                    self.colors.selection,
                                    &self.bounds,
                                    point(px(0.), line_y + y),
                                    point(line_width, line_y + y + self.snapshot.line_height),
                                );
                            }

                            // Last visual line
                            paint_selection_quad(
                                window,
                                self.colors.selection,
                                &self.bounds,
                                point(px(0.), line_y + end_pos.y),
                                point(end_pos.x, line_y + end_pos.y + self.snapshot.line_height),
                            );
                        }
                    }
                }
            }
            super::InputLayout::SingleLine => {
                let precomputed = self
                    .precomputed_first_line
                    .as_ref()
                    .expect("missing precomputed single-line");
                let start_x = pos_in_string_for_char_index(
                    &self.snapshot.content,
                    &precomputed.char_positions,
                    self.snapshot.selected_range.start,
                    &precomputed.text_width,
                ) - self.snapshot.scroll_offset;
                let end_x = pos_in_string_for_char_index(
                    &self.snapshot.content,
                    &precomputed.char_positions,
                    self.snapshot.selected_range.end,
                    &precomputed.text_width,
                ) - self.snapshot.scroll_offset;

                let y_offset =
                    (self.bounds.size.height - self.snapshot.line_height).max(px(0.)) / 2.0;

                paint_selection_quad(
                    window,
                    self.colors.selection,
                    &self.bounds,
                    point(start_x, y_offset),
                    point(end_x, y_offset + self.snapshot.line_height),
                );
            }
        }
    }

    fn paint_placeholder(&self, window: &mut Window, cx: &mut App) {
        let baseline = matches!(self.snapshot.layout, super::InputLayout::SingleLine);
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
        if baseline {
            let y_offset = (self.bounds.size.height - line_height).max(px(0.)) / 2.0;
            paint_origin.y += y_offset;
        }

        let _ = shaped_line.paint(paint_origin, line_height, TextAlign::Left, None, window, cx);
    }

    fn paint_text(&self, window: &mut Window, cx: &mut App) {
        match self.snapshot.layout {
            super::InputLayout::MultiLine => {
                for line_layout in &self.snapshot.line_layouts {
                    let line_y = line_layout.y_offset - self.snapshot.scroll_offset;

                    if !is_line_visible(
                        line_y,
                        self.snapshot.line_height,
                        line_layout.visual_line_count,
                        self.bounds.size.height,
                    ) {
                        continue;
                    }

                    if let Some(wrapped) = &line_layout.wrapped_line {
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
            }
            super::InputLayout::SingleLine => {
                let Some(line_layout) = self.snapshot.line_layouts.first() else {
                    return;
                };
                let Some(wrapped_line) = &line_layout.wrapped_line else {
                    return;
                };

                let y_offset =
                    (self.bounds.size.height - self.snapshot.line_height).max(px(0.)) / 2.0;
                let paint_origin = point(
                    self.bounds.origin.x - self.snapshot.scroll_offset,
                    self.bounds.origin.y + y_offset,
                );

                let _ = wrapped_line.paint(
                    paint_origin,
                    self.snapshot.line_height,
                    TextAlign::Left,
                    Some(self.bounds),
                    window,
                    cx,
                );
            }
        }
    }

    fn paint_marked_underline(&self, window: &mut Window) {
        let Some(marked_range) = &self.snapshot.marked_range else {
            return;
        };
        if marked_range.is_empty() {
            return;
        }
        match self.snapshot.layout {
            super::InputLayout::MultiLine => {
                let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
                let underline_offset = self.snapshot.line_height - underline_thickness;

                for line in &self.snapshot.line_layouts {
                    let line_y = line.y_offset - self.snapshot.scroll_offset;

                    if !is_line_visible(
                        line_y,
                        self.snapshot.line_height,
                        line.visual_line_count,
                        self.bounds.size.height,
                    ) {
                        continue;
                    }

                    if !line_intersects_range(&line.text_range, marked_range) {
                        continue;
                    }

                    if line.text_range.is_empty() {
                        continue;
                    }

                    if let Some(wrapped) = &line.wrapped_line {
                        let line_start = line.text_range.start;
                        let line_end = line.text_range.end;

                        let mark_start = marked_range.start.max(line_start) - line_start;
                        let mark_end = marked_range.end.min(line_end) - line_start;

                        let start_pos = wrapped
                            .position_for_index(mark_start, self.snapshot.line_height)
                            .unwrap_or(point(px(0.), px(0.)));
                        let end_pos = wrapped
                            .position_for_index(mark_end, self.snapshot.line_height)
                            .unwrap_or_else(|| {
                                let last_line_y =
                                    self.snapshot.line_height * (line.visual_line_count - 1) as f32;
                                point(wrapped.width(), last_line_y)
                            });

                        let start_visual_line =
                            compute_visual_line_index(start_pos.y, self.snapshot.line_height);
                        let end_visual_line =
                            compute_visual_line_index(end_pos.y, self.snapshot.line_height);

                        if start_visual_line == end_visual_line {
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    point(
                                        self.bounds.left() + start_pos.x,
                                        self.bounds.top() + line_y + start_pos.y + underline_offset,
                                    ),
                                    point(
                                        self.bounds.left() + end_pos.x,
                                        self.bounds.top()
                                            + line_y
                                            + start_pos.y
                                            + self.snapshot.line_height,
                                    ),
                                ),
                                self.colors.cursor,
                            ));
                        } else {
                            // First visual line
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    point(
                                        self.bounds.left() + start_pos.x,
                                        self.bounds.top() + line_y + start_pos.y + underline_offset,
                                    ),
                                    point(
                                        self.bounds.left() + wrapped.width(),
                                        self.bounds.top()
                                            + line_y
                                            + start_pos.y
                                            + self.snapshot.line_height,
                                    ),
                                ),
                                self.colors.cursor,
                            ));

                            // Middle visual lines
                            for visual_line in (start_visual_line + 1)..end_visual_line {
                                let y = self.snapshot.line_height * visual_line as f32;
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        point(
                                            self.bounds.left(),
                                            self.bounds.top() + line_y + y + underline_offset,
                                        ),
                                        point(
                                            self.bounds.left() + wrapped.width(),
                                            self.bounds.top()
                                                + line_y
                                                + y
                                                + self.snapshot.line_height,
                                        ),
                                    ),
                                    self.colors.cursor,
                                ));
                            }

                            // Last visual line
                            window.paint_quad(fill(
                                Bounds::from_corners(
                                    point(
                                        self.bounds.left(),
                                        self.bounds.top() + line_y + end_pos.y + underline_offset,
                                    ),
                                    point(
                                        self.bounds.left() + end_pos.x,
                                        self.bounds.top()
                                            + line_y
                                            + end_pos.y
                                            + self.snapshot.line_height,
                                    ),
                                ),
                                self.colors.cursor,
                            ));
                        }
                    }
                }
            }
            super::InputLayout::SingleLine => {
                let Some(precomputed) = &self.precomputed_first_line else {
                    return;
                };
                let start_x = pos_in_string_for_char_index(
                    &self.snapshot.content,
                    &precomputed.char_positions,
                    marked_range.start,
                    &precomputed.text_width,
                ) - self.snapshot.scroll_offset;
                let end_x = pos_in_string_for_char_index(
                    &self.snapshot.content,
                    &precomputed.char_positions,
                    marked_range.end,
                    &precomputed.text_width,
                ) - self.snapshot.scroll_offset;

                let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
                let y_offset =
                    (self.bounds.size.height - self.snapshot.line_height).max(px(0.)) / 2.0;
                let underline_y =
                    self.bounds.top() + y_offset + self.snapshot.line_height - underline_thickness;

                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(self.bounds.left() + start_x, underline_y),
                        point(
                            self.bounds.left() + end_x,
                            underline_y + underline_thickness,
                        ),
                    ),
                    self.colors.cursor,
                ));
            }
        }
    }

    fn paint_cursor(&self, window: &mut Window) {
        match self.snapshot.layout {
            super::InputLayout::MultiLine => {
                for line in &self.snapshot.line_layouts {
                    let line_y = line.y_offset - self.snapshot.scroll_offset;

                    if !is_line_visible(
                        line_y,
                        self.snapshot.line_height,
                        line.visual_line_count,
                        self.bounds.size.height,
                    ) {
                        continue;
                    }

                    // Since range is non-inclusive of the end value we need to check for it explicitly
                    let is_cursor_in_line = if line.text_range.is_empty() {
                        self.snapshot.cursor_offset == line.text_range.start
                    } else {
                        line.text_range.contains(&self.snapshot.cursor_offset)
                            || self.snapshot.cursor_offset == line.text_range.end
                    };

                    if !is_cursor_in_line {
                        continue;
                    }

                    let cursor_position = if let Some(wrapped) = &line.wrapped_line {
                        let local_offset = self
                            .snapshot
                            .cursor_offset
                            .saturating_sub(line.text_range.start);
                        wrapped
                            .position_for_index(local_offset, self.snapshot.line_height)
                            .unwrap_or(point(px(0.), px(0.)))
                    } else {
                        point(px(0.), px(0.))
                    };

                    window.paint_quad(fill(
                        Bounds::new(
                            point(
                                self.bounds.left() + cursor_position.x,
                                self.bounds.top() + line_y + cursor_position.y,
                            ),
                            size(px(CURSOR_WIDTH), self.snapshot.line_height),
                        ),
                        self.colors.cursor,
                    ));
                    break;
                }
            }
            super::InputLayout::SingleLine => {
                let Some(precomputed) = &self.precomputed_first_line else {
                    return;
                };
                let cursor_x = pos_in_string_for_char_index(
                    &self.snapshot.content,
                    &precomputed.char_positions,
                    self.snapshot.cursor_offset,
                    &precomputed.text_width,
                ) - self.snapshot.scroll_offset;

                let y_offset =
                    (self.bounds.size.height - self.snapshot.line_height).max(px(0.)) / 2.0;

                window.paint_quad(fill(
                    Bounds::new(
                        point(self.bounds.left() + cursor_x, self.bounds.top() + y_offset),
                        size(px(CURSOR_WIDTH), self.snapshot.line_height),
                    ),
                    self.colors.cursor,
                ));
            }
        }
    }
}

fn is_line_visible(
    line_y: Pixels,
    line_height: Pixels,
    visual_line_count: usize,
    visible_height: Pixels,
) -> bool {
    let line_bottom = line_y + line_height * visual_line_count as f32;
    line_bottom >= px(0.) && line_y <= visible_height
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

fn compute_visual_line_index(y: Pixels, line_height: Pixels) -> usize {
    (y / line_height).floor() as usize
}

struct PrecomputedLinePosition {
    text_width: Pixels,
    char_positions: Vec<Pixels>,
}
impl PrecomputedLinePosition {
    fn new(string: &str, line: &WrappedLine, line_height: Pixels) -> Self {
        let text_width = line.width();
        let mut char_positions = Vec::new();

        let mut idx = 0;
        for ch in string.chars() {
            if let Some(pos) = line.position_for_index(idx, line_height) {
                char_positions.push(pos.x);
            } else {
                char_positions.push(text_width);
            }
            idx += ch.len_utf8();
        }
        char_positions.push(text_width);

        Self {
            text_width,
            char_positions,
        }
    }
}

fn pos_in_string_for_char_index<'chars>(
    content: &SharedString,
    char_positions: &'chars Vec<Pixels>,
    index: usize,
    default: &Pixels,
) -> Pixels {
    let char_index = content[..index.min(content.len())].chars().count();
    char_positions.get(char_index).unwrap_or(default).clone()
}

fn paint_selection_quad(
    window: &mut Window,
    color: Hsla,
    bounds: &Bounds<Pixels>,
    offset_start: Point<Pixels>,
    offset_end: Point<Pixels>,
) {
    let top_left = point(bounds.left(), bounds.top());
    window.paint_quad(fill(
        Bounds::from_corners(top_left + offset_start, top_left + offset_end),
        color,
    ));
}
