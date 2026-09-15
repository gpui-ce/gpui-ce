#[cfg(test)]
use crate::editable_text::StringStorage;

#[cfg(test)]
use gpui::{
    AppContext, CaretAffinity, CaretPosition, Context, EntityInputHandler, HeadlessAppContext,
    PlatformInput, PlatformTextSystem, Render, ScaledPixels, TestTextSystem, WindowHandle, div,
    hsla, prelude::*,
};

#[cfg(test)]
use gpui_parley::{ParleyTextSystem, SystemFonts};

#[cfg(test)]
use std::{borrow::Cow, collections::HashSet};

use crate::editable_text::{
    BLINK_INTERVAL_500MS, Caret, EditableTextState,
    actions::{DEFAULT_INPUT_CONTEXT, EditableTextActionElement, EditableTextActionHandler},
    layout::{EditableTextLayoutResult, EditableTextLayoutState},
    state::AccessibilityText,
};
use gpui::{
    A11ySubtreeBuilder, App, Bounds, CursorStyle, DefiniteLength, DispatchPhase, Display, Element,
    ElementId, ElementInputHandler, Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior, Hsla,
    InteractiveElement, Interactivity, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NavigationDirection, PaintQuad, Pixels, Point, SharedString,
    Size, StatefulInteractiveElement, Style, StyleRefinement, Styled, TextAlign, TextLayout,
    WeakEntity, Window, WrappedLine, accesskit, fill, point, px, relative, size,
};
use palette::IntoColor;
use smallvec::SmallVec;
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc, time::Duration};

/// Creates a text input element.
/// See [`EditableTextElement`] for usage.
///
/// By default it is multiline, and therefore this is semantically equivalent to [`text_area`].
#[track_caller]
pub fn editable_text(id: impl Into<ElementId>) -> EditableTextElement {
    let mut this = EditableTextElement {
        interactivity: Interactivity::default(),
        state_entity: Rc::new(RefCell::new(WeakEntity::new_invalid())),
        supports_multiline: true,
        placeholder: None,
        accepts_input: true,
        colors: EditableTextColors::default(),
        caret_blink_interval: None,
        caret_width: px(2.),
        caret_height: relative(1.).into(),
    };
    this.interactivity.element_id = Some(id.into());

    this = this.key_context(DEFAULT_INPUT_CONTEXT);
    this.register_actions();

    this
}

/// Creates a singleline text input element.
/// See [`EditableTextElement`] for usage.
#[track_caller]
pub fn text_input(id: impl Into<ElementId>) -> EditableTextElement {
    editable_text(id).multiline(false)
}

/// Creates a multiline text input element.
/// See [`EditableTextElement`] for usage.
#[track_caller]
pub fn text_area(id: impl Into<ElementId>) -> EditableTextElement {
    editable_text(id).multiline(true)
}

/// An input field which users can type text into.
pub struct EditableTextElement {
    interactivity: Interactivity,
    // Populated on first render with an entity stored/attached to the view.
    // This reference is shared with the action handlers, which are processed between renders
    // and therefore cannot otherwise access state attached to the view.
    state_entity: Rc<RefCell<WeakEntity<EditableTextState>>>,
    supports_multiline: bool,
    placeholder: Option<SharedString>,
    accepts_input: bool,
    colors: EditableTextColors,
    caret_blink_interval: Option<Duration>,
    caret_width: Pixels,
    caret_height: DefiniteLength,
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
    /// Color of IME marked underlines
    ime_underline: Hsla,
}
impl Default for EditableTextColors {
    fn default() -> Self {
        use palette::RgbHue;
        const WHITE_50PC: Hsla = Hsla::new_const(RgbHue::new(0.), 0., 1., 0.5);
        const WHITE_70PC: Hsla = Hsla::new_const(RgbHue::new(0.), 0., 1., 0.7);
        // approx rgb(38 79 120) or oklch(41.9% 0.0829 250.4)
        const LIGHT_NAVY_BLUE_50PC: Hsla = Hsla::new_const(RgbHue::new(210.), 0.519, 0.31, 0.5);
        Self {
            placeholder: WHITE_50PC,
            selection: LIGHT_NAVY_BLUE_50PC,
            caret: gpui::white(),
            ime_underline: WHITE_70PC,
        }
    }
}

impl EditableTextElement {
    /// Assigns the underlying state of this element, which should persist across multiple frames.
    /// The user should either create the entity once or utilize `Window::use_keyed_state`
    /// to create an entity intrinsicly linked to the element.
    /// If no state is configured, one will be linked to this element on first render via `Window::use_keyed_state`.
    pub fn state(self, state: WeakEntity<EditableTextState>) -> Self {
        *self.state_entity.borrow_mut() = state;
        self
    }

    /// Configures whether the field supports multiple lines of text.
    /// Disabling this prevents actions like `enter` and navigating up and down.
    ///
    /// It doesnt not automatically sanitize inputs from containing newlines (e.g. on paste).
    /// This is a limitation of the current state of implementation and requires further iteration.
    pub fn multiline(mut self, enabled: bool) -> Self {
        self.supports_multiline = enabled;
        self
    }

