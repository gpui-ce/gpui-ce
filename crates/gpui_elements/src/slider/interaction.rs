use std::rc::Rc;

use gpui::{
    App, Axis, Bounds, ClickEvent, DispatchPhase, ElementId, Entity, FocusHandle, IntoElement,
    MouseButton, MouseMoveEvent, ParentElement, Pixels, Point, Size, StatefulInteractiveElement,
    Styled, Window, canvas, px,
};
use smallvec::SmallVec;

use super::model::{
    BaseSliderModel, BaseSliderThumbAlignment, BaseSliderThumbCollisionBehavior, BaseSliderValue,
};

/// Shared handler for proposed slider value changes.
pub type OnChange = Rc<dyn Fn(&BaseSliderChange, &mut Window, &mut App)>;
/// Shared handler for committed slider value changes.
pub type OnCommit = Rc<dyn Fn(&BaseSliderChange, &mut Window, &mut App)>;

#[derive(Clone)]
pub(super) struct BaseSliderInteraction {
    state: Entity<BaseSliderInteractionState>,
    on_change: Option<OnChange>,
    on_commit: Option<OnCommit>,
}

impl BaseSliderInteraction {
    pub(super) fn new(
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

    pub(super) fn rendered_values(&self, fallback: &BaseSliderValue, cx: &App) -> BaseSliderValue {
        self.state
            .read(cx)
            .preview_values
            .clone()
            .unwrap_or_else(|| fallback.clone())
    }

    pub(super) fn control_size(&self, cx: &App) -> Size<Pixels> {
        self.state.read(cx).control_bounds.size
    }

    pub(super) fn thumb_size(&self, thumb_idx: usize, cx: &App) -> Size<Pixels> {
        self.state.read(cx).thumb_size(thumb_idx)
    }

    fn cancel(&self, cx: &mut App) {
        if self.state.read(cx).active_thumb_index.is_none() {
            return;
        }

        self.state.update(cx, |state, _cx| state.cancel());
    }

    pub(super) fn measure_bounds(&self, thumb_idx: Option<usize>) -> impl IntoElement {
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

    pub(super) fn install_pointer<Element>(
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
    pub(super) fn propose_discrete(
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
    use super::super::BaseSlider;
    use super::{
        BaseSliderChange, BaseSliderChangeReason, BaseSliderInteraction,
        BaseSliderInteractionState, BaseSliderThumbAlignment, BaseSliderValue,
    };

    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use gpui::{AppContext, Bounds, TestAppContext, point, px, size};

    #[gpui::test]
    fn touch_tap_reports_live_and_committed_changes(cx: &mut TestAppContext) {
        let control = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(200.), px(20.)),
        };
        let model = BaseSlider::new("touch-slider")
            .step(5.)
            .min_steps_between_values(2)
            .thumb_alignment(BaseSliderThumbAlignment::Center)
            .model(2);
        let cx = cx.add_empty_window();

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
                model,
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
    }
}
