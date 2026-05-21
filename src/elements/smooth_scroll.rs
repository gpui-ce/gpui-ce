use crate::Pixels;
use std::time::Instant;

/// Controls how scroll motion is visually presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmoothScrollMode {
    /// Disables smooth scrolling and snaps immediately to the target offset.
    Disabled,
    /// Smoothly interpolates toward the target scroll offset over time.
    Interpolated,
}

/// Stores state used for smooth scrolling animation.
#[derive(Clone, Debug)]
pub struct SmoothScrollState {
    /// The current smooth scrolling mode.
    pub mode: SmoothScrollMode,
    /// The currently rendered visual scroll offset.
    pub visual_offset: Pixels,
    /// The target logical scroll offset.
    pub target_offset: Pixels,
    /// Whether a smooth scroll animation is currently active.
    pub animating: bool,
    /// Timestamp of the previous animation frame.
    pub last_frame_time: Option<Instant>,
    /// Interpolation strength used during smoothing.
    ///
    /// Higher values move faster toward the target.
    pub smoothing_factor: f32,
}

impl Default for SmoothScrollState {
    fn default() -> Self {
        Self {
            mode: SmoothScrollMode::Interpolated,
            visual_offset: Pixels::ZERO,
            target_offset: Pixels::ZERO,
            animating: false,
            last_frame_time: None,
            smoothing_factor: 0.18,
        }
    }
}

impl SmoothScrollState {
    /// Sets the smooth scrolling mode.
    ///
    /// Disabling smooth scrolling immediately snaps the visual
    /// offset to the target offset.
    pub fn set_mode(&mut self, mode: SmoothScrollMode) {
        self.mode = mode;

        if mode == SmoothScrollMode::Disabled {
            self.visual_offset = self.target_offset;
            self.animating = false;
        }
    }

    /// Updates the target scroll offset.
    ///
    /// If smooth scrolling is enabled, animation toward the
    /// target begins automatically.
    pub fn set_target(&mut self, target: Pixels) {
        self.target_offset = target;

        if self.mode == SmoothScrollMode::Disabled {
            self.visual_offset = target;
            self.animating = false;
            return;
        }

        if !self.animating {
            self.last_frame_time = Some(Instant::now());
        }

        self.animating = true;
    }

    /// Advances the smooth scrolling animation.
    ///
    /// Returns `true` if the visual offset changed and another
    /// animation frame should be requested.
    pub fn update(&mut self) -> bool {
        if self.mode == SmoothScrollMode::Disabled {
            self.visual_offset = self.target_offset;
            self.animating = false;
            return false;
        }

        if !self.animating {
            return false;
        }

        let now = Instant::now();

        let dt = if let Some(last) = self.last_frame_time {
            now.duration_since(last).as_secs_f32()
        } else {
            0.016
        };

        self.last_frame_time = Some(now);

        let factor = 1.0 - (1.0 - self.smoothing_factor).powf(dt * 60.0);

        self.visual_offset.0 += (self.target_offset.0 - self.visual_offset.0) * factor;

        let remaining = (self.target_offset.0 - self.visual_offset.0).abs();

        if remaining < 0.5 {
            self.visual_offset = self.target_offset;
            self.animating = false;
            return false;
        }

        true
    }

    /// Returns the current visual scroll offset.
    ///
    /// When smooth scrolling is disabled, this returns the
    /// target offset directly.
    pub fn current(&self) -> Pixels {
        if self.mode == SmoothScrollMode::Disabled {
            self.target_offset
        } else {
            self.visual_offset
        }
    }
}
