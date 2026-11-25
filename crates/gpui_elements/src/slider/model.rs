use gpui::{Axis, Bounds, Length, Pixels, Point, Size, px, relative};
use smallvec::SmallVec;

use super::BaseSlider;

/// How thumb bounds align with the control's endpoints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseSliderThumbAlignment {
    /// Places thumb centers at the endpoints at the minimum and maximum values.
    Center,
    /// Keeps the entire thumb inside the control at the endpoints.
    Edge,
}

#[derive(Clone, Copy)]
pub(super) struct BaseSliderModel {
    pub(super) minimum: f32,
    pub(super) maximum: f32,
    pub(super) step: f32,
    minimum_spacing: f32,
    pub(super) orientation: Axis,
    pub(super) alignment: BaseSliderThumbAlignment,
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

    pub(super) fn normalized_values(self, values: &BaseSliderValue) -> BaseSliderValue {
        let mut values = values
            .0
            .iter()
            .map(|value| value.clamp(self.minimum, self.maximum))
            .collect::<SmallVec<_>>();
        values.sort_by(f32::total_cmp);

        BaseSliderValue(values)
    }

    pub(super) fn fraction(self, value: f32) -> f32 {
        ((value.clamp(self.minimum, self.maximum) - self.minimum) / (self.maximum - self.minimum))
            .clamp(0., 1.)
    }

    pub(super) fn effective_bounds(self, values: &BaseSliderValue, thumb_idx: usize) -> (f32, f32) {
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

    pub(super) fn thumb_position(
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
}

impl BaseSlider {
    pub(super) fn model(&self, value_count: usize) -> BaseSliderModel {
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
            minimum_spacing: BaseSliderModel::minimum_spacing(
                minimum,
                maximum,
                self.step,
                self.min_steps_between_values,
                value_count,
            ),
            orientation: self.orientation,
            alignment: self.thumb_alignment,
        }
    }
}

/// One or more values controlled by the application that renders a slider.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseSliderValue(pub(super) SmallVec<[f32; 2]>);

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
