use gpui::{
    Action, App, AppContext, Bounds, ClipboardItem, ContentMask, Context, CursorStyle,
    DispatchPhase, Element, ElementId, ElementInputHandler, Entity, EntityId, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, KeyBinding, LayoutId,
    Length, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollWheelEvent, SharedString, StyleRefinement, Styled, Subscription, TextAlign, TextRun,
    TextStyle, UTF16Selection, Window, WrappedLine, actions, fill, point, px, relative, size,
};
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

const CURSOR_WIDTH: f32 = 2.0;
const MARKED_TEXT_UNDERLINE_THICKNESS: f32 = 2.0;

/// Default interval for cursor blinking.
const DEFAULT_BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// The key context used for input element keybindings.
pub const DEFAULT_INPUT_CONTEXT: &str = "Input";

actions!(
    actions,
    [
        /// Delete the character before the cursor.
        Backspace,
        /// Delete the character after the cursor.
        Delete,
        /// Blur focus from the input.
        Escape,
        /// Delete the word before the cursor.
        DeleteWordLeft,
        /// Delete the word after the cursor.
        DeleteWordRight,
        /// Delete from the cursor to the beginning of the line.
        DeleteToBeginningOfLine,
        /// Delete from the cursor to the end of the line.
        DeleteToEndOfLine,
        /// Insert a tab character at the cursor position.
        Tab,
        /// Move the cursor one character to the left.
        Left,
        /// Move the cursor one character to the right.
        Right,
        /// Move the cursor up one visual line.
        Up,
        /// Move the cursor down one visual line.
        Down,
        /// Extend selection one character to the left.
        SelectLeft,
        /// Extend selection one character to the right.
        SelectRight,
        /// Extend selection up one visual line.
        SelectUp,
        /// Extend selection down one visual line.
        SelectDown,
        /// Select all text content.
        SelectAll,
        /// Move cursor to the start of the current line.
        Home,
        /// Move cursor to the end of the current line.
        End,
        /// Extend selection to the beginning of the content.
        SelectToBeginning,
        /// Extend selection to the end of the content.
        SelectToEnd,
        /// Move cursor to the beginning of the content.
        MoveToBeginning,
        /// Move cursor to the end of the content.
        MoveToEnd,
        /// Paste from clipboard at the cursor position.
        Paste,
        /// Cut selected text to clipboard.
        Cut,
        /// Copy selected text to clipboard.
        Copy,
        /// Insert a newline at the cursor position.
        Enter,
        /// Move cursor one word to the left.
        WordLeft,
        /// Move cursor one word to the right.
        WordRight,
        /// Extend selection one word to the left.
        SelectWordLeft,
        /// Extend selection one word to the right.
        SelectWordRight,
        /// Undo the last edit.
        Undo,
        /// Redo the last undone edit.
        Redo,
    ]
);

#[track_caller]
pub fn input(input_state: &Entity<InputState>, cx: &App) -> Input {
    Input::new(input_state, cx)
}

pub fn input_bindings() -> gpui::ActionBindingCollection {
    let mut bindings = gpui::ActionBindingCollection::default();

    #[cfg(target_os = "macos")]
    {
        bindings = bindings
            .with::<Backspace>("backspace")
            .with::<Delete>("delete")
            .with::<DeleteWordLeft>("alt-backspace")
            .with::<DeleteWordRight>("alt-delete")
            .with::<DeleteToBeginningOfLine>("cmd-backspace")
            .with::<DeleteToEndOfLine>("ctrl-k")
            .with::<Tab>("tab")
            .with::<Enter>("enter")
            .with::<Left>("left")
            .with::<Right>("right")
            .with::<Up>("up")
            .with::<Down>("down")
            .with::<SelectLeft>("shift-left")
            .with::<SelectRight>("shift-right")
            .with::<SelectUp>("shift-up")
            .with::<SelectDown>("shift-down")
            .with::<SelectAll>("cmd-a")
            // Mac keyboards don't have Home/End keys, so cmd-left/right are standard
            .with::<Home>("cmd-left")
            .with::<End>("cmd-right")
            .with::<MoveToBeginning>("cmd-up")
            .with::<MoveToEnd>("cmd-down")
            .with::<SelectToBeginning>("cmd-shift-up")
            .with::<SelectToEnd>("cmd-shift-down")
            .with::<WordLeft>("alt-left")
            .with::<WordRight>("alt-right")
            .with::<SelectWordLeft>("alt-shift-left")
            .with::<SelectWordRight>("alt-shift-right")
            .with::<Copy>("cmd-c")
            .with::<Cut>("cmd-x")
            .with::<Paste>("cmd-v")
            .with::<Undo>("cmd-z")
            .with::<Redo>("cmd-shift-z")
            .with::<Escape>("escape");
    }

    #[cfg(not(target_os = "macos"))]
    {
        bindings = bindings
            .with::<Backspace>("backspace")
            .with::<Delete>("delete")
            .with::<DeleteWordLeft>("ctrl-backspace")
            .with::<DeleteWordRight>("ctrl-delete")
            .with::<DeleteToBeginningOfLine>("ctrl-shift-backspace")
            .with::<DeleteToEndOfLine>("ctrl-shift-delete")
            .with::<Tab>("tab")
            .with::<Enter>("enter")
            .with::<Left>("left")
            .with::<Right>("right")
            .with::<Up>("up")
            .with::<Down>("down")
            .with::<SelectLeft>("shift-left")
            .with::<SelectRight>("shift-right")
            .with::<SelectUp>("shift-up")
            .with::<SelectDown>("shift-down")
            .with::<SelectAll>("ctrl-a")
            .with::<Home>("home")
            .with::<End>("end")
            .with::<MoveToBeginning>("ctrl-home")
            .with::<MoveToEnd>("ctrl-end")
            .with::<SelectToBeginning>("ctrl-shift-home")
            .with::<SelectToEnd>("ctrl-shift-end")
            .with::<WordLeft>("ctrl-left")
            .with::<WordRight>("ctrl-right")
            .with::<SelectWordLeft>("ctrl-shift-left")
            .with::<SelectWordRight>("ctrl-shift-right")
            .with::<Copy>("ctrl-c")
            .with::<Cut>("ctrl-x")
            .with::<Paste>("ctrl-v")
            .with::<Undo>("ctrl-z")
            .with::<Redo>("ctrl-shift-z")
            .with::<Escape>("escape");
    }

    bindings
}

#[derive(Clone, Copy, Debug)]
struct PaintColors {
    selection: Hsla,
    cursor: Hsla,
    placeholder: Hsla,
}

impl Default for PaintColors {
    fn default() -> Self {
        Self {
            selection: Hsla::blue().opacity(0.2),
            cursor: Hsla::white().opacity(0.8),
            placeholder: gpui::hsla(0.6, 0.6, 0.6, 1.0),
        }
    }
}

/// A text editing element that supports both single-line and multi-line modes.
pub struct Input {
    input: Entity<InputState>,
    interactivity: Interactivity,
    placeholder: Option<SharedString>,
    colors: PaintColors,
    multiline: bool,
}

impl Input {
    #[track_caller]
    fn new(input_state: &Entity<InputState>, cx: &App) -> Self {
        let focus_handle = input_state.focus_handle(cx);
        let mut input = Input {
            input: input_state.clone(),
            interactivity: Interactivity::new(),
            placeholder: None,
            colors: PaintColors::default(),
            multiline: false,
        };
        input.register_actions();
        input
            .key_context(DEFAULT_INPUT_CONTEXT)
            .track_focus(&focus_handle)
    }

