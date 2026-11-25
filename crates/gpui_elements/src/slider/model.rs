use gpui::{Axis, Bounds, Length, Pixels, Point, Size, px, relative};
use smallvec::SmallVec;

use super::BaseSlider;

/// How pointer interactions resolve collisions between neighboring thumbs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseSliderThumbCollisionBehavior {
    /// Moves neighboring values to maintain the minimum spacing.
    Push,
    /// Changes the active thumb's index as it crosses neighboring values.
    Swap,
    /// Stops the active value at the spacing limit imposed by its neighbors.
    Clamp,
}

impl BaseSliderThumbCollisionBehavior {
    pub(super) fn resolve(
        self,
        values: &BaseSliderValue,
        thumb_idx: usize,
        target: f32,
        model: BaseSliderModel,
    ) -> (BaseSliderValue, usize) {
        let mut next = values.clone();
        let mut active_idx = thumb_idx;

        match self {
            Self::Push => {
                let room_before = model.minimum_spacing * thumb_idx as f32;
                let room_after = model.minimum_spacing * (values.len() - thumb_idx - 1) as f32;
                next.0[thumb_idx] =
                    target.clamp(model.minimum + room_before, model.maximum - room_after);

                for current_idx in (0..thumb_idx).rev() {
                    let maximum = next.0[current_idx + 1] - model.minimum_spacing;
                    next.0[current_idx] = next.0[current_idx].min(maximum);
                }

                for current_idx in thumb_idx + 1..next.len() {
                    let minimum = next.0[current_idx - 1] + model.minimum_spacing;
                    next.0[current_idx] = next.0[current_idx].max(minimum);
                }
            }
            Self::Clamp | Self::Swap => {
                if self == Self::Swap {
                    while active_idx > 0 && target < next.0[active_idx - 1] {
                        next.0.swap(active_idx, active_idx - 1);
                        active_idx -= 1;
                    }

                    while active_idx + 1 < next.len() && target > next.0[active_idx + 1] {
                        next.0.swap(active_idx, active_idx + 1);
                        active_idx += 1;
                    }
                }

                let (minimum, maximum) = model.effective_bounds(&next, active_idx);
                next.0[active_idx] = target.clamp(minimum, maximum);
            }
        }

        (next, active_idx)
    }
}

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
    pub(super) collision_behavior: BaseSliderThumbCollisionBehavior,
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

    pub(super) fn snap(self, value: f32) -> f32 {
        let step_idx =
            ((f64::from(value) - f64::from(self.minimum)) / f64::from(self.step)).round();
        let snapped = f64::from(self.minimum) + step_idx * f64::from(self.step);

        (snapped as f32).clamp(self.minimum, self.maximum)
    }

    pub(super) fn fraction(self, value: f32) -> f32 {
        ((value.clamp(self.minimum, self.maximum) - self.minimum) / (self.maximum - self.minimum))
            .clamp(0., 1.)
    }

    pub(super) fn value_at_pointer(
        self,
        position: Point<Pixels>,
        control_bounds: Bounds<Pixels>,
        pointer_offset: Pixels,
        thumb_size: Size<Pixels>,
    ) -> f32 {
        let pointer = self.axis_position(position) - pointer_offset;
        let control_start = self.bounds_start(control_bounds);
        let control_length = self.size_along(control_bounds.size);
        let thumb_length = self.size_along(thumb_size);
        let (start, length) = match self.alignment {
            BaseSliderThumbAlignment::Center => (control_start, control_length),
            BaseSliderThumbAlignment::Edge => (
                control_start + thumb_length / 2.,
                (control_length - thumb_length).max(px(0.)),
            ),
        };
        let mut fraction = if length > px(0.) {
            (pointer - start) / length
        } else {
            0.
        };

        if self.orientation == Axis::Vertical {
            fraction = 1. - fraction;
        }

        let value = self.minimum + fraction.clamp(0., 1.) * (self.maximum - self.minimum);

        self.snap(value)
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

    pub(super) fn axis_position(self, position: Point<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => position.x,
            Axis::Vertical => position.y,
        }
    }

    pub(super) fn bounds_center_axis(self, bounds: Bounds<Pixels>) -> Pixels {
        self.axis_position(bounds.center())
    }

    pub(super) fn bounds_start(self, bounds: Bounds<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => bounds.left(),
            Axis::Vertical => bounds.top(),
        }
    }

    pub(super) fn size_along(self, size: Size<Pixels>) -> Pixels {
        match self.orientation {
            Axis::Horizontal => size.width,
            Axis::Vertical => size.height,
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
            collision_behavior: self.collision_behavior,
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

#[cfg(test)]
mod tests {
    use super::{
        BaseSliderModel, BaseSliderThumbAlignment, BaseSliderThumbCollisionBehavior,
        BaseSliderValue,
    };

    use gpui::Axis;

    fn model(
        orientation: Axis,
        behavior: BaseSliderThumbCollisionBehavior,
        minimum_spacing: f32,
    ) -> BaseSliderModel {
        BaseSliderModel {
            minimum: 0.,
            maximum: 100.,
            step: 5.,
            minimum_spacing,
            orientation,
            alignment: BaseSliderThumbAlignment::Center,
            collision_behavior: behavior,
        }
    }

    #[test]
    fn thumb_collisions_preserve_spacing_and_active_index() {
        assert_eq!(BaseSliderModel::minimum_spacing(0., 10., 4., 2, 3), 5.);

        let values = BaseSliderValue::from([20., 50., 80.]);
        let cases = [
            (
                BaseSliderThumbCollisionBehavior::Clamp,
                1,
                100.,
                [20., 70., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Push,
                1,
                95.,
                [20., 90., 100.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Push,
                1,
                5.,
                [0., 10., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Swap,
                0,
                55.,
                [50., 60., 80.],
                1,
            ),
            (
                BaseSliderThumbCollisionBehavior::Swap,
                2,
                45.,
                [20., 40., 50.],
                1,
            ),
        ];

        for (behavior, thumb_idx, target, expected, expected_idx) in cases {
            let model = model(Axis::Horizontal, behavior, 10.);
            let (actual, active_idx) = behavior.resolve(&values, thumb_idx, target, model);

            assert_eq!(
                actual.as_slice(),
                &expected,
                "{behavior:?}, thumb {thumb_idx}"
            );
            assert_eq!(active_idx, expected_idx, "{behavior:?}, thumb {thumb_idx}");
        }

        let push_model = model(
            Axis::Horizontal,
            BaseSliderThumbCollisionBehavior::Push,
            10.,
        );
        let pushed = BaseSliderThumbCollisionBehavior::Push
            .resolve(&values, 0, 70., push_model)
            .0;
        let restored = BaseSliderThumbCollisionBehavior::Push
            .resolve(&pushed, 0, 20., push_model)
            .0;

        assert_eq!(pushed, BaseSliderValue::from([70., 80., 90.]));
        assert_eq!(restored, BaseSliderValue::from([20., 80., 90.]));
    }
}