    /// Assigns the text that should be displayed when storage of the element is empty.
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Configures whether the element can accept input (effectively means "is the element currently enabled").
    pub fn accepts_input(mut self, enabled: bool) -> Self {
        self.accepts_input = enabled;
        self
    }

    /// Sets the blinking interval of the caret.
    pub fn caret_blink_interval(mut self, duration: Duration) -> Self {
        self.caret_blink_interval = Some(duration);
        self
    }

    /// Sets the blinking interval of the caret to 500ms
    pub fn caret_blink_interval_500ms(self) -> Self {
        self.caret_blink_interval(BLINK_INTERVAL_500MS)
    }

    /// Sets the color of the placeholder text which is rendered when the element's stored text is empty.
    ///
    /// Cannot be refined via [`StyleRefinement`](gpui::StyleRefinement) due to limitations in the fields of [`Style`](gpui::Style).
    pub fn placeholder_color(mut self, color: impl IntoColor<Hsla>) -> Self {
        self.colors.placeholder = color.into_color();
        self
    }

    /// Sets the width of the caret / text-cursor.
    pub fn caret_w(mut self, width: impl Into<Pixels>) -> Self {
        self.caret_width = width.into();
        self
    }

    /// Sets the height of the caret / text-cursor.
    ///
    /// Relative lengths are resolved against the current line height. The default is
    /// `relative(1.)`, which makes the caret as tall as the line.
    pub fn caret_h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.caret_height = height.into();
        self
    }

    /// Sets the color of the box highlighting selected text.
    ///
    /// Cannot be refined via [`StyleRefinement`](gpui::StyleRefinement) due to limitations in the fields of [`Style`](gpui::Style).
    pub fn selection_color(mut self, color: Hsla) -> Self {
        self.colors.selection = color;
        self
    }

    /// Sets the color of the caret / text-cursor.
    ///
    /// Cannot be refined via [`StyleRefinement`](gpui::StyleRefinement) due to limitations in the fields of [`Style`](gpui::Style).
    pub fn caret_color(mut self, color: Hsla) -> Self {
        self.colors.caret = color;
        self
    }

    /// Sets the color of the underlines rendered underneath text being editted/marked by InputMethodEditors
    /// (for writing Chinese, Japanese, and Korean utf-16).
    ///
    /// Cannot be refined via [`StyleRefinement`](gpui::StyleRefinement) due to limitations in the fields of [`Style`](gpui::Style).
    pub fn marked_color(mut self, color: Hsla) -> Self {
        self.colors.ime_underline = color;
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

struct PrelayoutState {
    state: Entity<EditableTextState>,
    prev_layout_state: EditableTextLayoutState,
    storage_version: u16,
    show_placeholder: bool,
    text: Option<SharedString>,
    placeholder_color: Hsla,
    supports_multiline: bool,
    accepts_input: bool,
}

#[doc(hidden)]
pub struct LayoutState {
    layout_id: LayoutId,
    state: Entity<EditableTextState>,
    caret: Entity<Caret>,
}

struct InteractivityPrepaint {
    hitbox: Option<Hitbox>,
    scroll_offset: Point<Pixels>,
    inner_bounds: Bounds<Pixels>,
    caret_visible: bool,
}

impl InteractivityPrepaint {
    fn document_origin(&self) -> Point<Pixels> {
        self.inner_bounds.origin + self.scroll_offset
    }
}

/// Internal type containing prepaint information used to paint the element
#[doc(hidden)]
pub struct PrepaintState {
    bounds: Bounds<Pixels>,
    interactivity: InteractivityPrepaint,
    focus_handle: FocusHandle,
    elements: PrepaintElements,
    accessible_text: Option<Arc<AccessibilityText>>,
    accessible_anchor: usize,
    accessible_focus: usize,
}

impl Element for EditableTextElement {
    type RequestLayoutState = LayoutState;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(if self.supports_multiline {
            accesskit::Role::MultilineTextInput
        } else {
            accesskit::Role::TextInput
        })
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if !self.accepts_input {
            node.set_read_only();
        }
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        let accessible_text = prepaint
            .accessible_text
            .as_ref()
            .expect("accessibility data was prepared while building the tree");
        let mut text_run = accesskit::Node::new(accesskit::Role::TextRun);
        text_run.set_value(accessible_text.text.clone());
        text_run.set_character_lengths(accessible_text.character_lengths.clone());
        let text_run_id = builder.synthetic_node_id("text");
        builder.push_child(text_run_id, text_run);
        builder
            .parent_node()
            .set_text_selection(accesskit::TextSelection {
                anchor: accesskit::TextPosition {
                    node: text_run_id,
                    character_index: prepaint.accessible_anchor,
                },
                focus: accesskit::TextPosition {
                    node: text_run_id,
                    character_index: prepaint.accessible_focus,
                },
            });
    }

    fn request_layout(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let entity = self.find_or_create_state(window, cx);
        let caret = self.find_or_create_caret(&entity, window, cx);

        if let Some(duration) = self.caret_blink_interval.take()
            && caret.read(cx).blink_interval() != duration
        {
            caret.update(cx, |caret, _cx| caret.set_blink_interval(duration));
        }

        // Read new state information from the underlying entity.
        // Block-wrapped so that the state being read is dropped before continuing.
        let (prelayout, next_scroll_offset) = {
            let state = entity.read(cx);
            let show_placeholder = state.as_str().is_empty();
            let text = match show_placeholder {
                false => Some(SharedString::from(state.as_str())),
                true => self.placeholder.clone(),
            };

            let prelayout = PrelayoutState {
                state: entity.clone(),
                prev_layout_state: state.layout_data.state,
                show_placeholder,
                storage_version: state.version(),
                text,
                placeholder_color: self.colors.placeholder,
                supports_multiline: self.supports_multiline,
                accepts_input: self.accepts_input,
            };
            (prelayout, state.layout_data.next_scroll_offset)
        };

        // Update the scroll offset of the element when the user's caret goes out of scope.
        if let Some(scroll_offset) = next_scroll_offset {
            self.interactivity
                .set_scroll_offset(global_id, window, -scroll_offset);

            // Clear scroll_layout here in the very likely event that we wont need to
            // recompute layout, in which case the layout result isnt rebuilt during `perform_text_layout`.
            entity.update(cx, |state, _cx| {
                state.layout_data.next_scroll_offset = None;
            });
        }

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| {
                window.with_text_style(style.text_style().cloned(), move |window| {
                    let text_layout_id = prelayout.perform_text_layout(window);
                    window.request_layout(style.clone(), Some(text_layout_id), cx)
                })
            },
        );

        (
            layout_id,
            LayoutState {
                layout_id,
                state: entity,
                caret,
            },
        )
    }

    fn prepaint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = window.parent_relative_layout_bounds(request_layout.layout_id);

        // should reflect the text content layout size of the stored text,
        // so that scrolling can take it into account during prepaint.
        let (content_size, focus_handle) = {
            let state = request_layout.state.read(cx);
            let content_size = state.layout_data.state.size.unwrap_or_else(|| bounds.size);
            let focus_handle = state.focus_handle(cx);
            (content_size, focus_handle)
        };

        let is_focused = focus_handle.is_focused(window);
        let caret_visible = request_layout
            .caret
            .update(cx, |caret, cx| caret.update_focus(is_focused, cx));
        window.set_focus_handle(&focus_handle, cx);

        let prepaint = self.interactivity.prepaint(
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
        let accessible_text = window.is_a11y_active().then(|| state.accessibility_text());
        let (accessible_anchor, accessible_focus) =
            accessible_text.as_ref().map_or((0, 0), |text| {
                accessible_selection(
                    &text.byte_offsets,
                    state.selected_range(),
                    state.selection_direction(),
                )
            });
        let elements = PrepaintElements::build_elements(
            state,
            &prepaint,
            &self.colors,
            self.caret_width,
            self.caret_height,
            window,
        );

        PrepaintState {
            bounds,
            interactivity: prepaint,
            focus_handle,
            elements,
            accessible_text,
            accessible_anchor,
            accessible_focus,
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&gpui::GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let bounds = prepaint.bounds;

        if let Some(hitbox) = &prepaint.interactivity.hitbox {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }

        let accepts_input = self.accepts_input;
        let hitbox = prepaint.interactivity.hitbox.clone();
        let line_height = request_layout.state.read(cx).layout_data.line_height;
        let perform_paint = |style: &Style, window: &mut Window, cx: &mut App| {
            if style.display == Display::None {
                return;
            }

            // Register event listeners to the window for the next frame
            if accepts_input {
                Self::process_frame_events(prepaint, bounds, &request_layout.state, window, cx);
            }

            // Actually draw the elements we constructed during prepaint
            for PrepaintLine { line, point, align } in prepaint.elements.lines.drain(..) {
                let _ = line.paint(point, line_height, align, Some(bounds), window, cx);
            }
            for quad in prepaint.elements.ime_marked.drain(..) {
                window.paint_quad(quad);
            }
            for quad in prepaint.elements.selection.drain(..) {
                window.paint_quad(quad);
            }
            if let Some(quad) = prepaint.elements.caret.take() {
                window.paint_quad(quad);
            }
        };

        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            perform_paint,
        );
    }
}