    fn register_actions(&mut self) {
        register_action(&mut self.interactivity, &self.input, InputState::left);
        register_action(&mut self.interactivity, &self.input, InputState::right);
        register_action(&mut self.interactivity, &self.input, InputState::up);
        register_action(&mut self.interactivity, &self.input, InputState::down);
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_left,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_right,
        );
        register_action(&mut self.interactivity, &self.input, InputState::select_up);
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_down,
        );
        register_action(&mut self.interactivity, &self.input, InputState::select_all);
        register_action(&mut self.interactivity, &self.input, InputState::home);
        register_action(&mut self.interactivity, &self.input, InputState::end);
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::move_to_beginning,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::move_to_end,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_to_beginning,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_to_end,
        );
        register_action(&mut self.interactivity, &self.input, InputState::word_left);
        register_action(&mut self.interactivity, &self.input, InputState::word_right);
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_word_left,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::select_word_right,
        );
        register_action(&mut self.interactivity, &self.input, InputState::backspace);
        register_action(&mut self.interactivity, &self.input, InputState::delete);
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::delete_word_left,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::delete_word_right,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::delete_to_beginning_of_line,
        );
        register_action(
            &mut self.interactivity,
            &self.input,
            InputState::delete_to_end_of_line,
        );
        register_action(&mut self.interactivity, &self.input, InputState::enter);
        register_action(&mut self.interactivity, &self.input, InputState::tab);
        register_action(&mut self.interactivity, &self.input, InputState::paste);
        register_action(&mut self.interactivity, &self.input, InputState::copy);
        register_action(&mut self.interactivity, &self.input, InputState::cut);
        register_action(&mut self.interactivity, &self.input, InputState::undo);
        register_action(&mut self.interactivity, &self.input, InputState::redo);

        self.interactivity
            .on_action::<Escape>(|_action, window, _cx| {
                window.blur();
            });
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }
}

fn register_action<A: Action>(
    interactivity: &mut Interactivity,
    input: &Entity<InputState>,
    listener: fn(&mut InputState, &A, &mut Window, &mut Context<InputState>),
) {
    let input = input.clone();
    interactivity.on_action::<A>(move |action, window, cx| {
        input.update(cx, |input, cx| {
            listener(input, action, window, cx);
        });
    });
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Input {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

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
        let multiline = self.multiline;

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |element_style, window, cx| {
                window.with_text_style(element_style.text_style().cloned(), |window| {
                    resolved_text_style = Some(window.text_style());

                    let mut layout_style = element_style.clone();
                    if multiline {
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

        let wrap_width = if self.multiline {
            bounds.size.width
        } else {
            px(100000.)
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

        let input = self.input.clone();
        let placeholder = self.placeholder.clone();
        let text_style = layout_state.text_style.clone();
        let multiline = self.multiline;
        let is_focused = focus_handle.is_focused(window);
        let cursor_visible = self
            .input
            .update(cx, |input, cx| input.cursor_visible(is_focused, cx));

        let colors = self.colors;
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            prepaint_state.hitbox.as_ref(),
            window,
            cx,
            |_style, window, cx| {
                handle_mouse(&input, bounds, multiline, window, cx);

                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    if multiline {
                        paint_multiline(
                            &input,
                            &focus_handle,
                            bounds,
                            &text_style,
                            placeholder.as_ref(),
                            &colors,
                            cursor_visible,
                            window,
                            cx,
                        );
                    } else {
                        paint_singleline(
                            &input,
                            &focus_handle,
                            bounds,
                            &text_style,
                            placeholder.as_ref(),
                            &colors,
                            cursor_visible,
                            window,
                            cx,
                        );
                    }
                });
            },
        );
    }
}

/// Registers all mouse event handlers for the input.
fn handle_mouse(
    input: &Entity<InputState>,
    bounds: Bounds<Pixels>,
    multiline: bool,
    window: &mut Window,
    cx: &App,
) {
    mouse_down(input.clone(), bounds, multiline, window);
    mouse_up(input.clone(), window);
    mouse_move(input.clone(), bounds, multiline, window);
    handle_scroll(input.clone(), bounds, multiline, window, cx);
}

fn mouse_down(
    input: Entity<InputState>,
    bounds: Bounds<Pixels>,
    multiline: bool,
    window: &mut Window,
) {
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
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
            let text_position =
                screen_to_text_position(event.position, bounds, input.scroll_offset, multiline);
            input.on_mouse_down(
                text_position,
                event.click_count,
                event.modifiers.shift,
                window,
                cx,
            );
        });
    });
}

fn mouse_up(input: Entity<InputState>, window: &mut Window) {
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }

        input.update(cx, |input, cx| {
            input.on_mouse_up(cx);
        });
    });
}

fn mouse_move(
    input: Entity<InputState>,
    bounds: Bounds<Pixels>,
    multiline: bool,
    window: &mut Window,
) {
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }

        input.update(cx, |input, cx| {
            let text_position =
                screen_to_text_position(event.position, bounds, input.scroll_offset, multiline);
            input.on_mouse_move(text_position, cx);
        });
    });
}

fn handle_scroll(
    input: Entity<InputState>,
    bounds: Bounds<Pixels>,
    multiline: bool,
    window: &mut Window,
    cx: &App,
) {
    let max_scroll = if multiline {
        let total_height = input.read(cx).total_content_height();
        (total_height - bounds.size.height).max(px(0.))
    } else {
        let text_width = input
            .read(cx)
            .line_layouts
            .first()
            .and_then(|l| l.wrapped_line.as_ref())
            .map(|w| w.width())
            .unwrap_or(px(0.));
        (text_width - bounds.size.width).max(px(0.))
    };

    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        if !bounds.contains(&event.position) {
            return;
        }

        let pixel_delta = event.delta.pixel_delta(px(20.));
        input.update(cx, |input, cx| {
            if multiline {
                input.scroll_offset =
                    (input.scroll_offset - pixel_delta.y).clamp(px(0.), max_scroll);
            } else {
                let delta = if pixel_delta.x.abs() > pixel_delta.y.abs() {
                    pixel_delta.x
                } else {
                    pixel_delta.y
                };
                input.scroll_offset = (input.scroll_offset - delta).clamp(px(0.), max_scroll);
            }
            cx.notify();
        });
    });
}

/// Converts a screen position to a position relative to the text area origin,
/// adjusted for scroll offset.
fn screen_to_text_position(
    screen_position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
    multiline: bool,
) -> Point<Pixels> {
    if multiline {
        point(
            screen_position.x - bounds.origin.x,
            screen_position.y - bounds.origin.y + scroll_offset,
        )
    } else {
        point(
            screen_position.x - bounds.origin.x + scroll_offset,
            screen_position.y - bounds.origin.y,
        )
    }
}

