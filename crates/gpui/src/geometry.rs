//! The GPUI geometry module is a collection of types and traits that
//! can be used to describe common units, concepts, and the relationships
//! between them.

use anyhow::{Context as _, anyhow};
use core::fmt::Debug;
use derive_more::{Add, AddAssign, Div, DivAssign, Mul, Neg, Sub, SubAssign};
use refineable::Refineable;
use schemars::{JsonSchema, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::borrow::Cow;
use std::ops::{AddAssign, Range};
use std::{
    cmp::{self, PartialOrd},
    f32::consts::GOLDEN_RATIO,
    fmt::{self, Display},
    hash::Hash,
    ops::{Add, Div, Mul, MulAssign, Neg, Sub},
};
use taffy::prelude::{TaffyGridLine, TaffyGridSpan};

use crate::{App, DisplayId};

/// Axis in a 2D cartesian space.
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Axis {
    /// The y axis, or up and down
    Vertical,
    /// The x axis, or left and right
    Horizontal,
}

impl Axis {
    /// Swap this axis to the opposite axis.
    pub fn invert(self) -> Self {
        match self {
            Axis::Vertical => Axis::Horizontal,
            Axis::Horizontal => Axis::Vertical,
        }
    }
}

/// A trait for accessing the given unit along a certain axis.
pub trait Along {
    /// The unit associated with this type
    type Unit;

    /// Returns the unit along the given axis.
    fn along(&self, axis: Axis) -> Self::Unit;

    /// Applies the given function to the unit along the given axis and returns a new value.
    fn apply_along(&self, axis: Axis, f: impl FnOnce(Self::Unit) -> Self::Unit) -> Self;
}

/// A point in 2D Cartesian space with coordinates of type `T`.
///
/// # Examples
///
/// ```
/// # use gpui::Point;
/// let point = Point { x: 10, y: 20 };
/// println!("{:?}", point); // Outputs: Point { x: 10, y: 20 }
/// ```
#[derive(
    Refineable,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    Hash,
    Neg,
)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Point<T: Clone + Debug + Default + PartialEq> {
    /// The x coordinate of the point.
    pub x: T,
    /// The y coordinate of the point.
    pub y: T,
}

/// Constructs a `Point<T>` with the given x and y coordinates.
///
/// # Examples
///
/// ```
/// use gpui::point;
/// let p = point(10, 20);
/// assert_eq!(p.x, 10);
/// assert_eq!(p.y, 20);
/// ```
pub const fn point<T: Clone + Debug + Default + PartialEq>(x: T, y: T) -> Point<T> {
    Point { x, y }
}

impl<T: Clone + Debug + Default + PartialEq> Point<T> {
    /// Creates a point with horizontal coordinate `x` and vertical coordinate `y`.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::Point;
    /// let p = Point::new(10, 20);
    /// assert_eq!(p.x, 10);
    /// assert_eq!(p.y, 20);
    /// ```
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Applies `f` to both coordinates, producing a `Point<U>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p = Point { x: 3, y: 4 };
    /// let p_float = p.map(|coord| coord as f32);
    /// assert_eq!(p_float, Point { x: 3.0, y: 4.0 });
    /// ```
    #[must_use]
    pub fn map<U: Clone + Debug + Default + PartialEq>(&self, f: impl Fn(T) -> U) -> Point<U> {
        Point {
            x: f(self.x.clone()),
            y: f(self.y.clone()),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> Along for Point<T> {
    type Unit = T;

    fn along(&self, axis: Axis) -> T {
        match axis {
            Axis::Horizontal => self.x.clone(),
            Axis::Vertical => self.y.clone(),
        }
    }

    fn apply_along(&self, axis: Axis, f: impl FnOnce(T) -> T) -> Point<T> {
        match axis {
            Axis::Horizontal => Point {
                x: f(self.x.clone()),
                y: self.y.clone(),
            },
            Axis::Vertical => Point {
                x: self.x.clone(),
                y: f(self.y.clone()),
            },
        }
    }
}

impl Point<Pixels> {
    /// Multiplies both coordinates by `factor`, usually the display scale factor.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Point, Pixels, ScaledPixels};
    /// let p = Point { x: Pixels::from(10.0), y: Pixels::from(20.0) };
    /// let scaled_p = p.scale(1.5);
    /// assert_eq!(scaled_p, Point { x: ScaledPixels::from(15.0), y: ScaledPixels::from(30.0) });
    /// ```
    pub fn scale(&self, factor: f32) -> Point<ScaledPixels> {
        Point {
            x: self.x.scale(factor),
            y: self.y.scale(factor),
        }
    }

    /// Calculates the Euclidean distance from the origin (0, 0) to this point.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Pixels, Point};
    /// let p = Point { x: Pixels::from(3.0), y: Pixels::from(4.0) };
    /// assert_eq!(p.magnitude(), 5.0);
    /// ```
    pub fn magnitude(&self) -> f64 {
        ((self.x.0.powi(2) + self.y.0.powi(2)) as f64).sqrt()
    }

    /// Returns true if the difference between self and `other` is less than `epsilon` in both coordinates.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Pixels, Point};
    /// let p1 = Point { x: Pixels::from(1.0), y: Pixels::from(1.0) };
    /// let p2 = Point { x: Pixels::from(3.0), y: Pixels::from(-2.0) };
    /// assert_eq!(p1.is_nearly_eq(&p2, Pixels::from(4.0)), true);
    /// ```
    pub fn is_nearly_eq(&self, other: &Self, epsilon: Pixels) -> bool {
        let diff = *self - *other;
        diff.x.abs() < epsilon && diff.y.abs() < epsilon
    }
}

impl<T> Point<T>
where
    T: Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Get the position of this point, relative to the given origin
    pub fn relative_to(&self, origin: &Point<T>) -> Point<T> {
        point(
            self.x.clone() - origin.x.clone(),
            self.y.clone() - origin.y.clone(),
        )
    }
}

impl<T, Rhs> Mul<Rhs> for Point<T>
where
    T: Mul<Rhs, Output = T> + Clone + Debug + Default + PartialEq,
    Rhs: Clone + Debug,
{
    type Output = Point<T>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Point {
            x: self.x * rhs.clone(),
            y: self.y * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Point<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.x = self.x.clone() * rhs.clone();
        self.y = self.y.clone() * rhs;
    }
}

impl<T, S> Div<S> for Point<T>
where
    T: Div<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    type Output = Self;

    fn div(self, rhs: S) -> Self::Output {
        Self {
            x: self.x / rhs.clone(),
            y: self.y / rhs,
        }
    }
}

impl<T> Point<T>
where
    T: PartialOrd + Clone + Debug + Default + PartialEq,
{
    /// Returns the maximum of `self` and `other` in each dimension.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p1 = Point { x: 3, y: 7 };
    /// let p2 = Point { x: 5, y: 2 };
    /// let max_point = p1.max(&p2);
    /// assert_eq!(max_point, Point { x: 5, y: 7 });
    /// ```
    pub fn max(&self, other: &Self) -> Self {
        Point {
            x: if self.x > other.x {
                self.x.clone()
            } else {
                other.x.clone()
            },
            y: if self.y > other.y {
                self.y.clone()
            } else {
                other.y.clone()
            },
        }
    }

    /// Returns the minimum of `self` and `other` in each dimension.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p1 = Point { x: 3, y: 7 };
    /// let p2 = Point { x: 5, y: 2 };
    /// let min_point = p1.min(&p2);
    /// assert_eq!(min_point, Point { x: 3, y: 2 });
    /// ```
    pub fn min(&self, other: &Self) -> Self {
        Point {
            x: if self.x <= other.x {
                self.x.clone()
            } else {
                other.x.clone()
            },
            y: if self.y <= other.y {
                self.y.clone()
            } else {
                other.y.clone()
            },
        }
    }

    /// Clamps each coordinate to the inclusive range defined by `min` and `max`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p = Point { x: 10, y: 20 };
    /// let min = Point { x: 0, y: 5 };
    /// let max = Point { x: 15, y: 25 };
    /// let clamped_p = p.clamp(&min, &max);
    /// assert_eq!(clamped_p, Point { x: 10, y: 20 });
    ///
    /// let p_out_of_bounds = Point { x: -5, y: 30 };
    /// let clamped_p_out_of_bounds = p_out_of_bounds.clamp(&min, &max);
    /// assert_eq!(clamped_p_out_of_bounds, Point { x: 0, y: 25 });
    /// ```
    pub fn clamp(&self, min: &Self, max: &Self) -> Self {
        self.max(min).min(max)
    }
}

impl<T: Clone + Debug + Default + PartialEq> Clone for Point<T> {
    fn clone(&self) -> Self {
        Self {
            x: self.x.clone(),
            y: self.y.clone(),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display> Display for Point<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// A two-dimensional size with width and height in units of type `T`.
#[derive(
    Add, Clone, Copy, Default, Deserialize, Div, Hash, Neg, PartialEq, Refineable, Serialize, Sub,
)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Size<T: Clone + Debug + Default + PartialEq> {
    /// The width component of the size.
    pub width: T,
    /// The height component of the size.
    pub height: T,
}

impl<T: Clone + Debug + Default + PartialEq> Size<T> {
    /// Create a new Size, a synonym for [`size`]
    pub fn new(width: T, height: T) -> Self {
        size(width, height)
    }
}

/// Constructs a `Size<T>` with the given width and height.
///
/// # Examples
///
/// ```
/// use gpui::size;
/// let my_size = size(10, 20);
/// assert_eq!(my_size.width, 10);
/// assert_eq!(my_size.height, 20);
/// ```
pub const fn size<T>(width: T, height: T) -> Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    Size { width, height }
}

impl<T> Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    /// Applies `f` to the width and height, producing a `Size<U>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Size;
    /// let my_size = Size { width: 10, height: 20 };
    /// let my_new_size = my_size.map(|dimension| dimension as f32 * 1.5);
    /// assert_eq!(my_new_size, Size { width: 15.0, height: 30.0 });
    /// ```
    pub fn map<U>(&self, f: impl Fn(T) -> U) -> Size<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Size {
            width: f(self.width.clone()),
            height: f(self.height.clone()),
        }
    }
}

