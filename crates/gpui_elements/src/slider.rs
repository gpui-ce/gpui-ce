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

#[cfg(test)]
mod tests {
    use super::{
        BaseSlider, BaseSliderChange, BaseSliderChangeReason, BaseSliderControl,
        BaseSliderIndicator, BaseSliderLabel, BaseSliderThumb, BaseSliderThumbAlignment,
        BaseSliderThumbCollisionBehavior, BaseSliderTrack, BaseSliderValue,
    };

    use gpui::{
        AppContext, Axis, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
        KeyUpEvent, Keystroke, Modifiers, MouseButton, ParentElement, Render,
        StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, Window, accesskit,
        div, point, prelude::FluentBuilder, px, size,
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

    #[gpui::test]
    fn input_routes_report_live_and_committed_changes(cx: &mut TestAppContext) {
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