fn accessible_selection(
    byte_offsets: &[usize],
    selection: Range<usize>,
    direction: Option<NavigationDirection>,
) -> (usize, usize) {
    let byte_to_character = |offset: usize| {
        byte_offsets
            .partition_point(|byte_offset| *byte_offset <= offset)
            .saturating_sub(1)
    };
    match direction {
        Some(NavigationDirection::Forward) => (
            byte_to_character(selection.end),
            byte_to_character(selection.start),
        ),
        Some(NavigationDirection::Back) => (
            byte_to_character(selection.start),
            byte_to_character(selection.end),
        ),
        None => {
            let caret = byte_to_character(selection.start);
            (caret, caret)
        }
    }
}

impl EditableTextElement {
    fn find_or_create_state(&self, window: &mut Window, cx: &mut App) -> Entity<EditableTextState> {
        if let Some(entity) = self.state_entity.borrow().upgrade() {
            return entity;
        }
        let Some(element_id) = self.interactivity.element_id.clone() else {
            unimplemented!("all input elements must be assigned an id")
        };

        let state = EditableTextState::use_keyed(element_id, window, cx);
        // store a reference to the entity owned by the element for access in action handlers
        *self.state_entity_rc().borrow_mut() = state.downgrade();
        state
    }

    fn find_or_create_caret(
        &self,
        state: &Entity<EditableTextState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Caret> {
        let Some(element_id) = self.interactivity.element_id.clone() else {
            unimplemented!("all input elements must be assigned an id")
        };

        window.use_keyed_state(element_id, cx, |_window, cx| {
            let mut caret = Caret::default();
            caret.subscribe_to(state, cx);
            caret
        })
    }

    fn process_frame_events(
        prepaint: &PrepaintState,
        bounds: Bounds<Pixels>,
        entity: &Entity<EditableTextState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let inner_bounds = prepaint.interactivity.inner_bounds;
        let document_origin = prepaint.interactivity.document_origin();

        let ime_handler = ElementInputHandler::new(inner_bounds, entity.clone());
        window.handle_input(&prepaint.focus_handle, ime_handler, cx);

        window.on_mouse_event({
            let focus_handle = prepaint.focus_handle.clone();
            let state = entity.clone();
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

                cx.stop_propagation();
                window.focus(&focus_handle, cx);

                let text_position = event.position - document_origin;
                state.update(cx, |state, cx| {
                    state.on_mouse_down(event, text_position, window, cx);
                });
            }
        });
        window.on_mouse_event({
            let state = entity.clone();
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
            let state = entity.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }

                let text_position = event.position - document_origin;
                state.update(cx, |state, cx| {
                    state.on_mouse_move(event, text_position, window, cx);
                });
            }
        });
    }
}