impl<T> Size<T>
where
    T: Clone + Debug + Default + PartialEq + Half,
{
    /// Compute the center point of the size.g
    pub fn center(&self) -> Point<T> {
        Point {
            x: self.width.half(),
            y: self.height.half(),
        }
    }

    /// Returns a point at an anchor along the imaginary edge of a box with this size.
    pub fn at_anchor(&self, anchor: Anchor) -> Point<T> {
        match anchor {
            Anchor::TopLeft => Point::new(T::default(), T::default()),
            Anchor::TopRight => Point::new(self.width.clone(), T::default()),
            Anchor::BottomLeft => Point::new(T::default(), self.height.clone()),
            Anchor::BottomRight => Point::new(self.width.clone(), self.height.clone()),
            Anchor::TopCenter => Point::new(self.width.half(), T::default()),
            Anchor::BottomCenter => Point::new(self.width.half(), self.height.clone()),
            Anchor::LeftCenter => Point::new(T::default(), self.height.half()),
            Anchor::RightCenter => Point::new(self.width.clone(), self.height.half()),
        }
    }
}

impl Size<Pixels> {
    /// Multiplies the width and height by `factor`, producing a `Size<ScaledPixels>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Size, Pixels, ScaledPixels};
    /// let size = Size { width: Pixels::from(100.0), height: Pixels::from(50.0) };
    /// let scaled_size = size.scale(2.0);
    /// assert_eq!(scaled_size, Size { width: ScaledPixels::from(200.0), height: ScaledPixels::from(100.0) });
    /// ```
    pub fn scale(&self, factor: f32) -> Size<ScaledPixels> {
        Size {
            width: self.width.scale(factor),
            height: self.height.scale(factor),
        }
    }
}

impl<T> Along for Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    type Unit = T;

    fn along(&self, axis: Axis) -> T {
        match axis {
            Axis::Horizontal => self.width.clone(),
            Axis::Vertical => self.height.clone(),
        }
    }

    /// Returns the value of this size along the given axis.
    fn apply_along(&self, axis: Axis, f: impl FnOnce(T) -> T) -> Self {
        match axis {
            Axis::Horizontal => Size {
                width: f(self.width.clone()),
                height: self.height.clone(),
            },
            Axis::Vertical => Size {
                width: self.width.clone(),
                height: f(self.height.clone()),
            },
        }
    }
}

impl<T> Size<T>
where
    T: PartialOrd + Clone + Debug + Default + PartialEq,
{
    /// Returns the maximum width and height from `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Size;
    /// let size1 = Size { width: 30, height: 40 };
    /// let size2 = Size { width: 50, height: 20 };
    /// let max_size = size1.max(&size2);
    /// assert_eq!(max_size, Size { width: 50, height: 40 });
    /// ```
    pub fn max(&self, other: &Self) -> Self {
        Size {
            width: if self.width >= other.width {
                self.width.clone()
            } else {
                other.width.clone()
            },
            height: if self.height >= other.height {
                self.height.clone()
            } else {
                other.height.clone()
            },
        }
    }

    /// Returns the minimum width and height from `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Size;
    /// let size1 = Size { width: 30, height: 40 };
    /// let size2 = Size { width: 50, height: 20 };
    /// let min_size = size1.min(&size2);
    /// assert_eq!(min_size, Size { width: 30, height: 20 });
    /// ```
    pub fn min(&self, other: &Self) -> Self {
        Size {
            width: if self.width >= other.width {
                other.width.clone()
            } else {
                self.width.clone()
            },
            height: if self.height >= other.height {
                other.height.clone()
            } else {
                self.height.clone()
            },
        }
    }
}

impl<T, Rhs> Mul<Rhs> for Size<T>
where
    T: Mul<Rhs, Output = Rhs> + Clone + Debug + Default + PartialEq,
    Rhs: Clone + Debug + Default + PartialEq,
{
    type Output = Size<Rhs>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Size {
            width: self.width * rhs.clone(),
            height: self.height * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Size<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.width = self.width.clone() * rhs.clone();
        self.height = self.height.clone() * rhs;
    }
}

impl<T> Eq for Size<T> where T: Eq + Clone + Debug + Default + PartialEq {}

impl<T> Debug for Size<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Size {{ {:?} × {:?} }}", self.width, self.height)
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display> Display for Size<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} × {}", self.width, self.height)
    }
}

impl<T: Clone + Debug + Default + PartialEq> From<Point<T>> for Size<T> {
    fn from(point: Point<T>) -> Self {
        Self {
            width: point.x,
            height: point.y,
        }
    }
}

impl From<Size<Pixels>> for Size<DefiniteLength> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

impl From<Size<Pixels>> for Size<AbsoluteLength> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

impl Size<Length> {
    /// Returns a `Size<Length>` with both dimensions set to 100% of the available space.
    pub fn full() -> Self {
        Self {
            width: relative(1.).into(),
            height: relative(1.).into(),
        }
    }
}

impl Size<Length> {
    /// Returns a size with both dimensions set to `Length::Auto`.
    ///
    /// Layout determines the size from content dimensions and available space.
    pub fn auto() -> Self {
        Self {
            width: Length::Auto,
            height: Length::Auto,
        }
    }
}

/// A rectangle with a top left origin and a width and height in units of type `T`.
///
/// # Examples
///
/// ```
/// # use gpui::{Bounds, Point, Size};
/// let origin = Point { x: 0, y: 0 };
/// let size = Size { width: 10, height: 20 };
/// let bounds = Bounds::new(origin, size);
///
/// assert_eq!(bounds.origin, origin);
/// assert_eq!(bounds.size, size);
/// ```
#[derive(Refineable, Copy, Clone, Default, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
#[refineable(Debug)]
#[repr(C)]
pub struct Bounds<T: Clone + Debug + Default + PartialEq> {
    /// The origin point of this area.
    pub origin: Point<T>,
    /// The size of the rectangle.
    pub size: Size<T>,
}

/// Create a bounds with the given origin and size
pub fn bounds<T: Clone + Debug + Default + PartialEq>(
    origin: Point<T>,
    size: Size<T>,
) -> Bounds<T> {
    Bounds { origin, size }
}

impl Bounds<Pixels> {
    /// Generate a centered bounds for the given display or primary display if none is provided
    pub fn centered(display_id: Option<DisplayId>, size: Size<Pixels>, cx: &App) -> Self {
        let display = display_id
            .and_then(|id| cx.find_display(id))
            .or_else(|| cx.primary_display());

        display
            .map(|display| Bounds::centered_at(display.bounds().center(), size))
            .unwrap_or_else(|| Bounds {
                origin: point(px(0.), px(0.)),
                size,
            })
    }

    /// Generate maximized bounds for the given display or primary display if none is provided
    pub fn maximized(display_id: Option<DisplayId>, cx: &App) -> Self {
        let display = display_id
            .and_then(|id| cx.find_display(id))
            .or_else(|| cx.primary_display());

        display
            .map(|display| display.bounds())
            .unwrap_or_else(|| Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(1024.), px(768.)),
            })
    }
}

impl<T> Bounds<T>
where
    T: Clone + Debug + Default + PartialEq,
{
    /// Creates bounds with the given origin and size.
    pub fn new(origin: Point<T>, size: Size<T>) -> Self {
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Creates bounds with `top_left` as the origin and `bottom_right - top_left` as the size.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point};
    /// let top_left = Point { x: 0, y: 0 };
    /// let bottom_right = Point { x: 10, y: 10 };
    /// let bounds = Bounds::from_corners(top_left, bottom_right);
    ///
    /// assert_eq!(bounds.origin, top_left);
    /// assert_eq!(bounds.size.width, 10);
    /// assert_eq!(bounds.size.height, 10);
    /// ```
    pub fn from_corners(top_left: Point<T>, bottom_right: Point<T>) -> Self {
        let origin = Point {
            x: top_left.x.clone(),
            y: top_left.y.clone(),
        };
        let size = Size {
            width: bottom_right.x - top_left.x,
            height: bottom_right.y - top_left.y,
        };
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Constructs a `Bounds` from a corner point and size, where the new origin is the point
    /// at the anchor along the edges of size subtracted from the original origin.
    pub fn from_anchor_and_size(corner: Anchor, origin: Point<T>, size: Size<T>) -> Bounds<T> {
        let origin = origin - size.at_anchor(corner);
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Creates a new bounds centered at the given point.
    pub fn centered_at(center: Point<T>, size: Size<T>) -> Self {
        let origin = Point {
            x: center.x - size.width.half(),
            y: center.y - size.height.half(),
        };
        Self::new(origin, size)
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Returns the top center point of the bounds.
    pub fn top_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone(),
        }
    }

    /// Returns the bottom center point of the bounds.
    pub fn bottom_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    /// Returns the left center point of the bounds.
    pub fn left_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone(),
            y: self.origin.y.clone() + self.size.height.half(),
        }
    }

    /// Returns the right center point of the bounds.
    pub fn right_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone() + self.size.height.half(),
        }
    }
}

