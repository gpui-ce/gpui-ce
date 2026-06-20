use crate::editable_text::{
    EditableInputActionElement, EditableTextActionHandler, InitStorage, StateBackedElement,
    TextInputLayoutData, TextInputState,
};
use gpui::{
    Along, App, Axis, Bounds, ContentMask, CursorStyle, DispatchPhase, Display, Element, ElementId,
    ElementInputHandler, Entity, FocusHandle, Focusable, Hitbox, HitboxBehavior, Hsla,
    InteractiveElement, Interactivity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style,
    StyleRefinement, Styled, TextRun, TextStyle, Window, fill, point, size,
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
    Line(ShapedLine),
    Quad(PaintQuad),
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
        pub scroll_x: Pixels,
        pub display_text: SharedString,
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
        let input = request_layout.state.read(cx);

        let focus_handle = input.focus_handle(cx);
        let caret_pos = input.caret_pos();
        let selection = input.selected_range();
        // TODO: horizontal scroll
        let scroll_x_input = Pixels::ZERO; //input.scroll_x();
        // TODO: Cursor blinking
        let cursor_visible = true; // input.cursor_visible();

        let text_color = Hsla::white(); // TODO: as an element param
        let placeholder_color = Hsla::black().opacity(0.5); // TODO: as an element param
        let selection_color = Hsla::blue().opacity(0.5); // TODO: as an element param
        let caret_color = Hsla::white(); // TODO: as an element param

        let mut elements = SmallVec::new();

        let text_value = input.storage().content_utf8();
        let is_empty = text_value.is_empty();

        let (display_text, run_color) = match is_empty {
            // TODO: Can the SharedString allocation be avoided?
            false => (SharedString::new(text_value), text_color),
            true => {
                let value = self.placeholder.as_ref().cloned().unwrap_or_default();
                (value, placeholder_color)
            }
        };

        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: run_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(display_text.clone(), font_size, &[run], None);

        let caret_x_line = line.x_for_index(caret_pos);
        let cursor_thickness = gpui::px(2.0);
        let max_cursor_x = (bounds.size.width - cursor_thickness).max(Pixels::ZERO);
        let max_scroll_x = (line.width - max_cursor_x).max(Pixels::ZERO);
        let mut scroll_x = scroll_x_input.clamp(Pixels::ZERO, max_scroll_x);
        if caret_x_line < scroll_x {
            scroll_x = caret_x_line;
        } else if caret_x_line > scroll_x + max_cursor_x {
            scroll_x = caret_x_line - max_cursor_x;
        }
        scroll_x = scroll_x.clamp(Pixels::ZERO, max_scroll_x);

        let has_selection = !selection.is_empty() && !is_empty;
        if has_selection {
            let start_x = line.x_for_index(selection.start);
            let end_x = line.x_for_index(selection.end);
            let quad = fill(
                Bounds::from_corners(
                    point(bounds.left() + start_x.min(end_x) - scroll_x, bounds.top()),
                    point(
                        bounds.left() + start_x.max(end_x) - scroll_x,
                        bounds.bottom(),
                    ),
                ),
                selection_color,
            );
            elements.push(PrepaintElement::Quad(quad));
        }

        elements.push(PrepaintElement::Line(line));

        let is_focused = focus_handle.is_focused(window);
        if !has_selection && is_focused && cursor_visible {
            let cursor_paint_x = bounds.left() + caret_x_line - scroll_x;
            let quad = fill(
                Bounds::new(
                    point(cursor_paint_x, bounds.top()),
                    size(cursor_thickness, bounds.bottom() - bounds.top()),
                ),
                caret_color,
            );
            elements.push(PrepaintElement::Quad(quad));
        }

        let hitbox = self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, cx| {
                hitbox.or_else(|| Some(window.insert_hitbox(bounds, HitboxBehavior::Normal)))
            },
        );

        Self::PrepaintState {
            hitbox,
            focus_handle,
            elements,
            scroll_x,
            display_text,
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

        let mut layout_data = TextInputLayoutData::default();
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

            layout_data = window.with_content_mask(Some(ContentMask { bounds }), |window| {
                let mut lines = Vec::with_capacity(prepaint.elements.len());
                for element in prepaint.elements.drain(..) {
                    match element {
                        PrepaintElement::Line(line) => {
                            let origin_x = bounds.left() - prepaint.scroll_x;
                            let _ = line.paint(
                                point(origin_x, bounds.top()),
                                window.line_height(),
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            lines.push(line);
                        }
                        PrepaintElement::Quad(quad) => window.paint_quad(quad),
                    }
                }

                // TODO: Render marked IME underlines

                TextInputLayoutData { lines, bounds }
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

        request_layout.state.update(cx, |state, _cx| {
            *state.layout_data_mut() = layout_data;
        });
    }
}