impl PrelayoutState {
    fn perform_text_layout(self, window: &mut Window) -> LayoutId {
        // NOTE: Loosely mirrors TextLayout::layout
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );

        let color = match self.show_placeholder {
            false => text_style.color,
            true => self.placeholder_color,
        };

        let text = self.text.unwrap_or_default();

        window.request_measured_layout(
            Default::default(),
            // This is invoked sometime in the near future (before prepaint but not immediately),
            // so we avoid doing any pre-emptive work until the layout engine is ready.
            move |known_dimensions, available_space, window, cx| {
                let runs = vec![gpui::TextRun {
                    len: text.len(),
                    font: text_style.font(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: None,
                }];

                let wrap_width = TextLayout::evaluate_wrap_width(
                    &text_style.white_space,
                    known_dimensions,
                    available_space,
                );

                let truncation =
                    TextLayout::evaluate_overflow(&text_style, known_dimensions, available_space);

                if let Some(size) = self.prev_layout_state.size
                    && (wrap_width.is_none() || wrap_width == self.prev_layout_state.wrap_width)
                    && truncation.width.is_none()
                    && self.storage_version == self.prev_layout_state.last_seen_storage_version
                {
                    return size;
                }

                let (text, runs) = TextLayout::apply_truncation(
                    text.clone(),
                    &text_style,
                    font_size,
                    line_height,
                    wrap_width,
                    &truncation,
                    &runs,
                    window,
                    cx,
                );
                let document = window
                    .text_system()
                    .shape_text(text, font_size, &runs, wrap_width, text_style.line_clamp)
                    .ok()
                    .map(Arc::new);
                let size = document
                    .as_ref()
                    .map_or_else(Size::default, |document| document.size(line_height));

                let layout_data = EditableTextLayoutResult {
                    supports_multiline: self.supports_multiline,
                    accepts_input: self.accepts_input,
                    // updated during prepaint
                    scroll_bounds: Bounds::default(),
                    state: EditableTextLayoutState {
                        wrap_width,
                        size: Some(size),
                        last_seen_storage_version: self.storage_version,
                    },
                    document,
                    line_height,
                    next_scroll_offset: None,
                };

                // Update the state for use in prepaint, paint, and action handlers.
                // request_measured_layout caches this scope for processing later
                // between layout and prepaint, so we cant just copy/move these values to the outer scope.
                self.state.update(cx, move |state, _cx| {
                    state.layout_data = layout_data;
                });

                size
            },
        )
    }
}

struct PrepaintLine {
    line: Arc<WrappedLine>,
    point: Point<Pixels>,
    align: TextAlign,
}

const STACK_ALLOCATED_LINES: usize = 100usize;
const STACK_ALLOCATED_QUADS_SELECTION: usize = 20usize;
const STACK_ALLOCATED_QUADS_IME_MARKED: usize = 2usize;