impl<T> Bounds<T>
where
    T: PartialOrd + Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Returns whether these bounds overlap `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds1 = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let bounds2 = Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let bounds3 = Bounds {
    ///     origin: Point { x: 20, y: 20 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    ///
    /// assert_eq!(bounds1.intersects(&bounds2), true); // Overlapping bounds
    /// assert_eq!(bounds1.intersects(&bounds3), false); // Non-overlapping bounds
    /// ```
    pub fn intersects(&self, other: &Bounds<T>) -> bool {
        let my_lower_right = self.bottom_right();
        let their_lower_right = other.bottom_right();

        self.origin.x < their_lower_right.x
            && my_lower_right.x > other.origin.x
            && self.origin.y < their_lower_right.y
            && my_lower_right.y > other.origin.y
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Returns the origin plus half the width and height.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let center = bounds.center();
    /// assert_eq!(center, Point { x: 5, y: 10 });
    /// ```
    pub fn center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone().half(),
            y: self.origin.y.clone() + self.size.height.clone().half(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Returns the half perimeter, the sum of the width and height.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let half_perimeter = bounds.half_perimeter();
    /// assert_eq!(half_perimeter, 30);
    /// ```
    pub fn half_perimeter(&self) -> T {
        self.size.width.clone() + self.size.height.clone()
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Expands the bounds by `amount` in every direction. Moves the origin by `-amount`
    /// and increases the width and height by twice `amount`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let mut bounds = Bounds {
    ///     origin: Point { x: 10, y: 10 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let expanded_bounds = bounds.dilate(5);
    /// assert_eq!(expanded_bounds, Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 20, height: 20 },
    /// });
    /// ```
    #[must_use]
    pub fn dilate(&self, amount: T) -> Bounds<T> {
        let double_amount = amount.clone() + amount.clone();
        Bounds {
            origin: self.origin.clone() - point(amount.clone(), amount),
            size: self.size.clone() + size(double_amount.clone(), double_amount),
        }
    }

    /// Extends the bounds different amounts in each direction.
    #[must_use]
    pub fn extend(&self, amount: Edges<T>) -> Bounds<T> {
        Bounds {
            origin: self.origin.clone() - point(amount.left.clone(), amount.top.clone()),
            size: self.size.clone()
                + size(
                    amount.left.clone() + amount.right.clone(),
                    amount.top.clone() + amount.bottom,
                ),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T>
        + Sub<T, Output = T>
        + Neg<Output = T>
        + Clone
        + Debug
        + Default
        + PartialEq,
{
    /// Insets the bounds, equivalent to `dilate` with the amount negated.
    ///
    /// May panic if `T` does not support negative values.
    pub fn inset(&self, amount: T) -> Self {
        self.dilate(-amount)
    }
}

impl<T: PartialOrd + Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq>
    Bounds<T>
{
    /// Returns the overlapping region of these bounds and `other`, with non-overlapping
    /// dimensions clamped to zero.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds1 = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let bounds2 = Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let intersection = bounds1.intersect(&bounds2);
    ///
    /// assert_eq!(intersection, Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 5, height: 5 },
    /// });
    /// ```
    pub fn intersect(&self, other: &Self) -> Self {
        let upper_left = self.origin.max(&other.origin);
        let bottom_right = self
            .bottom_right()
            .min(&other.bottom_right())
            .max(&upper_left);
        Self::from_corners(upper_left, bottom_right)
    }

    /// Returns the smallest bounds containing both `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds1 = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let bounds2 = Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 15, height: 15 },
    /// };
    /// let union_bounds = bounds1.union(&bounds2);
    ///
    /// assert_eq!(union_bounds, Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 20, height: 20 },
    /// });
    /// ```
    pub fn union(&self, other: &Self) -> Self {
        let top_left = self.origin.min(&other.origin);
        let bottom_right = self.bottom_right().max(&other.bottom_right());
        Bounds::from_corners(top_left, bottom_right)
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Computes the space available within outer bounds.
    pub fn space_within(&self, outer: &Self) -> Edges<T> {
        Edges {
            top: self.top() - outer.top(),
            right: outer.right() - self.right(),
            bottom: outer.bottom() - self.bottom(),
            left: self.left() - outer.left(),
        }
    }
}

impl<T, Rhs> Mul<Rhs> for Bounds<T>
where
    T: Mul<Rhs, Output = Rhs> + Clone + Debug + Default + PartialEq,
    Point<T>: Mul<Rhs, Output = Point<Rhs>>,
    Rhs: Clone + Debug + Default + PartialEq,
{
    type Output = Bounds<Rhs>;

    fn mul(self, rhs: Rhs) -> Self::Output {
        Bounds {
            origin: self.origin * rhs.clone(),
            size: self.size * rhs,
        }
    }
}

impl<T, S> MulAssign<S> for Bounds<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.origin *= rhs.clone();
        self.size *= rhs;
    }
}

impl<T, S> Div<S> for Bounds<T>
where
    Size<T>: Div<S, Output = Size<T>>,
    T: Div<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    type Output = Self;

    fn div(self, rhs: S) -> Self {
        Self {
            origin: self.origin / rhs.clone(),
            size: self.size / rhs,
        }
    }
}

impl<T> Add<Point<T>> for Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn add(self, rhs: Point<T>) -> Self {
        Self {
            origin: self.origin + rhs,
            size: self.size,
        }
    }
}

impl<T> Sub<Point<T>> for Bounds<T>
where
    T: Sub<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn sub(self, rhs: Point<T>) -> Self {
        Self {
            origin: self.origin - rhs,
            size: self.size,
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> From<Size<T>> for Point<T> {
    fn from(size: Size<T>) -> Self {
        Self {
            x: size.width,
            y: size.height,
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Returns the y-coordinate of the top edge.
    pub fn top(&self) -> T {
        self.origin.y.clone()
    }

    /// Returns the y-coordinate of the bottom edge.
    pub fn bottom(&self) -> T {
        self.origin.y.clone() + self.size.height.clone()
    }

    /// Returns the x-coordinate of the left edge.
    pub fn left(&self) -> T {
        self.origin.x.clone()
    }

    /// Returns the x-coordinate of the right edge.
    pub fn right(&self) -> T {
        self.origin.x.clone() + self.size.width.clone()
    }

    /// Returns the top right corner.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let top_right = bounds.top_right();
    /// assert_eq!(top_right, Point { x: 10, y: 0 });
    /// ```
    pub fn top_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone(),
        }
    }

    /// Returns the bottom right corner.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let bottom_right = bounds.bottom_right();
    /// assert_eq!(bottom_right, Point { x: 10, y: 20 });
    /// ```
    pub fn bottom_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    /// Returns the bottom left corner.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let bottom_left = bounds.bottom_left();
    /// assert_eq!(bottom_left, Point { x: 0, y: 20 });
    /// ```
    pub fn bottom_left(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Returns the requested corner point.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{Bounds, Anchor, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// let bottom_left = bounds.corner(Anchor::BottomLeft);
    /// assert_eq!(bottom_left, Point { x: 0, y: 20 });
    /// ```
    pub fn corner(&self, corner: Anchor) -> Point<T> {
        match corner {
            Anchor::TopLeft => self.origin.clone(),
            Anchor::TopRight => self.top_right(),
            Anchor::BottomLeft => self.bottom_left(),
            Anchor::BottomRight => self.bottom_right(),
            Anchor::TopCenter => self.top_center(),
            Anchor::BottomCenter => self.bottom_center(),
            Anchor::LeftCenter => self.left_center(),
            Anchor::RightCenter => self.right_center(),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + PartialOrd + Clone + Debug + Default + PartialEq,
{
    /// Returns whether the point lies within the bounds, including the edges.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Point, Bounds, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let inside_point = Point { x: 5, y: 5 };
    /// let outside_point = Point { x: 15, y: 15 };
    ///
    /// assert!(bounds.contains(&inside_point));
    /// assert!(!bounds.contains(&outside_point));
    /// ```
    pub fn contains(&self, point: &Point<T>) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x.clone() + self.size.width.clone()
            && point.y >= self.origin.y
            && point.y < self.origin.y.clone() + self.size.height.clone()
    }

    /// Returns whether `other` contains both the origin and bottom right corner of these bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let outer_bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 20, height: 20 },
    /// };
    /// let inner_bounds = Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let overlapping_bounds = Bounds {
    ///     origin: Point { x: 15, y: 15 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    ///
    /// assert!(inner_bounds.is_contained_within(&outer_bounds));
    /// assert!(!overlapping_bounds.is_contained_within(&outer_bounds));
    /// ```
    pub fn is_contained_within(&self, other: &Self) -> bool {
        other.contains(&self.origin) && other.contains(&self.bottom_right())
    }

    /// Applies `f` to each origin coordinate and size dimension, producing a `Bounds<U>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 10.0, y: 10.0 },
    ///     size: Size { width: 10.0, height: 20.0 },
    /// };
    /// let new_bounds = bounds.map(|value| value as f64 * 1.5);
    ///
    /// assert_eq!(new_bounds, Bounds {
    ///     origin: Point { x: 15.0, y: 15.0 },
    ///     size: Size { width: 15.0, height: 30.0 },
    /// });
    /// ```
    pub fn map<U>(&self, f: impl Fn(T) -> U) -> Bounds<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Bounds {
            origin: self.origin.map(&f),
            size: self.size.map(f),
        }
    }

    /// Applies a function to the origin  of the bounds, producing a new `Bounds` with the new origin
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 10.0, y: 10.0 },
    ///     size: Size { width: 10.0, height: 20.0 },
    /// };
    /// let new_bounds = bounds.map_origin(|value| value * 1.5);
    ///
    /// assert_eq!(new_bounds, Bounds {
    ///     origin: Point { x: 15.0, y: 15.0 },
    ///     size: Size { width: 10.0, height: 20.0 },
    /// });
    /// ```
    pub fn map_origin(self, f: impl Fn(T) -> T) -> Bounds<T> {
        Bounds {
            origin: self.origin.map(f),
            size: self.size,
        }
    }

    /// Applies a function to the origin  of the bounds, producing a new `Bounds` with the new origin
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 10.0, y: 10.0 },
    ///     size: Size { width: 10.0, height: 20.0 },
    /// };
    /// let new_bounds = bounds.map_size(|value| value * 1.5);
    ///
    /// assert_eq!(new_bounds, Bounds {
    ///     origin: Point { x: 10.0, y: 10.0 },
    ///     size: Size { width: 15.0, height: 30.0 },
    /// });
    /// ```
    pub fn map_size(self, f: impl Fn(T) -> T) -> Bounds<T> {
        Bounds {
            origin: self.origin,
            size: self.size.map(f),
        }
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<T, Output = T> + PartialOrd + Clone + Debug + Default + PartialEq,
{
    /// Convert a point to the coordinate space defined by this Bounds
    pub fn localize(&self, point: &Point<T>) -> Option<Point<T>> {
        self.contains(point)
            .then(|| point.relative_to(&self.origin))
    }
}

impl<T: PartialOrd + Clone + Debug + Default + PartialEq> Bounds<T> {
    /// Returns whether the width or height is zero or negative.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.width <= T::default() || self.size.height <= T::default()
    }
}

impl<T: Clone + Debug + Default + PartialEq + Display + Add<T, Output = T>> Display for Bounds<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} - {} (size {})",
            self.origin,
            self.bottom_right(),
            self.size
        )
    }
}

impl Size<DevicePixels> {
    /// Converts the size from physical to logical pixels.
    pub fn to_pixels(self, scale_factor: f32) -> Size<Pixels> {
        size(
            px(self.width.0 as f32 / scale_factor),
            px(self.height.0 as f32 / scale_factor),
        )
    }
}

impl Size<Pixels> {
    /// Converts the size from logical to physical pixels.
    pub fn to_device_pixels(self, scale_factor: f32) -> Size<DevicePixels> {
        size(
            DevicePixels((self.width.0 * scale_factor).round() as i32),
            DevicePixels((self.height.0 * scale_factor).round() as i32),
        )
    }
}

impl Bounds<Pixels> {
    /// Multiplies the origin and size by `factor`, producing `Bounds<ScaledPixels>`.
    /// Usually used with the display scale factor to account for pixel density.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size, Pixels, ScaledPixels, DevicePixels};
    /// let bounds = Bounds {
    ///     origin: Point { x: Pixels::from(10.0), y: Pixels::from(20.0) },
    ///     size: Size { width: Pixels::from(30.0), height: Pixels::from(40.0) },
    /// };
    /// let display_scale_factor = 2.0;
    /// let scaled_bounds = bounds.scale(display_scale_factor);
    /// assert_eq!(scaled_bounds, Bounds {
    ///     origin: Point {
    ///         x: ScaledPixels::from(20.0),
    ///         y: ScaledPixels::from(40.0),
    ///     },
    ///     size: Size {
    ///         width: ScaledPixels::from(60.0),
    ///         height: ScaledPixels::from(80.0)
    ///     },
    /// });
    /// ```
    pub fn scale(&self, factor: f32) -> Bounds<ScaledPixels> {
        Bounds {
            origin: self.origin.scale(factor),
            size: self.size.scale(factor),
        }
    }

    /// Convert the bounds from logical pixels to physical pixels
    pub fn to_device_pixels(self, factor: f32) -> Bounds<DevicePixels> {
        Bounds {
            origin: point(
                DevicePixels((self.origin.x.0 * factor).round() as i32),
                DevicePixels((self.origin.y.0 * factor).round() as i32),
            ),
            size: self.size.to_device_pixels(factor),
        }
    }
}

impl Bounds<DevicePixels> {
    /// Convert the bounds from physical pixels to logical pixels
    pub fn to_pixels(self, scale_factor: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(
                px(self.origin.x.0 as f32 / scale_factor),
                px(self.origin.y.0 as f32 / scale_factor),
            ),
            size: self.size.to_pixels(scale_factor),
        }
    }
}

/// Represents the edges of a box in a 2D space, such as padding or margin.
///
/// Each field represents the size of the edge on one side of the box: `top`, `right`, `bottom`, and `left`.
///
/// # Examples
///
/// ```
/// # use gpui::Edges;
/// let edges = Edges {
///     top: 10.0,
///     right: 20.0,
///     bottom: 30.0,
///     left: 40.0,
/// };
///
/// assert_eq!(edges.top, 10.0);
/// assert_eq!(edges.right, 20.0);
/// assert_eq!(edges.bottom, 30.0);
/// assert_eq!(edges.left, 40.0);
/// ```
#[derive(Refineable, Clone, Default, Debug, Eq, PartialEq)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Edges<T: Clone + Debug + Default + PartialEq> {
    /// The size of the top edge.
    pub top: T,
    /// The size of the right edge.
    pub right: T,
    /// The size of the bottom edge.
    pub bottom: T,
    /// The size of the left edge.
    pub left: T,
}

