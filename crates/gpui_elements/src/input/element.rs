use crate::input::{InputColors, InputState};
use gpui::{
    Action, App, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement, Interactivity,
    IntoElement, SharedString, StyleRefinement, Styled, Window,
};

#[track_caller]
pub fn input(input_state: &Entity<InputState>, cx: &App) -> Input {
    Input::new(input_state, cx)
}

/// A text editing element that supports both single-line and multi-line modes.
pub struct Input {
    pub(super) input: Entity<InputState>,
    pub(super) interactivity: Interactivity,
    pub(super) placeholder: Option<SharedString>,
    pub(super) colors: InputColors,
}

impl Input {
    #[track_caller]
    fn new(input_state: &Entity<InputState>, cx: &App) -> Self {
        let focus_handle = input_state.focus_handle(cx);
        let mut input = Input {
            input: input_state.clone(),
            interactivity: Interactivity::new(),
            placeholder: None,
            colors: InputColors::default(),
        };
        input.register_actions();
        input
            .key_context(super::actions::DEFAULT_INPUT_CONTEXT)
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
            .on_action::<super::actions::Escape>(|_action, window, _cx| {
                window.blur();
            });
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Sets the styling colors for the input element
    pub fn colors(mut self, colors: InputColors) -> Self {
        self.colors = colors;
        self
    }

    /// Sets the "selection" color for the input element.
    /// This is the background color applied to the range of text that is currently selected by the user.
    pub fn selection_color(mut self, color: Hsla) -> Self {
        self.colors.selection = color;
        self
    }

    /// Sets the "cursor" color for the input element.
    /// This is the color of the user's text cursor.
    pub fn cursor_color(mut self, color: Hsla) -> Self {
        self.colors.cursor = color;
        self
    }

    /// Sets the "placeholder" color for the input element.
    /// This is the color of the placeholder string, when one is assigned and the text field is empty.
    pub fn placeholder_color(mut self, color: Hsla) -> Self {
        self.colors.placeholder = color;
        self
    }

    /// Sets the "marked" color for the input element.
    /// Marking text comes from IME and needs further doc clarification.
    pub fn marked_color(mut self, color: Hsla) -> Self {
        self.colors.marked = color;
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