fn paint_multiline(
    input: &Entity<InputState>,
    focus_handle: &FocusHandle,
    bounds: Bounds<Pixels>,
    text_style: &TextStyle,
    placeholder: Option<&SharedString>,
    colors: &PaintColors,
    cursor_visible: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let input_state = input.read(cx);
    let content = input_state.content().to_string();
    let selected_range = input_state.selected_range().clone();
    let marked_range = input_state.marked_range().cloned();
    let cursor_offset = input_state.cursor_offset();
    let line_layouts = input_state.line_layouts.clone();
    let scroll_offset = input_state.scroll_offset;
    let line_height = input_state.line_height;
    let is_focused = focus_handle.is_focused(window);

    if !selected_range.is_empty() {
        paint_multiline_selection(
            &line_layouts,
            &selected_range,
            bounds,
            scroll_offset,
            line_height,
            colors.selection,
            window,
        );
    }

    if content.is_empty() {
        if let Some(placeholder_str) = placeholder {
            if !placeholder_str.is_empty() {
                paint_placeholder(
                    placeholder_str,
                    bounds,
                    text_style,
                    colors.placeholder,
                    window,
                    cx,
                    false,
                );
            }
        }
    } else {
        paint_multiline_text(
            &line_layouts,
            bounds,
            scroll_offset,
            line_height,
            window,
            cx,
        );
    }

    if let Some(marked_range) = &marked_range {
        if !marked_range.is_empty() {
            paint_multiline_marked_underline(
                &line_layouts,
                marked_range,
                bounds,
                scroll_offset,
                line_height,
                colors.cursor,
                window,
            );
        }
    }

    if is_focused && selected_range.is_empty() && cursor_visible {
        paint_multiline_cursor(
            &line_layouts,
            cursor_offset,
            &content,
            bounds,
            scroll_offset,
            line_height,
            colors.cursor,
            window,
        );
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

fn paint_multiline_selection(
    line_layouts: &[InputLineLayout],
    selected_range: &std::ops::Range<usize>,
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
    line_height: Pixels,
    selection_color: Hsla,
    window: &mut Window,
) {
    for line in line_layouts {
        let line_y = line.y_offset - scroll_offset;

        if !is_line_visible(
            line_y,
            line_height,
            line.visual_line_count,
            bounds.size.height,
        ) {
            continue;
        }

        if !line_intersects_range(&line.text_range, selected_range) {
            continue;
        }

        if line.text_range.is_empty() {
            let empty_line_selection_width = px(6.);
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left(), bounds.top() + line_y),
                    point(
                        bounds.left() + empty_line_selection_width,
                        bounds.top() + line_y + line_height,
                    ),
                ),
                selection_color,
            ));
        } else if let Some(wrapped) = &line.wrapped_line {
            let line_start = line.text_range.start;
            let line_end = line.text_range.end;

            let sel_start = selected_range.start.max(line_start) - line_start;
            let sel_end = selected_range.end.min(line_end) - line_start;

            let start_pos = wrapped
                .position_for_index(sel_start, line_height)
                .unwrap_or(point(px(0.), px(0.)));
            let end_pos = wrapped
                .position_for_index(sel_end, line_height)
                .unwrap_or_else(|| {
                    let last_line_y = line_height * (line.visual_line_count - 1) as f32;
                    point(wrapped.width(), last_line_y)
                });

            let start_visual_line = compute_visual_line_index(start_pos.y, line_height);
            let end_visual_line = compute_visual_line_index(end_pos.y, line_height);

            if start_visual_line == end_visual_line {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + start_pos.x,
                            bounds.top() + line_y + start_pos.y,
                        ),
                        point(
                            bounds.left() + end_pos.x,
                            bounds.top() + line_y + start_pos.y + line_height,
                        ),
                    ),
                    selection_color,
                ));
            } else {
                let line_width = wrapped.width();

                // First visual line
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + start_pos.x,
                            bounds.top() + line_y + start_pos.y,
                        ),
                        point(
                            bounds.left() + line_width,
                            bounds.top() + line_y + start_pos.y + line_height,
                        ),
                    ),
                    selection_color,
                ));

                // Middle visual lines
                for visual_line in (start_visual_line + 1)..end_visual_line {
                    let y = line_height * visual_line as f32;
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(bounds.left(), bounds.top() + line_y + y),
                            point(
                                bounds.left() + line_width,
                                bounds.top() + line_y + y + line_height,
                            ),
                        ),
                        selection_color,
                    ));
                }

                // Last visual line
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(bounds.left(), bounds.top() + line_y + end_pos.y),
                        point(
                            bounds.left() + end_pos.x,
                            bounds.top() + line_y + end_pos.y + line_height,
                        ),
                    ),
                    selection_color,
                ));
            }
        }
    }
}

fn paint_multiline_text(
    line_layouts: &[InputLineLayout],
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
    line_height: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    for line_layout in line_layouts {
        let line_y = line_layout.y_offset - scroll_offset;

        if !is_line_visible(
            line_y,
            line_height,
            line_layout.visual_line_count,
            bounds.size.height,
        ) {
            continue;
        }

        if let Some(wrapped) = &line_layout.wrapped_line {
            let paint_pos = point(bounds.left(), bounds.top() + line_y);
            let _ = wrapped.paint(
                paint_pos,
                line_height,
                TextAlign::Left,
                Some(bounds),
                window,
                cx,
            );
        }
    }
}

fn paint_multiline_marked_underline(
    line_layouts: &[InputLineLayout],
    marked_range: &std::ops::Range<usize>,
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
    line_height: Pixels,
    underline_color: Hsla,
    window: &mut Window,
) {
    let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
    let underline_offset = line_height - underline_thickness;

    for line in line_layouts {
        let line_y = line.y_offset - scroll_offset;

        if !is_line_visible(
            line_y,
            line_height,
            line.visual_line_count,
            bounds.size.height,
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
                .position_for_index(mark_start, line_height)
                .unwrap_or(point(px(0.), px(0.)));
            let end_pos = wrapped
                .position_for_index(mark_end, line_height)
                .unwrap_or_else(|| {
                    let last_line_y = line_height * (line.visual_line_count - 1) as f32;
                    point(wrapped.width(), last_line_y)
                });

            let start_visual_line = compute_visual_line_index(start_pos.y, line_height);
            let end_visual_line = compute_visual_line_index(end_pos.y, line_height);

            if start_visual_line == end_visual_line {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + start_pos.x,
                            bounds.top() + line_y + start_pos.y + underline_offset,
                        ),
                        point(
                            bounds.left() + end_pos.x,
                            bounds.top() + line_y + start_pos.y + line_height,
                        ),
                    ),
                    underline_color,
                ));
            } else {
                // First visual line
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + start_pos.x,
                            bounds.top() + line_y + start_pos.y + underline_offset,
                        ),
                        point(
                            bounds.left() + wrapped.width(),
                            bounds.top() + line_y + start_pos.y + line_height,
                        ),
                    ),
                    underline_color,
                ));

                // Middle visual lines
                for visual_line in (start_visual_line + 1)..end_visual_line {
                    let y = line_height * visual_line as f32;
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(bounds.left(), bounds.top() + line_y + y + underline_offset),
                            point(
                                bounds.left() + wrapped.width(),
                                bounds.top() + line_y + y + line_height,
                            ),
                        ),
                        underline_color,
                    ));
                }

                // Last visual line
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left(),
                            bounds.top() + line_y + end_pos.y + underline_offset,
                        ),
                        point(
                            bounds.left() + end_pos.x,
                            bounds.top() + line_y + end_pos.y + line_height,
                        ),
                    ),
                    underline_color,
                ));
            }
        }
    }
}

fn paint_multiline_cursor(
    line_layouts: &[InputLineLayout],
    cursor_offset: usize,
    _content: &str,
    bounds: Bounds<Pixels>,
    scroll_offset: Pixels,
    line_height: Pixels,
    cursor_color: Hsla,
    window: &mut Window,
) {
    for line in line_layouts.iter() {
        let line_y = line.y_offset - scroll_offset;

        if !is_line_visible(
            line_y,
            line_height,
            line.visual_line_count,
            bounds.size.height,
        ) {
            continue;
        }

        // Since range is non-inclusive of the end value we need to check for it explicitly
        let is_cursor_in_line = if line.text_range.is_empty() {
            cursor_offset == line.text_range.start
        } else {
            line.text_range.contains(&cursor_offset) || cursor_offset == line.text_range.end
        };

        if !is_cursor_in_line {
            continue;
        }

        let cursor_position = if let Some(wrapped) = &line.wrapped_line {
            let local_offset = cursor_offset.saturating_sub(line.text_range.start);
            wrapped
                .position_for_index(local_offset, line_height)
                .unwrap_or(point(px(0.), px(0.)))
        } else {
            point(px(0.), px(0.))
        };

        window.paint_quad(fill(
            Bounds::new(
                point(
                    bounds.left() + cursor_position.x,
                    bounds.top() + line_y + cursor_position.y,
                ),
                size(px(CURSOR_WIDTH), line_height),
            ),
            cursor_color,
        ));
        break;
    }
}

/// State for single-line painting that pre-computes character positions.
struct SingleLinePaintState {
    content: String,
    selected_range: std::ops::Range<usize>,
    marked_range: Option<std::ops::Range<usize>>,
    cursor_offset: usize,
    scroll_offset: Pixels,
    line_height: Pixels,
    text_width: Pixels,
    is_focused: bool,
    char_positions: Vec<Pixels>,
    wrapped_line: Option<Arc<WrappedLine>>,
}