impl<T> Mul for Edges<T>
where
    T: Mul<Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            top: self.top.clone() * rhs.top,
            right: self.right.clone() * rhs.right,
            bottom: self.bottom.clone() * rhs.bottom,
            left: self.left * rhs.left,
        }
    }
}

impl<T, S> MulAssign<S> for Edges<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.top = self.top.clone() * rhs.clone();
        self.right = self.right.clone() * rhs.clone();
        self.bottom = self.bottom.clone() * rhs.clone();
        self.left = self.left.clone() * rhs;
    }
}

impl<T: Clone + Debug + Default + PartialEq + Copy> Copy for Edges<T> {}

impl<T: Clone + Debug + Default + PartialEq> Edges<T> {
    /// Creates edges with all four sides set to `value`, for uniform padding or margins.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Edges;
    /// let uniform_edges = Edges::all(10.0);
    /// assert_eq!(uniform_edges.top, 10.0);
    /// assert_eq!(uniform_edges.right, 10.0);
    /// assert_eq!(uniform_edges.bottom, 10.0);
    /// assert_eq!(uniform_edges.left, 10.0);
    /// ```
    pub fn all(value: T) -> Self {
        Self {
            top: value.clone(),
            right: value.clone(),
            bottom: value.clone(),
            left: value,
        }
    }

    /// Constructs `Edges` with the left and right sides set to the given value
    /// and the top and bottom sides set to their default.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Edges;
    /// let edges = Edges::horizontal(10.0);
    /// assert_eq!(edges.top, 0.0);
    /// assert_eq!(edges.right, 10.0);
    /// assert_eq!(edges.bottom, 0.0);
    /// assert_eq!(edges.left, 10.0);
    /// ```
    pub fn horizontal(value: T) -> Self {
        Self {
            top: T::default(),
            right: value.clone(),
            bottom: T::default(),
            left: value,
        }
    }

    /// Constructs `Edges` with the top and bottom sides set to the given value
    /// and the left and right sides set to their default.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Edges;
    /// let edges = Edges::vertical(10.0);
    /// assert_eq!(edges.top, 10.0);
    /// assert_eq!(edges.right, 0.0);
    /// assert_eq!(edges.bottom, 10.0);
    /// assert_eq!(edges.left, 0.0);
    /// ```
    pub fn vertical(value: T) -> Self {
        Self {
            top: value.clone(),
            right: T::default(),
            bottom: value,
            left: T::default(),
        }
    }

    /// Applies `f` to each edge by reference, producing `Edges<U>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Edges;
    /// let edges = Edges { top: 10, right: 20, bottom: 30, left: 40 };
    /// let edges_float = edges.map(|&value| value as f32 * 1.1);
    /// assert_eq!(edges_float, Edges { top: 11.0, right: 22.0, bottom: 33.0, left: 44.0 });
    /// ```
    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> Edges<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Edges {
            top: f(&self.top),
            right: f(&self.right),
            bottom: f(&self.bottom),
            left: f(&self.left),
        }
    }

    /// Returns whether any edge satisfies `predicate`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Edges;
    /// let edges = Edges {
    ///     top: 10,
    ///     right: 0,
    ///     bottom: 5,
    ///     left: 0,
    /// };
    ///
    /// assert!(edges.any(|value| *value == 0));
    /// assert!(edges.any(|value| *value > 0));
    /// assert!(!edges.any(|value| *value > 10));
    /// ```
    pub fn any<F: Fn(&T) -> bool>(&self, predicate: F) -> bool {
        predicate(&self.top)
            || predicate(&self.right)
            || predicate(&self.bottom)
            || predicate(&self.left)
    }
}

impl Edges<Length> {
    /// Returns edges set to `Length::Auto`, letting the layout engine determine their sizes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Edges, Length};
    /// let auto_edges = Edges::auto();
    /// assert_eq!(auto_edges.top, Length::Auto);
    /// assert_eq!(auto_edges.right, Length::Auto);
    /// assert_eq!(auto_edges.bottom, Length::Auto);
    /// assert_eq!(auto_edges.left, Length::Auto);
    /// ```
    pub fn auto() -> Self {
        Self {
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
        }
    }

    /// Returns zero-length edges.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{DefiniteLength, Edges, Length, Pixels};
    /// let no_edges = Edges::<Length>::zero();
    /// assert_eq!(no_edges.top, Length::Definite(DefiniteLength::from(Pixels::ZERO)));
    /// assert_eq!(no_edges.right, Length::Definite(DefiniteLength::from(Pixels::ZERO)));
    /// assert_eq!(no_edges.bottom, Length::Definite(DefiniteLength::from(Pixels::ZERO)));
    /// assert_eq!(no_edges.left, Length::Definite(DefiniteLength::from(Pixels::ZERO)));
    /// ```
    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }
}

impl Edges<DefiniteLength> {
    /// Returns zero-length edges.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{px, DefiniteLength, Edges};
    /// let no_edges = Edges::<DefiniteLength>::zero();
    /// assert_eq!(no_edges.top, DefiniteLength::from(px(0.)));
    /// assert_eq!(no_edges.right, DefiniteLength::from(px(0.)));
    /// assert_eq!(no_edges.bottom, DefiniteLength::from(px(0.)));
    /// assert_eq!(no_edges.left, DefiniteLength::from(px(0.)));
    /// ```
    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }

    /// Converts each edge to pixels using `parent_size` for percentage lengths
    /// and `rem_size` for rem lengths.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Edges, DefiniteLength, px, relative, AbsoluteLength, rems, Size};
    /// let edges = Edges {
    ///     top: DefiniteLength::Absolute(AbsoluteLength::Pixels(px(10.0))),
    ///     right: relative(0.5).into(),
    ///     bottom: DefiniteLength::Absolute(AbsoluteLength::Rems(rems(2.0))),
    ///     left: relative(0.25).into(),
    /// };
    /// let parent_size = Size {
    ///     width: AbsoluteLength::Pixels(px(200.0)),
    ///     height: AbsoluteLength::Pixels(px(100.0)),
    /// };
    /// let rem_size = px(16.0);
    /// let edges_in_pixels = edges.to_pixels(parent_size, rem_size);
    ///
    /// assert_eq!(edges_in_pixels.top, px(10.0)); // Absolute length in pixels
    /// assert_eq!(edges_in_pixels.right, px(100.0)); // 50% of parent width
    /// assert_eq!(edges_in_pixels.bottom, px(32.0)); // 2 rems
    /// assert_eq!(edges_in_pixels.left, px(50.0)); // 25% of parent width
    /// ```
    pub fn to_pixels(self, parent_size: Size<AbsoluteLength>, rem_size: Pixels) -> Edges<Pixels> {
        Edges {
            top: self.top.to_pixels(parent_size.height, rem_size),
            right: self.right.to_pixels(parent_size.width, rem_size),
            bottom: self.bottom.to_pixels(parent_size.height, rem_size),
            left: self.left.to_pixels(parent_size.width, rem_size),
        }
    }
}

impl Edges<AbsoluteLength> {
    /// Returns zero-length edges.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AbsoluteLength, Edges, Pixels};
    /// let no_edges = Edges::<AbsoluteLength>::zero();
    /// assert_eq!(no_edges.top, AbsoluteLength::Pixels(Pixels::ZERO));
    /// assert_eq!(no_edges.right, AbsoluteLength::Pixels(Pixels::ZERO));
    /// assert_eq!(no_edges.bottom, AbsoluteLength::Pixels(Pixels::ZERO));
    /// assert_eq!(no_edges.left, AbsoluteLength::Pixels(Pixels::ZERO));
    /// ```
    pub fn zero() -> Self {
        Self {
            top: px(0.).into(),
            right: px(0.).into(),
            bottom: px(0.).into(),
            left: px(0.).into(),
        }
    }