#[derive(Default)]
struct PrepaintElements {
    lines: SmallVec<[PrepaintLine; STACK_ALLOCATED_LINES]>,
    selection: SmallVec<[PaintQuad; STACK_ALLOCATED_QUADS_SELECTION]>,
    ime_marked: SmallVec<[PaintQuad; STACK_ALLOCATED_QUADS_IME_MARKED]>,
    caret: Option<PaintQuad>,
}

impl PrepaintElements {
    fn build_quads(
        offset_corners: Vec<(Point<Pixels>, Point<Pixels>)>,
        origin: Point<Pixels>,
        color: Hsla,
    ) -> impl Iterator<Item = PaintQuad> {
        offset_corners
            .into_iter()
            .map(move |(offset_start, offset_end)| {
                let bounds = Bounds::from_corners(origin + offset_start, origin + offset_end);
                fill(bounds, color)
            })
    }

    fn build_elements(
        state: &EditableTextState,
        prepaint: &InteractivityPrepaint,
        colors: &EditableTextColors,
        caret_width: Pixels,
        caret_height: DefiniteLength,
        window: &mut Window,
    ) -> PrepaintElements {
        let InteractivityPrepaint {
            hitbox: _,
            scroll_offset,
            inner_bounds,
            caret_visible,
        } = prepaint;

        let caret = state.visible_caret();
        let selection = state.selected_range();
        let ime_range = state.marked_range();

        let mut elements = PrepaintElements::default();

        let line_height = state.layout_data.line_height;
        let mut caret_point = None::<Point<Pixels>>;

        if let Some(document) = &state.layout_data.document {
            let line_y = scroll_offset.y;
            let line_bottom = line_y + line_height * document.line_count() as f32;
            let line_visible = line_bottom >= Pixels::ZERO && line_y <= inner_bounds.size.height;

            if line_visible {
                let document_origin = prepaint.document_origin();
                elements.lines.push(PrepaintLine {
                    line: document.clone(),
                    point: document_origin,
                    align: TextAlign::Left,
                });

                if !selection.is_empty() {
                    let offset_corners =
                        build_quad_over_text(&selection, document, line_height, Pixels::ZERO);
                    elements.selection.extend(PrepaintElements::build_quads(
                        offset_corners,
                        document_origin,
                        colors.selection,
                    ));
                }

                if let Some(ime_range) = &ime_range
                    && !ime_range.is_empty()
                {
                    const MARKED_TEXT_UNDERLINE_THICKNESS: f32 = 2.0;
                    let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
                    let underline_offset = line_height - underline_thickness;

                    let offset_corners =
                        build_quad_over_text(&ime_range, document, line_height, underline_offset);
                    elements.ime_marked.extend(PrepaintElements::build_quads(
                        offset_corners,
                        document_origin,
                        colors.ime_underline,
                    ));
                }

                let caret_px = document
                    .position_for_caret(caret, line_height)
                    .unwrap_or_default();
                caret_point = Some(document_origin + caret_px);
            }
        }

        if *caret_visible && let Some(caret_point) = caret_point {
            let caret_height = caret_height.to_pixels(line_height.into(), window.rem_size());
            let vertical_offset = (line_height - caret_height) / 2.;
            let quad = fill(
                Bounds::new(
                    caret_point + point(Pixels::ZERO, vertical_offset),
                    size(caret_width, caret_height),
                ),
                colors.caret,
            );
            elements.caret = Some(quad);
        }

        elements
    }
}