impl SingleLinePaintState {
    fn from_input(
        input: &Entity<InputState>,
        focus_handle: &FocusHandle,
        window: &Window,
        cx: &App,
    ) -> Self {
        let input_state = input.read(cx);

        let mut char_positions = Vec::new();
        let mut text_width = px(0.);

        if let Some(line) = input_state.line_layouts.first() {
            if let Some(wrapped) = &line.wrapped_line {
                text_width = wrapped.width();
                let content = input_state.content();
                let mut idx = 0;
                for ch in content.chars() {
                    if let Some(pos) = wrapped.position_for_index(idx, input_state.line_height) {
                        char_positions.push(pos.x);
                    } else {
                        char_positions.push(text_width);
                    }
                    idx += ch.len_utf8();
                }
                char_positions.push(text_width);
            }
        }

        let wrapped_line = input_state
            .line_layouts
            .first()
            .and_then(|l| l.wrapped_line.clone());

        Self {
            content: input_state.content().to_string(),
            selected_range: input_state.selected_range().clone(),
            marked_range: input_state.marked_range().cloned(),
            cursor_offset: input_state.cursor_offset(),
            scroll_offset: input_state.scroll_offset,
            line_height: input_state.line_height,
            text_width,
            is_focused: focus_handle.is_focused(window),
            char_positions,
            wrapped_line,
        }
    }

    fn x_for_index(&self, index: usize) -> Pixels {
        let char_index = self.content[..index.min(self.content.len())]
            .chars()
            .count();
        self.char_positions
            .get(char_index)
            .copied()
            .unwrap_or(self.text_width)
    }
}

fn paint_singleline(
    input: &Entity<InputState>,
    focus_handle: &FocusHandle,
    bounds: Bounds<Pixels>,
    text_style: &TextStyle,
    placeholder: Option<&SharedString>,
    colors: &PaintColors,
    cursor_visible: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let state = SingleLinePaintState::from_input(input, focus_handle, window, cx);

    if !state.selected_range.is_empty() {
        paint_singleline_selection(&state, bounds, colors.selection, window);
    }

    if state.content.is_empty() {
        if let Some(placeholder_str) = placeholder {
            if !placeholder_str.is_empty() {
                paint_placeholder(
                    placeholder_str,
                    bounds,
                    text_style,
                    colors.placeholder,
                    window,
                    cx,
                    true,
                );
            }
        }
    } else {
        paint_singleline_text(&state, bounds, window, cx);
    }

    if let Some(marked_range) = &state.marked_range {
        if !marked_range.is_empty() {
            paint_singleline_marked_underline(&state, marked_range, bounds, colors.cursor, window);
        }
    }

    if state.is_focused && state.selected_range.is_empty() && cursor_visible {
        paint_singleline_cursor(&state, bounds, colors.cursor, window);
    }
}

fn paint_singleline_selection(
    state: &SingleLinePaintState,
    bounds: Bounds<Pixels>,
    selection_color: Hsla,
    window: &mut Window,
) {
    let start_x = state.x_for_index(state.selected_range.start) - state.scroll_offset;
    let end_x = state.x_for_index(state.selected_range.end) - state.scroll_offset;

    let y_offset = (bounds.size.height - state.line_height).max(px(0.)) / 2.0;

    window.paint_quad(fill(
        Bounds::from_corners(
            point(bounds.left() + start_x, bounds.top() + y_offset),
            point(
                bounds.left() + end_x,
                bounds.top() + y_offset + state.line_height,
            ),
        ),
        selection_color,
    ));
}