    /// Converts each edge to pixels. Pixel lengths stay unchanged, and rem lengths
    /// are multiplied by `rem_size`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Edges, AbsoluteLength, Pixels, px, rems};
    /// let edges = Edges {
    ///     top: AbsoluteLength::Pixels(px(10.0)),
    ///     right: AbsoluteLength::Rems(rems(1.0)),
    ///     bottom: AbsoluteLength::Pixels(px(20.0)),
    ///     left: AbsoluteLength::Rems(rems(2.0)),
    /// };
    /// let rem_size = px(16.0);
    /// let edges_in_pixels = edges.to_pixels(rem_size);
    ///
    /// assert_eq!(edges_in_pixels.top, px(10.0)); // Already in pixels
    /// assert_eq!(edges_in_pixels.right, px(16.0)); // 1 rem converted to pixels
    /// assert_eq!(edges_in_pixels.bottom, px(20.0)); // Already in pixels
    /// assert_eq!(edges_in_pixels.left, px(32.0)); // 2 rems converted to pixels
    /// ```
    pub fn to_pixels(self, rem_size: Pixels) -> Edges<Pixels> {
        Edges {
            top: self.top.to_pixels(rem_size),
            right: self.right.to_pixels(rem_size),
            bottom: self.bottom.to_pixels(rem_size),
            left: self.left.to_pixels(rem_size),
        }
    }
}

impl Edges<Pixels> {
    /// Multiplies each edge by `factor`, producing `Edges<ScaledPixels>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Edges, Pixels, ScaledPixels};
    /// let edges = Edges {
    ///     top: Pixels::from(10.0),
    ///     right: Pixels::from(20.0),
    ///     bottom: Pixels::from(30.0),
    ///     left: Pixels::from(40.0),
    /// };
    /// let scaled_edges = edges.scale(2.0);
    /// assert_eq!(scaled_edges.top, ScaledPixels::from(20.0));
    /// assert_eq!(scaled_edges.right, ScaledPixels::from(40.0));
    /// assert_eq!(scaled_edges.bottom, ScaledPixels::from(60.0));
    /// assert_eq!(scaled_edges.left, ScaledPixels::from(80.0));
    /// ```
    pub fn scale(&self, factor: f32) -> Edges<ScaledPixels> {
        Edges {
            top: self.top.scale(factor),
            right: self.right.scale(factor),
            bottom: self.bottom.scale(factor),
            left: self.left.scale(factor),
        }
    }

    /// Returns the maximum value among all four edges.
    pub fn max(&self) -> Pixels {
        self.top.max(self.right).max(self.bottom).max(self.left)
    }
}

impl From<f32> for Edges<Pixels> {
    fn from(val: f32) -> Self {
        let val: Pixels = val.into();
        val.into()
    }
}

impl From<Pixels> for Edges<Pixels> {
    fn from(val: Pixels) -> Self {
        Edges {
            top: val,
            right: val,
            bottom: val,
            left: val,
        }
    }
}

/// Identifies a reference point on a 2D box, used to anchor positioned elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The top left corner
    TopLeft,
    /// The top right corner
    TopRight,
    /// The bottom left corner
    BottomLeft,
    /// The bottom right corner
    BottomRight,
    /// The top center position
    TopCenter,
    /// The bottom center position
    BottomCenter,
    /// The left center position
    LeftCenter,
    /// The right center position
    RightCenter,
}

impl Anchor {
    /// Returns the directly opposite corner.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Anchor;
    /// assert_eq!(Anchor::TopLeft.opposite(), Anchor::BottomRight);
    /// ```
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Anchor::TopLeft => Anchor::BottomRight,
            Anchor::TopRight => Anchor::BottomLeft,
            Anchor::BottomLeft => Anchor::TopRight,
            Anchor::BottomRight => Anchor::TopLeft,
            Anchor::TopCenter => Anchor::BottomCenter,
            Anchor::BottomCenter => Anchor::TopCenter,
            Anchor::LeftCenter => Anchor::RightCenter,
            Anchor::RightCenter => Anchor::LeftCenter,
        }
    }

    /// Returns the corner across from this corner, moving along the specified axis.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Axis, Anchor};
    /// let result = Anchor::TopLeft.other_side_along(Axis::Horizontal);
    /// assert_eq!(result, Anchor::TopRight);
    /// ```
    #[must_use]
    pub fn other_side_along(self, axis: Axis) -> Self {
        match axis {
            Axis::Vertical => match self {
                Anchor::TopLeft => Anchor::BottomLeft,
                Anchor::TopRight => Anchor::BottomRight,
                Anchor::BottomLeft => Anchor::TopLeft,
                Anchor::BottomRight => Anchor::TopRight,
                Anchor::TopCenter => Anchor::BottomCenter,
                Anchor::BottomCenter => Anchor::TopCenter,
                Anchor::LeftCenter => Anchor::LeftCenter,
                Anchor::RightCenter => Anchor::RightCenter,
            },
            Axis::Horizontal => match self {
                Anchor::TopLeft => Anchor::TopRight,
                Anchor::TopRight => Anchor::TopLeft,
                Anchor::BottomLeft => Anchor::BottomRight,
                Anchor::BottomRight => Anchor::BottomLeft,
                Anchor::TopCenter => Anchor::TopCenter,
                Anchor::BottomCenter => Anchor::BottomCenter,
                Anchor::LeftCenter => Anchor::RightCenter,
                Anchor::RightCenter => Anchor::LeftCenter,
            },
        }
    }

    /// Returns true if at the center.
    #[inline]
    pub fn is_center(&self) -> bool {
        matches!(
            self,
            Self::TopCenter | Self::BottomCenter | Self::LeftCenter | Self::RightCenter
        )
    }
}

/// Represents the corners of a box in a 2D space, such as border radius.
///
/// Each field represents the size of the corner on one side of the box: `top_left`, `top_right`, `bottom_right`, and `bottom_left`.
#[derive(Refineable, Clone, Default, Debug, Eq, PartialEq)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Corners<T: Clone + Debug + Default + PartialEq> {
    /// The value associated with the top left corner.
    pub top_left: T,
    /// The value associated with the top right corner.
    pub top_right: T,
    /// The value associated with the bottom right corner.
    pub bottom_right: T,
    /// The value associated with the bottom left corner.
    pub bottom_left: T,
}

impl<T> Corners<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Creates corners with all four values set to `value`, for a uniform border radius.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Corners;
    /// let uniform_corners = Corners::all(5.0);
    /// assert_eq!(uniform_corners.top_left, 5.0);
    /// assert_eq!(uniform_corners.top_right, 5.0);
    /// assert_eq!(uniform_corners.bottom_right, 5.0);
    /// assert_eq!(uniform_corners.bottom_left, 5.0);
    /// ```
    pub fn all(value: T) -> Self {
        Self {
            top_left: value.clone(),
            top_right: value.clone(),
            bottom_right: value.clone(),
            bottom_left: value,
        }
    }

    /// Returns the field value for a corner position.
    ///
    /// For `TopCenter`, `BottomCenter`, `LeftCenter`, and `RightCenter`, returns
    /// the average of the two adjacent corners, supporting all eight positions.
    ///
    /// # Examples
    ///
    /// Basic corner positions:
    ///
    /// ```
    /// # use gpui::{Anchor, Corners};
    /// let corners = Corners {
    ///     top_left: 10,
    ///     top_right: 20,
    ///     bottom_left: 30,
    ///     bottom_right: 40
    /// };
    /// assert_eq!(corners.corner(Anchor::TopLeft), 10);
    /// assert_eq!(corners.corner(Anchor::BottomRight), 40);
    /// ```
    ///
    /// Center positions (calculated as average of adjacent corners):
    ///
    /// ```
    /// # use gpui::{Anchor, Corners};
    /// let corners = Corners {
    ///     top_left: 10,
    ///     top_right: 20,
    ///     bottom_left: 30,
    ///     bottom_right: 40
    /// };
    /// assert_eq!(corners.corner(Anchor::TopCenter), 15);
    /// assert_eq!(corners.corner(Anchor::BottomCenter), 35);
    /// assert_eq!(corners.corner(Anchor::LeftCenter), 20);
    /// assert_eq!(corners.corner(Anchor::RightCenter), 30);
    /// ```
    #[must_use]
    pub fn corner(&self, corner: Anchor) -> T {
        match corner {
            Anchor::TopLeft => self.top_left.clone(),
            Anchor::TopRight => self.top_right.clone(),
            Anchor::BottomLeft => self.bottom_left.clone(),
            Anchor::BottomRight => self.bottom_right.clone(),
            Anchor::TopCenter => (self.top_left.clone() + self.top_right.clone()).half(),
            Anchor::BottomCenter => (self.bottom_left.clone() + self.bottom_right.clone()).half(),
            Anchor::LeftCenter => (self.top_left.clone() + self.bottom_left.clone()).half(),
            Anchor::RightCenter => (self.top_right.clone() + self.bottom_right.clone()).half(),
        }
    }
}

impl Corners<AbsoluteLength> {
    /// Converts each corner to pixels, using `rem_size` as the pixel size of one rem.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, AbsoluteLength, Pixels, Rems, Size};
    /// let corners = Corners {
    ///     top_left: AbsoluteLength::Pixels(Pixels::from(15.0)),
    ///     top_right: AbsoluteLength::Rems(Rems(1.0)),
    ///     bottom_right: AbsoluteLength::Pixels(Pixels::from(30.0)),
    ///     bottom_left: AbsoluteLength::Rems(Rems(2.0)),
    /// };
    /// let rem_size = Pixels::from(16.0);
    /// let corners_in_pixels = corners.to_pixels(rem_size);
    ///
    /// assert_eq!(corners_in_pixels.top_left, Pixels::from(15.0));
    /// assert_eq!(corners_in_pixels.top_right, Pixels::from(16.0)); // 1 rem converted to pixels
    /// assert_eq!(corners_in_pixels.bottom_right, Pixels::from(30.0));
    /// assert_eq!(corners_in_pixels.bottom_left, Pixels::from(32.0)); // 2 rems converted to pixels
    /// ```
    pub fn to_pixels(self, rem_size: Pixels) -> Corners<Pixels> {
        Corners {
            top_left: self.top_left.to_pixels(rem_size),
            top_right: self.top_right.to_pixels(rem_size),
            bottom_right: self.bottom_right.to_pixels(rem_size),
            bottom_left: self.bottom_left.to_pixels(rem_size),
        }
    }
}

