use std::rc::Rc;

use gpui::{
    AnyElement, App, Axis, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, accesskit, div, prelude::FluentBuilder,
};

use super::{BaseSliderRenderContext, ValueTextFormatter};

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
        window: &mut Window,
        cx: &mut App,
    ) -> (AnyElement, FocusHandle) {
        let values = &context.values;
        let model = context.model;
        let interaction = &context.interaction;
        let value = values.0[thumb_idx];

        let control_size = interaction.control_size(cx);
        let thumb_size = interaction.thumb_size(thumb_idx, cx);
        let position = model.thumb_position(control_size, thumb_size, value);

        let (effective_minimum, effective_maximum) = model.effective_bounds(values, thumb_idx);
        let focus_handle = use_focus_handle(self.element_id.clone(), window, cx, None);

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
            .when(context.interactive, |this| this.track_focus(&focus_handle));

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
