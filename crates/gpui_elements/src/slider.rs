//! Unstyled, controlled sliders with composable labels, tracks, indicators, and thumbs.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, Div, ElementId, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement,
    Styled, Window, div, prelude::FluentBuilder,
};

mod interaction;
mod model;
mod parts;
mod thumb;

pub use interaction::{BaseSliderChange, BaseSliderChangeReason, OnChange, OnCommit};
pub use model::{BaseSliderThumbAlignment, BaseSliderThumbCollisionBehavior, BaseSliderValue};
pub use parts::{BaseSliderControl, BaseSliderIndicator, BaseSliderLabel, BaseSliderTrack};
pub use thumb::BaseSliderThumb;

use interaction::BaseSliderInteraction;
use model::BaseSliderModel;

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