impl Corners<Pixels> {
    /// Multiplies each corner by `factor`, producing `Corners<ScaledPixels>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, Pixels, ScaledPixels};
    /// let corners = Corners {
    ///     top_left: Pixels::from(10.0),
    ///     top_right: Pixels::from(20.0),
    ///     bottom_right: Pixels::from(30.0),
    ///     bottom_left: Pixels::from(40.0),
    /// };
    /// let scaled_corners = corners.scale(2.0);
    /// assert_eq!(scaled_corners.top_left, ScaledPixels::from(20.0));
    /// assert_eq!(scaled_corners.top_right, ScaledPixels::from(40.0));
    /// assert_eq!(scaled_corners.bottom_right, ScaledPixels::from(60.0));
    /// assert_eq!(scaled_corners.bottom_left, ScaledPixels::from(80.0));
    /// ```
    #[must_use]
    pub fn scale(&self, factor: f32) -> Corners<ScaledPixels> {
        Corners {
            top_left: self.top_left.scale(factor),
            top_right: self.top_right.scale(factor),
            bottom_right: self.bottom_right.scale(factor),
            bottom_left: self.bottom_left.scale(factor),
        }
    }

    /// Returns the maximum value among all four corners.
    #[must_use]
    pub fn max(&self) -> Pixels {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl<T: Div<f32, Output = T> + Ord + Clone + Debug + Default + PartialEq> Corners<T> {
    /// Clamps each corner radius to at most half the shortest side of `size`.
    #[must_use]
    pub fn clamp_radii_for_quad_size(self, size: Size<T>) -> Corners<T> {
        let max = cmp::min(size.width, size.height) / 2.;
        Corners {
            top_left: cmp::min(self.top_left, max.clone()),
            top_right: cmp::min(self.top_right, max.clone()),
            bottom_right: cmp::min(self.bottom_right, max.clone()),
            bottom_left: cmp::min(self.bottom_left, max),
        }
    }
}

impl<T: Clone + Debug + Default + PartialEq> Corners<T> {
    /// Applies `f` to each corner by reference, producing `Corners<U>`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, Pixels, Rems};
    /// let corners = Corners {
    ///     top_left: Pixels::from(10.0),
    ///     top_right: Pixels::from(20.0),
    ///     bottom_right: Pixels::from(30.0),
    ///     bottom_left: Pixels::from(40.0),
    /// };
    /// let corners_in_rems = corners.map(|&px| Rems(f32::from(px) / 16.0));
    /// assert_eq!(corners_in_rems, Corners {
    ///     top_left: Rems(0.625),
    ///     top_right: Rems(1.25),
    ///     bottom_right: Rems(1.875),
    ///     bottom_left: Rems(2.5),
    /// });
    /// ```
    #[must_use]
    pub fn map<U>(&self, f: impl Fn(&T) -> U) -> Corners<U>
    where
        U: Clone + Debug + Default + PartialEq,
    {
        Corners {
            top_left: f(&self.top_left),
            top_right: f(&self.top_right),
            bottom_right: f(&self.bottom_right),
            bottom_left: f(&self.bottom_left),
        }
    }
}

impl<T> Mul for Corners<T>
where
    T: Mul<Output = T> + Clone + Debug + Default + PartialEq,
{
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            top_left: self.top_left.clone() * rhs.top_left,
            top_right: self.top_right.clone() * rhs.top_right,
            bottom_right: self.bottom_right.clone() * rhs.bottom_right,
            bottom_left: self.bottom_left * rhs.bottom_left,
        }
    }
}

impl<T, S> MulAssign<S> for Corners<T>
where
    T: Mul<S, Output = T> + Clone + Debug + Default + PartialEq,
    S: Clone,
{
    fn mul_assign(&mut self, rhs: S) {
        self.top_left = self.top_left.clone() * rhs.clone();
        self.top_right = self.top_right.clone() * rhs.clone();
        self.bottom_right = self.bottom_right.clone() * rhs.clone();
        self.bottom_left = self.bottom_left.clone() * rhs;
    }
}

impl<T> Copy for Corners<T> where T: Copy + Clone + Debug + Default + PartialEq {}

impl From<f32> for Corners<Pixels> {
    fn from(val: f32) -> Self {
        Corners {
            top_left: val.into(),
            top_right: val.into(),
            bottom_right: val.into(),
            bottom_left: val.into(),
        }
    }
}

impl From<Pixels> for Corners<Pixels> {
    fn from(val: Pixels) -> Self {
        Corners {
            top_left: val,
            top_right: val,
            bottom_right: val,
            bottom_left: val,
        }
    }
}

/// Represents an angle in Radians
#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    Debug,
)]
#[repr(transparent)]
pub struct Radians(pub f32);

/// Create a `Radian` from a raw value
pub fn radians(value: f32) -> Radians {
    Radians(value)
}

/// A type representing a percentage value.
#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    Debug,
)]
#[repr(transparent)]
pub struct Percentage(pub f32);

/// Generate a `Radian` from a percentage of a full circle.
pub fn percentage(value: f32) -> Percentage {
    debug_assert!(
        (0.0..=1.0).contains(&value),
        "Percentage must be between 0 and 1"
    );
    Percentage(value)
}

impl From<Percentage> for Radians {
    fn from(value: Percentage) -> Self {
        radians(value.0 * std::f32::consts::PI * 2.0)
    }
}

/// A logical pixel length stored as an `f32` for subpixel precision.
/// Values are typically rounded when mapped to a physical pixel grid.
///
/// # Examples
///
/// ```
/// use gpui::{Pixels, ScaledPixels};
///
/// // Define a length of 10 pixels
/// let length = Pixels::from(10.0);
///
/// // Define a length and scale it by a factor of 2
/// let scaled_length = length.scale(2.0);
/// assert_eq!(scaled_length, ScaledPixels::from(20.0));
/// ```
#[derive(
    Clone,
    Copy,
    Default,
    Add,
    AddAssign,
    Sub,
    SubAssign,
    Neg,
    Div,
    DivAssign,
    PartialEq,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[repr(transparent)]
pub struct Pixels(pub(crate) f32);

impl Div for Pixels {
    type Output = f32;

    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

impl std::ops::DivAssign for Pixels {
    fn div_assign(&mut self, rhs: Self) {
        *self = Self(self.0 / rhs.0);
    }
}

impl std::ops::RemAssign for Pixels {
    fn rem_assign(&mut self, rhs: Self) {
        self.0 %= rhs.0;
    }
}

impl std::ops::Rem for Pixels {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

impl Mul<f32> for Pixels {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs)
    }
}

impl Mul<Pixels> for f32 {
    type Output = Pixels;

    fn mul(self, rhs: Pixels) -> Self::Output {
        rhs * self
    }
}

impl Mul<usize> for Pixels {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        self * (rhs as f32)
    }
}

impl Mul<Pixels> for usize {
    type Output = Pixels;

    fn mul(self, rhs: Pixels) -> Pixels {
        rhs * self
    }
}

impl MulAssign<f32> for Pixels {
    fn mul_assign(&mut self, rhs: f32) {
        self.0 *= rhs;
    }
}

impl Display for Pixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}px", self.0)
    }
}

impl Debug for Pixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl std::iter::Sum for Pixels {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + b)
    }
}

impl<'a> std::iter::Sum<&'a Pixels> for Pixels {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + *b)
    }
}

impl TryFrom<&'_ str> for Pixels {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value
            .strip_suffix("px")
            .context("expected 'px' suffix")
            .and_then(|number| Ok(number.parse()?))
            .map(Self)
    }
}

impl Pixels {
    /// Represents zero pixels.
    pub const ZERO: Pixels = Pixels(0.0);
    /// The maximum value that can be represented by `Pixels`.
    pub const MAX: Pixels = Pixels(f32::MAX);
    /// The minimum value that can be represented by `Pixels`.
    pub const MIN: Pixels = Pixels(f32::MIN);

    /// Returns the raw `f32` value of this `Pixels`.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Rounds down to the nearest whole pixel.
    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    /// Rounds to the nearest whole pixel.
    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    /// Rounds up to the nearest whole pixel.
    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }

    /// Multiplies by `factor`, usually the display scale factor, producing `ScaledPixels`
    /// for rendering on displays with different pixel densities.
    #[must_use]
    pub fn scale(&self, factor: f32) -> ScaledPixels {
        ScaledPixels(self.0 * factor)
    }

    /// Raises the pixel value to `exponent`.
    pub fn pow(&self, exponent: f32) -> Self {
        Self(self.0.powf(exponent))
    }

    /// Returns the absolute pixel value.
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Returns `1.0` for positive values and `-1.0` for negative values.
    pub fn signum(&self) -> f32 {
        self.0.signum()
    }

    /// Returns the pixel value as an `f64`.
    pub fn to_f64(self) -> f64 {
        self.0 as f64
    }
}

impl Eq for Pixels {}

impl PartialOrd for Pixels {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pixels {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl std::hash::Hash for Pixels {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl From<f64> for Pixels {
    fn from(pixels: f64) -> Self {
        Pixels(pixels as f32)
    }
}

impl From<f32> for Pixels {
    fn from(pixels: f32) -> Self {
        Pixels(pixels)
    }
}

impl From<Pixels> for f32 {
    fn from(pixels: Pixels) -> Self {
        pixels.0
    }
}

impl From<&Pixels> for f32 {
    fn from(pixels: &Pixels) -> Self {
        pixels.0
    }
}

impl From<Pixels> for f64 {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as f64
    }
}

impl From<Pixels> for u32 {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as u32
    }
}

impl From<&Pixels> for u32 {
    fn from(pixels: &Pixels) -> Self {
        pixels.0 as u32
    }
}

impl From<u32> for Pixels {
    fn from(pixels: u32) -> Self {
        Pixels(pixels as f32)
    }
}

impl From<Pixels> for usize {
    fn from(pixels: Pixels) -> Self {
        pixels.0 as usize
    }
}

impl From<usize> for Pixels {
    fn from(pixels: usize) -> Self {
        Pixels(pixels as f32)
    }
}

/// A count of physical display pixels, independent of the logical scale factor.
#[derive(
    Add,
    AddAssign,
    Clone,
    Copy,
    Default,
    Div,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Sub,
    SubAssign,
    Serialize,
    Deserialize,
)]
#[repr(transparent)]
pub struct DevicePixels(pub i32);

impl DevicePixels {
    /// Returns the byte count for this many pixels at `bytes_per_pixel` bytes each.
    /// Use when sizing image or framebuffer storage.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::DevicePixels;
    /// let pixels = DevicePixels(10); // 10 device pixels
    /// let bytes_per_pixel = 4; // Assume each pixel is represented by 4 bytes (e.g., RGBA)
    /// let total_bytes = pixels.to_bytes(bytes_per_pixel);
    /// assert_eq!(total_bytes, 40); // 10 pixels * 4 bytes/pixel = 40 bytes
    /// ```
    pub fn to_bytes(self, bytes_per_pixel: u8) -> u32 {
        self.0 as u32 * bytes_per_pixel as u32
    }
}

