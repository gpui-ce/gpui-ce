//! When renderers may release the offscreen targets that filters need.
//!
//! Filters allocate full-resolution targets (the offscreen scene, a backdrop snapshot and one per
//! isolated group) plus downsampled blur scratch on first use. Keeping them for as long as the
//! window lives would pin that memory after the last filter disappears, so renderers drop each
//! kind of target once frames have stopped using it for a while.

use gpui::ScenePlanRequirements;

/// Consecutive rendered frames a filter target may go unused before it is released. Long enough
/// that a filter toggled on and off, or briefly scrolled out of view, doesn't reallocate.
pub const FILTER_TARGET_IDLE_FRAMES: u32 = 120;

/// A set of filter target kinds, e.g. those a frame uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilterTargets {
    /// The full-resolution target the scene renders into, so filters can sample it.
    pub offscreen: bool,
    /// The downsampled blur scratch textures.
    pub blur: bool,
    /// The full-resolution backdrop snapshot for backdrop filters inside isolated groups.
    pub backdrop_snapshot: bool,
    /// The full-resolution targets of isolated filter and opacity groups.
    pub isolated_groups: bool,
}

impl FilterTargets {
    /// The targets a scene with these requirements renders with.
    pub fn required_by(requirements: &ScenePlanRequirements) -> Self {
        let offscreen = requirements.uses_offscreen_target;
        Self {
            offscreen,
            blur: offscreen && requirements.uses_blur_target,
            backdrop_snapshot: offscreen
                && requirements.backdrop_filter_count > 0
                && requirements.isolated_target_count > 0,
            isolated_groups: offscreen && requirements.isolated_target_count > 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }

    fn kinds(self) -> [bool; 4] {
        [
            self.offscreen,
            self.blur,
            self.backdrop_snapshot,
            self.isolated_groups,
        ]
    }

    fn from_kinds([offscreen, blur, backdrop_snapshot, isolated_groups]: [bool; 4]) -> Self {
        Self {
            offscreen,
            blur,
            backdrop_snapshot,
            isolated_groups,
        }
    }
}

/// Counts, per target kind, the consecutive frames that haven't used it.
#[derive(Clone, Debug, Default)]
pub struct FilterTargetRetention {
    idle_frames: [u32; 4],
}

impl FilterTargetRetention {
    /// Records the targets a rendered frame uses, before the renderer allocates them, and
    /// returns the kinds that have now gone [`FILTER_TARGET_IDLE_FRAMES`] frames without use.
    /// Each idle stretch reports a kind once; releasing a target that was never allocated (or was
    /// already dropped, e.g. on resize) must be a no-op.
    pub fn record_frame(&mut self, used: FilterTargets) -> FilterTargets {
        let mut expired = [false; 4];
        for ((idle_frames, used), expired) in self
            .idle_frames
            .iter_mut()
            .zip(used.kinds())
            .zip(&mut expired)
        {
            if used {
                *idle_frames = 0;
            } else {
                *idle_frames = idle_frames.saturating_add(1);
                *expired = *idle_frames == FILTER_TARGET_IDLE_FRAMES;
            }
        }
        FilterTargets::from_kinds(expired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUR: FilterTargets = FilterTargets {
        offscreen: true,
        blur: true,
        backdrop_snapshot: false,
        isolated_groups: false,
    };
    const ALL: FilterTargets = FilterTargets {
        offscreen: true,
        blur: true,
        backdrop_snapshot: true,
        isolated_groups: true,
    };

    #[test]
    fn requirements_select_the_targets_renderers_allocate() {
        assert!(FilterTargets::required_by(&ScenePlanRequirements::default()).is_empty());
        // Without an offscreen target, nothing else is allocated either.
        assert!(
            FilterTargets::required_by(&ScenePlanRequirements {
                uses_blur_target: true,
                isolated_target_count: 2,
                ..Default::default()
            })
            .is_empty()
        );
        assert_eq!(
            FilterTargets::required_by(&ScenePlanRequirements {
                uses_offscreen_target: true,
                uses_blur_target: true,
                backdrop_filter_count: 1,
                ..Default::default()
            }),
            BLUR
        );
        assert_eq!(
            FilterTargets::required_by(&ScenePlanRequirements {
                uses_offscreen_target: true,
                backdrop_filter_count: 1,
                isolated_target_count: 1,
                ..Default::default()
            }),
            FilterTargets {
                offscreen: true,
                blur: false,
                backdrop_snapshot: true,
                isolated_groups: true,
            }
        );
    }

    #[test]
    fn targets_are_released_once_after_enough_idle_frames() {
        let mut retention = FilterTargetRetention::default();
        assert!(retention.record_frame(ALL).is_empty());
        for _ in 1..FILTER_TARGET_IDLE_FRAMES {
            assert!(retention.record_frame(FilterTargets::default()).is_empty());
        }
        assert_eq!(retention.record_frame(FilterTargets::default()), ALL);
        for _ in 0..FILTER_TARGET_IDLE_FRAMES * 2 {
            assert!(retention.record_frame(FilterTargets::default()).is_empty());
        }
    }

    #[test]
    fn any_use_restarts_the_idle_count() {
        let mut retention = FilterTargetRetention::default();
        for _ in 0..3 {
            assert!(retention.record_frame(ALL).is_empty());
            for _ in 1..FILTER_TARGET_IDLE_FRAMES {
                assert!(retention.record_frame(FilterTargets::default()).is_empty());
            }
        }
        let expired = retention.record_frame(BLUR);
        assert!(!expired.offscreen && !expired.blur);
        // A frame that only needs the offscreen target lets the blur scratch expire alone.
        let offscreen_only = FilterTargets {
            blur: false,
            ..BLUR
        };
        for _ in 1..FILTER_TARGET_IDLE_FRAMES {
            let expired = retention.record_frame(offscreen_only);
            assert!(!expired.offscreen && !expired.blur);
        }
        let expired = retention.record_frame(offscreen_only);
        assert!(!expired.offscreen && expired.blur);
    }

    #[test]
    fn kinds_never_used_expire_without_effect() {
        // Renderers ignore releases of targets they never allocated, so a fresh retention may
        // report every kind once; it must not keep reporting them.
        let mut retention = FilterTargetRetention::default();
        let mut reports = 0;
        for _ in 0..FILTER_TARGET_IDLE_FRAMES * 3 {
            reports += usize::from(!retention.record_frame(FilterTargets::default()).is_empty());
        }
        assert_eq!(reports, 1);
    }
}
