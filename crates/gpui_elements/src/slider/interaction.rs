use gpui::{App, Bounds, ElementId, Entity, IntoElement, Pixels, Size, Styled, Window, canvas};
use smallvec::SmallVec;

#[derive(Clone)]
pub(super) struct BaseSliderInteraction {
    state: Entity<BaseSliderInteractionState>,
}

impl BaseSliderInteraction {
    pub(super) fn new(element_id: &ElementId, window: &mut Window, cx: &mut App) -> Self {
        let state =
            window.use_keyed_state((element_id.clone(), "state:slider"), cx, |_window, _cx| {
                BaseSliderInteractionState::default()
            });

        Self { state }
    }

    pub(super) fn control_size(&self, cx: &App) -> Size<Pixels> {
        self.state.read(cx).control_bounds.size
    }

    pub(super) fn thumb_size(&self, thumb_idx: usize, cx: &App) -> Size<Pixels> {
        self.state.read(cx).thumb_size(thumb_idx)
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
}

#[derive(Clone, Default)]
struct BaseSliderInteractionState {
    control_bounds: Bounds<Pixels>,
    thumb_bounds: SmallVec<[Bounds<Pixels>; 2]>,
}

impl BaseSliderInteractionState {
    fn thumb_size(&self, thumb_idx: usize) -> Size<Pixels> {
        self.thumb_bounds
            .get(thumb_idx)
            .map(|bounds| bounds.size)
            .unwrap_or_default()
    }
}