impl fmt::Debug for DevicePixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} px (device)", self.0)
    }
}

impl From<DevicePixels> for i32 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0
    }
}

impl From<i32> for DevicePixels {
    fn from(device_pixels: i32) -> Self {
        DevicePixels(device_pixels)
    }
}

impl From<u32> for DevicePixels {
    fn from(device_pixels: u32) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

impl From<DevicePixels> for u32 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as u32
    }
}

impl From<DevicePixels> for u64 {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as u64
    }
}

impl From<u64> for DevicePixels {
    fn from(device_pixels: u64) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

impl From<DevicePixels> for usize {
    fn from(device_pixels: DevicePixels) -> Self {
        device_pixels.0 as usize
    }
}

impl From<usize> for DevicePixels {
    fn from(device_pixels: usize) -> Self {
        DevicePixels(device_pixels as i32)
    }
}

/// A pixel length multiplied by the device scale factor.
/// One logical pixel may span multiple physical pixels on a high-density display.
#[derive(Clone, Copy, Default, Add, AddAssign, Sub, SubAssign, Div, DivAssign, PartialEq)]
#[repr(transparent)]
pub struct ScaledPixels(pub f32);

impl ScaledPixels {
    /// Returns the raw `f32` value of this `ScaledPixels`.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Rounds down to the nearest whole scaled pixel.
    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    /// Rounds to the nearest whole scaled pixel.
    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    /// Rounds up to the nearest whole scaled pixel.
    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }
}

impl Eq for ScaledPixels {}

impl PartialOrd for ScaledPixels {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScaledPixels {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Debug for ScaledPixels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}px (scaled)", self.0)
    }
}

impl From<ScaledPixels> for DevicePixels {
    fn from(scaled: ScaledPixels) -> Self {
        DevicePixels(scaled.0.ceil() as i32)
    }
}

impl From<DevicePixels> for ScaledPixels {
    fn from(device: DevicePixels) -> Self {
        ScaledPixels(device.0 as f32)
    }
}

impl From<ScaledPixels> for f64 {
    fn from(scaled_pixels: ScaledPixels) -> Self {
        scaled_pixels.0 as f64
    }
}

impl From<ScaledPixels> for u32 {
    fn from(pixels: ScaledPixels) -> Self {
        pixels.0 as u32
    }
}

impl From<f32> for ScaledPixels {
    fn from(pixels: f32) -> Self {
        Self(pixels)
    }
}

impl Div for ScaledPixels {
    type Output = f32;

    fn div(self, rhs: Self) -> Self::Output {
        self.0 / rhs.0
    }
}

impl std::ops::DivAssign for ScaledPixels {
    fn div_assign(&mut self, rhs: Self) {
        *self = Self(self.0 / rhs.0);
    }
}

impl std::ops::RemAssign for ScaledPixels {
    fn rem_assign(&mut self, rhs: Self) {
        self.0 %= rhs.0;
    }
}

impl std::ops::Rem for ScaledPixels {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

impl Mul<f32> for ScaledPixels {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self {
        Self(self.0 * rhs)
    }
}

impl Mul<ScaledPixels> for f32 {
    type Output = ScaledPixels;

    fn mul(self, rhs: ScaledPixels) -> Self::Output {
        rhs * self
    }
}

impl Mul<usize> for ScaledPixels {
    type Output = Self;

    fn mul(self, rhs: usize) -> Self {
        self * (rhs as f32)
    }
}

impl Mul<ScaledPixels> for usize {
    type Output = ScaledPixels;

    fn mul(self, rhs: ScaledPixels) -> ScaledPixels {
        rhs * self
    }
}

impl MulAssign<f32> for ScaledPixels {
    fn mul_assign(&mut self, rhs: f32) {
        self.0 *= rhs;
    }
}

/// A length in units of the window font size, set with [`Window::set_rem_size`][set_rem_size].
///
/// For a font size of `16px`, `1rem` is `16px` and `2rem` is `32px`.
///
/// [set_rem_size]: crate::Window::set_rem_size
#[derive(Clone, Copy, Default, Add, Sub, Mul, Div, Neg, PartialEq)]
pub struct Rems(pub f32);

impl Rems {
    /// A length of zero.
    pub const ZERO: Self = Self(0.0);
    /// Convert this Rem value to pixels.
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        self * rem_size
    }
    /// Convert from pixels to Rem
    pub fn from_pixels(length: Pixels, window: &gpui::Window) -> Self {
        Self(length / window.rem_size())
    }
}

impl Mul<Pixels> for Rems {
    type Output = Pixels;

    fn mul(self, other: Pixels) -> Pixels {
        Pixels(self.0 * other.0)
    }
}

impl AddAssign<Rems> for Rems {
    fn add_assign(&mut self, rhs: Rems) {
        self.0 += rhs.0
    }
}

impl Display for Rems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}rem", self.0)
    }
}

impl Debug for Rems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl TryFrom<&'_ str> for Rems {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value
            .strip_suffix("rem")
            .context("expected 'rem' suffix")
            .and_then(|number| Ok(number.parse()?))
            .map(Self)
    }
}

/// A length stored as a fraction of another length.
///
/// `1.0` uses the full reference length, and `0.5` uses half. Fractions may be negative or greater
/// than `1.0`.
///
/// # Examples
///
/// ```
/// use gpui::{AbsoluteLength, Relative, px};
///
/// let half = Relative::from(0.5);
/// let base_size = AbsoluteLength::Pixels(px(200.0));
///
/// assert_eq!(half.to_pixels(base_size, px(16.0)), px(100.0));
/// assert_eq!(half.to_string(), "50%");
/// ```
#[derive(Clone, Copy, Default, Add, Sub, Mul, Div, Neg, PartialEq)]
#[repr(transparent)]
pub struct Relative(pub(crate) f32);

impl Relative {
    /// A relative length of zero.
    pub const ZERO: Self = Self(0.0);

    /// Returns the raw `f32` value of this `Relative`.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Resolves this fraction against `base_size`.
    ///
    /// When `base_size` is in rems, `rem_size` supplies the pixel size of one rem.
    pub fn to_pixels(self, base_size: AbsoluteLength, rem_size: Pixels) -> Pixels {
        match base_size {
            AbsoluteLength::Pixels(pixels) => pixels * self.0,
            AbsoluteLength::Rems(rems) => rems * rem_size * self.0,
        }
    }
}

impl From<f32> for Relative {
    fn from(fraction: f32) -> Self {
        Self(fraction)
    }
}

impl AddAssign<Relative> for Relative {
    fn add_assign(&mut self, rhs: Relative) {
        self.0 += rhs.0;
    }
}

impl Display for Relative {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0 * 100.0)
    }
}

impl Debug for Relative {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl TryFrom<&'_ str> for Relative {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        value
            .strip_suffix('%')
            .context("expected '%' suffix")
            .and_then(|number| Ok(number.parse::<f32>()?))
            .map(|percentage| Self(percentage / 100.0))
    }
}

/// An absolute length in pixels or rems. Pixels do not depend on font size,
/// while rems scale with the root font size.
#[derive(Clone, Copy, Neg, PartialEq)]
pub enum AbsoluteLength {
    /// A length in pixels.
    Pixels(Pixels),
    /// A length in rems.
    Rems(Rems),
}

impl AbsoluteLength {
    /// Checks if the absolute length is zero.
    pub fn is_zero(&self) -> bool {
        match self {
            AbsoluteLength::Pixels(px) => px.0 == 0.0,
            AbsoluteLength::Rems(rems) => rems.0 == 0.0,
        }
    }
}

impl From<Pixels> for AbsoluteLength {
    fn from(pixels: Pixels) -> Self {
        AbsoluteLength::Pixels(pixels)
    }
}

impl From<Rems> for AbsoluteLength {
    fn from(rems: Rems) -> Self {
        AbsoluteLength::Rems(rems)
    }
}

impl AbsoluteLength {
    /// Converts to pixels, using `rem_size` as the pixel size of one rem.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AbsoluteLength, Pixels, Rems};
    /// let length_in_pixels = AbsoluteLength::Pixels(Pixels::from(42.0));
    /// let length_in_rems = AbsoluteLength::Rems(Rems(2.0));
    /// let rem_size = Pixels::from(16.0);
    ///
    /// assert_eq!(length_in_pixels.to_pixels(rem_size), Pixels::from(42.0));
    /// assert_eq!(length_in_rems.to_pixels(rem_size), Pixels::from(32.0));
    /// ```
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        match self {
            AbsoluteLength::Pixels(pixels) => pixels,
            AbsoluteLength::Rems(rems) => rems.to_pixels(rem_size),
        }
    }

    /// Converts to rems, using `rem_size` as the pixel size of one rem.
    pub fn to_rems(self, rem_size: Pixels) -> Rems {
        match self {
            AbsoluteLength::Pixels(pixels) => Rems(pixels.0 / rem_size.0),
            AbsoluteLength::Rems(rems) => rems,
        }
    }
}

impl Default for AbsoluteLength {
    fn default() -> Self {
        px(0.).into()
    }
}

impl Display for AbsoluteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pixels(pixels) => write!(f, "{pixels}"),
            Self::Rems(rems) => write!(f, "{rems}"),
        }
    }
}

impl Debug for AbsoluteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

const EXPECTED_ABSOLUTE_LENGTH: &str = "number with 'px' or 'rem' suffix";

impl TryFrom<&'_ str> for AbsoluteLength {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if let Ok(pixels) = value.try_into() {
            Ok(Self::Pixels(pixels))
        } else if let Ok(rems) = value.try_into() {
            Ok(Self::Rems(rems))
        } else {
            Err(anyhow!(
                "invalid AbsoluteLength '{value}', expected {EXPECTED_ABSOLUTE_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for AbsoluteLength {
    fn schema_name() -> Cow<'static, str> {
        "AbsoluteLength".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^-?\d+(\.\d+)?(px|rem)$"
        })
    }
}

