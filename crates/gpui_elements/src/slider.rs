//! Unstyled, controlled sliders with composable labels, tracks, indicators, and thumbs.

use std::rc::Rc;

use gpui::{
    AccessibleAction, AnyElement, App, Axis, Bounds, ClickEvent, DispatchPhase, Div, ElementId,
    Entity, FocusHandle, InteractiveElement, Interactivity, IntoElement, KeyDownEvent, Length,
    MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, RenderOnce, Role, SharedString,
    Size, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, accesskit, canvas,
    div, prelude::FluentBuilder, px, relative,
};
use smallvec::SmallVec;

/// Shared handler for proposed slider value changes.
pub type OnChange = Rc<dyn Fn(&BaseSliderChange, &mut Window, &mut App)>;
/// Shared handler for committed slider value changes.
pub type OnCommit = Rc<dyn Fn(&BaseSliderChange, &mut Window, &mut App)>;

type ValueTextFormatter = Rc<dyn Fn(f32) -> SharedString>;

/// The root of an unstyled, controlled slider.
#[derive(IntoElement, Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSlider {
    element_id: ElementId,
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    values: BaseSliderValue,
    minimum: f32,
    maximum: f32,
    step: f32,
    large_step: f32,
    min_steps_between_values: usize,
    collision_behavior: BaseSliderThumbCollisionBehavior,
    thumb_alignment: BaseSliderThumbAlignment,
    orientation: Axis,
    disabled: bool,
    label: Option<BaseSliderLabel>,
    control: Option<BaseSliderControl>,
    #[children]
    children: Vec<AnyElement>,
    on_change: Option<OnChange>,
    on_commit: Option<OnCommit>,
    value_text_formatter: Option<ValueTextFormatter>,
}

impl InteractiveElement for BaseSlider {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSlider {
    /// Initial slider value and fallback for invalid values.
    pub const DEFAULT_VALUE: f32 = 0.;
    /// Initial lower bound and fallback for non-finite minimums.
    pub const DEFAULT_MINIMUM: f32 = 0.;
    /// Initial upper bound and fallback for non-finite maximums.
    pub const DEFAULT_MAXIMUM: f32 = 100.;
    /// Initial step size and fallback for non-finite step sizes.
    pub const DEFAULT_STEP: f32 = 1.;
    /// Initial large step size and fallback for non-finite large step sizes.
    pub const DEFAULT_LARGE_STEP: f32 = 10.;
    /// Initial minimum gap between adjacent values, measured in steps.
    pub const DEFAULT_MIN_STEPS_BETWEEN_VALUES: usize = 0;

    const MINIMUM_STEP: f32 = 0.0_f32.next_up();

    /// Creates a horizontal slider with the default numeric settings.
    /// Attach a control before rendering and a change handler to enable input.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        let element_id = element_id.into();

        Self {
            element_id: element_id.clone(),
            base: div().id(element_id),
            style: StyleRefinement::default(),
            values: BaseSliderValue::from(Self::DEFAULT_VALUE),
            minimum: Self::DEFAULT_MINIMUM,
            maximum: Self::DEFAULT_MAXIMUM,
            step: Self::DEFAULT_STEP,
            large_step: Self::DEFAULT_LARGE_STEP,
            min_steps_between_values: Self::DEFAULT_MIN_STEPS_BETWEEN_VALUES,
            collision_behavior: BaseSliderThumbCollisionBehavior::Push,
            thumb_alignment: BaseSliderThumbAlignment::Edge,
            orientation: Axis::Horizontal,
            disabled: false,
            label: None,
            control: None,
            children: Vec::new(),
            on_change: None,
            on_commit: None,
            value_text_formatter: None,
        }
    }

    /// Sets the application-controlled value of a single-thumb slider.
    /// See [`BaseSliderValue::new`] for invalid-input handling.
    pub fn value(mut self, value: f32) -> Self {
        self.values = BaseSliderValue::from(value);

        self
    }

    /// Sets the application-controlled values, sorted and clamped when rendered.
    /// See [`BaseSliderValue::new`] for invalid-input handling.
    pub fn values(mut self, values: impl IntoIterator<Item = f32>) -> Self {
        self.values = BaseSliderValue::new(values);

        self
    }

    /// Sets the inclusive lower bound, which must be finite.
    /// Invalid inputs trigger a debug assertion or use [`Self::DEFAULT_MINIMUM`]
    /// when debug assertions are disabled.
    pub fn min(mut self, minimum: f32) -> Self {
        debug_assert!(minimum.is_finite(), "the slider minimum must be finite");

        self.minimum = if minimum.is_finite() {
            minimum
        } else {
            Self::DEFAULT_MINIMUM
        };

        self
    }

    /// Sets the inclusive upper bound, which must be finite.
    /// Invalid inputs trigger a debug assertion or use [`Self::DEFAULT_MAXIMUM`]
    /// when debug assertions are disabled.
    pub fn max(mut self, maximum: f32) -> Self {
        debug_assert!(maximum.is_finite(), "the slider maximum must be finite");

        self.maximum = if maximum.is_finite() {
            maximum
        } else {
            Self::DEFAULT_MAXIMUM
        };

        self
    }

    /// Sets the positive, finite step used for snapping and arrow-key changes.
    /// With debug assertions disabled, non-finite inputs use [`Self::DEFAULT_STEP`]
    /// and nonpositive inputs clamp to the smallest positive float.
    pub fn step(mut self, step: f32) -> Self {
        debug_assert!(
            step.is_finite() && step > 0.,
            "the slider step must be finite and positive"
        );

        self.step = if step.is_finite() {
            step.max(Self::MINIMUM_STEP)
        } else {
            Self::DEFAULT_STEP
        };

        self
    }

    /// Sets the positive, finite increment for Page Up, Page Down, and Shift+Arrow.
    /// With debug assertions disabled, non-finite inputs use [`Self::DEFAULT_LARGE_STEP`]
    /// and nonpositive inputs clamp to the smallest positive float.
    pub fn large_step(mut self, large_step: f32) -> Self {
        debug_assert!(
            large_step.is_finite() && large_step > 0.,
            "the slider large step must be finite and positive"
        );

        self.large_step = if large_step.is_finite() {
            large_step.max(Self::MINIMUM_STEP)
        } else {
            Self::DEFAULT_LARGE_STEP
        };

        self
    }

    /// Sets the minimum gap between adjacent values, measured in steps.
    /// The gap is limited to the available range.
    pub fn min_steps_between_values(mut self, steps: usize) -> Self {
        self.min_steps_between_values = steps;

        self
    }

    /// Selects how pointer interactions handle collisions between thumbs.
    /// Keyboard and accessibility changes always clamp to neighboring values.
    pub fn thumb_collision_behavior(mut self, behavior: BaseSliderThumbCollisionBehavior) -> Self {
        self.collision_behavior = behavior;

        self
    }

    /// Sets how thumb bounds align with the control's endpoints.
    /// Defaults to [`BaseSliderThumbAlignment::Edge`].
    pub fn thumb_alignment(mut self, alignment: BaseSliderThumbAlignment) -> Self {
        self.thumb_alignment = alignment;

        self
    }

    /// Sets the slider's axis, which defaults to horizontal.
    /// Vertical values increase from bottom to top.
    pub fn orientation(mut self, orientation: Axis) -> Self {
        self.orientation = orientation;

        self
    }

    /// Sets the slider's axis to horizontal.
    pub fn orientation_x(self) -> Self {
        self.orientation(Axis::Horizontal)
    }

    /// Sets the slider's axis to vertical, with values increasing from bottom to top.
    pub fn orientation_y(self) -> Self {
        self.orientation(Axis::Vertical)
    }

    /// Disables value changes and applies disabled styles to the slider's parts.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;

        self
    }

    /// Adds a visible label and an accessible-name fallback for unnamed thumbs.
    pub fn label(mut self, label: BaseSliderLabel) -> Self {
        self.label = Some(label);

        self
    }

    /// Adds the clickable control and its track.
    pub fn control(mut self, control: BaseSliderControl) -> Self {
        self.control = Some(control);

        self
    }

    /// Formats each thumb's value for assistive technology.
    pub fn value_text_formatter<Text>(mut self, formatter: impl Fn(f32) -> Text + 'static) -> Self
    where
        Text: Into<SharedString>,
    {
        self.value_text_formatter = Some(Rc::new(move |value| formatter(value).into()));

        self
    }

    /// Registers a handler for proposed value changes.
    /// Update the application's stored values and render again to accept a change.
    pub fn on_change(
        mut self,
        handler: impl Fn(&BaseSliderChange, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));

        self
    }

    /// Registers a handler for completed changes.
    /// Runs after a changed pointer gesture ends or a discrete input changes a value.
    pub fn on_commit(
        mut self,
        handler: impl Fn(&BaseSliderChange, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_commit = Some(Rc::new(handler));

        self
    }

    fn model(&self, value_count: usize) -> BaseSliderModel {
        debug_assert!(
            self.maximum > self.minimum,
            "the slider maximum must be greater than its minimum"
        );

        let minimum = self.minimum.min(self.maximum).min(f32::MAX.next_down());
        let maximum = self.maximum.max(self.minimum).max(minimum.next_up());

        BaseSliderModel {
            minimum,
            maximum,
            step: self.step,
            large_step: self.large_step,
            minimum_spacing: BaseSliderModel::minimum_spacing(
                minimum,
                maximum,
                self.step,
                self.min_steps_between_values,
                value_count,
            ),
            orientation: self.orientation,
            alignment: self.thumb_alignment,
            collision_behavior: self.collision_behavior,
        }
    }
}

