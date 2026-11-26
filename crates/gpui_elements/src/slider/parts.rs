use gpui::{
    AnyElement, App, Axis, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, MouseButton, ParentElement, Role, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};
use smallvec::SmallVec;

use super::{BaseSliderRenderContext, BaseSliderThumb};

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

    pub(super) fn accessible_name(&self) -> Option<SharedString> {
        self.text.clone()
    }

    pub(super) fn render(self, disabled: bool) -> AnyElement {
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

    pub(super) fn render(
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
