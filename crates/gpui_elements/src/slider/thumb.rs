use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, Div, ElementId, InteractiveElement, Interactivity, IntoElement,
    ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement,
    Styled, accesskit, div, prelude::FluentBuilder,
};

use super::{BaseSliderRenderContext, ValueTextFormatter};

/// A thumb that displays one slider value.
#[derive(Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseSliderThumb {
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
    /// Creates a thumb with the supplied element ID.
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        Self {
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
    /// Overrides the formatter on [`BaseSlider`](super::BaseSlider).
    pub fn value_text_formatter<Text>(mut self, formatter: impl Fn(f32) -> Text + 'static) -> Self
    where
        Text: Into<SharedString>,
    {
        self.value_text_formatter = Some(Rc::new(move |value| formatter(value).into()));

        self
    }

    pub(super) fn render(
        self,
        context: &BaseSliderRenderContext,
        thumb_idx: usize,
        cx: &mut App,
    ) -> AnyElement {
        let values = &context.values;
        let model = context.model;
        let interaction = &context.interaction;
        let value = values.0[thumb_idx];

        let control_size = interaction.control_size(cx);
        let thumb_size = interaction.thumb_size(thumb_idx, cx);
        let position = model.thumb_position(control_size, thumb_size, value);

        let (effective_minimum, effective_maximum) = model.effective_bounds(values, thumb_idx);
        let aria_label = self.aria_label.or_else(|| context.label.clone());
        let value_text_formatter = self
            .value_text_formatter
            .or_else(|| context.value_text_formatter.clone());

        self.base
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
            .into_any_element()
    }
}