impl<'de> Deserialize<'de> for AbsoluteLength {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = AbsoluteLength;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_ABSOLUTE_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                AbsoluteLength::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for AbsoluteLength {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

/// A non-auto length in pixels, rems, or a fraction of the parent size.
#[derive(Clone, Copy, Neg, PartialEq)]
pub enum DefiniteLength {
    /// An absolute length specified in pixels or rems.
    Absolute(AbsoluteLength),
    /// A length relative to the parent's size.
    Relative(Relative),
}

impl DefiniteLength {
    /// Converts to pixels, multiplying fractions by `base_size` and using `rem_size`
    /// as the pixel size of one rem.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{DefiniteLength, AbsoluteLength, Pixels, px, relative, rems};
    /// let length_in_pixels = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(42.0)));
    /// let length_in_rems = DefiniteLength::Absolute(AbsoluteLength::Rems(rems(2.0)));
    /// let length_as_fraction = DefiniteLength::from(relative(0.5));
    /// let base_size = AbsoluteLength::Pixels(px(100.0));
    /// let rem_size = px(16.0);
    ///
    /// assert_eq!(length_in_pixels.to_pixels(base_size, rem_size), Pixels::from(42.0));
    /// assert_eq!(length_in_rems.to_pixels(base_size, rem_size), Pixels::from(32.0));
    /// assert_eq!(length_as_fraction.to_pixels(base_size, rem_size), Pixels::from(50.0));
    /// ```
    pub fn to_pixels(self, base_size: AbsoluteLength, rem_size: Pixels) -> Pixels {
        match self {
            DefiniteLength::Absolute(size) => size.to_pixels(rem_size),
            DefiniteLength::Relative(relative) => relative.to_pixels(base_size, rem_size),
        }
    }
}

impl Debug for DefiniteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for DefiniteLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefiniteLength::Absolute(length) => write!(f, "{length}"),
            DefiniteLength::Relative(relative) => write!(f, "{relative}"),
        }
    }
}

const EXPECTED_DEFINITE_LENGTH: &str = "expected number with 'px', 'rem', or '%' suffix";

impl TryFrom<&'_ str> for DefiniteLength {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if value.ends_with('%') {
            let relative = Relative::try_from(value).with_context(|| {
                format!("invalid DefiniteLength '{value}', expected {EXPECTED_DEFINITE_LENGTH}")
            })?;
            Ok(DefiniteLength::Relative(relative))
        } else if let Ok(absolute_length) = value.try_into() {
            Ok(DefiniteLength::Absolute(absolute_length))
        } else {
            Err(anyhow!(
                "invalid DefiniteLength '{value}', expected {EXPECTED_DEFINITE_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for DefiniteLength {
    fn schema_name() -> Cow<'static, str> {
        "DefiniteLength".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^-?\d+(\.\d+)?(px|rem|%)$"
        })
    }
}

impl<'de> Deserialize<'de> for DefiniteLength {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = DefiniteLength;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_DEFINITE_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                DefiniteLength::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for DefiniteLength {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

impl From<Pixels> for DefiniteLength {
    fn from(pixels: Pixels) -> Self {
        Self::Absolute(pixels.into())
    }
}

impl From<Rems> for DefiniteLength {
    fn from(rems: Rems) -> Self {
        Self::Absolute(rems.into())
    }
}

impl From<Relative> for DefiniteLength {
    fn from(relative: Relative) -> Self {
        Self::Relative(relative)
    }
}

impl From<AbsoluteLength> for DefiniteLength {
    fn from(length: AbsoluteLength) -> Self {
        Self::Absolute(length)
    }
}

impl Default for DefiniteLength {
    fn default() -> Self {
        Self::Absolute(AbsoluteLength::default())
    }
}

/// A length that can be defined in pixels, rems, percent of parent, or auto.
#[derive(Clone, Copy, PartialEq)]
pub enum Length {
    /// A definite length specified either in pixels, rems, or as a fraction of the parent's size.
    Definite(DefiniteLength),
    /// An automatic length that is determined by the context in which it is used.
    Auto,
}

impl Debug for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Length::Definite(definite_length) => write!(f, "{}", definite_length),
            Length::Auto => write!(f, "auto"),
        }
    }
}

const EXPECTED_LENGTH: &str = "expected 'auto' or number with 'px', 'rem', or '%' suffix";

impl TryFrom<&'_ str> for Length {
    type Error = anyhow::Error;

    fn try_from(value: &'_ str) -> Result<Self, Self::Error> {
        if value == "auto" {
            Ok(Length::Auto)
        } else if let Ok(definite_length) = value.try_into() {
            Ok(Length::Definite(definite_length))
        } else {
            Err(anyhow!(
                "invalid Length '{value}', expected {EXPECTED_LENGTH}"
            ))
        }
    }
}

impl JsonSchema for Length {
    fn schema_name() -> Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^(auto|-?\d+(\.\d+)?(px|rem|%))$"
        })
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringVisitor;

        impl de::Visitor<'_> for StringVisitor {
            type Value = Length;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{EXPECTED_LENGTH}")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Length::try_from(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(StringVisitor)
    }
}

impl Serialize for Length {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{self}"))
    }
}

/// Creates a [`Relative`] length, convertible to [`DefiniteLength`] or [`Length`].
///
/// A fraction of `1.0` uses the full reference length. Values may be negative or greater than `1.0`.
pub const fn relative(fraction: f32) -> Relative {
    Relative(fraction)
}

/// Returns the Golden Ratio, i.e. `~(1.0 + sqrt(5.0)) / 2.0`.
pub const fn phi() -> Relative {
    relative(GOLDEN_RATIO)
}

/// Creates a length of `rems` rems.
pub const fn rems(rems: f32) -> Rems {
    Rems(rems)
}

/// Creates a length of `pixels` logical pixels.
pub const fn px(pixels: f32) -> Pixels {
    Pixels(pixels)
}

/// Returns `Length::Auto`, letting layout determine the length.
///
/// Used when dimensions should follow the container size or intrinsic content size
/// rather than an explicit length.
pub const fn auto() -> Length {
    Length::Auto
}

impl From<Pixels> for Length {
    fn from(pixels: Pixels) -> Self {
        Self::Definite(pixels.into())
    }
}

impl From<Rems> for Length {
    fn from(rems: Rems) -> Self {
        Self::Definite(rems.into())
    }
}

impl From<Relative> for Length {
    fn from(relative: Relative) -> Self {
        Self::Definite(relative.into())
    }
}

impl From<DefiniteLength> for Length {
    fn from(length: DefiniteLength) -> Self {
        Self::Definite(length)
    }
}

impl From<AbsoluteLength> for Length {
    fn from(length: AbsoluteLength) -> Self {
        Self::Definite(length.into())
    }
}

impl Default for Length {
    fn default() -> Self {
        Self::Definite(DefiniteLength::default())
    }
}

impl From<()> for Length {
    fn from(_: ()) -> Self {
        Self::Definite(DefiniteLength::default())
    }
}

/// A location in a grid layout.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub struct GridLocation {
    /// The rows this item uses within the grid.
    pub row: Range<GridPlacement>,
    /// The columns this item uses within the grid.
    pub column: Range<GridPlacement>,
}

/// The placement of an item within a grid layout's column or row.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub enum GridPlacement {
    /// The grid line index to place this item.
    Line(i16),
    /// The number of grid lines to span.
    Span(u16),
    /// Automatically determine the placement, equivalent to Span(1)
    #[default]
    Auto,
}

impl From<GridPlacement> for taffy::GridPlacement {
    fn from(placement: GridPlacement) -> Self {
        match placement {
            GridPlacement::Line(index) => taffy::GridPlacement::from_line_index(index),
            GridPlacement::Span(span) => taffy::GridPlacement::from_span(span),
            GridPlacement::Auto => taffy::GridPlacement::Auto,
        }
    }
}

/// Types that can return half their value.
pub trait Half {
    /// Returns half the current value.
    fn half(&self) -> Self;
}

impl Half for i32 {
    fn half(&self) -> Self {
        self / 2
    }
}

impl Half for f32 {
    fn half(&self) -> Self {
        self / 2.
    }
}

impl Half for DevicePixels {
    fn half(&self) -> Self {
        Self(self.0 / 2)
    }
}

impl Half for ScaledPixels {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

impl Half for Pixels {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

impl Half for Rems {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

impl Half for Relative {
    fn half(&self) -> Self {
        Self(self.0 / 2.)
    }
}

/// Types that can be checked for zero.
pub trait IsZero {
    /// Returns whether the value is zero.
    fn is_zero(&self) -> bool;
}

impl IsZero for DevicePixels {
    fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl IsZero for ScaledPixels {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for Pixels {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for Rems {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for Relative {
    fn is_zero(&self) -> bool {
        self.0 == 0.
    }
}

impl IsZero for AbsoluteLength {
    fn is_zero(&self) -> bool {
        match self {
            AbsoluteLength::Pixels(pixels) => pixels.is_zero(),
            AbsoluteLength::Rems(rems) => rems.is_zero(),
        }
    }
}

impl IsZero for DefiniteLength {
    fn is_zero(&self) -> bool {
        match self {
            DefiniteLength::Absolute(length) => length.is_zero(),
            DefiniteLength::Relative(relative) => relative.is_zero(),
        }
    }
}

impl IsZero for Length {
    fn is_zero(&self) -> bool {
        match self {
            Length::Definite(length) => length.is_zero(),
            Length::Auto => false,
        }
    }
}

impl<T: IsZero + Clone + Debug + Default + PartialEq> IsZero for Point<T> {
    fn is_zero(&self) -> bool {
        self.x.is_zero() && self.y.is_zero()
    }
}

impl<T> IsZero for Size<T>
where
    T: IsZero + Clone + Debug + Default + PartialEq,
{
    fn is_zero(&self) -> bool {
        self.width.is_zero() || self.height.is_zero()
    }
}

impl<T: IsZero + Clone + Debug + Default + PartialEq> IsZero for Bounds<T> {
    fn is_zero(&self) -> bool {
        self.size.is_zero()
    }
}

impl<T> IsZero for Corners<T>
where
    T: IsZero + Clone + Debug + Default + PartialEq,
{
    fn is_zero(&self) -> bool {
        self.top_left.is_zero()
            && self.top_right.is_zero()
            && self.bottom_right.is_zero()
            && self.bottom_left.is_zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounds_intersects() {
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 4.0, y: 4.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 5.0,
                height: 5.0,
            },
        };

        // Test Case 1: Intersecting bounds
        assert!(bounds1.intersects(&bounds2));

        // Test Case 2: Non-Intersecting bounds
        assert!(!bounds1.intersects(&bounds3));

        // Test Case 3: Bounds intersecting with themselves
        assert!(bounds1.intersects(&bounds1));
    }
}