impl RenderOnce for BaseSlider {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let model = self.model(self.values.len());
        let values = model.normalized_values(&self.values);
        let interactive = !self.disabled && self.on_change.is_some();

        let interaction = BaseSliderInteraction::new(
            &self.element_id,
            values.clone(),
            interactive,
            self.on_change.clone(),
            self.on_commit.clone(),
            window,
            cx,
        );

        let rendered_values = interaction.rendered_values(&values, cx);

        let context = BaseSliderRenderContext {
            values: rendered_values,
            model,
            interaction,
            interactive,
            disabled: self.disabled,
            label: self
                .label
                .as_ref()
                .and_then(BaseSliderLabel::accessible_name),
            value_text_formatter: self.value_text_formatter.clone(),
        };
        let label = self.label.take().map(|label| label.render(self.disabled));
        let control = self
            .control
            .take()
            .expect("a slider requires a control")
            .render(&context, window, cx);

        self.base
            .relative()
            .disabled(self.disabled)
            .refine_style(&self.style)
            .when_some(label, |this, label| this.child(label))
            .children(self.children)
            .child(control)
    }
}

/// How pointer interactions resolve collisions between neighboring thumbs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseSliderThumbCollisionBehavior {
    /// Moves neighboring values to maintain the minimum spacing.
    Push,
    /// Changes the active thumb's index as it crosses neighboring values.
    Swap,
    /// Stops the active value at the spacing limit imposed by its neighbors.
    Clamp,
}

impl BaseSliderThumbCollisionBehavior {
    fn resolve(
        self,
        values: &BaseSliderValue,
        thumb_idx: usize,
        target: f32,
        model: BaseSliderModel,
    ) -> (BaseSliderValue, usize) {
        let mut next = values.clone();
        let mut active_idx = thumb_idx;

        match self {
            Self::Push => {
                let room_before = model.minimum_spacing * thumb_idx as f32;
                let room_after = model.minimum_spacing * (values.len() - thumb_idx - 1) as f32;
                next.0[thumb_idx] =
                    target.clamp(model.minimum + room_before, model.maximum - room_after);

                for current_idx in (0..thumb_idx).rev() {
                    let maximum = next.0[current_idx + 1] - model.minimum_spacing;
                    next.0[current_idx] = next.0[current_idx].min(maximum);
                }

                for current_idx in thumb_idx + 1..next.len() {
                    let minimum = next.0[current_idx - 1] + model.minimum_spacing;
                    next.0[current_idx] = next.0[current_idx].max(minimum);
                }
            }
            Self::Clamp | Self::Swap => {
                if self == Self::Swap {
                    while active_idx > 0 && target < next.0[active_idx - 1] {
                        next.0.swap(active_idx, active_idx - 1);
                        active_idx -= 1;
                    }

                    while active_idx + 1 < next.len() && target > next.0[active_idx + 1] {
                        next.0.swap(active_idx, active_idx + 1);
                        active_idx += 1;
                    }
                }

                let (minimum, maximum) = model.effective_bounds(&next, active_idx);
                next.0[active_idx] = target.clamp(minimum, maximum);
            }
        }

        (next, active_idx)
    }
}

/// How thumb bounds align with the control's endpoints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseSliderThumbAlignment {
    /// Places thumb centers at the endpoints at the minimum and maximum values.
    Center,
    /// Keeps the entire thumb inside the control at the endpoints.
    Edge,
}

#[derive(Clone, Copy)]
struct BaseSliderModel {
    minimum: f32,
    maximum: f32,
    step: f32,
    large_step: f32,
    minimum_spacing: f32,
    orientation: Axis,
    alignment: BaseSliderThumbAlignment,
    collision_behavior: BaseSliderThumbCollisionBehavior,
}

impl BaseSliderModel {
    fn minimum_spacing(
        minimum: f32,
        maximum: f32,
        step: f32,
        steps: usize,
        value_count: usize,
    ) -> f32 {
        if value_count <= 1 {
            return 0.;
        }

        (step * steps as f32).min((maximum - minimum) / (value_count - 1) as f32)
    }

    fn normalized_values(self, values: &BaseSliderValue) -> BaseSliderValue {
        let mut values = values
            .0
            .iter()
            .map(|value| value.clamp(self.minimum, self.maximum))
            .collect::<SmallVec<_>>();
        values.sort_by(f32::total_cmp);

        BaseSliderValue(values)
    }

    fn snap(self, value: f32) -> f32 {
        let step_idx =
            ((f64::from(value) - f64::from(self.minimum)) / f64::from(self.step)).round();
        let snapped = f64::from(self.minimum) + step_idx * f64::from(self.step);

        (snapped as f32).clamp(self.minimum, self.maximum)
    }

    fn fraction(self, value: f32) -> f32 {
        ((value.clamp(self.minimum, self.maximum) - self.minimum) / (self.maximum - self.minimum))
            .clamp(0., 1.)
    }

    fn value_at_pointer(
        self,
        position: Point<Pixels>,
        control_bounds: Bounds<Pixels>,
        pointer_offset: Pixels,
        thumb_size: Size<Pixels>,
    ) -> f32 {
        let pointer = self.axis_position(position) - pointer_offset;
        let control_start = self.bounds_start(control_bounds);
        let control_length = self.size_along(control_bounds.size);
        let thumb_length = self.size_along(thumb_size);
        let (start, length) = match self.alignment {
            BaseSliderThumbAlignment::Center => (control_start, control_length),
            BaseSliderThumbAlignment::Edge => (
                control_start + thumb_length / 2.,
                (control_length - thumb_length).max(px(0.)),
            ),
        };
        let mut fraction = if length > px(0.) {
            (pointer - start) / length
        } else {
            0.
        };

        if self.orientation == Axis::Vertical {
            fraction = 1. - fraction;
        }

        let value = self.minimum + fraction.clamp(0., 1.) * (self.maximum - self.minimum);

        self.snap(value)
    }

    fn effective_bounds(self, values: &BaseSliderValue, thumb_idx: usize) -> (f32, f32) {
        let minimum = if thumb_idx == 0 {
            self.minimum
        } else {
            values.0[thumb_idx - 1] + self.minimum_spacing
        };
        let maximum = if thumb_idx + 1 == values.len() {
            self.maximum
        } else {
            values.0[thumb_idx + 1] - self.minimum_spacing
        };

        (minimum.min(maximum), maximum.max(minimum))
    }

    fn keyboard_target(
        self,
        event: &KeyDownEvent,
        value: f32,
        minimum: f32,
        maximum: f32,
    ) -> Option<f32> {
        let increment = if event.keystroke.modifiers.shift {
            self.large_step
        } else {
            self.step
        };

        match event.keystroke.key.as_str() {
            "right" | "up" => Some((value + increment).min(maximum)),
            "left" | "down" => Some((value - increment).max(minimum)),
            "pageup" => Some((value + self.large_step).min(maximum)),
            "pagedown" => Some((value - self.large_step).max(minimum)),
            "home" => Some(minimum),
            "end" => Some(maximum),
            _ => None,
        }
    }

    fn thumb_position(
        self,
        control_size: Size<Pixels>,
        thumb_size: Size<Pixels>,
        value: f32,
    ) -> Point<Length> {
        let fraction = self.fraction(value);
        let (horizontal_fraction, vertical_fraction) = match self.orientation {
            Axis::Horizontal => (fraction, 0.5),
            Axis::Vertical => (0.5, 1. - fraction),
        };
        let unmeasured = control_size == Size::default() || thumb_size == Size::default();

        let offset = |axis: Axis, fraction: f32| -> Length {
            if unmeasured {
                return relative(fraction).into();
            }

            let (control_length, thumb_length) = match axis {
                Axis::Horizontal => (control_size.width, thumb_size.width),
                Axis::Vertical => (control_size.height, thumb_size.height),
            };
            let alignment = if axis == self.orientation {
                self.alignment
            } else {
                BaseSliderThumbAlignment::Center
            };

            Self::aligned_offset(control_length, thumb_length, fraction, alignment).into()
        };

        Point {
            x: offset(Axis::Horizontal, horizontal_fraction),
            y: offset(Axis::Vertical, vertical_fraction),
        }
    }

