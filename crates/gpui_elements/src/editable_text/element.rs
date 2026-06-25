use crate::editable_text::{
    EditableTextState, InitStorage,
    actions::{DEFAULT_INPUT_CONTEXT, EditableTextActionElement, EditableTextActionHandler},
    layout::{TextInputLayoutData, TextLineSegment},
};
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Display, Element, ElementId, ElementInputHandler,
    Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior, Hsla, InteractiveElement,
    Interactivity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, SharedString, Size, StatefulInteractiveElement, Style,
    StyleRefinement, Styled, TextAlign, TextLayout, WeakEntity, Window, WrappedLine, fill, point,
    px, size,
};
use smallvec::SmallVec;
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

#[track_caller]
pub fn editable_text(id: impl Into<ElementId>) -> EditableTextElement {
    let mut this = EditableTextElement {
        interactivity: Interactivity::default(),
        state_entity: Rc::new(RefCell::new(WeakEntity::new_invalid())),
        supports_multiline: true,
        init_storage: InitStorage::default(),
        placeholder: None,
        accepts_input: true,
        colors: EditableTextColors::default(),
    };
    this.interactivity.element_id = Some(id.into());

    this = this.key_context(DEFAULT_INPUT_CONTEXT);
    this.register_actions();

    this
}

#[track_caller]
pub fn text_input(id: impl Into<ElementId>) -> EditableTextElement {
    editable_text(id).multiline(false)
}

#[track_caller]
pub fn text_area(id: impl Into<ElementId>) -> EditableTextElement {
    editable_text(id).multiline(true)
}

pub struct EditableTextElement {
    interactivity: Interactivity,
    // Populated on first render with an entity stored/attached to the view.
    // This reference is shared with the action handlers, which are processed between renders
    // and therefore cannot otherwise access state attached to the view.
    state_entity: Rc<RefCell<WeakEntity<EditableTextState>>>,
    init_storage: InitStorage,
    supports_multiline: bool,
    placeholder: Option<SharedString>,
    accepts_input: bool,
    colors: EditableTextColors,
}

/// EditableText styling that goes beyond what Style/StyleRefinement supports
struct EditableTextColors {
    /// Color of the placeholder text when the storage is empty.
    /// Could be reconceived as a refinement of text_color when the field is empty
    placeholder: Hsla,
    /// Color of the selection box.
    /// Could be driven by platform-provided styling?
    selection: Hsla,
    /// Color of the caret / text cursor
    caret: Hsla,
}
impl Default for EditableTextColors {
    fn default() -> Self {
        Self {
            placeholder: Hsla::white().opacity(0.5),
            selection: Hsla {
                h: 0.583,
                s: 0.519,
                l: 0.31,
                a: 0.5,
            },
            caret: Hsla::white(),
        }
    }
}

impl EditableTextElement {
    pub fn multiline(mut self, enabled: bool) -> Self {
        self.supports_multiline = enabled;
        self
    }

    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Swaps the default storage container (standard String) with a custom initializer of [`UnicodeTextStorage`].
    pub fn with_storage(mut self, fn_init: impl Into<InitStorage>) -> Self {
        self.init_storage = fn_init.into();
        self
    }

    /// Swaps the default storage container. The new initializer is a standard String using the provided value.
    ///
    /// Incompatible with [`with_storage`] (they establish the same internal value).
    /// If you initialize custom storage, you should be able to initialize its default value.
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        let storage = super::StringStorage::from(value.into());
        self.init_storage = InitStorage::new_typed(move |_cx| storage.clone());
        self
    }

    /// Configures whether the element can accept input (effectively is the element currently enabled).
    pub fn accepts_input(mut self, enabled: bool) -> Self {
        self.accepts_input = enabled;
        self
    }

    /// Sets the color of the placeholder text which is rendered when the element's stored text is empty.
    pub fn placeholder_color(mut self, color: Hsla) -> Self {
        self.colors.placeholder = color;
        self
    }

    /// Sets the color of the box highlighting selected text.
    pub fn selection_color(mut self, color: Hsla) -> Self {
        self.colors.selection = color;
        self
    }

    /// Sets the color of the caret / text-cursor.
    pub fn caret_color(mut self, color: Hsla) -> Self {
        self.colors.caret = color;
        self
    }
}