fn build_quad_over_text(
    containing_range: &Range<usize>,
    document: &WrappedLine,
    line_height: Pixels,
    offset_y: Pixels,
) -> Vec<(Point<Pixels>, Point<Pixels>)> {
    let start = containing_range.start.min(document.text.len());
    let end = containing_range.end.min(document.text.len());
    document
        .selection_bounds(start..end, line_height)
        .into_iter()
        .map(|bounds| {
            (
                point(bounds.left(), bounds.top() + offset_y),
                point(bounds.right(), bounds.bottom()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER_COLOR: Hsla = hsla(0.72, 0.45, 0.32, 1.0);
    const INPUT_COLOR: Hsla = hsla(0.08, 0.55, 0.28, 1.0);
    const SELECTION_COLOR: Hsla = hsla(0.37, 0.65, 0.42, 1.0);
    const CARET_COLOR: Hsla = hsla(0.95, 0.8, 0.6, 1.0);
    const TEXT_COLOR: Hsla = hsla(0.0, 0.0, 0.1, 1.0);
    const MARKED_COLOR: Hsla = hsla(0.55, 0.8, 0.6, 1.0);

    struct BidiInputView {
        input: Entity<EditableTextState>,
        padding: Pixels,
        width: Pixels,
        wrap: bool,
    }

    impl Render for BidiInputView {
        fn render(
            &mut self,
            _window: &mut Window,
            _context: &mut Context<Self>,
        ) -> impl IntoElement {
            div().p(px(24.)).child(
                editable_text("bidi-input")
                    .state(self.input.downgrade())
                    .p(self.padding)
                    .w(self.width)
                    .h(px(120.))
                    .border_1()
                    .rounded_lg()
                    .bg(INPUT_COLOR)
                    .text_color(TEXT_COLOR)
                    .selection_color(SELECTION_COLOR)
                    .caret_color(CARET_COLOR)
                    .marked_color(MARKED_COLOR)
                    .text_size(px(20.))
                    .line_height(px(if self.wrap { 27.5 } else { 28. }))
                    .when(self.wrap, |input| {
                        input.flex_col().whitespace_normal().overflow_y_scroll()
                    })
                    .when(!self.wrap, |input| {
                        input.whitespace_nowrap().overflow_x_scroll()
                    }),
            )
        }
    }

    struct BidiInputFixture {
        input: Entity<EditableTextState>,
        window: WindowHandle<BidiInputView>,
        padding: Pixels,
        scale: f32,
        context: HeadlessAppContext,
    }

    impl BidiInputFixture {
        fn new(text: &str, padding: f32, width: f32, wrap: bool, scale: f32) -> Self {
            let system = ParleyTextSystem::new_with_system_font(SystemFonts::Skip, "IBM Plex Sans")
                .with_fallback_families(["IBM Plex Sans", "Noto Sans Hebrew", "Noto Sans Arabic"]);
            system
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/noto-sans-hebrew/NotoSansHebrew-Regular.ttf"
                    )),
                    Cow::Borrowed(include_bytes!(
                        "../../../../assets/fonts/noto-sans-arabic/NotoSansArabic-Regular.ttf"
                    )),
                ])
                .unwrap();

            let mut context = HeadlessAppContext::new(Arc::new(system));
            let input = context.update(|context| {
                context.new(|context| EditableTextState::new(StringStorage::from(text), context))
            });

            let window = context
                .open_window(size(px(500.), px(240.)), |window, context| {
                    window.set_scale_factor(scale);

                    context.new(|_context| BidiInputView {
                        input: input.clone(),
                        padding: px(padding),
                        width: px(width),
                        wrap,
                    })
                })
                .unwrap();
            context.run_until_parked();

            Self {
                input,
                window,
                padding: px(padding),
                scale,
                context,
            }
        }

        fn origin(&mut self) -> Point<Pixels> {
            let bounds = only_quad(&mut self.context, self.window.into(), INPUT_COLOR)
                .map(|value| px(value.as_f32() / self.scale));
            let scroll = self
                .context
                .update(|context| self.input.read(context).layout_data.scroll_bounds.origin);

            bounds.origin + point(self.padding, self.padding) - scroll
        }

        fn document(&mut self) -> Arc<WrappedLine> {
            self.context.update(|context| {
                self.input
                    .read(context)
                    .layout_data
                    .document
                    .clone()
                    .unwrap()
            })
        }

        fn line_height(&mut self) -> Pixels {
            self.context
                .update(|context| self.input.read(context).layout_data.line_height)
        }

        fn scroll(&mut self, offset: Point<Pixels>) {
            self.context.update(|context| {
                self.input.update(context, |state, context| {
                    state.layout_data.next_scroll_offset = Some(offset);
                    context.notify();
                })
            });
            self.context.run_until_parked();
            self.context.update(|context| {
                assert_eq!(
                    self.input.read(context).layout_data.scroll_bounds.origin,
                    offset
                );
            });
        }

        fn point(&mut self, idx: usize) -> Point<Pixels> {
            let caret = CaretPosition::new(idx, CaretAffinity::Downstream);
            let line_height = self.line_height();
            let local = self
                .document()
                .position_for_caret(caret, line_height)
                .unwrap();

            self.origin() + local + point(px(0.), line_height / 2.)
        }

        fn dispatch(&mut self, event: PlatformInput) {
            self.context
                .update_window(self.window.into(), |_view, window, context| {
                    window.dispatch_event(event, context);
                })
                .unwrap();
            self.context.run_until_parked();
        }

        fn down(&mut self, position: Point<Pixels>) {
            self.dispatch(PlatformInput::MouseDown(MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }));
        }

        fn drag(&mut self, idx: usize) {
            let position = self.point(idx);
            self.dispatch(PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            }));
        }

        fn up(&mut self, idx: usize) {
            let position = self.point(idx);
            self.dispatch(PlatformInput::MouseUp(MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }));
        }

        fn assert_selection(&mut self, anchor: usize, focus: usize) {
            let range = anchor.min(focus)..anchor.max(focus);
            let caret = self.context.update(|context| {
                let state = self.input.read(context);
                assert_eq!(state.selected_range(), range);
                assert_eq!(state.caret().index, focus);

                state.caret()
            });

            let origin = self.origin();
            let document = self.document();
            let line_height = self.line_height();
            let caret_point = document.position_for_caret(caret, line_height).unwrap();
            self.assert_quads(
                CARET_COLOR,
                vec![Bounds::new(origin + caret_point, size(px(2.), line_height))],
            );

            let expected = document
                .selection_bounds(range, line_height)
                .into_iter()
                .map(|bounds| Bounds::new(origin + bounds.origin, bounds.size))
                .collect();
            self.assert_quads(SELECTION_COLOR, expected);
        }

        fn assert_quads(&mut self, color: Hsla, expected: Vec<Bounds<Pixels>>) {
            let actual = self
                .context
                .solid_quad_bounds(self.window.into(), color)
                .unwrap();
            assert_eq!(actual.len(), expected.len(), "quad count for {color:?}");

            for (actual, expected) in actual.into_iter().zip(expected) {
                let actual = actual.map(|value| px(value.as_f32() / self.scale));
                assert!(
                    (actual.origin - expected.origin).magnitude() <= 1.,
                    "{actual:?} != {expected:?}"
                );
                assert!((actual.size.width - expected.size.width).abs() <= px(1.));
                assert!((actual.size.height - expected.size.height).abs() <= px(1.));
            }
        }
    }

    #[test]
    fn parley_input_clicks_and_drags_preserve_visual_carets() {
        for (text, anchor, targets) in [
            ("שלום עולם", 15, [13, 15, 17]),
            (
                "مرحبا بالعالم",
                "مرحبا بالعا".len(),
                [
                    "مرحبا بالع".len(),
                    "مرحبا بالعا".len(),
                    "مرحبا بالعال".len(),
                ],
            ),
            ("abc אבגד def", 8, [6, 8, 10]),
            ("hello world", 3, [4, 3, 2]),
        ] {
            for (padding, scale) in [(0., 1.), (8., 1.), (8., 1.5)] {
                let mut fixture = BidiInputFixture::new(text, padding, 320., false, scale);

                for offset in [-0.25, 0., 0.25] {
                    let position = fixture.point(anchor) + point(px(offset), px(0.));
                    fixture.down(position);
                    fixture.assert_selection(anchor, anchor);
                    fixture.up(anchor);
                }

                let position = fixture.point(anchor);
                fixture.down(position);

                for target in targets {
                    fixture.drag(target);
                    fixture.assert_selection(anchor, target);
                }

                fixture.up(targets[2]);
                fixture.drag(anchor);
                fixture.assert_selection(anchor, targets[2]);
            }
        }
    }

    #[test]
    fn parley_hebrew_click_uses_the_painted_gap() {
        let mut fixture = BidiInputFixture::new("שלום עולם", 8., 320., false, 1.5);
        let mut glyphs = fixture
            .context
            .glyph_bounds(fixture.window.into(), TEXT_COLOR)
            .unwrap();
        glyphs.sort_by(|left, right| left.origin.x.partial_cmp(&right.origin.x).unwrap());
        assert_eq!(glyphs.len(), 8);

        let gap =
            px((glyphs[0].right().as_f32() + glyphs[1].left().as_f32()) / (2. * fixture.scale));
        let position = point(gap, fixture.point(15).y);
        fixture.down(position);
        fixture.assert_selection(15, 15);
        fixture.drag(13);
        fixture.assert_selection(15, 13);
    }

    #[test]
    fn parley_scrolled_input_keeps_selection_and_marked_text_under_the_glyphs() {
        let text = "0123456789 שלום עולם 0123456789";
        let anchor = "0123456789 שלום עול".len();
        let focus = "0123456789 שלום עו".len();
        let mut fixture = BidiInputFixture::new(text, 8., 200., false, 1.5);
        fixture.scroll(point(px(45.), px(0.)));

        let position = fixture.point(anchor);
        fixture.down(position);
        fixture.drag(focus);
        fixture.assert_selection(anchor, focus);
        fixture.up(focus);

        let marked_start = text[..focus].encode_utf16().count();
        fixture
            .context
            .update_window(fixture.window.into(), |_view, window, context| {
                fixture.input.update(context, |state, context| {
                    state.replace_and_mark_text_in_range(
                        Some(marked_start..marked_start + 1),
                        "ל",
                        None,
                        window,
                        context,
                    );
                });
            })
            .unwrap();
        fixture.context.run_until_parked();
        fixture.scroll(point(px(45.), px(0.)));

        let origin = fixture.origin();
        let expected = fixture
            .document()
            .selection_bounds(focus..anchor, px(28.))
            .into_iter()
            .map(|bounds| {
                Bounds::new(
                    origin + bounds.origin + point(px(0.), px(26.)),
                    size(bounds.size.width, px(2.)),
                )
            })
            .collect();
        fixture.assert_quads(MARKED_COLOR, expected);
    }

    #[test]
    fn parley_wrapped_rtl_drag_preserves_the_anchor_across_rows() {
        let mut fixture = BidiInputFixture::new("שלום עולם שלום עולם", 8., 120., true, 1.5);
        assert!(fixture.document().line_count() > 1);

        let anchor = 2;
        let focus = "שלום עולם של".len();
        let start = fixture.point(anchor);
        assert!(fixture.point(focus).y > start.y);
        fixture.down(start);
        let position = fixture.point(focus) - point(px(0.), fixture.line_height() / 2. - px(0.05));
        fixture.dispatch(PlatformInput::MouseMove(MouseMoveEvent {
            position,
            pressed_button: Some(MouseButton::Left),
            ..Default::default()
        }));
        fixture.assert_selection(anchor, focus);
        fixture.drag(0);
        fixture.assert_selection(anchor, 0);
        fixture.up(0);
    }

    struct CenteredEditableTextView {
        extent: f32,
        input: Entity<EditableTextState>,
    }

    impl Render for CenteredEditableTextView {
        fn render(
            &mut self,
            _window: &mut Window,
            _context: &mut Context<Self>,
        ) -> impl IntoElement {
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(320.0 + self.extent))
                .h(px(160.0 + self.extent))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(118.0))
                        .h(px(31.0))
                        .bg(CONTAINER_COLOR)
                        .child(
                            text_input("input")
                                .state(self.input.downgrade())
                                .bg(INPUT_COLOR)
                                .selection_color(SELECTION_COLOR)
                                .text_size(px(14.0)),
                        ),
                )
        }
    }

    fn only_quad(
        context: &mut HeadlessAppContext,
        window: gpui::AnyWindowHandle,
        color: Hsla,
    ) -> Bounds<ScaledPixels> {
        let bounds = context.solid_quad_bounds(window, color).unwrap();
        assert_eq!(bounds.len(), 1, "expected one rendered quad for {color:?}");
        bounds[0]
    }

    #[test]
    fn accessibility_selection_uses_character_offsets_and_preserves_direction() {
        let text = "A😀日本B";
        let mut byte_offsets = text
            .char_indices()
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        byte_offsets.push(text.len());
        let selection = 1.."A😀日本".len();

        assert_eq!(
            accessible_selection(
                &byte_offsets,
                selection.clone(),
                Some(NavigationDirection::Forward),
            ),
            (4, 1)
        );
        assert_eq!(
            accessible_selection(&byte_offsets, selection, Some(NavigationDirection::Back)),
            (1, 4)
        );
        assert_eq!(accessible_selection(&byte_offsets, 5..5, None), (2, 2));
    }

    #[test]
    fn editable_text_keeps_its_device_pixel_offset_when_its_parent_moves() {
        for scale_factor in [1.0, 1.5] {
            let mut context = HeadlessAppContext::new(Arc::new(TestTextSystem));
            let window = context
                .open_window(size(px(420.0), px(260.0)), |window, context| {
                    window.set_scale_factor(scale_factor);
                    let input = context.new(|context| {
                        let mut state = EditableTextState::new(StringStorage::from("x"), context);
                        state.select_document(context);
                        state
                    });

                    context.new(|_| CenteredEditableTextView { extent: 0.0, input })
                })
                .unwrap();

            context.run_until_parked();
            let any_window = window.into();
            let initial_container = only_quad(&mut context, any_window, CONTAINER_COLOR);
            let initial_input = only_quad(&mut context, any_window, INPUT_COLOR);
            let initial_selection = only_quad(&mut context, any_window, SELECTION_COLOR);
            let expected_input_offset = initial_input.origin - initial_container.origin;
            let expected_selection_offset = initial_selection.origin - initial_input.origin;
            let mut container_origins = HashSet::from([(
                initial_container.origin.x.as_f32() as i32,
                initial_container.origin.y.as_f32() as i32,
            )]);

            for step in 1..=32 {
                window
                    .update(&mut context, |view, _, context| {
                        view.extent = step as f32;
                        context.notify();
                    })
                    .unwrap();
                context.run_until_parked();

                let container = only_quad(&mut context, any_window, CONTAINER_COLOR);
                let input = only_quad(&mut context, any_window, INPUT_COLOR);
                let selection = only_quad(&mut context, any_window, SELECTION_COLOR);
                assert_eq!(
                    input.origin - container.origin,
                    expected_input_offset,
                    "editable control moved within its parent at scale {scale_factor}, step {step}"
                );
                assert_eq!(
                    selection.origin - input.origin,
                    expected_selection_offset,
                    "selected text moved within its control at scale {scale_factor}, step {step}"
                );
                container_origins.insert((
                    container.origin.x.as_f32() as i32,
                    container.origin.y.as_f32() as i32,
                ));
            }

            assert!(
                container_origins.len() > 8,
                "fixture did not cross enough device pixels at scale {scale_factor}"
            );
        }
    }
}

#[cfg(test)]
mod customization_tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn custom_placeholder_color_and_caret_size() {
        let color = rgba(0x33669980);
        let input = editable_text("i")
            .placeholder_color(color)
            .caret_w(px(3.))
            .caret_h(relative(0.5));
        assert_eq!(input.colors.placeholder, color.into_color());
        assert_eq!(input.caret_width, px(3.));
        assert_eq!(
            input.caret_height.to_pixels(px(20.).into(), px(16.)),
            px(10.)
        );
    }
}