    fn aligned_offset(
        control_length: Pixels,
        thumb_length: Pixels,
        fraction: f32,
        alignment: BaseSliderThumbAlignment,
    ) -> Pixels {
        match alignment {
            BaseSliderThumbAlignment::Center => control_length * fraction - thumb_length / 2.,
            BaseSliderThumbAlignment::Edge => {
                (control_length - thumb_length).max(px(0.)) * fraction
            }
        }
    }

    fn axis_position(self, position: Point<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => position.x,
            Axis::Vertical => position.y,
        }
    }

    fn bounds_center_axis(self, bounds: Bounds<Pixels>) -> Pixels {
        self.axis_position(bounds.center())
    }

    fn bounds_start(self, bounds: Bounds<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => bounds.left(),
            Axis::Vertical => bounds.top(),
        }
    }

    fn size_along(self, size: Size<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => size.width,
            Axis::Vertical => size.height,
        }
    }
}

#[derive(Clone)]
struct BaseSliderRenderContext {
    values: BaseSliderValue,
    model: BaseSliderModel,
    interaction: BaseSliderInteraction,
    interactive: bool,
    disabled: bool,
    label: Option<SharedString>,
    value_text_formatter: Option<ValueTextFormatter>,
}

#[derive(Clone)]
struct BaseSliderInteraction {
    state: Entity<BaseSliderInteractionState>,
    on_change: Option<OnChange>,
    on_commit: Option<OnCommit>,
}

impl BaseSliderInteraction {
    fn new(
        element_id: &ElementId,
        values: BaseSliderValue,
        interactive: bool,
        on_change: Option<OnChange>,
        on_commit: Option<OnCommit>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let state =
            window.use_keyed_state((element_id.clone(), "state:slider"), cx, |_window, _cx| {
                BaseSliderInteractionState::default()
            });
        let interaction = Self {
            state,
            on_change,
            on_commit,
        };

        let current = interaction.state.read(cx);
        let value_count_changed =
            current.active_thumb_index.is_some() && current.pressed_values.len() != values.len();

        if !interactive || value_count_changed {
            interaction.cancel(cx);
        }

        interaction
    }

    fn rendered_values(&self, fallback: &BaseSliderValue, cx: &App) -> BaseSliderValue {
        self.state
            .read(cx)
            .preview_values
            .clone()
            .unwrap_or_else(|| fallback.clone())
    }

    fn thumb_size(&self, thumb_idx: usize, cx: &App) -> Size<Pixels> {
        self.state.read(cx).thumb_size(thumb_idx)
    }

    fn cancel(&self, cx: &mut App) {
        if self.state.read(cx).active_thumb_index.is_none() {
            return;
        }

        self.state.update(cx, |state, _cx| state.cancel());
    }

    fn measure_bounds(&self, thumb_idx: Option<usize>) -> impl IntoElement {
        let state = self.state.clone();

        canvas(
            |bounds, _window, _cx| bounds,
            move |_bounds, measured, _window, cx| {
                state.update(cx, |state, cx| {
                    let bounds = if let Some(thumb_idx) = thumb_idx {
                        if state.thumb_bounds.len() <= thumb_idx {
                            state.thumb_bounds.resize(thumb_idx + 1, Bounds::default());
                        }

                        &mut state.thumb_bounds[thumb_idx]
                    } else {
                        &mut state.control_bounds
                    };

                    if *bounds == measured {
                        return;
                    }

                    *bounds = measured;
                    cx.notify();
                });
            },
        )
        .absolute()
        .inset_0()
    }

    fn install_pointer<Element>(
        &self,
        control: Element,
        values: BaseSliderValue,
        model: BaseSliderModel,
        focus_handles: SmallVec<[FocusHandle; 2]>,
    ) -> Element
    where
        Element: ParentElement + StatefulInteractiveElement,
    {
        let down_interaction = self.clone();
        let up_interaction = self.clone();
        let touch_interaction = self.clone();
        let touch_values = values.clone();

        control
            .child(self.track_mouse(model, focus_handles.clone()))
            .on_mouse_down_all(move |event, phase, hitbox, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }

                cx.stop_propagation();
                let change = down_interaction.state.update(cx, |state, cx| {
                    state.control_bounds = hitbox.bounds;
                    let thumb_idx = state.closest_thumb(event.position, &values, model);
                    let pressed_thumb = state
                        .thumb_bounds
                        .get(thumb_idx)
                        .is_some_and(|bounds| bounds.contains(&event.position));
                    let pointer_offset = if pressed_thumb {
                        model.axis_position(event.position)
                            - model.bounds_center_axis(state.thumb_bounds[thumb_idx])
                    } else {
                        px(0.)
                    };

                    state.start(thumb_idx, values.clone(), pointer_offset);
                    cx.notify();

                    if pressed_thumb {
                        return None;
                    }

                    state.update_from_pointer(
                        event.position,
                        BaseSliderChangeReason::TrackPress,
                        model,
                    )
                });

                let thumb_idx = down_interaction.state.read(cx).active_thumb_index.unwrap();

                if let Some(focus_handle) = focus_handles.get(thumb_idx) {
                    focus_handle.focus(window, cx);
                }

                down_interaction.emit(change.as_ref(), None, window, cx);
            })
            .on_mouse_up_all(move |event, phase, _hitbox, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || up_interaction.state.read(cx).active_thumb_index.is_none()
                {
                    return;
                }

                cx.stop_propagation();
                let (live, committed) = up_interaction.state.update(cx, |state, cx| {
                    let live = state.update_from_pointer(
                        event.position,
                        BaseSliderChangeReason::Drag,
                        model,
                    );
                    let committed = state.finish();
                    cx.notify();

                    (live, committed)
                });

                up_interaction.emit(live.as_ref(), committed.as_ref(), window, cx);
            })
            .on_click(move |event, window, cx| {
                let ClickEvent::Touch(touch) = event else {
                    return;
                };

                cx.stop_propagation();
                touch_interaction.propose_touch_tap(
                    touch.position,
                    &touch_values,
                    model,
                    window,
                    cx,
                );
            })
    }

    fn track_mouse(
        &self,
        model: BaseSliderModel,
        focus_handles: SmallVec<[FocusHandle; 2]>,
    ) -> impl IntoElement {
        let interaction = self.clone();

        canvas(
            |_bounds, _window, _cx| (),
            move |_bounds, _prepaint, window, _cx| {
                let interaction = interaction.clone();
                let focus_handles = focus_handles.clone();

                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture
                        || !event.dragging()
                        || interaction.state.read(cx).active_thumb_index.is_none()
                    {
                        return;
                    }

                    let previous_idx = interaction.state.read(cx).active_thumb_index;
                    let change = interaction.state.update(cx, |state, cx| {
                        let change = state.update_from_pointer(
                            event.position,
                            BaseSliderChangeReason::Drag,
                            model,
                        );

                        if change.is_some() {
                            cx.notify();
                        }

                        change
                    });
                    let active_idx = interaction.state.read(cx).active_thumb_index;

                    if active_idx != previous_idx
                        && let Some(active_idx) = active_idx
                        && let Some(focus_handle) = focus_handles.get(active_idx)
                    {
                        focus_handle.focus(window, cx);
                    }

                    interaction.emit(change.as_ref(), None, window, cx);
                });
            },
        )
        .absolute()
        .inset_0()
    }

    fn propose_touch_tap(
        &self,
        position: Point<Pixels>,
        values: &BaseSliderValue,
        model: BaseSliderModel,
        window: &mut Window,
        cx: &mut App,
    ) {
        let change = self.state.update(cx, |state, _cx| {
            let thumb_idx = state.closest_thumb(position, values, model);
            let target = model.value_at_pointer(
                position,
                state.control_bounds,
                px(0.),
                state.thumb_size(thumb_idx),
            );
            let (value, active_thumb_index) = model
                .collision_behavior
                .resolve(values, thumb_idx, target, model);

            BaseSliderChange::between(
                values,
                value,
                active_thumb_index,
                BaseSliderChangeReason::TrackPress,
            )
        });

        self.emit(change.as_ref(), change.as_ref(), window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn propose_discrete(
        &self,
        values: &BaseSliderValue,
        thumb_idx: usize,
        target: f32,
        reason: BaseSliderChangeReason,
        model: BaseSliderModel,
        window: &mut Window,
        cx: &mut App,
    ) {
        let target = target.clamp(model.minimum, model.maximum);
        let target = if matches!(target, value if value == model.minimum || value == model.maximum)
        {
            target
        } else {
            model.snap(target)
        };

        let (value, active_thumb_index) =
            BaseSliderThumbCollisionBehavior::Clamp.resolve(values, thumb_idx, target, model);
        let change = BaseSliderChange::between(values, value, active_thumb_index, reason);

        self.emit(change.as_ref(), change.as_ref(), window, cx);
    }

    fn emit(
        &self,
        live: Option<&BaseSliderChange>,
        committed: Option<&BaseSliderChange>,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (handler, change) in [(&self.on_change, live), (&self.on_commit, committed)] {
            if let (Some(handler), Some(change)) = (handler, change) {
                handler(change, window, cx);
            }
        }
    }
}

#[derive(Clone, Default)]
struct BaseSliderInteractionState {
    control_bounds: Bounds<Pixels>,
    thumb_bounds: SmallVec<[Bounds<Pixels>; 2]>,
    active_thumb_index: Option<usize>,
    pressed_values: SmallVec<[f32; 2]>,
    preview_values: Option<BaseSliderValue>,
    pointer_offset: Pixels,
    last_reason: Option<BaseSliderChangeReason>,
}

impl BaseSliderInteractionState {
    fn start(&mut self, thumb_idx: usize, values: BaseSliderValue, pointer_offset: Pixels) {
        self.active_thumb_index = Some(thumb_idx);
        self.pressed_values = values.0.clone();
        self.preview_values = Some(values);
        self.pointer_offset = pointer_offset;
        self.last_reason = None;
    }

    fn cancel(&mut self) {
        self.active_thumb_index = None;
        self.pressed_values.clear();
        self.preview_values = None;
        self.pointer_offset = px(0.);
        self.last_reason = None;
    }

    fn finish(&mut self) -> Option<BaseSliderChange> {
        let active_thumb_index = self.active_thumb_index?;
        let value = self.preview_values.take()?;
        let changed = value.as_slice() != self.pressed_values.as_slice();
        let reason = self.last_reason;

        self.cancel();

        if !changed {
            return None;
        }

        Some(BaseSliderChange {
            value,
            active_thumb_index,
            reason: reason.expect("a changed slider has a change reason"),
        })
    }

    fn update_from_pointer(
        &mut self,
        position: Point<Pixels>,
        reason: BaseSliderChangeReason,
        model: BaseSliderModel,
    ) -> Option<BaseSliderChange> {
        let thumb_idx = self.active_thumb_index?;
        let values = self.preview_values.as_ref()?;
        let target = model.value_at_pointer(
            position,
            self.control_bounds,
            self.pointer_offset,
            self.thumb_size(thumb_idx),
        );
        let (value, active_thumb_index) = model
            .collision_behavior
            .resolve(values, thumb_idx, target, model);
        let change = BaseSliderChange::between(values, value, active_thumb_index, reason)?;

        self.active_thumb_index = Some(active_thumb_index);
        self.preview_values = Some(change.value.clone());
        self.last_reason = Some(reason);

        Some(change)
    }

    fn closest_thumb(
        &self,
        position: Point<Pixels>,
        values: &BaseSliderValue,
        model: BaseSliderModel,
    ) -> usize {
        let pointer = model.axis_position(position);

        values
            .0
            .iter()
            .enumerate()
            .min_by(|(left_idx, left), (right_idx, right)| {
                let left_center = self.thumb_center(*left_idx, **left, model);
                let right_center = self.thumb_center(*right_idx, **right, model);
                let left_distance = f32::from((pointer - left_center).abs());
                let right_distance = f32::from((pointer - right_center).abs());

                left_distance
                    .total_cmp(&right_distance)
                    .then_with(|| right_idx.cmp(left_idx))
            })
            .map(|(thumb_idx, _value)| thumb_idx)
            .expect("slider values are non-empty")
    }

    fn thumb_center(&self, thumb_idx: usize, value: f32, model: BaseSliderModel) -> Pixels {
        let fraction = model.fraction(value);
        let size = self.thumb_size(thumb_idx);
        let axis_size = model.size_along(size);
        let control_start = model.bounds_start(self.control_bounds);
        let control_length = model.size_along(self.control_bounds.size);
        let usable_length = match model.alignment {
            BaseSliderThumbAlignment::Center => control_length,
            BaseSliderThumbAlignment::Edge => (control_length - axis_size).max(px(0.)),
        };
        let start_inset = match model.alignment {
            BaseSliderThumbAlignment::Center => px(0.),
            BaseSliderThumbAlignment::Edge => axis_size / 2.,
        };
        let directional_fraction = match model.orientation {
            Axis::Horizontal => fraction,
            Axis::Vertical => 1. - fraction,
        };

        control_start + start_inset + usable_length * directional_fraction
    }

    fn thumb_size(&self, thumb_idx: usize) -> Size<Pixels> {
        self.thumb_bounds
            .get(thumb_idx)
            .map(|bounds| bounds.size)
            .unwrap_or_default()
    }
}

/// A visible label associated with the slider thumbs.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderLabel {
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    text: Option<SharedString>,
    #[children]
    children: Vec<AnyElement>,
}