impl InteractiveElement for EditableTextElement {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

// forced implementation since the API for the element doesnt use Stateful<Element>
impl StatefulInteractiveElement for EditableTextElement {}

impl Styled for EditableTextElement {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl IntoElement for EditableTextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl EditableTextActionElement<EditableTextState> for EditableTextElement {
    fn state_entity_rc(&self) -> &Rc<RefCell<WeakEntity<EditableTextState>>> {
        &self.state_entity
    }
}

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

// NOTE: This value is not currently practical. Current estimation is:
// 1 for caret, 1 for ime, n for each real line that may be wrapped,
// n for selection (at least 1 per line that is wrapped that it encapsulates)
const STACK_ALLOCATED_ELEMENTS: usize = 3usize;

#[doc(hidden)]
pub struct LayoutState<State> {
    state: Entity<State>,
}

struct InteractivityPrepaint {
    hitbox: Option<Hitbox>,
    scroll_offset: Point<Pixels>,
    inner_bounds: Bounds<Pixels>,
    caret_visible: bool,
}

#[doc(hidden)]
pub struct PrepaintState {
    interactivity: InteractivityPrepaint,
    focus_handle: FocusHandle,
    elements: SmallVec<[PrepaintElement; STACK_ALLOCATED_ELEMENTS]>,
}

impl Element for EditableTextElement {
    type RequestLayoutState = LayoutState<EditableTextState>;
    type PrepaintState = PrepaintState;

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
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        // Fetches or initializes the internal state of the field
        let state = match &self.interactivity().element_id {
            None => unimplemented!("all input elements must be assigned an id"),
            Some(element_id) => {
                let state = window.use_keyed_state(element_id.clone(), cx, |_window, cx| {
                    EditableTextState::new(self.init_storage.exec(cx), cx)
                });
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

            if let Some(scroll_offset) = state.layout_data.next_scroll_offset {
                self.interactivity
                    .set_scroll_offset(global_id, window, -scroll_offset);
            }
        }

        // TODO: This required a gpui api change in order to sync the focus handle between Interactivity and TextInputStateBase
        self.interactivity.track_focus(focus_handle);

        let placeholder = self.placeholder.clone();
        let placeholder_color = self.colors.placeholder;
        let supports_multiline = self.supports_multiline;
        let accepts_input = self.accepts_input;
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| {
                let state = state.clone();
                window.with_text_style(style.text_style().cloned(), move |window| {
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
                            let text_len = text.len();

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

                                let mut line_len = line.len();
                                if line_len < text_len {
                                    // to offset for new-line characters that are
                                    // omitted from WrappedLine range
                                    line_len += 1;
                                }

                                let segment = TextLineSegment {
                                    text_range: line_start..line_start + line_len,
                                    wrapped_line: Some(Arc::new(line)),
                                    pos_y,
                                };
                                line_start += line_len;
                                pos_y += segment.row_count();
                                lines.push(segment);
                            }

                            let layout_data = TextInputLayoutData {
                                supports_multiline,
                                accepts_input,
                                scroll_bounds: Bounds::default(),
                                wrap_width,
                                size: Some(size),
                                last_seen_storage_version,
                                lines,
                                line_height,
                                next_scroll_offset: None,
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

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // should reflect the text content layout size of the stored text,
        // so that scrolling can take it into account during prepaint.
        let content_size;
        let caret;
        let focus_handle;
        {
            let state = request_layout.state.read(cx);
            content_size = state.layout_data.size.unwrap_or_else(|| bounds.size);
            caret = state.caret_entity().clone();
            focus_handle = state.focus_handle(cx);
        }

        let is_focused = focus_handle.is_focused(window);
        let caret_visible = caret.update(cx, |caret, cx| caret.update_focus(is_focused, cx));

        let prepaint = self.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, cx| {
                let hitbox =
                    hitbox.or_else(|| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal)));
                let inner_bounds = {
                    let padding = style
                        .padding
                        .to_pixels(bounds.size.into(), window.rem_size());

                    let mut bounds = bounds;
                    bounds.origin += point(padding.left, padding.top);
                    bounds.size.width -= padding.left + padding.right;
                    bounds.size.height -= padding.top + padding.bottom;
                    bounds
                };
                request_layout.state.update(cx, |state, _cx| {
                    // while gpui tracks scroll_offset with negative values,
                    // this is converted into positive for usage with bounds
                    state.layout_data.scroll_bounds =
                        Bounds::new(-scroll_offset, inner_bounds.size);
                });
                InteractivityPrepaint {
                    hitbox,
                    scroll_offset,
                    inner_bounds,
                    caret_visible,
                }
            },
        );

        let state = request_layout.state.read(cx);
        let elements = self.build_elements(state, &prepaint, window);

        PrepaintState {
            interactivity: prepaint,
            focus_handle,
            elements,
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(hitbox) = &prepaint.interactivity.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let accepts_input = self.accepts_input;
        let inner_bounds = prepaint.interactivity.inner_bounds;
        let to_local_position = -(bounds.origin + prepaint.interactivity.scroll_offset);
        let perform_paint = |style: &Style, window: &mut Window, cx: &mut App| {
            if style.display == Display::None {
                return;
            }

            if accepts_input {
                let ime_handler =
                    ElementInputHandler::new(inner_bounds, request_layout.state.clone());
                window.handle_input(&prepaint.focus_handle, ime_handler, cx);
            }

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

                    let text_position = event.position + to_local_position;
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

                    let text_position = event.position + to_local_position;
                    state.update(cx, |state, cx| {
                        state.on_mouse_move(event, text_position, window, cx);
                    });
                }
            });

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
        };
        self.interactivity().paint(
            global_id,
            inspector_id,
            bounds.clone(),
            prepaint.interactivity.hitbox.as_ref(),
            window,
            cx,
            perform_paint,
        );
    }
}