fn paint_placeholder(
    placeholder: &SharedString,
    bounds: Bounds<Pixels>,
    text_style: &TextStyle,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
    baseline: bool,
) {
    let run = TextRun {
        len: placeholder.len(),
        font: text_style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    let font_size = text_style.font_size.to_pixels(window.rem_size());
    let shaped_line = window
        .text_system()
        .shape_line(placeholder.clone(), font_size, &[run], None);
    let line_height = text_style.line_height_in_pixels(window.rem_size());

    let mut paint_origin = bounds.origin;
    if baseline {
        let y_offset = (bounds.size.height - line_height).max(px(0.)) / 2.0;
        paint_origin.y += y_offset;
    }

    let _ = shaped_line.paint(paint_origin, line_height, TextAlign::Left, None, window, cx);
}

fn paint_singleline_text(
    state: &SingleLinePaintState,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(wrapped_line) = &state.wrapped_line else {
        return;
    };

    let y_offset = (bounds.size.height - state.line_height).max(px(0.)) / 2.0;
    let paint_origin = point(
        bounds.origin.x - state.scroll_offset,
        bounds.origin.y + y_offset,
    );

    let _ = wrapped_line.paint(
        paint_origin,
        state.line_height,
        TextAlign::Left,
        Some(bounds),
        window,
        cx,
    );
}

fn paint_singleline_marked_underline(
    state: &SingleLinePaintState,
    marked_range: &std::ops::Range<usize>,
    bounds: Bounds<Pixels>,
    underline_color: Hsla,
    window: &mut Window,
) {
    let start_x = state.x_for_index(marked_range.start) - state.scroll_offset;
    let end_x = state.x_for_index(marked_range.end) - state.scroll_offset;

    let underline_thickness = px(MARKED_TEXT_UNDERLINE_THICKNESS);
    let y_offset = (bounds.size.height - state.line_height).max(px(0.)) / 2.0;
    let underline_y = bounds.top() + y_offset + state.line_height - underline_thickness;

    window.paint_quad(fill(
        Bounds::from_corners(
            point(bounds.left() + start_x, underline_y),
            point(bounds.left() + end_x, underline_y + underline_thickness),
        ),
        underline_color,
    ));
}

fn paint_singleline_cursor(
    state: &SingleLinePaintState,
    bounds: Bounds<Pixels>,
    cursor_color: Hsla,
    window: &mut Window,
) {
    let cursor_x = state.x_for_index(state.cursor_offset) - state.scroll_offset;

    let y_offset = (bounds.size.height - state.line_height).max(px(0.)) / 2.0;

    window.paint_quad(fill(
        Bounds::new(
            point(bounds.left() + cursor_x, bounds.top() + y_offset),
            size(px(CURSOR_WIDTH), state.line_height),
        ),
        cursor_color,
    ));
}

impl Focusable for Input {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl IntoElement for Input {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Default interval for grouping consecutive edits into a single undo entry.
const DEFAULT_GROUP_INTERVAL: Duration = Duration::from_millis(300);

/// Maximum number of history entries to keep.
const MAX_HISTORY_LEN: usize = 1000;

/// Events emitted by InputState when significant changes occur.
#[derive(Clone, Debug)]
pub enum InputStateEvent {
    /// Emitted when the input gains focus.
    Focus,
    /// Emitted when the input loses focus.
    Blur,
    /// Emitted when the text content changes.
    TextChanged,
    /// Emitted when an undo operation is performed.
    Undo,
    /// Emitted when a redo operation is performed.
    Redo,
}

impl EventEmitter<InputStateEvent> for InputState {}

/// A patch-based history entry for memory-efficient undo/redo operations.
/// Instead of storing the full content, we store only the change needed to reverse the edit.
#[derive(Clone, Debug)]
struct HistoryEntry {
    /// The byte range that was modified (after the edit, for undo; before the edit, for redo).
    range: Range<usize>,
    /// The text that was replaced (to restore on undo).
    old_text: String,
    /// The length of the new text that replaced old_text (to know how much to remove on undo).
    new_text_len: usize,
    /// The selection range before the edit.
    selected_range: Range<usize>,
    /// Whether the selection was reversed before the edit.
    selection_reversed: bool,
    /// Timestamp for grouping consecutive edits.
    timestamp: Instant,
}

impl HistoryEntry {
    /// Apply this patch to undo an edit, returning the reverse patch for redo.
    fn apply_undo(&self, content: &mut String) -> HistoryEntry {
        let undo_start = self.range.start;
        let undo_end = (self.range.start + self.new_text_len).min(content.len());

        // Capture what we're about to remove (the "new" text that was inserted)
        let removed_text = content[undo_start..undo_end].to_string();

        // Replace with the old text
        content.replace_range(undo_start..undo_end, &self.old_text);

        // Return reverse patch for redo
        HistoryEntry {
            range: undo_start..undo_start + self.old_text.len(),
            old_text: removed_text,
            new_text_len: self.old_text.len(),
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            timestamp: self.timestamp,
        }
    }

    /// Apply this patch to redo an edit, returning the reverse patch for undo.
    fn apply_redo(&self, content: &mut String) -> HistoryEntry {
        // Redo is the same operation as undo - we're reversing the undo
        self.apply_undo(content)
    }
}

/// `Input` is the state model for text input components. It handles:
/// - Text content storage and manipulation
/// - Selection and cursor management
/// - Keyboard navigation and editing actions
/// - IME (Input Method Editor) support via `EntityInputHandler`
/// ```
pub struct InputState {
    entity_id: EntityId,
    focus_handle: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    pub(crate) line_height: Pixels,
    pub(crate) line_layouts: Vec<InputLineLayout>,
    pub(crate) wrap_width: Option<Pixels>,
    pub(crate) text_style: Option<TextStyle>,
    pub(crate) needs_layout: bool,
    is_selecting: bool,
    last_click_position: Option<Point<Pixels>>,
    click_count: usize,
    /// Scroll offset - vertical for multiline, horizontal for single-line
    pub(crate) scroll_offset: Pixels,
    pub(crate) available_height: Pixels,
    pub(crate) available_width: Pixels,
    multiline: bool,
    /// Stack of previous states for undo.
    undo_stack: Vec<HistoryEntry>,
    /// Stack of undone states for redo.
    redo_stack: Vec<HistoryEntry>,
    /// Optional entity and subscription tracking the blinking of the text cursor.
    cursor_blink: Option<(Entity<super::CursorBlink>, Subscription)>,
    /// Tracks whether we were focused on the last update.
    was_focused: bool,
    /// Cached UTF-16 length of content for faster IME operations.
    /// Lazily computed when None.
    cached_utf16_len: Option<usize>,
}

/// Layout information for a single logical line of text in an input.
///
/// A logical line corresponds to content between newlines in the input text.
/// When text wrapping is enabled, a logical line may span multiple visual lines.
#[derive(Clone, Debug)]
pub struct InputLineLayout {
    /// The byte range in the content string that this line covers.
    pub text_range: Range<usize>,
    /// The shaped and wrapped text for this line, if available.
    pub wrapped_line: Option<Arc<WrappedLine>>,
    /// The vertical offset from the top of the text area in pixels.
    pub y_offset: Pixels,
    /// The number of visual lines this logical line spans (due to wrapping).
    pub visual_line_count: usize,
}

pub enum CursorBlinkType<'app> {
    Disabled,
    Enabled {
        app: &'app mut App,
        interval: Option<Duration>,
    },
}

impl InputState {
    /// Creates a new `Input` with the specified multiline setting.
    /// Cursor blinking is enabled by default.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            entity_id: cx.entity_id(),
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            line_height: px(0.),
            line_layouts: Vec::new(),
            wrap_width: None,
            text_style: None,
            needs_layout: true,
            is_selecting: false,
            last_click_position: None,
            click_count: 0,
            scroll_offset: px(0.),
            available_height: px(0.),
            available_width: px(0.),
            multiline: false,
            undo_stack: Vec::new(),
            cached_utf16_len: None,
            redo_stack: Vec::new(),
            cursor_blink: None,
            was_focused: false,
        };
        this = this.cursor_blink(CursorBlinkType::Enabled {
            app: cx,
            interval: None,
        });
        this
    }

    pub fn cursor_blink<'app>(mut self, args: CursorBlinkType<'app>) -> Self {
        self.cursor_blink = match args {
            CursorBlinkType::Disabled => None,
            CursorBlinkType::Enabled { app: cx, interval } => {
                let interval = interval.unwrap_or(DEFAULT_BLINK_INTERVAL);
                let cursor_blink = cx.new(|cx| super::CursorBlink::new(interval, cx));
                let entity_id = self.entity_id;
                let subscription = cx.observe(&cursor_blink, move |_, cx| cx.notify(entity_id));
                Some((cursor_blink, subscription))
            }
        };
        self
    }

    /// Returns whether the cursor should be visible (for blinking).
    ///
    /// If blinking is not enabled, always returns `true`.
    /// This method also updates the blink manager's enabled state based on focus.
    pub fn cursor_visible(&mut self, is_focused: bool, cx: &mut Context<Self>) -> bool {
        // Update cursor blink based on focus changes
        if let Some((cursor_blink, _)) = &self.cursor_blink {
            if is_focused && !self.was_focused {
                cursor_blink.update(cx, |cb, cx| cb.enable(cx));
                cx.emit(InputStateEvent::Focus);
            } else if !is_focused && self.was_focused {
                cursor_blink.update(cx, |cb, cx| cb.disable(cx));
                cx.emit(InputStateEvent::Blur);
            }
        }
        self.was_focused = is_focused;

        self.cursor_blink
            .as_ref()
            .map(|(cb, _)| cb.read(cx).visible())
            .unwrap_or(true)
    }

    /// Pauses cursor blinking temporarily (e.g., during typing).
    fn pause_cursor_blink(&self, cx: &mut Context<Self>) {
        if let Some((cursor_blink, _)) = &self.cursor_blink {
            cursor_blink.update(cx, |cb, cx| cb.pause_blinking(cx));
        }
    }

    /// Sets the text style used for layout. Marks layout as dirty if the style changed.
    pub(crate) fn set_text_style(&mut self, style: &TextStyle) {
        let changed = self
            .text_style
            .as_ref()
            .map_or(true, |current| current != style);

        if changed {
            self.text_style = Some(style.clone());
            self.needs_layout = true;
        }
    }

    /// Returns the current text content.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Sets the text content, resetting selection to the beginning.
    /// This clears the undo/redo history.
    pub fn set_content(&mut self, content: impl Into<String>, cx: &mut Context<Self>) {
        let content = content.into();
        self.content = if self.multiline {
            content
        } else {
            // Strip newlines for single-line input
            content.replace('\n', " ").replace('\r', "")
        };
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.needs_layout = true;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.cached_utf16_len = None;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Returns whether undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns whether redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Records a patch for undo. Called before making changes to content.
    /// Returns true if a new entry was created, false if grouped with previous.
    fn push_undo_patch(&mut self, range: Range<usize>, new_text_len: usize) {
        // Don't record during IME composition
        if self.marked_range.is_some() {
            return;
        }

        let now = Instant::now();

        // Check if we should group with the last entry
        if let Some(last) = self.undo_stack.last() {
            if now.duration_since(last.timestamp) < DEFAULT_GROUP_INTERVAL {
                // Within group interval - extend the existing patch
                // We need to merge this edit with the previous one
                return;
            }
        }

        // Capture the text that will be replaced
        let old_text = self.content[range.clone()].to_string();

        self.undo_stack.push(HistoryEntry {
            range: range.start..range.start + new_text_len,
            old_text,
            new_text_len,
            selected_range: self.selected_range.clone(),
            selection_reversed: self.selection_reversed,
            timestamp: now,
        });

        // Limit history size
        if self.undo_stack.len() > MAX_HISTORY_LEN {
            self.undo_stack.remove(0);
        }

        // New edit invalidates redo stack
        self.redo_stack.clear();
    }

    /// Undoes the last edit by applying the reverse patch.
    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            // Remember selection to restore
            let selected_range = entry.selected_range.clone();
            let selection_reversed = entry.selection_reversed;

            // Apply the undo patch and get the redo patch
            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            // Restore selection state
            self.selected_range = selected_range;
            self.selection_reversed = selection_reversed;
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    /// Redoes the last undone edit by applying the forward patch.
    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            // Apply the redo patch and get the undo patch
            let undo_entry = entry.apply_redo(&mut self.content);

            // The undo entry contains the selection state after the original edit
            // We need to restore cursor to end of inserted text
            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_reversed = false;

            self.undo_stack.push(undo_entry);
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }

    /// Returns the placeholder text shown when content is empty.
    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    /// Sets the placeholder text.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Returns the current selection range.
    pub fn selected_range(&self) -> &Range<usize> {
        &self.selected_range
    }

    /// Returns true if the selection is reversed (cursor at start).
    pub fn selection_reversed(&self) -> bool {
        self.selection_reversed
    }

    /// Returns the current cursor offset.
    pub fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Returns the marked text range (for IME composition).
    pub fn marked_range(&self) -> Option<&Range<usize>> {
        self.marked_range.as_ref()
    }

    /// Sets the selection range directly.
    pub fn set_selected_range(&mut self, range: Range<usize>) {
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.selected_range = range;
        self.selection_reversed = false;
    }

    /// Returns the selected text range in UTF-16 offsets (for IME).
    pub fn selected_text_range_utf16(&self) -> Range<usize> {
        self.range_to_utf16(&self.selected_range)
    }

    /// Inserts text at the current cursor position, replacing any selection.
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self
            .marked_range
            .clone()
            .unwrap_or(self.selected_range.clone());
        let range = range.start.min(self.content.len())..range.end.min(self.content.len());

        let sanitized_text;
        let text_to_insert = if self.multiline {
            text
        } else {
            sanitized_text = text.replace('\n', " ").replace('\r', "");
            &sanitized_text
        };

        // Record patch for undo before modifying content
        self.push_undo_patch(range.clone(), text_to_insert.len());

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.content.replace_range(range.clone(), text_to_insert);
        self.selected_range =
            range.start + text_to_insert.len()..range.start + text_to_insert.len();
        self.marked_range.take();
        self.needs_layout = true;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    /// Deletes the character before the cursor (convenience method for benchmarks).
    pub fn delete_backward(&mut self, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.insert_text("", cx);
    }

    /// Undoes the last edit (convenience method without Window).
    pub fn undo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.undo_stack.pop() {
            let selected_range = entry.selected_range.clone();
            let selection_reversed = entry.selection_reversed;

            let redo_entry = entry.apply_undo(&mut self.content);
            self.redo_stack.push(redo_entry);

            self.selected_range = selected_range;
            self.selection_reversed = selection_reversed;
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Undo);
            cx.notify();
        }
    }

    /// Redoes the last undone edit (convenience method without Window).
    pub fn redo_action(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.redo_stack.pop() {
            let undo_entry = entry.apply_redo(&mut self.content);

            let cursor_pos = undo_entry.range.start;
            self.selected_range = cursor_pos..cursor_pos;
            self.selection_reversed = false;

            self.undo_stack.push(undo_entry);
            self.needs_layout = true;
            self.cached_utf16_len = None;
            self.scroll_to_cursor();
            cx.emit(InputStateEvent::Redo);
            cx.notify();
        }
    }

    /// Selects all text.
    pub fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    pub(crate) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.previous_boundary(self.cursor_offset());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    pub(crate) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let new_pos = self.next_boundary(self.cursor_offset());
            self.move_to(new_pos, cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    pub(crate) fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        if !self.multiline {
            // In single-line mode, up moves to start
            self.selected_range = 0..0;
            self.selection_reversed = false;
            self.scroll_to_cursor();
            cx.notify();
            return;
        }
        if let Some(new_offset) = self.move_vertically(self.cursor_offset(), -1) {
            self.selected_range = new_offset..new_offset;
            self.selection_reversed = false;
            self.scroll_to_cursor();
            cx.notify();
        }
    }

    pub(crate) fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        if !self.multiline {
            // In single-line mode, down moves to end
            let end = self.content.len();
            self.selected_range = end..end;
            self.selection_reversed = false;
            self.scroll_to_cursor();
            cx.notify();
            return;
        }
        if let Some(new_offset) = self.move_vertically(self.cursor_offset(), 1) {
            self.selected_range = new_offset..new_offset;
            self.selection_reversed = false;
            self.scroll_to_cursor();
            cx.notify();
        }
    }

    pub(crate) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    pub(crate) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    pub(crate) fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        if !self.multiline {
            // In single-line mode, select_up selects to start
            self.select_to(0, cx);
            return;
        }
        if let Some(new_offset) = self.move_vertically(self.cursor_offset(), -1) {
            if self.selection_reversed {
                self.selected_range.start = new_offset;
            } else {
                self.selected_range.end = new_offset;
            }
            if self.selected_range.end < self.selected_range.start {
                self.selection_reversed = !self.selection_reversed;
                self.selected_range = self.selected_range.end..self.selected_range.start;
            }
            self.scroll_to_cursor();
            cx.notify();
        }
    }

    pub(crate) fn select_down(
        &mut self,
        _: &SelectDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pause_cursor_blink(cx);
        if !self.multiline {
            // In single-line mode, select_down selects to end
            self.select_to(self.content.len(), cx);
            return;
        }
        if let Some(new_offset) = self.move_vertically(self.cursor_offset(), 1) {
            if self.selection_reversed {
                self.selected_range.start = new_offset;
            } else {
                self.selected_range.end = new_offset;
            }
            if self.selected_range.end < self.selected_range.start {
                self.selection_reversed = !self.selection_reversed;
                self.selected_range = self.selected_range.end..self.selected_range.start;
            }
            self.scroll_to_cursor();
            cx.notify();
        }
    }

    pub(crate) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let line_start = self.find_line_start(self.cursor_offset());
        self.move_to(line_start, cx);
    }

    pub(crate) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let line_end = self.find_line_end(self.cursor_offset());
        self.move_to(line_end, cx);
    }

    pub(crate) fn move_to_beginning(
        &mut self,
        _: &MoveToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(0, cx);
    }

    pub(crate) fn move_to_end(&mut self, _: &MoveToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub(crate) fn select_to_beginning(
        &mut self,
        _: &SelectToBeginning,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    pub(crate) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.content.len(), cx);
    }

    pub(crate) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.previous_word_boundary(self.cursor_offset());
        self.move_to(new_pos, cx);
    }

    pub(crate) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let new_pos = self.next_word_boundary(self.cursor_offset());
        self.move_to(new_pos, cx);
    }

    pub(crate) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.previous_word_boundary(self.cursor_offset());
        self.select_to(new_pos, cx);
    }

    pub(crate) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pos = self.next_word_boundary(self.cursor_offset());
        self.select_to(new_pos, cx);
    }

    pub(crate) fn enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    pub(crate) fn tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\t", window, cx);
    }

    pub(crate) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_word_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.find_line_end(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if self.multiline {
                self.replace_text_in_range(None, &text, window, cx);
            } else {
                // Strip newlines for single-line input
                let text = text.replace('\n', " ").replace('\r', "");
                self.replace_text_in_range(None, &text, window, cx);
            }
        }
    }

    pub(crate) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    pub(crate) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            // Cut selected text
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        } else {
            // No selection: cut the entire current line (including newline)
            let cursor = self.cursor_offset();
            let line_start = self.find_line_start(cursor);
            let line_end = self.find_line_end(cursor);

            // Include the newline character if there is one after the line
            let cut_end = if line_end < self.content.len() {
                line_end + 1 // Include the newline
            } else if line_start > 0 {
                // Last line with no trailing newline - include preceding newline instead
                line_end
            } else {
                line_end
            };

            // For last line, also remove the preceding newline if it exists
            let cut_start = if line_end >= self.content.len() && line_start > 0 {
                line_start - 1 // Include preceding newline for last line
            } else {
                line_start
            };

            let line_text = self.content[cut_start..cut_end].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(line_text));

            self.selected_range = cut_start..cut_end;
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    pub(crate) fn on_mouse_down(
        &mut self,
        position: Point<Pixels>,
        click_count: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        let is_same_position = self
            .last_click_position
            .map(|last| {
                let threshold = px(4.);
                (position.x - last.x).abs() < threshold && (position.y - last.y).abs() < threshold
            })
            .unwrap_or(false);

        if is_same_position && click_count > 1 {
            self.click_count = click_count;
        } else {
            self.click_count = 1;
        }
        self.last_click_position = Some(position);

        let clicked_offset = self.index_for_position(position);

        match self.click_count {
            2 => {
                let (word_start, word_end) = self.word_range_at(clicked_offset);
                self.selected_range = word_start..word_end;
                self.selection_reversed = false;
                cx.notify();
            }
            3 => {
                let line_start = self.find_line_start(clicked_offset);
                let line_end = self.find_line_end(clicked_offset);
                let line_end_with_newline = if line_end < self.content.len() {
                    line_end + 1
                } else {
                    line_end
                };
                self.selected_range = line_start..line_end_with_newline;
                self.selection_reversed = false;
                cx.notify();
            }
            _ => {
                if shift {
                    self.select_to(clicked_offset, cx);
                } else {
                    self.move_to(clicked_offset, cx);
                }
            }
        }
    }

    pub(crate) fn on_mouse_up(&mut self, _cx: &mut Context<Self>) {
        self.is_selecting = false;
    }

    pub(crate) fn on_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.is_selecting && self.click_count == 1 {
            self.select_to(self.index_for_position(position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.scroll_to_cursor();
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.pause_cursor_blink(cx);
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.scroll_to_cursor();
        cx.notify();
    }

    pub(crate) fn find_line_start(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    pub(crate) fn find_line_end(&self, offset: usize) -> usize {
        self.content[offset.min(self.content.len())..]
            .find('\n')
            .map(|pos| offset + pos)
            .unwrap_or(self.content.len())
    }

    fn move_vertically(&self, offset: usize, direction: i32) -> Option<usize> {
        let (visual_line_idx, x_pixels) = self.find_visual_line_and_x_offset(offset);
        let target_visual_line_idx = (visual_line_idx as i32 + direction).max(0) as usize;

        let mut current_visual_line = 0;
        for layout in self.line_layouts.iter() {
            let visual_lines_in_layout = layout.visual_line_count;

            if target_visual_line_idx < current_visual_line + visual_lines_in_layout {
                let visual_line_within_layout = target_visual_line_idx - current_visual_line;

                if layout.text_range.is_empty() {
                    return Some(layout.text_range.start);
                }

                if let Some(wrapped) = &layout.wrapped_line {
                    let y_within_wrapped = self.line_height * visual_line_within_layout as f32;
                    let target_point = point(px(x_pixels), y_within_wrapped);

                    let closest_result =
                        wrapped.closest_index_for_position(target_point, self.line_height);

                    let closest_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = closest_idx.min(wrapped.text.len());
                    let result = layout.text_range.start + clamped;

                    return Some(result);
                }

                return Some(layout.text_range.start);
            }

            current_visual_line += visual_lines_in_layout;
        }

        if direction > 0 {
            Some(self.content.len())
        } else {
            None
        }
    }

    fn find_visual_line_and_x_offset(&self, offset: usize) -> (usize, f32) {
        if self.line_layouts.is_empty() {
            return (0, 0.0);
        }

        let mut visual_line_idx = 0;

        for line in &self.line_layouts {
            if line.text_range.is_empty() {
                if offset == line.text_range.start {
                    return (visual_line_idx, 0.0);
                }
            } else if offset >= line.text_range.start && offset <= line.text_range.end {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = (offset - line.text_range.start).min(wrapped.text.len());
                    if let Some(position) =
                        wrapped.position_for_index(local_offset, self.line_height)
                    {
                        let visual_line_within = (position.y / self.line_height).floor() as usize;
                        return (visual_line_idx + visual_line_within, position.x.into());
                    }
                }
                return (visual_line_idx, 0.0);
            }
            visual_line_idx += line.visual_line_count;
        }

        (visual_line_idx.saturating_sub(1), 0.0)
    }

    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        for line in self.line_layouts.iter() {
            let line_height_total = self.line_height * line.visual_line_count as f32;

            if position.y >= line.y_offset && position.y < line.y_offset + line_height_total {
                if line.text_range.is_empty() {
                    return line.text_range.start;
                }

                if let Some(wrapped) = &line.wrapped_line {
                    let relative_y = position.y - line.y_offset;
                    let relative_point = point(position.x, relative_y);

                    let closest_result =
                        wrapped.closest_index_for_position(relative_point, self.line_height);

                    let local_idx = closest_result.unwrap_or_else(|closest| closest);
                    let clamped = local_idx.min(wrapped.text.len());
                    return line.text_range.start + clamped;
                }
                return line.text_range.start;
            }
        }

        self.content.len()
    }

    pub(crate) fn scroll_to_cursor(&mut self) {
        if self.line_layouts.is_empty() {
            return;
        }

        let cursor_offset = self.cursor_offset();

        if self.multiline {
            self.scroll_to_cursor_vertical(cursor_offset);
        } else {
            self.scroll_to_cursor_horizontal(cursor_offset);
        }
    }

    fn scroll_to_cursor_vertical(&mut self, cursor_offset: usize) {
        if self.available_height <= px(0.) {
            return;
        }

        let line_height = self.line_height;

        for line in &self.line_layouts {
            let is_cursor_in_line = if line.text_range.is_empty() {
                cursor_offset == line.text_range.start
            } else {
                line.text_range.contains(&cursor_offset)
                    || (cursor_offset == line.text_range.end && cursor_offset == self.content.len())
            };

            if is_cursor_in_line {
                let cursor_visual_y = if let Some(wrapped) = &line.wrapped_line {
                    let local_offset = cursor_offset.saturating_sub(line.text_range.start);
                    if let Some(position) =
                        wrapped.position_for_index(local_offset, self.line_height)
                    {
                        line.y_offset + position.y
                    } else {
                        line.y_offset
                    }
                } else {
                    line.y_offset
                };

                let visible_top = self.scroll_offset;
                let visible_bottom = self.scroll_offset + self.available_height;

                if cursor_visual_y < visible_top {
                    self.scroll_offset = cursor_visual_y;
                } else if cursor_visual_y + line_height > visible_bottom {
                    self.scroll_offset = (cursor_visual_y + line_height) - self.available_height;
                }

                self.scroll_offset = self.scroll_offset.max(px(0.));
                break;
            }
        }
    }

    fn scroll_to_cursor_horizontal(&mut self, cursor_offset: usize) {
        if self.available_width <= px(0.) {
            return;
        }

        // For single-line input, get cursor x position from the first (only) line
        let Some(line) = self.line_layouts.first() else {
            return;
        };

        let cursor_x = if let Some(wrapped) = &line.wrapped_line {
            let local_offset = cursor_offset.saturating_sub(line.text_range.start);
            wrapped
                .position_for_index(local_offset, self.line_height)
                .map(|p| p.x)
                .unwrap_or(px(0.))
        } else {
            px(0.)
        };

        let visible_left = self.scroll_offset;
        let visible_right = self.scroll_offset + self.available_width;

        // Add some padding so cursor isn't right at the edge
        let padding = px(2.0);

        if cursor_x < visible_left + padding {
            self.scroll_offset = (cursor_x - padding).max(px(0.));
        } else if cursor_x > visible_right - padding {
            self.scroll_offset = cursor_x - self.available_width + padding;
        }

        self.scroll_offset = self.scroll_offset.max(px(0.));
    }

    pub(crate) fn update_line_layouts(
        &mut self,
        width: Pixels,
        line_height: Pixels,
        text_style: &TextStyle,
        window: &mut Window,
    ) {
        self.line_height = line_height;
        self.set_text_style(text_style);

        if !self.needs_layout && self.wrap_width == Some(width) {
            return;
        }

        self.line_layouts.clear();
        self.wrap_width = Some(width);

        let text_color = text_style.color;
        let font_size = text_style.font_size.to_pixels(window.rem_size());

        if self.content.is_empty() {
            self.line_layouts.push(InputLineLayout {
                text_range: 0..0,
                wrapped_line: None,
                y_offset: px(0.),
                visual_line_count: 1,
            });
            self.needs_layout = false;
            return;
        }

        let mut y_offset = px(0.);
        let mut current_pos = 0;

        while current_pos < self.content.len() {
            let line_end = self.content[current_pos..]
                .find('\n')
                .map(|pos| current_pos + pos)
                .unwrap_or(self.content.len());

            let line_text = &self.content[current_pos..line_end];

            if line_text.is_empty() {
                self.line_layouts.push(InputLineLayout {
                    text_range: current_pos..current_pos,
                    wrapped_line: None,
                    y_offset,
                    visual_line_count: 1,
                });
                y_offset += line_height;
            } else {
                let run = TextRun {
                    len: line_text.len(),
                    font: text_style.font(),
                    color: text_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };

                let wrapped_lines = window
                    .text_system()
                    .shape_text(
                        SharedString::from(line_text.to_string()),
                        font_size,
                        &[run],
                        Some(width),
                        None,
                    )
                    .unwrap_or_default();

                for wrapped in wrapped_lines {
                    let visual_line_count = wrapped.wrap_boundaries().len() + 1;
                    let line_height_total = line_height * visual_line_count as f32;

                    self.line_layouts.push(InputLineLayout {
                        text_range: current_pos..line_end,
                        wrapped_line: Some(Arc::new(wrapped)),
                        y_offset,
                        visual_line_count,
                    });

                    y_offset += line_height_total;
                }
            }

            current_pos = if line_end < self.content.len() {
                line_end + 1
            } else {
                self.content.len()
            };
        }

        if self.content.ends_with('\n') {
            self.line_layouts.push(InputLineLayout {
                text_range: self.content.len()..self.content.len(),
                wrapped_line: None,
                y_offset,
                visual_line_count: 1,
            });
        }

        self.needs_layout = false;
        self.scroll_to_cursor();
    }

    pub(crate) fn total_content_height(&self) -> Pixels {
        self.line_layouts
            .last()
            .map(|last| last.y_offset + self.line_height * last.visual_line_count as f32)
            .unwrap_or(px(0.))
    }

    /// Returns true if the scroll position is at the top.
    pub fn at_top(&self) -> bool {
        self.scroll_offset <= px(0.)
    }

    /// Returns true if the scroll position is at the bottom.
    pub fn at_bottom(&self) -> bool {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;

        if content_height <= visible_height {
            return true;
        }

        self.scroll_offset + visible_height >= content_height
    }

    /// Returns the scroll progress as a value from 0.0 (top) to 1.0 (bottom).
    pub fn scroll_progress(&self) -> f32 {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return 0.0;
        }

        (self.scroll_offset / max_scroll).clamp(0.0, 1.0)
    }

    /// Returns how far the content is scrolled from the top in pixels.
    pub fn distance_from_top(&self) -> Pixels {
        self.scroll_offset.max(px(0.))
    }

    /// Returns how far the content is from the bottom in pixels.
    pub fn distance_from_bottom(&self) -> Pixels {
        let content_height = self.total_content_height();
        let visible_height = self.available_height;
        let max_scroll = content_height - visible_height;

        if max_scroll <= px(0.) {
            return px(0.);
        }

        (max_scroll - self.scroll_offset).max(px(0.))
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if offset == 0 {
            return 0;
        }

        // Fast path: if we have cached length and offset is at or past end
        if let Some(utf16_len) = self.cached_utf16_len {
            if offset >= utf16_len {
                return self.content.len();
            }
        }

        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for character in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += character.len_utf16();
            utf8_offset += character.len_utf8();
        }

        utf8_offset.min(self.content.len())
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if offset == 0 {
            return 0;
        }

        // Fast path: if offset is at or past end, return cached length
        if offset >= self.content.len() {
            return self.utf16_len();
        }

        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for character in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += character.len_utf8();
            utf16_offset += character.len_utf16();
        }

        utf16_offset
    }

    /// Returns the UTF-16 length of the content, computing and caching if necessary.
    fn utf16_len(&self) -> usize {
        if let Some(len) = self.cached_utf16_len {
            return len;
        }
        self.content.chars().map(|c| c.len_utf16()).sum()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];
        text_before
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];
        text_after
            .grapheme_indices(true)
            .nth(1)
            .map(|(i, _)| offset + i)
            .unwrap_or(self.content.len())
    }

    fn previous_word_boundary(&self, offset: usize) -> usize {
        if offset == 0 {
            return 0;
        }

        let text_before = &self.content[..offset.min(self.content.len())];

        let mut last_word_start = 0;
        for (idx, _) in text_before.unicode_word_indices() {
            if idx < offset {
                last_word_start = idx;
            }
        }

        if last_word_start == 0 && offset > 0 {
            let trimmed = text_before.trim_end();
            if trimmed.is_empty() {
                return 0;
            }
            for (idx, _) in trimmed.unicode_word_indices() {
                last_word_start = idx;
            }
        }

        last_word_start
    }

    fn next_word_boundary(&self, offset: usize) -> usize {
        if offset >= self.content.len() {
            return self.content.len();
        }

        let text_after = &self.content[offset..];

        for (idx, word) in text_after.unicode_word_indices() {
            let word_end = offset + idx + word.len();
            if word_end > offset {
                return word_end;
            }
        }

        self.content.len()
    }

    fn word_range_at(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.content.len());

        for (idx, word) in self.content.unicode_word_indices() {
            let word_end = idx + word.len();
            if offset >= idx && offset <= word_end {
                return (idx, word_end);
            }
        }

        (offset, offset)
    }
}