impl InteractiveElement for BaseSliderLabel {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSliderLabel {
    /// Creates an empty slider label with the supplied element ID.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(element_id),
            style: StyleRefinement::default(),
            text: None,
            children: Vec::new(),
        }
    }

    /// Sets the visible text and default accessible name for the slider thumbs.
    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = Some(text.into());

        self
    }

    fn accessible_name(&self) -> Option<SharedString> {
        self.text.clone()
    }

    fn render(self, disabled: bool) -> AnyElement {
        self.base
            .role(Role::Label)
            .when(disabled, |this| this.disabled(true))
            .refine_style(&self.style)
            .when_some(self.text, |this, text| this.child(text))
            .children(self.children)
            .into_any_element()
    }
}

/// Contains the indicator, custom marks, and slider thumbs.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderTrack {
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    indicator: Option<BaseSliderIndicator>,
    thumbs: Vec<BaseSliderThumb>,
    #[children]
    children: Vec<AnyElement>,
}

impl InteractiveElement for BaseSliderTrack {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSliderTrack {
    /// Creates a track without an indicator or thumbs.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(element_id),
            style: StyleRefinement::default(),
            indicator: None,
            thumbs: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Sets the element that draws the selected range.
    pub fn indicator(mut self, indicator: BaseSliderIndicator) -> Self {
        self.indicator = Some(indicator);

        self
    }

    /// Appends a thumb for the next slider value.
    /// Add one thumb per value, in value order.
    pub fn thumb(mut self, thumb: BaseSliderThumb) -> Self {
        self.thumbs.push(thumb);

        self
    }

    fn render(
        self,
        context: &BaseSliderRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> RenderedBaseSliderTrack {
        debug_assert_eq!(
            self.thumbs.len(),
            context.values.len(),
            "a slider requires one thumb for each value"
        );

        let mut element = self
            .base
            .relative()
            .size_full()
            .when(context.disabled, |this| this.disabled(true))
            .refine_style(&self.style)
            .when_some(self.indicator, |this, indicator| {
                this.child(indicator.render(context))
            })
            .children(self.children);
        let mut focus_handles = SmallVec::new();

        for (thumb_idx, thumb) in self
            .thumbs
            .into_iter()
            .take(context.values.len())
            .enumerate()
        {
            let (thumb, focus_handle) = thumb.render(context, thumb_idx, window, cx);

            element = element.child(thumb);
            focus_handles.push(focus_handle);
        }

        RenderedBaseSliderTrack {
            element: element.into_any_element(),
            focus_handles,
        }
    }
}

struct RenderedBaseSliderTrack {
    element: AnyElement,
    focus_handles: SmallVec<[FocusHandle; 2]>,
}

/// Shows the selected portion of the slider track.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderIndicator {
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    #[children]
    children: Vec<AnyElement>,
}

impl InteractiveElement for BaseSliderIndicator {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSliderIndicator {
    /// Creates an indicator for the selected range.
    /// A single value fills from the minimum; multiple values fill from first to last.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(element_id),
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }

    fn render(self, context: &BaseSliderRenderContext) -> AnyElement {
        let values = &context.values;
        let start_value = if values.len() == 1 {
            context.model.minimum
        } else {
            values.0[0]
        };
        let end_value = *values.0.last().expect("slider values are non-empty");
        let start = context.model.fraction(start_value);
        let end = context.model.fraction(end_value);

        self.base
            .absolute()
            .when(context.disabled, |this| this.disabled(true))
            .when(context.model.orientation == Axis::Horizontal, |this| {
                this.top(px(0.))
                    .bottom(px(0.))
                    .left(relative(start))
                    .right(relative(1. - end))
            })
            .when(context.model.orientation == Axis::Vertical, |this| {
                this.left(px(0.))
                    .right(px(0.))
                    .bottom(relative(start))
                    .top(relative(1. - end))
            })
            .refine_style(&self.style)
            .children(self.children)
            .into_any_element()
    }
}

/// A focusable thumb that controls one slider value.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderThumb {
    element_id: ElementId,
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    #[children]
    children: Vec<AnyElement>,
    aria_label: Option<SharedString>,
    value_text_formatter: Option<ValueTextFormatter>,
}

impl InteractiveElement for BaseSliderThumb {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSliderThumb {
    /// Creates a thumb whose element ID identifies its persistent focus state.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        let element_id = element_id.into();