impl EditableTextElement {
    fn build_elements(
        &self,
        state: &EditableTextState,
        prepaint: &InteractivityPrepaint,
        window: &mut Window,
    ) -> SmallVec<[PrepaintElement; STACK_ALLOCATED_ELEMENTS]> {
        let InteractivityPrepaint {
            hitbox: _,
            scroll_offset,
            inner_bounds,
            caret_visible,
        } = prepaint;

        let caret_pos = state.caret_pos();
        let selection = state.selected_range();
        let ime_range = state.marked_range();

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
        let mut caret_point = None::<Point<Pixels>>;
        for segment in &state.layout_data.lines {
            let line_distance_from_top = segment.pos_y * line_height;
            let line_y = line_distance_from_top + scroll_offset.y;
            let line_bottom = line_y + line_height * segment.row_count() as f32;
            let line_visible = line_bottom >= Pixels::ZERO && line_y <= inner_bounds.size.height;
            if !line_visible {
                continue;
            }

            // TODO: First render all lines (underlines for IME), then all selections, then cursor if no selection

            if let Some(wrapped) = &segment.wrapped_line {
                let point = inner_bounds.origin + point(scroll_offset.x, line_y);
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
                        self.colors.selection,
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
                        self.colors.selection,
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
                        self.colors.selection,
                    ));
                }
            }

            let is_cursor_in_line = segment.contains_position(caret_pos, true);
            if is_cursor_in_line && let Some(wrapped) = &segment.wrapped_line {
                let local_offset = caret_pos.saturating_sub(segment.text_range.start);
                let caret_px = wrapped
                    .position_for_index(local_offset, line_height)
                    .unwrap_or_default();
                caret_point = Some(caret_px + point(scroll_offset.x, line_y));
            }
        }

        if *caret_visible && let Some(carent_point) = caret_point {
            const CURSOR_WIDTH: f32 = 2.0;
            let quad = fill(
                Bounds::new(
                    inner_bounds.origin + carent_point,
                    size(gpui::px(CURSOR_WIDTH), line_height),
                ),
                self.colors.caret,
            );
            elements.push(PrepaintElement::Quad(quad));
        }

        elements
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
            let last_line_y = line_height * (segment.row_count() - 1) as f32;
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