impl EntityInputHandler for InputState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        let clamped_range = range.start.min(self.content.len())..range.end.min(self.content.len());
        adjusted_range.replace(self.range_to_utf16(&clamped_range));
        Some(self.content[clamped_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let range = range.start.min(self.content.len())..range.end.min(self.content.len());

        // Strip newlines for single-line input
        let sanitized_text;
        let text_to_insert = if self.multiline {
            new_text
        } else {
            sanitized_text = new_text.replace('\n', " ").replace('\r', "");
            &sanitized_text
        };

        // Record patch for undo before modifying content
        self.push_undo_patch(range.clone(), text_to_insert.len());

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.content.replace_range(range.clone(), text_to_insert);
        self.selected_range =
            range.start + text_to_insert.len()..range.start + text_to_insert.len();
        self.marked_range.take();
        self.needs_layout = true;
        self.pause_cursor_blink(cx);
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        let range = range.start.min(self.content.len())..range.end.min(self.content.len());

        // Strip newlines for single-line input
        let sanitized_text;
        let text_to_insert = if self.multiline {
            new_text
        } else {
            sanitized_text = new_text.replace('\n', " ").replace('\r', "");
            &sanitized_text
        };

        // Update cached UTF-16 length incrementally if available
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize = self.content[range.clone()]
                .chars()
                .map(|c| c.len_utf16())
                .sum();
            let added_utf16_len: usize = text_to_insert.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }

        self.content.replace_range(range.clone(), text_to_insert);

        if !text_to_insert.is_empty() {
            self.marked_range = Some(range.start..range.start + text_to_insert.len());
        } else {
            self.marked_range = None;
        }

        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| {
                range.start + text_to_insert.len()..range.start + text_to_insert.len()
            });

        self.needs_layout = true;
        cx.emit(InputStateEvent::TextChanged);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);

        for line in &self.line_layouts {
            if line.text_range.is_empty() {
                if range.start == line.text_range.start {
                    return Some(Bounds::from_corners(
                        point(bounds.left(), bounds.top() + line.y_offset),
                        point(
                            bounds.left() + px(4.),
                            bounds.top() + line.y_offset + self.line_height,
                        ),
                    ));
                }
            } else if line.text_range.contains(&range.start) {
                if let Some(wrapped) = &line.wrapped_line {
                    let local_start = range.start - line.text_range.start;
                    let local_end = (range.end - line.text_range.start).min(wrapped.text.len());

                    let start_pos = wrapped
                        .position_for_index(local_start, self.line_height)
                        .unwrap_or(point(px(0.), px(0.)));
                    let end_pos = wrapped
                        .position_for_index(local_end, self.line_height)
                        .unwrap_or_else(|| {
                            let last_line_y =
                                self.line_height * (line.visual_line_count - 1) as f32;
                            point(wrapped.width(), last_line_y)
                        });

                    let start_visual_line = (start_pos.y / self.line_height).floor() as usize;
                    let end_visual_line = (end_pos.y / self.line_height).floor() as usize;

                    if start_visual_line == end_visual_line {
                        return Some(Bounds::from_corners(
                            point(
                                bounds.left() + start_pos.x,
                                bounds.top() + line.y_offset + start_pos.y,
                            ),
                            point(
                                bounds.left() + end_pos.x,
                                bounds.top() + line.y_offset + start_pos.y + self.line_height,
                            ),
                        ));
                    } else {
                        return Some(Bounds::from_corners(
                            point(
                                bounds.left() + start_pos.x,
                                bounds.top() + line.y_offset + start_pos.y,
                            ),
                            point(
                                bounds.left() + wrapped.width(),
                                bounds.top() + line.y_offset + start_pos.y + self.line_height,
                            ),
                        ));
                    }
                }
            }
        }
        None
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let index = self.index_for_position(point);
        Some(self.offset_to_utf16(index))
    }
}

impl Focusable for InputState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