        Self {
            element_id: element_id.clone(),
            base: div().id(element_id),
            style: StyleRefinement::default(),
            children: Vec::new(),
            aria_label: None,
            value_text_formatter: None,
        }
    }

    /// Sets this thumb's accessible name, overriding the visible slider label.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());

        self
    }

    /// Formats this thumb's value for assistive technology.
    /// Overrides the formatter on [`BaseSlider`].
    pub fn value_text_formatter<Text>(mut self, formatter: impl Fn(f32) -> Text + 'static) -> Self
    where
        Text: Into<SharedString>,
    {
        self.value_text_formatter = Some(Rc::new(move |value| formatter(value).into()));

        self
    }

    fn render(
        self,
        context: &BaseSliderRenderContext,
        thumb_idx: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> (AnyElement, FocusHandle) {
        let values = &context.values;
        let model = context.model;
        let interaction = &context.interaction;
        let value = values.0[thumb_idx];

        let bounds = interaction.state.read(cx).control_bounds;
        let thumb_size = interaction.thumb_size(thumb_idx, cx);
        let position = model.thumb_position(bounds.size, thumb_size, value);

        let (effective_minimum, effective_maximum) = model.effective_bounds(values, thumb_idx);
        let focus_handle = use_focus_handle(self.element_id.clone(), window, cx, None);

        let keyboard_values = values.clone();
        let keyboard_interaction = interaction.clone();

        let accessibility_values = values.clone();
        let increment_interaction = interaction.clone();
        let decrement_values = values.clone();
        let decrement_interaction = interaction.clone();
        let set_value_values = values.clone();
        let set_value_interaction = interaction.clone();

        let aria_label = self.aria_label.or_else(|| context.label.clone());
        let value_text_formatter = self
            .value_text_formatter
            .or_else(|| context.value_text_formatter.clone());

        let thumb = self
            .base
            .absolute()
            .role(Role::Slider)
            .when(context.disabled, |this| this.disabled(true))
            .aria_numeric_value(f64::from(value))
            .aria_min_numeric_value(f64::from(effective_minimum))
            .aria_max_numeric_value(f64::from(effective_maximum))
            .aria_numeric_value_step(f64::from(model.step))
            .aria_orientation(match model.orientation {
                Axis::Horizontal => accesskit::Orientation::Horizontal,
                Axis::Vertical => accesskit::Orientation::Vertical,
            })
            .when_some(aria_label, |this, label| this.aria_label(label))
            .when_some(value_text_formatter, |this, formatter| {
                this.aria_value(formatter(value))
            })
            .left(position.x)
            .top(position.y)
            .refine_style(&self.style)
            .children(self.children)
            .child(interaction.measure_bounds(Some(thumb_idx)))
            .when(context.interactive, |this| {
                this.track_focus(&focus_handle)
                    .on_key_down(move |event, window, cx| {
                        let Some(target) = model.keyboard_target(
                            event,
                            keyboard_values.0[thumb_idx],
                            effective_minimum,
                            effective_maximum,
                        ) else {
                            return;
                        };

                        cx.stop_propagation();
                        keyboard_interaction.propose_discrete(
                            &keyboard_values,
                            thumb_idx,
                            target,
                            BaseSliderChangeReason::Keyboard,
                            model,
                            window,
                            cx,
                        );
                    })
                    .on_a11y_action(AccessibleAction::Increment, move |_data, window, cx| {
                        increment_interaction.propose_discrete(
                            &accessibility_values,
                            thumb_idx,
                            accessibility_values.0[thumb_idx] + model.step,
                            BaseSliderChangeReason::Accessibility,
                            model,
                            window,
                            cx,
                        );
                    })
                    .on_a11y_action(AccessibleAction::Decrement, move |_data, window, cx| {
                        decrement_interaction.propose_discrete(
                            &decrement_values,
                            thumb_idx,
                            decrement_values.0[thumb_idx] - model.step,
                            BaseSliderChangeReason::Accessibility,
                            model,
                            window,
                            cx,
                        );
                    })
                    .on_a11y_action(AccessibleAction::SetValue, move |data, window, cx| {
                        let Some(accesskit::ActionData::NumericValue(target)) = data else {
                            return;
                        };

                        let target = *target as f32;

                        if !target.is_finite() {
                            return;
                        }

                        set_value_interaction.propose_discrete(
                            &set_value_values,
                            thumb_idx,
                            target,
                            BaseSliderChangeReason::Accessibility,
                            model,
                            window,
                            cx,
                        );
                    })
            });

        (thumb.into_any_element(), focus_handle)
    }
}

fn use_focus_handle(
    base_id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    focus_handle: Option<FocusHandle>,
) -> FocusHandle {
    focus_handle.unwrap_or_else(|| {
        window
            .use_keyed_state((base_id.into(), "state:focus_handle"), cx, |_window, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone()
    })
}

/// The clickable area of a slider.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderControl {
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    track: Option<BaseSliderTrack>,
    #[children]
    children: Vec<AnyElement>,
}

impl InteractiveElement for BaseSliderControl {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl BaseSliderControl {
    /// Creates an empty slider control. Attach a track before rendering.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(element_id),
            style: StyleRefinement::default(),
            track: None,
            children: Vec::new(),
        }
    }

    /// Sets the track that contains the indicator and thumbs.
    pub fn track(mut self, track: BaseSliderTrack) -> Self {
        self.track = Some(track);

        self
    }

    fn render(
        self,
        context: &BaseSliderRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let track = self
            .track
            .expect("a slider control requires a track")
            .render(context, window, cx);
        let base = self
            .base
            .relative()
            .size_full()
            .when(context.disabled, |this| this.disabled(true))
            .refine_style(&self.style)
            .child(track.element)
            .children(self.children)
            .child(context.interaction.measure_bounds(None));

        if context.interactive {
            return context
                .interaction
                .install_pointer(
                    base,
                    context.values.clone(),
                    context.model,
                    track.focus_handles,
                )
                .into_any_element();
        }

        base.child(div().absolute().inset_0().on_mouse_down(
            MouseButton::Left,
            |_event, _window, cx| {
                cx.stop_propagation();
            },
        ))
        .into_any_element()
    }
}

/// One or more values controlled by the application that renders a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseSliderValue(SmallVec<[f32; 2]>);

impl BaseSliderValue {
    /// Creates one or more finite slider values. Invalid input triggers a debug
    /// assertion. Release builds replace non-finite values with zero and empty
    /// input with a single zero.
    pub fn new(values: impl IntoIterator<Item = f32>) -> Self {
        let mut values = values.into_iter().collect::<SmallVec<[f32; 2]>>();

        debug_assert!(!values.is_empty(), "a slider requires at least one value");
        debug_assert!(
            values.iter().all(|value| value.is_finite()),
            "slider values must be finite"
        );

        for value in &mut values {
            if !value.is_finite() {
                *value = BaseSlider::DEFAULT_VALUE;
            }
        }

        if values.is_empty() {
            values.push(BaseSlider::DEFAULT_VALUE);
        }

        Self(values)
    }

    /// Borrows the values in their stored order.
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Returns the number of values, with one value per thumb.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether there are no values.
    /// Values created through the public constructors are always nonempty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<[f32]> for BaseSliderValue {
    fn as_ref(&self) -> &[f32] {
        self.as_slice()
    }
}

impl From<f32> for BaseSliderValue {
    fn from(value: f32) -> Self {
        Self::new([value])
    }
}

impl<const COUNT: usize> From<[f32; COUNT]> for BaseSliderValue {
    fn from(values: [f32; COUNT]) -> Self {
        Self::new(values)
    }
}

impl FromIterator<f32> for BaseSliderValue {
    fn from_iter<Values: IntoIterator<Item = f32>>(values: Values) -> Self {
        Self::new(values)
    }
}

/// The values, active thumb, and input source of a slider change.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseSliderChange {
    /// Values proposed by the interaction.
    pub value: BaseSliderValue,
    /// Index of the active thumb after applying collision behavior.
    pub active_thumb_index: usize,
    /// Input that produced this change.
    pub reason: BaseSliderChangeReason,
}

impl BaseSliderChange {
    fn between(
        previous: &BaseSliderValue,
        value: BaseSliderValue,
        active_thumb_index: usize,
        reason: BaseSliderChangeReason,
    ) -> Option<Self> {
        if value == *previous {
            return None;
        }

        Some(Self {
            value,
            active_thumb_index,
            reason,
        })
    }
}

/// The input responsible for a slider value change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseSliderChangeReason {
    /// A mouse press on the track or a touch tap.
    TrackPress,
    /// A mouse drag while a slider gesture is active.
    Drag,
    /// A keyboard adjustment.
    Keyboard,
    /// An increment, decrement, or set-value action from assistive technology.
    Accessibility,
}

#[cfg(test)]
mod tests {
    use super::{
        BaseSlider, BaseSliderChange, BaseSliderChangeReason, BaseSliderControl,
        BaseSliderIndicator, BaseSliderInteraction, BaseSliderInteractionState, BaseSliderLabel,
        BaseSliderModel, BaseSliderThumb, BaseSliderThumbAlignment,
        BaseSliderThumbCollisionBehavior, BaseSliderTrack, BaseSliderValue,
    };

    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use gpui::{
        AppContext, Axis, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
        KeyUpEvent, Keystroke, Modifiers, MouseButton, ParentElement, Render, Size,
        StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, Window, accesskit,
        div, point, prelude::FluentBuilder, px, relative, size,
    };

    const SLIDER: &str = "base-slider";
    const CONTROL: &str = "slider-under-test-control";
    const TRACK: &str = "slider-under-test-track";
    const INDICATOR: &str = "slider-under-test-indicator";
    const FIRST_THUMB: &str = "slider-under-test-thumb-0";
    const SECOND_THUMB: &str = "slider-under-test-thumb-1";
    const LABEL: &str = "slider-under-test-label";
    const VALUE: &str = "slider-under-test-value";
    const ROOT_CHILD: &str = "slider-under-test-root-child";
    const TRACK_MARK: &str = "slider-under-test-track-mark";

    #[derive(Clone)]
    struct SliderHarness {
        values: BaseSliderValue,
        thumb_count: Option<usize>,
        thumb_labels: bool,
        disabled: bool,
        has_handler: bool,
        accept_changes: bool,
        orientation: Axis,
        alignment: BaseSliderThumbAlignment,
        collision_behavior: BaseSliderThumbCollisionBehavior,
        live: Vec<BaseSliderChange>,
        committed: Vec<BaseSliderChange>,
        parent_key_downs: usize,
        parent_clicks: usize,
    }

    impl Render for SliderHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let track = BaseSliderTrack::new("slider-track")
                .indicator(
                    BaseSliderIndicator::new("slider-indicator")
                        .disabled_style(|style| style.w(px(7.)))
                        .debug_selector(|| INDICATOR.to_owned()),
                )
                .child(
                    div()
                        .id("track-mark")
                        .absolute()
                        .w(px(13.))
                        .h(px(1.))
                        .debug_selector(|| TRACK_MARK.to_owned()),
                )
                .disabled_style(|style| style.h(px(12.)))
                .debug_selector(|| TRACK.to_owned());
            let thumb_count = self.thumb_count.unwrap_or(self.values.len());
            let track = (0..thumb_count).fold(track, |track, thumb_idx| {
                let selector = format!("slider-under-test-thumb-{thumb_idx}");
                let label = if thumb_idx == 0 { "Minimum" } else { "Maximum" };

                let mut thumb = BaseSliderThumb::new(("slider-thumb", thumb_idx))
                    .size(px(20.))
                    .disabled_style(|style| style.w(px(16.)))
                    .debug_selector(move || selector);

                if self.thumb_labels {
                    thumb = thumb.aria_label(label);
                }

                if thumb_idx == 0 {
                    thumb = thumb.value_text_formatter(|value| format!("{value} percent"));
                }

                track.thumb(thumb)
            });
            let control = BaseSliderControl::new("slider-control")
                .track(track)
                .disabled_style(|style| style.h(px(16.)))
                .debug_selector(|| CONTROL.to_owned());

            div()
                .id("slider-parent")
                .tab_group()
                .size_full()
                .on_key_down(cx.listener(|this, _event, _window, _cx| this.parent_key_downs += 1))
                .on_click(cx.listener(|this, _event, _window, _cx| this.parent_clicks += 1))
                .child(
                    BaseSlider::new("slider-under-test")
                        .values(self.values.as_slice().iter().copied())
                        .min(0.)
                        .max(100.)
                        .step(5.)
                        .large_step(20.)
                        .min_steps_between_values(2)
                        .thumb_collision_behavior(self.collision_behavior)
                        .thumb_alignment(self.alignment)
                        .orientation(self.orientation)
                        .disabled(self.disabled)
                        .label(
                            BaseSliderLabel::new("slider-label")
                                .text("Range")
                                .absolute()
                                .disabled_style(|style| style.w(px(9.)))
                                .debug_selector(|| LABEL.to_owned()),
                        )
                        .child(
                            div()
                                .id("slider-value")
                                .child(format!("{} values", self.values.len()))
                                .absolute()
                                .debug_selector(|| VALUE.to_owned()),
                        )
                        .control(control)
                        .child(
                            div()
                                .id("root-child")
                                .absolute()
                                .w(px(11.))
                                .h(px(1.))
                                .debug_selector(|| ROOT_CHILD.to_owned()),
                        )
                        .absolute()
                        .left(px(40.))
                        .top(px(40.))
                        .w(px(200.))
                        .h(px(20.))
                        .disabled_style(|style| style.w(px(180.)))
                        .when(self.orientation == Axis::Vertical, |this| {
                            this.w(px(20.)).h(px(200.))
                        })
                        .debug_selector(|| SLIDER.to_owned())
                        .when(self.has_handler, |this| {
                            this.on_change(cx.listener(
                                |this, change: &BaseSliderChange, _window, cx| {
                                    this.live.push(change.clone());

                                    if this.accept_changes {
                                        this.values = change.value.clone();
                                    }

                                    cx.notify();
                                },
                            ))
                        })
                        .on_commit(
                            cx.listener(|this, change: &BaseSliderChange, _window, _cx| {
                                this.committed.push(change.clone());
                            }),
                        ),
                )
        }
    }

    fn setup(
        cx: &mut TestAppContext,
        values: impl IntoIterator<Item = f32>,
    ) -> (Entity<SliderHarness>, &mut VisualTestContext) {
        let values = BaseSliderValue::new(values);

        cx.add_window_view(move |_window, _cx| SliderHarness {
            values,
            thumb_count: None,
            thumb_labels: true,
            disabled: false,
            has_handler: true,
            accept_changes: true,
            orientation: Axis::Horizontal,
            alignment: BaseSliderThumbAlignment::Center,
            collision_behavior: BaseSliderThumbCollisionBehavior::Push,
            live: Vec::new(),
            committed: Vec::new(),
            parent_key_downs: 0,
            parent_clicks: 0,
        })
    }

    fn model(
        orientation: Axis,
        behavior: BaseSliderThumbCollisionBehavior,
        minimum_spacing: f32,
    ) -> BaseSliderModel {
        BaseSliderModel {
            minimum: 0.,
            maximum: 100.,
            step: 5.,
            large_step: 20.,
            minimum_spacing,
            orientation,
            alignment: BaseSliderThumbAlignment::Center,
            collision_behavior: behavior,
        }
    }

    fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<gpui::Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should be rendered"))
    }

    fn activate_key(cx: &mut VisualTestContext, key: &str) {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });

        cx.simulate_event(KeyUpEvent { keystroke });
    }

    fn snapshot(view: &Entity<SliderHarness>, cx: &VisualTestContext) -> SliderHarness {
        cx.read_entity(view, |view, _cx| view.clone())
    }

    fn accessibility_nodes(cx: &mut VisualTestContext) -> [accesskit::Node; 2] {
        cx.update(|window, cx| {
            window.set_a11y_forced(true);
            window.draw(cx).clear(cx);

            let nodes = window
                .a11y_tree()
                .expect("the slider should have an accessibility tree")
                .nodes
                .iter()
                .filter(|(_node_id, node)| node.role() == accesskit::Role::Slider)
                .map(|(_node_id, node)| node.clone())
                .collect::<Vec<_>>();

            nodes
                .try_into()
                .expect("the range should expose two thumbs")
        })
    }

    #[test]
    fn numeric_configuration_normalizes_bounds_steps_and_values() {
        let snap_cases = [
            (-10., 10., 20., 2.5, 10.),
            (21., 10., 20., 2.5, 20.),
            (13.7, 10., 20., 0.5, 13.5),
            (0.29, 0.1, 0.5, 0.1, 0.3),
        ];

        for (value, minimum, maximum, step, expected) in snap_cases {
            let model = BaseSliderModel {
                minimum,
                maximum,
                step,
                ..model(
                    Axis::Horizontal,
                    BaseSliderThumbCollisionBehavior::Clamp,
                    0.,
                )
            };

            assert_eq!(model.snap(value), expected);
        }

        let tiny_step_model = BaseSlider::new("tiny-step")
            .max(1e-39)
            .step(0.0_f32.next_up())
            .model(1);

        assert_eq!(tiny_step_model.snap(1e-40), 1e-40);

        let fraction_model = BaseSliderModel {
            minimum: 10.,
            maximum: 20.,
            ..model(
                Axis::Horizontal,
                BaseSliderThumbCollisionBehavior::Clamp,
                0.,
            )
        };

        assert_eq!(fraction_model.fraction(-5.), 0.);
        assert_eq!(fraction_model.fraction(15.), 0.5);
        assert_eq!(fraction_model.fraction(25.), 1.);

        for (minimum, maximum, values, expected) in [
            (10., 20., [120., -5.], [10., 20.]),
            (0., 100., [78., 22.], [22., 78.]),
        ] {
            let model = BaseSliderModel {
                minimum,
                maximum,
                ..model(Axis::Horizontal, BaseSliderThumbCollisionBehavior::Push, 0.)
            };

            assert_eq!(
                model
                    .normalized_values(&BaseSliderValue::from(values))
                    .as_slice(),
                &expected
            );
        }

        #[cfg(not(debug_assertions))]
        {
            let defaults = BaseSlider::new("default-inputs");

            for value in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
                let slider = BaseSlider::new("finite-inputs")
                    .min(10.)
                    .max(20.)
                    .step(2.)
                    .large_step(5.)
                    .min(value)
                    .max(value)
                    .step(value)
                    .large_step(value);

                assert_eq!(
                    (slider.minimum, slider.maximum),
                    (defaults.minimum, defaults.maximum)
                );
                assert_eq!(
                    (slider.step, slider.large_step),
                    (defaults.step, defaults.large_step)
                );
            }

            for value in [0., -1., -f32::MAX] {
                let slider = BaseSlider::new("positive-steps")
                    .step(value)
                    .large_step(value);
                let model = slider.model(1);

                assert!(model.step.is_finite() && model.step > 0.);
                assert!(model.large_step.is_finite() && model.large_step > 0.);
                assert!(model.snap(25.).is_finite());
            }

            for (minimum, maximum) in [
                (100., 0.),
                (10., 10.),
                (0., 0.),
                (f32::MIN, f32::MIN),
                (f32::MAX, f32::MAX),
            ] {
                let model = BaseSlider::new("ordered-bounds")
                    .min(minimum)
                    .max(maximum)
                    .model(1);

                assert!(model.minimum.is_finite());
                assert!(model.maximum.is_finite());
                assert!(model.minimum < model.maximum);
                assert_eq!(model.fraction(model.minimum), 0.);
                assert_eq!(model.fraction(model.maximum), 1.);
            }

            let values = BaseSliderValue::new([f32::NAN, f32::NEG_INFINITY, f32::INFINITY, 25.]);

            assert_eq!(values.as_slice(), &[0., 0., 0., 25.]);
            assert_eq!(BaseSliderValue::new([]).as_slice(), &[0.]);
        }
    }

    #[test]
    fn thumb_collisions_preserve_spacing_and_active_index() {
        assert_eq!(BaseSliderModel::minimum_spacing(0., 10., 4., 2, 3), 5.);

        let values = BaseSliderValue::from([20., 50., 80.]);
        let cases = [
            (
                BaseSliderThumbCollisionBehavior::Clamp,
                1,
                100.,
                [20., 70., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Push,
                1,
                95.,
                [20., 90., 100.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Push,
                1,
                5.,
                [0., 10., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Swap,
                0,
                55.,
                [50., 60., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Swap,
                2,
                45.,
                [20., 40., 50.],
                1,
            ),
        ];

        for (behavior, thumb_idx, target, expected, expected_idx) in cases {
            let model = model(Axis::Horizontal, behavior, 10.);
            let (actual, active_idx) = behavior.resolve(&values, thumb_idx, target, model);

            assert_eq!(
                actual.as_slice(),
                &expected,
                "{behavior:?}, thumb {thumb_idx}"
            );
            assert_eq!(active_idx, expected_idx, "{behavior:?}, thumb {thumb_idx}");
        }

        let push_model = model(
            Axis::Horizontal,
            BaseSliderThumbCollisionBehavior::Push,
            10.,
        );
        let pushed = BaseSliderThumbCollisionBehavior::Push
            .resolve(&values, 0, 70., push_model)
            .0;
        let restored = BaseSliderThumbCollisionBehavior::Push
            .resolve(&pushed, 0, 20., push_model)
            .0;

        assert_eq!(pushed, BaseSliderValue::from([70., 80., 90.]));
        assert_eq!(restored, BaseSliderValue::from([20., 80., 90.]));
    }

    #[gpui::test]
    fn input_routes_report_live_and_committed_changes(cx: &mut TestAppContext) {
        let control = Bounds {
            origin: point(px(10.), px(20.)),
            size: size(px(100.), px(100.)),
        };
        let thumb_size = size(px(20.), px(20.));

        for (orientation, pointer, expected) in [
            (Axis::Horizontal, point(px(60.), px(70.)), 50.),
            (Axis::Vertical, point(px(60.), px(45.)), 75.),
        ] {
            let model = model(orientation, BaseSliderThumbCollisionBehavior::Clamp, 0.);

            assert_eq!(
                model.value_at_pointer(pointer, control, px(0.), thumb_size),
                expected,
                "{orientation:?}"
            );
        }

        let (view, cx) = setup(cx, [25., 75.]);
        let control = bounds(cx, CONTROL);
        let press = point(control.left() + px(130.), control.center().y);
        let drag = point(control.left() + px(170.), control.center().y);
        let outside = point(control.right() + px(30.), control.center().y);

        cx.simulate_mouse_down(press, MouseButton::Left, Modifiers::none());

        let state = snapshot(&view, cx);
        assert_eq!(state.values, BaseSliderValue::from([25., 65.]));
        assert_eq!(state.live.len(), 1);
        assert_eq!(state.live[0].reason, BaseSliderChangeReason::TrackPress);
        assert_eq!(state.live[0].active_thumb_index, 1);

        cx.simulate_mouse_move(drag, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::none());

        let state = snapshot(&view, cx);
        assert_eq!(state.values, BaseSliderValue::from([25., 100.]));
        assert_eq!(
            state
                .live
                .iter()
                .map(|change| change.reason)
                .collect::<Vec<_>>(),
            vec![
                BaseSliderChangeReason::TrackPress,
                BaseSliderChangeReason::Drag,
                BaseSliderChangeReason::Drag,
            ]
        );
        assert_eq!(state.committed.len(), 1);
        assert_eq!(state.committed[0].value, BaseSliderValue::from([25., 100.]));
        assert_eq!(state.committed[0].reason, BaseSliderChangeReason::Drag);
        assert_eq!(state.parent_clicks, 0);

        let thumb = bounds(cx, FIRST_THUMB);
        let unchanged = point(thumb.right() - px(1.), thumb.center().y);

        for (drag, live_count) in [
            (unchanged, 3),
            (point(control.left() + px(80.), control.center().y), 5),
        ] {
            cx.simulate_mouse_down(unchanged, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(drag, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(unchanged, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_up(unchanged, MouseButton::Left, Modifiers::none());

            let state = snapshot(&view, cx);
            assert_eq!(state.values, BaseSliderValue::from([25., 100.]));
            assert_eq!(state.live.len(), live_count);
            assert_eq!(state.committed.len(), 1);
        }

        view.update(cx, |view, cx| {
            view.values = BaseSliderValue::from([20., 80.]);
            view.live.clear();
            view.committed.clear();
            cx.notify();
        });

        for key in ["right", "shift-right", "pageup", "pagedown", "home", "end"] {
            activate_key(cx, key);
        }

        let state = snapshot(&view, cx);
        let expected = [25., 45., 65., 45., 0., 70.]
            .into_iter()
            .map(|value| BaseSliderChange {
                value: BaseSliderValue::from([value, 80.]),
                active_thumb_index: 0,
                reason: BaseSliderChangeReason::Keyboard,
            })
            .collect::<Vec<_>>();

        assert_eq!(state.live, expected);
        assert_eq!(state.committed, state.live);
        assert_eq!(state.parent_key_downs, 0);

        let touch_events = Arc::new(Mutex::new(Vec::new()));
        let captured_live = touch_events.clone();
        let captured_committed = touch_events.clone();
        cx.update(|window, cx| {
            let state = cx.new(|_cx| BaseSliderInteractionState {
                control_bounds: control,
                ..Default::default()
            });
            let interaction = BaseSliderInteraction {
                state,
                on_change: Some(Rc::new(move |change, _window, _cx| {
                    captured_live.lock().unwrap().push(("live", change.clone()));
                })),
                on_commit: Some(Rc::new(move |change, _window, _cx| {
                    captured_committed
                        .lock()
                        .unwrap()
                        .push(("committed", change.clone()));
                })),
            };

            interaction.propose_touch_tap(
                control.center(),
                &BaseSliderValue::from([25., 75.]),
                model(
                    Axis::Horizontal,
                    BaseSliderThumbCollisionBehavior::Push,
                    10.,
                ),
                window,
                cx,
            );
        });

        let expected = BaseSliderChange {
            value: BaseSliderValue::from([25., 50.]),
            active_thumb_index: 1,
            reason: BaseSliderChangeReason::TrackPress,
        };

        assert_eq!(
            touch_events.lock().unwrap().as_slice(),
            &[("live", expected.clone()), ("committed", expected)]
        );

        #[cfg(not(debug_assertions))]
        for (value_count, thumb_count) in [(1, 2), (2, 1), (1, 0)] {
            view.update(cx, |view, cx| {
                view.values = BaseSliderValue::new([25., 75.].into_iter().take(value_count));
                view.thumb_count = Some(thumb_count);
                view.live.clear();
                view.committed.clear();
                cx.notify();
            });

            let position = point(control.right() - px(5.), control.center().y);

            assert!(cx.debug_bounds(SECOND_THUMB).is_none());

            cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::none());

            assert_eq!(snapshot(&view, cx).live.len(), 1);
        }
    }

    #[gpui::test]
    fn controlled_values_and_configuration_changes_cancel_gestures(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx, [25.]);
        view.update(cx, |view, cx| {
            view.accept_changes = false;
            cx.notify();
        });

        let control = bounds(cx, CONTROL);
        let proposed = point(control.left() + px(150.), control.center().y);
        cx.simulate_mouse_down(proposed, MouseButton::Left, Modifiers::none());
        assert_eq!(bounds(cx, FIRST_THUMB).center().x, proposed.x);
        cx.simulate_mouse_up(proposed, MouseButton::Left, Modifiers::none());

        let rejected = snapshot(&view, cx);
        assert_eq!(rejected.values, BaseSliderValue::from(25.));
        assert_eq!(bounds(cx, FIRST_THUMB).center().x, control.left() + px(50.));

        for (disabled, has_handler, value_count, drag_offset, release_offset) in [
            (true, true, 1, 100., 220.),
            (false, false, 1, 120., 120.),
            (false, true, 2, 140., 140.),
        ] {
            view.update(cx, |view, cx| {
                view.accept_changes = true;
                view.disabled = false;
                view.has_handler = true;
                view.values = BaseSliderValue::from(25.);
                view.live.clear();
                view.committed.clear();
                cx.notify();
            });

            let thumb = bounds(cx, FIRST_THUMB);
            let drag = point(control.left() + px(drag_offset), control.center().y);
            let release = point(control.left() + px(release_offset), control.center().y);

            cx.simulate_mouse_down(thumb.center(), MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(drag, MouseButton::Left, Modifiers::none());

            view.update(cx, |view, cx| {
                view.disabled = disabled;
                view.has_handler = has_handler;

                if value_count == 2 {
                    view.values = BaseSliderValue::from([25., 75.]);
                }

                cx.notify();
            });

            cx.simulate_mouse_up(release, MouseButton::Left, Modifiers::none());
            assert!(
                snapshot(&view, cx).committed.is_empty(),
                "disabled={disabled}, has_handler={has_handler}, value_count={value_count}"
            );

            if disabled {
                let control = bounds(cx, CONTROL);
                cx.simulate_click(control.center(), Modifiers::none());

                let state = snapshot(&view, cx);

                assert!(state.committed.is_empty());
                assert_eq!(state.parent_clicks, 0);
            }
        }

        view.update(cx, |view, cx| {
            view.has_handler = true;
            view.values = BaseSliderValue::from(25.);
            cx.notify();
        });

        let restored_control = bounds(cx, CONTROL);
        let restored_press = point(
            restored_control.left() + px(160.),
            restored_control.center().y,
        );
        cx.simulate_mouse_down(restored_press, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(restored_press, MouseButton::Left, Modifiers::none());

        assert_eq!(snapshot(&view, cx).committed.len(), 1);
    }

    #[gpui::test]
    fn composition_preserves_geometry_and_accessibility(cx: &mut TestAppContext) {
        let control_size = size(px(100.), px(100.));
        let thumb_size = size(px(20.), px(20.));

        assert_eq!(
            BaseSliderModel::aligned_offset(
                px(100.),
                px(20.),
                0.,
                BaseSliderThumbAlignment::Center,
            ),
            px(-10.)
        );
        assert_eq!(
            BaseSliderModel::aligned_offset(px(100.), px(20.), 1., BaseSliderThumbAlignment::Edge,),
            px(80.)
        );

        for (orientation, x, y) in [(Axis::Horizontal, 0.25, 0.5), (Axis::Vertical, 0.5, 0.75)] {
            let model = model(orientation, BaseSliderThumbCollisionBehavior::Clamp, 0.);
            let expected = point(relative(x).into(), relative(y).into());

            for (control_size, thumb_size) in [
                (Size::default(), thumb_size),
                (control_size, Size::default()),
            ] {
                assert_eq!(
                    model.thumb_position(control_size, thumb_size, 25.),
                    expected,
                    "{orientation:?}"
                );
            }
        }

        let (view, visual_cx) = setup(cx, [75., 25.]);

        assert_eq!(bounds(visual_cx, SLIDER).size, size(px(200.), px(20.)));
        assert_eq!(bounds(visual_cx, CONTROL).size, size(px(200.), px(20.)));
        assert_eq!(bounds(visual_cx, TRACK).size, size(px(200.), px(20.)));
        assert_eq!(bounds(visual_cx, INDICATOR).size.width, px(100.));
        assert_eq!(bounds(visual_cx, FIRST_THUMB).center().x, px(90.));
        assert_eq!(bounds(visual_cx, SECOND_THUMB).center().x, px(190.));
        assert!(bounds(visual_cx, LABEL).size.width > px(0.));
        assert!(bounds(visual_cx, VALUE).size.width > px(0.));
        assert_eq!(bounds(visual_cx, ROOT_CHILD).size.width, px(11.));
        assert_eq!(bounds(visual_cx, TRACK_MARK).size.width, px(13.));

        view.update(visual_cx, |view, cx| {
            view.orientation = Axis::Vertical;
            view.alignment = BaseSliderThumbAlignment::Edge;
            cx.notify();
        });
        visual_cx.update(|window, cx| window.draw(cx).clear(cx));
        visual_cx.update(|window, cx| window.draw(cx).clear(cx));

        assert_eq!(bounds(visual_cx, SLIDER).size, size(px(20.), px(200.)));
        assert_eq!(bounds(visual_cx, FIRST_THUMB).center().y, px(185.));
        assert_eq!(bounds(visual_cx, SECOND_THUMB).center().y, px(95.));

        view.update(visual_cx, |view, cx| {
            view.thumb_labels = false;
            cx.notify();
        });

        let [lower, upper] = accessibility_nodes(visual_cx);

        assert_eq!(lower.role(), accesskit::Role::Slider);
        assert_eq!(lower.label(), Some("Range"));
        assert_eq!(lower.numeric_value(), Some(25.));
        assert_eq!(lower.min_numeric_value(), Some(0.));
        assert_eq!(lower.max_numeric_value(), Some(65.));
        assert_eq!(lower.numeric_value_step(), Some(5.));
        assert_eq!(lower.value(), Some("25 percent"));
        assert_eq!(lower.orientation(), Some(accesskit::Orientation::Vertical));
        assert!(lower.supports_action(accesskit::Action::Increment));
        assert!(lower.supports_action(accesskit::Action::Decrement));
        assert!(lower.supports_action(accesskit::Action::SetValue));
        assert!(lower.supports_action(accesskit::Action::Focus));
        assert!(!lower.is_disabled());

        assert_eq!(upper.label(), Some("Range"));
        assert_eq!(upper.numeric_value(), Some(75.));
        assert_eq!(upper.min_numeric_value(), Some(35.));
        assert_eq!(upper.max_numeric_value(), Some(100.));
        assert!(!upper.is_disabled());

        view.update(visual_cx, |view, cx| {
            view.thumb_labels = true;
            view.disabled = true;
            cx.notify();
        });

        for (node, label) in accessibility_nodes(visual_cx)
            .into_iter()
            .zip(["Minimum", "Maximum"])
        {
            assert_eq!(node.label(), Some(label));
            assert!(node.is_disabled());

            for action in [
                accesskit::Action::Increment,
                accesskit::Action::Decrement,
                accesskit::Action::SetValue,
                accesskit::Action::Focus,
            ] {
                assert!(!node.supports_action(action));
            }
        }

        assert_eq!(bounds(visual_cx, SLIDER).size.width, px(180.));
        assert_eq!(bounds(visual_cx, CONTROL).size.height, px(16.));
        assert_eq!(bounds(visual_cx, TRACK).size.height, px(12.));
        assert_eq!(bounds(visual_cx, INDICATOR).size.width, px(7.));
        assert_eq!(bounds(visual_cx, FIRST_THUMB).size.width, px(16.));
        assert_eq!(bounds(visual_cx, LABEL).size.width, px(9.));

        view.update(visual_cx, |view, cx| {
            view.disabled = false;
            view.has_handler = false;
            cx.notify();
        });

        for node in accessibility_nodes(visual_cx) {
            assert!(!node.is_disabled());
            assert!(!node.supports_action(accesskit::Action::Increment));
        }

        assert_eq!(bounds(visual_cx, SLIDER).size.width, px(20.));
        assert_eq!(bounds(visual_cx, FIRST_THUMB).size.width, px(20.));
    }
}
