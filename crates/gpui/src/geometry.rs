//! Value types for positions, sizes, bounds, lengths and pixel units.
//!
//! The public items are re-exported from the crate root. [`Point`] explains the coordinate
//! system, and [`Pixels`] explains the difference between logical and device pixels.

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

/// An axis of two-dimensional space.
///
/// Use an axis with [`Along`] to read or update a [`Point`] or [`Size`] along one direction,
/// without choosing between `x` and `width` by hand.
///
/// # Examples
///
/// ```
/// use gpui::{Along, Axis, px, size};
///
/// let size = size(px(100.), px(40.));
/// assert_eq!(size.along(Axis::Horizontal), px(100.));
/// assert_eq!(size.along(Axis::Vertical), px(40.));
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Axis {
    /// The vertical axis, running up and down. `y` and `height` belong to it.
    Vertical,
    /// The horizontal axis, running left and right. `x` and `width` belong to it.
    Horizontal,
}

impl Axis {
    /// Returns the other axis.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Axis;
    /// assert_eq!(Axis::Vertical.invert(), Axis::Horizontal);
    /// assert_eq!(Axis::Horizontal.invert(), Axis::Vertical);
    /// ```
    pub fn invert(self) -> Self {
        match self {
            Axis::Vertical => Axis::Horizontal,
            Axis::Horizontal => Axis::Vertical,
        }
    }
}

/// Reads and updates the value along one [`Axis`].
///
/// [`Point`] and [`Size`] implement this trait. The horizontal axis is `x` for a point and
/// `width` for a size. The vertical axis is `y` and `height`.
///
/// # Examples
///
/// ```
/// use gpui::{Along, Axis, point, px};
///
/// let mut position = point(px(10.), px(20.));
/// position = position.apply_along(Axis::Vertical, |y| y + px(5.));
/// assert_eq!(position.along(Axis::Vertical), px(25.));
/// ```
pub trait Along {
    /// The type of the value along each axis.
    type Unit;

    /// Returns the value along `axis`.
    fn along(&self, axis: Axis) -> Self::Unit;

    /// Returns a copy with `f` applied to the value along `axis`. The other axis is unchanged.
    fn apply_along(&self, axis: Axis, f: impl FnOnce(Self::Unit) -> Self::Unit) -> Self;
}

/// A position or offset in two-dimensional space.
///
/// GPUI's coordinate system has its origin at the top left corner of the window. The `x` axis
/// increases to the right and the `y` axis increases downward, so a larger `y` is lower on the
/// screen. Coordinates are in logical pixels: `px(1.)` is one logical pixel, which the platform
/// scales to device pixels for the display. Use [`Pixels`] for positions that the UI lays out.
///
/// `Point<T>` works with any `T` that is `Clone`, `Debug`, `Default` and `PartialEq`. The
/// arithmetic and ordering methods also need `T` to support the matching operators.
///
/// [`Bounds`] uses points for its origin. For how positions map onto the layout tree, see
/// [layout and style](crate::_documentation::concepts::layout_and_style).
///
/// # Examples
///
/// ```
/// use gpui::{Point, point, px};
///
/// let card = point(px(20.), px(30.));
/// let offset = point(px(5.), px(-10.));
/// assert_eq!(card + offset, point(px(25.), px(20.)));
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
    /// The horizontal coordinate, increasing to the right.
    pub x: T,
    /// The vertical coordinate, increasing downward.
    pub y: T,
}

/// Creates a [`Point`] from its `x` and `y` coordinates.
///
/// Same as [`Point::new`].
///
/// # Examples
///
/// ```
/// use gpui::point;
///
/// let p = point(10, 20);
/// assert_eq!(p.x, 10);
/// assert_eq!(p.y, 20);
/// ```
pub const fn point<T: Clone + Debug + Default + PartialEq>(x: T, y: T) -> Point<T> {
    Point { x, y }
}

impl<T: Clone + Debug + Default + PartialEq> Point<T> {
    /// Creates a point from its `x` and `y` coordinates. Same as [`point`].
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::Point;
    ///
    /// let p = Point::new(10, 20);
    /// assert_eq!(p.x, 10);
    /// assert_eq!(p.y, 20);
    /// ```
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Returns a new point with `f` applied to both coordinates.
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
    /// Multiplies both coordinates by `factor`, producing [`ScaledPixels`].
    ///
    /// Pass the display's scale factor, such as `2.0` on a Retina display, to convert a logical
    /// position into the scaled space that GPUI draws in.
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

    /// Returns the distance from `(0, 0)` to this point.
    ///
    /// The result is an `f64`, not [`Pixels`].
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

    /// Returns `true` if both coordinates differ from `other` by less than `epsilon`.
    ///
    /// Use this to compare positions computed with floating point arithmetic. The comparison is
    /// strict: a difference of exactly `epsilon` returns `false`.
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
    /// Returns this point's position relative to `origin`.
    ///
    /// Subtracts `origin` from both coordinates. [`Bounds::localize`] uses it to convert a point in
    /// window coordinates into a bounds rectangle's local coordinates.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Point, point, px};
    /// let pointer = point(px(150.), px(90.));
    /// let element_origin = point(px(100.), px(80.));
    /// assert_eq!(pointer.relative_to(&element_origin), point(px(50.), px(10.)));
    /// ```
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
    /// Returns a point with the larger `x` and the larger `y` of `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p1 = Point { x: 3, y: 7 };
    /// let p2 = Point { x: 5, y: 2 };
    /// assert_eq!(p1.max(&p2), Point { x: 5, y: 7 });
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

    /// Returns a point with the smaller `x` and the smaller `y` of `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p1 = Point { x: 3, y: 7 };
    /// let p2 = Point { x: 5, y: 2 };
    /// assert_eq!(p1.min(&p2), Point { x: 3, y: 2 });
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

    /// Limits each coordinate to the range from `min` to `max`.
    ///
    /// A coordinate below `min` becomes `min`, and a coordinate above `max` becomes `max`. If `min`
    /// is greater than `max` in a coordinate, that coordinate becomes `max`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Point;
    /// let p = Point { x: 10, y: 20 };
    /// let min = Point { x: 0, y: 5 };
    /// let max = Point { x: 15, y: 25 };
    /// assert_eq!(p.clamp(&min, &max), Point { x: 10, y: 20 });
    ///
    /// let p_out_of_bounds = Point { x: -5, y: 30 };
    /// assert_eq!(p_out_of_bounds.clamp(&min, &max), Point { x: 0, y: 25 });
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

/// A width and height, such as the size of a window or an element.
///
/// `Size<Pixels>` holds layout sizes in logical pixels. `Size<Length>` holds style values, which
/// can also be `auto` or a fraction of the parent. `Size<DevicePixels>` holds whole physical
/// pixels, such as the size of an image. Construct a size with [`size`].
///
/// # Examples
///
/// ```
/// use gpui::{px, size};
///
/// let window = size(px(800.), px(600.));
/// assert_eq!(window.width, px(800.));
/// assert_eq!(window.height, px(600.));
/// ```
#[derive(
    Add, Clone, Copy, Default, Deserialize, Div, Hash, Neg, PartialEq, Refineable, Serialize, Sub,
)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Size<T: Clone + Debug + Default + PartialEq> {
    /// The width, along the horizontal axis.
    pub width: T,
    /// The height, along the vertical axis.
    pub height: T,
}

impl<T: Clone + Debug + Default + PartialEq> Size<T> {
    /// Creates a size from its width and height. Same as [`size`].
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::Size;
    ///
    /// let s = Size::new(10, 20);
    /// assert_eq!(s.width, 10);
    /// assert_eq!(s.height, 20);
    /// ```
    pub fn new(width: T, height: T) -> Self {
        size(width, height)
    }
}

/// Creates a [`Size`] from its width and height.
///
/// Same as [`Size::new`].
///
/// # Examples
///
/// ```
/// use gpui::size;
///
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
    /// Returns a new size with `f` applied to the width and the height.
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
    /// Returns the point at the center of a box of this size, measured from `(0, 0)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{point, px, size};
    /// assert_eq!(size(px(100.), px(40.)).center(), point(px(50.), px(20.)));
    /// ```
    pub fn center(&self) -> Point<T> {
        Point {
            x: self.width.half(),
            y: self.height.half(),
        }
    }

    /// Returns the point at `anchor` on the edge or corner of a box of this size.
    ///
    /// The box's top left corner is `(0, 0)`, so `Anchor::BottomRight` returns the size itself.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Anchor, point, px, size};
    /// let s = size(px(100.), px(40.));
    /// assert_eq!(s.at_anchor(Anchor::BottomCenter), point(px(50.), px(40.)));
    /// assert_eq!(s.at_anchor(Anchor::BottomRight), point(px(100.), px(40.)));
    /// ```
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
    /// Multiplies the width and height by `factor`, producing [`ScaledPixels`].
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
    /// Returns a size with the larger width and the larger height of `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Size;
    /// let size1 = Size { width: 30, height: 40 };
    /// let size2 = Size { width: 50, height: 20 };
    /// assert_eq!(size1.max(&size2), Size { width: 50, height: 40 });
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

    /// Returns a size with the smaller width and the smaller height of `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Size;
    /// let size1 = Size { width: 30, height: 40 };
    /// let size2 = Size { width: 50, height: 20 };
    /// assert_eq!(size1.min(&size2), Size { width: 30, height: 20 });
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
    /// Returns a size whose width and height are both `relative(1.)`, filling the parent.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{DefiniteLength, Length, Size, relative};
    /// let full = Size::<Length>::full();
    /// assert_eq!(full.width, Length::Definite(DefiniteLength::from(relative(1.))));
    /// assert_eq!(full.height, Length::Definite(DefiniteLength::from(relative(1.))));
    /// ```
    pub fn full() -> Self {
        Self {
            width: relative(1.).into(),
            height: relative(1.).into(),
        }
    }
}

impl Size<Length> {
    /// Returns a size whose width and height are both [`Length::Auto`].
    ///
    /// With `auto` in both dimensions, layout sizes the element from its content.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Length, Size};
    /// let auto = Size::<Length>::auto();
    /// assert_eq!(auto.width, Length::Auto);
    /// assert_eq!(auto.height, Length::Auto);
    /// ```
    pub fn auto() -> Self {
        Self {
            width: Length::Auto,
            height: Length::Auto,
        }
    }
}

/// A rectangle, defined by its top left corner and its size.
///
/// The rectangle covers the area from [`origin`](Bounds::origin) to the bottom right corner.
/// [`Bounds::contains`] treats the left and top edges as inside and the right and bottom edges
/// as outside, so adjacent bounds don't overlap. Elements receive their bounds in logical pixels
/// when they are laid out and painted. See [layout and style](crate::_documentation::concepts::layout_and_style).
///
/// # Examples
///
/// ```
/// use gpui::{Bounds, Point, Size};
///
/// let bounds = Bounds::new(Point { x: 10, y: 20 }, Size { width: 30, height: 40 });
/// assert_eq!(bounds.right(), 40);
/// assert_eq!(bounds.bottom(), 60);
/// ```
#[derive(Refineable, Copy, Clone, Default, Debug, Eq, PartialEq, Serialize, Deserialize, Hash)]
#[refineable(Debug)]
#[repr(C)]
pub struct Bounds<T: Clone + Debug + Default + PartialEq> {
    /// The top left corner of the rectangle.
    pub origin: Point<T>,
    /// The width and height of the rectangle.
    pub size: Size<T>,
}

/// Creates [`Bounds`] from an origin and a size.
///
/// Same as [`Bounds::new`].
///
/// # Examples
///
/// ```
/// use gpui::{bounds, point, px, size};
///
/// let b = bounds(point(px(10.), px(10.)), size(px(100.), px(50.)));
/// assert_eq!(b.right(), px(110.));
/// assert_eq!(b.bottom(), px(60.));
/// ```
pub fn bounds<T: Clone + Debug + Default + PartialEq>(
    origin: Point<T>,
    size: Size<T>,
) -> Bounds<T> {
    Bounds { origin, size }
}

impl Bounds<Pixels> {
    /// Returns bounds of `size` centered on a display.
    ///
    /// Uses the display with `display_id`, or the primary display if `display_id` is `None` or
    /// doesn't match a connected display. If there is no display at all, the bounds start at
    /// `(0, 0)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, TestApp, bounds, point, px, size};
    /// # let app = TestApp::new();
    /// app.read(|cx| {
    ///     // The test platform has one 1920 by 1080 display.
    ///     let window = Bounds::centered(None, size(px(800.), px(600.)), cx);
    ///     assert_eq!(window, bounds(point(px(560.), px(240.)), size(px(800.), px(600.))));
    /// });
    /// ```
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

    /// Returns bounds that cover a display.
    ///
    /// Uses the display with `display_id`, or the primary display if `display_id` is `None` or
    /// doesn't match a connected display. If there is no display at all, returns 1024 by 768
    /// pixels at `(0, 0)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, TestApp, bounds, point, px, size};
    /// # let app = TestApp::new();
    /// app.read(|cx| {
    ///     // The test platform has one 1920 by 1080 display.
    ///     let screen = Bounds::maximized(None, cx);
    ///     assert_eq!(screen, bounds(point(px(0.), px(0.)), size(px(1920.), px(1080.))));
    /// });
    /// ```
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
    /// Creates bounds from an origin and a size. Same as [`bounds`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let b = Bounds::new(Point { x: 0, y: 0 }, Size { width: 10, height: 20 });
    /// assert_eq!(b.size.height, 20);
    /// ```
    pub fn new(origin: Point<T>, size: Size<T>) -> Self {
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Creates bounds from its top left and bottom right corners.
    ///
    /// The size is `bottom_right - top_left` and is not clamped. A `bottom_right` above or to the
    /// left of `top_left` gives a negative width or height, which [`Bounds::is_empty`] reports as
    /// empty.
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
    /// Creates bounds of `size` whose `corner` sits at `origin`.
    ///
    /// Use this to place a box by one of its corners or edge centers: the box's top left corner is
    /// `origin` minus the offset of `corner` within the size.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Anchor, Bounds, point, px, size};
    /// let b = Bounds::from_anchor_and_size(
    ///     Anchor::BottomRight,
    ///     point(px(10.), px(10.)),
    ///     size(px(4.), px(2.)),
    /// );
    /// assert_eq!(b.origin, point(px(6.), px(8.)));
    /// assert_eq!(b.bottom_right(), point(px(10.), px(10.)));
    /// ```
    pub fn from_anchor_and_size(corner: Anchor, origin: Point<T>, size: Size<T>) -> Bounds<T> {
        let origin = origin - size.at_anchor(corner);
        Bounds { origin, size }
    }
}

impl<T> Bounds<T>
where
    T: Sub<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Creates bounds of `size` centered on `center`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, point, px, size};
    /// let b = Bounds::centered_at(point(px(50.), px(50.)), size(px(20.), px(10.)));
    /// assert_eq!(b.origin, point(px(40.), px(45.)));
    /// assert_eq!(b.center(), point(px(50.), px(50.)));
    /// ```
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
    /// Returns the midpoint of the top edge.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.top_center(), Point { x: 5, y: 0 });
    /// ```
    pub fn top_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone(),
        }
    }

    /// Returns the midpoint of the bottom edge.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.bottom_center(), Point { x: 5, y: 20 });
    /// ```
    pub fn bottom_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.half(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    /// Returns the midpoint of the left edge.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.left_center(), Point { x: 0, y: 10 });
    /// ```
    pub fn left_center(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone(),
            y: self.origin.y.clone() + self.size.height.half(),
        }
    }

    /// Returns the midpoint of the right edge.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.right_center(), Point { x: 10, y: 10 });
    /// ```
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
    /// Returns `true` if this rectangle and `other` share some area.
    ///
    /// Bounds that only touch along an edge don't intersect.
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
    /// assert_eq!(bounds1.intersects(&bounds2), true);
    /// assert_eq!(bounds1.intersects(&bounds3), false);
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
    /// Returns the center point of the bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.center(), Point { x: 5, y: 10 });
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
    /// Returns the width plus the height.
    ///
    /// The perimeter of the bounds is twice this value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.half_perimeter(), 30);
    /// ```
    pub fn half_perimeter(&self) -> T {
        self.size.width.clone() + self.size.height.clone()
    }
}

impl<T> Bounds<T>
where
    T: Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq,
{
    /// Returns the bounds grown by `amount` on every side.
    ///
    /// The origin moves by `-amount` and the width and height grow by twice `amount`, so the
    /// center stays in place. A negative `amount` shrinks the bounds. Use [`Bounds::inset`] to
    /// shrink by a positive amount.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 10, y: 10 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// assert_eq!(bounds.dilate(5), Bounds {
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

    /// Returns the bounds grown by a different amount on each side.
    ///
    /// Each side of `amount` moves the matching edge outward: `top` grows upward, `right` grows to
    /// the right, and so on.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Edges, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 10, y: 10 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let amount = Edges { top: 1, right: 2, bottom: 3, left: 4 };
    /// assert_eq!(bounds.extend(amount), Bounds {
    ///     origin: Point { x: 6, y: 9 },
    ///     size: Size { width: 16, height: 14 },
    /// });
    /// ```
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
    /// Returns the bounds shrunk by `amount` on every side.
    ///
    /// Same as [`Bounds::dilate`] with `amount` negated.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0.0, y: 0.0 },
    ///     size: Size { width: 10.0, height: 10.0 },
    /// };
    /// assert_eq!(bounds.inset(2.0), Bounds {
    ///     origin: Point { x: 2.0, y: 2.0 },
    ///     size: Size { width: 6.0, height: 6.0 },
    /// });
    /// ```
    pub fn inset(&self, amount: T) -> Self {
        self.dilate(-amount)
    }
}

impl<T: PartialOrd + Add<T, Output = T> + Sub<Output = T> + Clone + Debug + Default + PartialEq>
    Bounds<T>
{
    /// Returns the area that this rectangle and `other` share.
    ///
    /// If the bounds don't overlap, the result has zero width or zero height. Its origin is then
    /// the larger of the two origins in each coordinate, which may lie outside both bounds.
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
    /// assert_eq!(bounds1.intersect(&bounds2), Bounds {
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

    /// Returns the smallest bounds that contain both rectangles.
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
    /// assert_eq!(bounds1.union(&bounds2), Bounds {
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
    /// Returns the distance from each edge of this rectangle to the matching edge of `outer`.
    ///
    /// A positive value means this rectangle lies inside `outer` on that side. A negative value means
    /// it extends past `outer`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Edges, Point, Size};
    /// let outer = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 100, height: 100 },
    /// };
    /// let inner = Bounds {
    ///     origin: Point { x: 10, y: 20 },
    ///     size: Size { width: 30, height: 40 },
    /// };
    /// assert_eq!(inner.space_within(&outer), Edges { top: 20, right: 60, bottom: 40, left: 10 });
    /// ```
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
    /// Returns the y coordinate of the top edge.
    pub fn top(&self) -> T {
        self.origin.y.clone()
    }

    /// Returns the y coordinate of the bottom edge, which is outside the bounds.
    pub fn bottom(&self) -> T {
        self.origin.y.clone() + self.size.height.clone()
    }

    /// Returns the x coordinate of the left edge.
    pub fn left(&self) -> T {
        self.origin.x.clone()
    }

    /// Returns the x coordinate of the right edge, which is outside the bounds.
    pub fn right(&self) -> T {
        self.origin.x.clone() + self.size.width.clone()
    }

    /// Returns the top right corner point of the bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.top_right(), Point { x: 10, y: 0 });
    /// ```
    pub fn top_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone(),
        }
    }

    /// Returns the bottom right corner point of the bounds.
    ///
    /// This point is outside the bounds: [`Bounds::contains`] returns `false` for it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.bottom_right(), Point { x: 10, y: 20 });
    /// ```
    pub fn bottom_right(&self) -> Point<T> {
        Point {
            x: self.origin.x.clone() + self.size.width.clone(),
            y: self.origin.y.clone() + self.size.height.clone(),
        }
    }

    /// Returns the bottom left corner point of the bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.bottom_left(), Point { x: 0, y: 20 });
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
    /// Returns the point at `corner` on this rectangle.
    ///
    /// Accepts the four corners and the four edge midpoints.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{Anchor, Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 20 },
    /// };
    /// assert_eq!(bounds.corner(Anchor::BottomLeft), Point { x: 0, y: 20 });
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
    /// Returns `true` if `point` lies inside the bounds.
    ///
    /// The left and top edges are inside, and the right and bottom edges are outside. A point at
    /// `(right, y)` or `(x, bottom)` returns `false`. With integer coordinates, a rectangle of
    /// width 10 at `x = 0` contains `x = 0` through `x = 9` only.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let bounds = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// assert!(bounds.contains(&Point { x: 5, y: 5 }));
    /// assert!(bounds.contains(&Point { x: 0, y: 0 }));
    /// assert!(!bounds.contains(&Point { x: 10, y: 5 }));
    /// assert!(!bounds.contains(&Point { x: 15, y: 15 }));
    /// ```
    pub fn contains(&self, point: &Point<T>) -> bool {
        point.x >= self.origin.x
            && point.x < self.origin.x.clone() + self.size.width.clone()
            && point.y >= self.origin.y
            && point.y < self.origin.y.clone() + self.size.height.clone()
    }

    /// Returns `true` if this rectangle lies entirely inside `other`.
    ///
    /// Checks that the top left and bottom right corners of this rectangle are inside `other`, using
    /// the rules of [`Bounds::contains`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let outer = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 20, height: 20 },
    /// };
    /// let inner = Bounds {
    ///     origin: Point { x: 5, y: 5 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    /// let overlapping = Bounds {
    ///     origin: Point { x: 15, y: 15 },
    ///     size: Size { width: 10, height: 10 },
    /// };
    ///
    /// assert!(inner.is_contained_within(&outer));
    /// assert!(!overlapping.is_contained_within(&outer));
    /// ```
    pub fn is_contained_within(&self, other: &Self) -> bool {
        other.contains(&self.origin) && other.contains(&self.bottom_right())
    }

    /// Returns bounds with `f` applied to the origin and the size, converting `T` to `U`.
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

    /// Returns bounds with `f` applied to each coordinate of the origin. The size is unchanged.
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

    /// Returns bounds with `f` applied to the width and the height. The origin is unchanged.
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
    /// Returns `point` relative to the origin of this rectangle, or `None` if `point` is outside.
    ///
    /// Use it to turn a window position into a position inside an element. The result follows
    /// the rules of [`Bounds::contains`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size, bounds, point, px, size};
    /// let panel = bounds(point(px(100.), px(50.)), size(px(200.), px(100.)));
    /// assert_eq!(panel.localize(&point(px(110.), px(60.))), Some(point(px(10.), px(10.))));
    /// assert_eq!(panel.localize(&point(px(10.), px(10.))), None);
    /// ```
    pub fn localize(&self, point: &Point<T>) -> Option<Point<T>> {
        self.contains(point)
            .then(|| point.relative_to(&self.origin))
    }
}

impl<T: PartialOrd + Clone + Debug + Default + PartialEq> Bounds<T> {
    /// Returns `true` if the width or the height is zero or less.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size};
    /// let empty = Bounds {
    ///     origin: Point { x: 0, y: 0 },
    ///     size: Size { width: 10, height: 0 },
    /// };
    /// assert!(empty.is_empty());
    /// ```
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
    /// Converts this size from device pixels to logical pixels by dividing by `scale_factor`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{DevicePixels, Size, px, size};
    /// let device = size(DevicePixels(200), DevicePixels(100));
    /// assert_eq!(device.to_pixels(2.), size(px(100.), px(50.)));
    /// ```
    pub fn to_pixels(self, scale_factor: f32) -> Size<Pixels> {
        size(
            px(self.width.0 as f32 / scale_factor),
            px(self.height.0 as f32 / scale_factor),
        )
    }
}

impl Size<Pixels> {
    /// Converts this size from logical pixels to device pixels.
    ///
    /// Multiplies each side by `scale_factor` and rounds it to the nearest whole device pixel.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{DevicePixels, px, size};
    /// let logical = size(px(100.), px(50.3));
    /// assert_eq!(logical.to_device_pixels(2.), size(DevicePixels(200), DevicePixels(101)));
    /// ```
    pub fn to_device_pixels(self, scale_factor: f32) -> Size<DevicePixels> {
        size(
            DevicePixels((self.width.0 * scale_factor).round() as i32),
            DevicePixels((self.height.0 * scale_factor).round() as i32),
        )
    }
}

impl Bounds<Pixels> {
    /// Multiplies the origin and the size by `factor`, producing [`ScaledPixels`].
    ///
    /// Pass the display's scale factor to convert logical bounds into the scaled space. The values
    /// aren't rounded: use [`Bounds::to_device_pixels`] to get whole device pixels.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, Point, Size, Pixels, ScaledPixels};
    /// let bounds = Bounds {
    ///     origin: Point { x: Pixels::from(10.0), y: Pixels::from(20.0) },
    ///     size: Size { width: Pixels::from(30.0), height: Pixels::from(40.0) },
    /// };
    /// let scaled_bounds = bounds.scale(2.0);
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

    /// Converts these logical bounds to device pixels.
    ///
    /// Multiplies each coordinate and size by `factor`, rounding each to the nearest whole device
    /// pixel. The origin and the size are rounded separately, so the bottom right corner can differ
    /// by one pixel from rounding the corner directly.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, DevicePixels, bounds, point, px, size};
    /// let logical = bounds(point(px(10.), px(20.)), size(px(30.), px(40.)));
    /// assert_eq!(
    ///     logical.to_device_pixels(1.5),
    ///     bounds(point(DevicePixels(15), DevicePixels(30)), size(DevicePixels(45), DevicePixels(60))),
    /// );
    /// ```
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
    /// Converts these device pixel bounds to logical pixels by dividing by `scale_factor`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Bounds, DevicePixels, bounds, point, px, size};
    /// let device = bounds(point(DevicePixels(200), DevicePixels(100)), size(DevicePixels(400), DevicePixels(300)));
    /// assert_eq!(device.to_pixels(2.), bounds(point(px(100.), px(50.)), size(px(200.), px(150.))));
    /// ```
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

/// The sizes of the four sides of a box, such as its padding or margin.
///
/// Each field holds the size of one side. The value is measured inward from that side: `top`
/// is the space along the top edge, and so on. `Edges<Pixels>` implements `From<f32>` and
/// `From<Pixels>` for the same value on all four sides.
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
    /// The size of the top side.
    pub top: T,
    /// The size of the right side.
    pub right: T,
    /// The size of the bottom side.
    pub bottom: T,
    /// The size of the left side.
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
    /// Returns edges with `value` on all four sides.
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

    /// Returns edges with `value` on the left and right sides, and the default on the top and bottom.
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

    /// Returns edges with `value` on the top and bottom sides, and the default on the left and right.
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

    /// Returns edges with `f` applied to each side.
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

    /// Returns `true` if `predicate` is `true` for at least one side.
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
    /// Returns edges with [`Length::Auto`] on all four sides.
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

    /// Returns edges of zero pixels on all four sides.
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
    /// Returns edges of zero on all four sides.
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

    /// Resolves each side to pixels.
    ///
    /// `top` and `bottom` are fractions of `parent_size.height`, and `left` and `right` are fractions
    /// of `parent_size.width`. Rems are multiplied by `rem_size`.
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
    /// assert_eq!(edges_in_pixels.top, px(10.0));
    /// assert_eq!(edges_in_pixels.right, px(100.0));
    /// assert_eq!(edges_in_pixels.bottom, px(32.0));
    /// assert_eq!(edges_in_pixels.left, px(50.0));
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
    /// Sets the edges of the `Edges` struct to zero, which means no size or thickness.
    ///
    /// This is typically used when you want to specify that a box (like a padding or margin area)
    /// should have no edges, effectively making it non-existent or invisible in layout calculations.
    ///
    /// # Returns
    ///
    /// Returns an `Edges<AbsoluteLength>` with all edges set to zero length.
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

    /// Resolves each side to pixels. Rems are multiplied by `rem_size`.
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
    /// assert_eq!(edges_in_pixels.top, px(10.0));
    /// assert_eq!(edges_in_pixels.right, px(16.0));
    /// assert_eq!(edges_in_pixels.bottom, px(20.0));
    /// assert_eq!(edges_in_pixels.left, px(32.0));
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
    /// Multiplies each side by `factor`, producing [`ScaledPixels`].
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

    /// Returns the largest of the four sides.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Edges, Pixels, px};
    /// let edges = Edges { top: px(4.), right: px(9.), bottom: px(2.), left: px(1.) };
    /// assert_eq!(edges.max(), px(9.));
    /// ```
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

/// A point on the edge or corner of a box, used to align a position to that box.
///
/// [`Size::at_anchor`], [`Bounds::corner`] and [`Bounds::from_anchor_and_size`] take an anchor.
/// Anchors don't carry a size, so they name a location on any box.
///
/// # Examples
///
/// ```
/// use gpui::{Anchor, Bounds, Point, Size};
/// let bounds = Bounds {
///     origin: Point { x: 0, y: 0 },
///     size: Size { width: 10, height: 20 },
/// };
/// assert_eq!(bounds.corner(Anchor::TopRight), Point { x: 10, y: 0 });
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
    /// The midpoint of the top edge.
    TopCenter,
    /// The midpoint of the bottom edge.
    BottomCenter,
    /// The midpoint of the left edge.
    LeftCenter,
    /// The midpoint of the right edge.
    RightCenter,
}

impl Anchor {
    /// Returns the anchor on the opposite side of the box.
    ///
    /// Corners map to the diagonally opposite corner, and each edge midpoint maps to the midpoint of
    /// the opposite edge.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Anchor;
    /// assert_eq!(Anchor::TopLeft.opposite(), Anchor::BottomRight);
    /// assert_eq!(Anchor::TopCenter.opposite(), Anchor::BottomCenter);
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

    /// Reflects this anchor across the center of the box along `axis`.
    ///
    /// [`Axis::Vertical`] swaps the top and bottom sides, and [`Axis::Horizontal`] swaps the left and
    /// right sides. An anchor on the center line of that axis stays where it is.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Anchor, Axis};
    /// assert_eq!(Anchor::TopLeft.other_side_along(Axis::Horizontal), Anchor::TopRight);
    /// assert_eq!(Anchor::TopLeft.other_side_along(Axis::Vertical), Anchor::BottomLeft);
    /// assert_eq!(Anchor::TopCenter.other_side_along(Axis::Horizontal), Anchor::TopCenter);
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

    /// Returns `true` for the midpoint of an edge, and `false` for a corner.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::Anchor;
    /// assert!(Anchor::LeftCenter.is_center());
    /// assert!(!Anchor::BottomRight.is_center());
    /// ```
    #[inline]
    pub fn is_center(&self) -> bool {
        matches!(
            self,
            Self::TopCenter | Self::BottomCenter | Self::LeftCenter | Self::RightCenter
        )
    }
}

/// The values at the four corners of a box, such as a border radius.
///
/// Use `Corners<Pixels>` for radii. `Corners<Pixels>` implements `From<f32>` and `From<Pixels>`
/// for the same value at every corner.
///
/// # Examples
///
/// ```
/// # use gpui::{Corners, Pixels, px};
/// let radii = Corners::all(px(8.));
/// assert_eq!(radii.top_left, Pixels::from(8.));
/// assert_eq!(radii.bottom_right, Pixels::from(8.));
/// ```
#[derive(Refineable, Clone, Default, Debug, Eq, PartialEq)]
#[refineable(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct Corners<T: Clone + Debug + Default + PartialEq> {
    /// The value at the top left corner.
    pub top_left: T,
    /// The value at the top right corner.
    pub top_right: T,
    /// The value at the bottom right corner.
    pub bottom_right: T,
    /// The value at the bottom left corner.
    pub bottom_left: T,
}

impl<T> Corners<T>
where
    T: Add<T, Output = T> + Half + Clone + Debug + Default + PartialEq,
{
    /// Returns corners with `value` at every corner.
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

    /// Returns the value at `corner`.
    ///
    /// The four corner anchors return their field. An edge midpoint returns the average of the two
    /// corners on that edge, computed with [`Half`]. For example, `Anchor::TopCenter` averages
    /// `top_left` and `top_right`.
    ///
    /// # Examples
    ///
    /// Corner positions:
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
    /// Edge midpoints, calculated as the average of the adjacent corners:
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
    /// Resolves each corner to pixels. Rems are multiplied by `rem_size`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, AbsoluteLength, Pixels, Rems};
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
    /// assert_eq!(corners_in_pixels.top_right, Pixels::from(16.0));
    /// assert_eq!(corners_in_pixels.bottom_right, Pixels::from(30.0));
    /// assert_eq!(corners_in_pixels.bottom_left, Pixels::from(32.0));
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
    /// Multiplies each corner by `factor`, producing [`ScaledPixels`].
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

    /// Returns the largest corner value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, Pixels, px};
    /// let corners = Corners {
    ///     top_left: px(1.),
    ///     top_right: px(4.),
    ///     bottom_right: px(2.),
    ///     bottom_left: px(3.),
    /// };
    /// assert_eq!(corners.max(), px(4.));
    /// ```
    #[must_use]
    pub fn max(&self) -> Pixels {
        self.top_left
            .max(self.top_right)
            .max(self.bottom_right)
            .max(self.bottom_left)
    }
}

impl<T: Div<f32, Output = T> + Ord + Clone + Debug + Default + PartialEq> Corners<T> {
    /// Limits each corner radius to half of the shorter side of `size`.
    ///
    /// Radii larger than that would overlap, so each one is replaced by the limit. Radii below the
    /// limit are unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Corners, px, size};
    /// let radii = Corners::all(px(30.)).clamp_radii_for_quad_size(size(px(40.), px(100.)));
    /// assert_eq!(radii, Corners::all(px(20.)));
    ///
    /// let small = Corners::all(px(5.)).clamp_radii_for_quad_size(size(px(40.), px(100.)));
    /// assert_eq!(small, Corners::all(px(5.)));
    /// ```
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
    /// Returns corners with `f` applied to each corner.
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

/// An angle in radians.
///
/// Use [`radians`] to create one, or convert a [`Percentage`] of a full turn with `From`.
///
/// # Examples
///
/// ```
/// use gpui::{Radians, percentage, radians};
/// let half_turn = radians(std::f32::consts::PI);
/// let quarter_turn = Radians::from(percentage(0.25));
/// assert!((quarter_turn.0 - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
/// assert!((half_turn.0 - std::f32::consts::PI).abs() < 1e-6);
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
    Debug,
)]
#[repr(transparent)]
pub struct Radians(pub f32);

/// Creates a [`Radians`] angle from a value in radians.
///
/// # Examples
///
/// ```
/// use gpui::radians;
/// assert_eq!(radians(1.5).0, 1.5);
/// ```
pub fn radians(value: f32) -> Radians {
    Radians(value)
}

/// A fraction of a full turn, from `0.0` to `1.0`.
///
/// Create one with [`percentage`]. Convert it to [`Radians`] with `From`.
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

/// Creates a [`Percentage`] of a full turn.
///
/// # Panics
///
/// Panics in debug builds if `value` is outside `0.0..=1.0`. Release builds don't check the
/// range.
///
/// # Examples
///
/// ```
/// use gpui::{Radians, percentage};
/// let turn = Radians::from(percentage(1.0));
/// assert!((turn.0 - std::f32::consts::TAU).abs() < 1e-6);
/// ```
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

/// A length in logical pixels, GPUI's unit for layout, painting and input.
///
/// The platform converts logical pixels to device pixels using the display's scale factor. Thus
/// `px(10.)` becomes 20 device pixels at scale factor 2. Styles, element bounds, [`Point`] and
/// [`Size`] values in the UI use `Pixels`.
///
/// GPUI has three pixel types, and they differ in what the number counts:
///
/// * `Pixels` is a logical length. Its value is an `f32`, so fractions are kept.
/// * [`ScaledPixels`] is a logical length multiplied by the scale factor, from [`Pixels::scale`].
///   It is still an `f32` and isn't rounded. Paint primitives in the scene are in scaled pixels.
/// * [`DevicePixels`] is a whole number of physical pixels on the display. Convert to it with
///   [`Size::to_device_pixels`] or [`Bounds::to_device_pixels`], which round each component to the
///   nearest pixel.
///
/// Create a `Pixels` with [`px`] or `Pixels::from`. It supports addition and subtraction with
/// other `Pixels`, multiplication and division by an `f32`, and division by another `Pixels`,
/// which gives a plain `f32` ratio.
///
/// # Examples
///
/// ```
/// use gpui::{Pixels, ScaledPixels, px};
///
/// let length = Pixels::from(10.0);
/// let scaled_length = length.scale(2.0);
/// assert_eq!(scaled_length, ScaledPixels::from(20.0));
/// assert_eq!(px(10.) / px(4.), 2.5);
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
    /// A length of zero pixels.
    pub const ZERO: Pixels = Pixels(0.0);
    /// The largest finite value that `Pixels` can hold.
    pub const MAX: Pixels = Pixels(f32::MAX);
    /// The smallest finite value that `Pixels` can hold.
    pub const MIN: Pixels = Pixels(f32::MIN);

    /// Returns the raw `f32` value.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Returns the largest whole number of pixels that is less than or equal to this value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(2.7).floor(), px(2.));
    /// assert_eq!(px(-2.3).floor(), px(-3.));
    /// ```
    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    /// Returns this value rounded to the nearest whole pixel. Halfway values round away from zero.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(2.4).round(), px(2.));
    /// assert_eq!(px(2.5).round(), px(3.));
    /// ```
    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    /// Returns the smallest whole number of pixels that is greater than or equal to this value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(2.1).ceil(), px(3.));
    /// assert_eq!(px(-2.7).ceil(), px(-2.));
    /// ```
    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }

    /// Multiplies this value by `factor`, producing [`ScaledPixels`].
    ///
    /// Pass the display's scale factor, such as `2.0` on a Retina display. The result isn't rounded.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{px, ScaledPixels};
    /// assert_eq!(px(10.).scale(1.5), ScaledPixels::from(15.));
    /// ```
    #[must_use]
    pub fn scale(&self, factor: f32) -> ScaledPixels {
        ScaledPixels(self.0 * factor)
    }

    /// Raises this value to the power `exponent`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(3.).pow(2.), px(9.));
    /// ```
    pub fn pow(&self, exponent: f32) -> Self {
        Self(self.0.powf(exponent))
    }

    /// Returns the absolute value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(-3.).abs(), px(3.));
    /// ```
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Returns `1.0` for positive values and `+0.0`, and `-1.0` for negative values and `-0.0`.
    ///
    /// The sign is a plain `f32`, not [`Pixels`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(-3.).signum(), -1.0);
    /// assert_eq!(px(3.).signum(), 1.0);
    /// ```
    pub fn signum(&self) -> f32 {
        self.0.signum()
    }

    /// Returns the value as an `f64`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::px;
    /// assert_eq!(px(1.5).to_f64(), 1.5);
    /// ```
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

/// A whole number of physical pixels on the display.
///
/// Device pixels are the pixels the hardware draws. Use them for sizes that are measured in
/// pixels of the output, such as the size of a rendered image or a window's drawable area.
/// Layout and painting use [`Pixels`]. Convert between the two with [`Size::to_device_pixels`]
/// and [`Size::to_pixels`], which apply the scale factor.
///
/// `DevicePixels` implements `Debug` as `"<n> px (device)"`.
///
/// # Examples
///
/// ```
/// use gpui::DevicePixels;
///
/// let pixels = DevicePixels(10);
/// assert_eq!(pixels.0, 10);
/// assert_eq!(DevicePixels::from(640_u32), DevicePixels(640));
/// ```
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
    /// Returns the number of bytes needed for this many pixels.
    ///
    /// Multiply by the number of bytes each pixel takes, such as 4 for RGBA. Only use this on
    /// non-negative values: a negative count is cast to `u32` before multiplying, which gives a
    /// meaningless result.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::DevicePixels;
    /// let pixels = DevicePixels(10);
    /// let bytes_per_pixel = 4;
    /// let total_bytes = pixels.to_bytes(bytes_per_pixel);
    /// assert_eq!(total_bytes, 40);
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

/// A logical length multiplied by the display's scale factor.
///
/// `ScaledPixels` is the result of [`Pixels::scale`]. It is a fraction-capable `f32` in the
/// space of physical pixels, so it can keep sub-pixel positions. Paint primitives in the scene
/// use this type. To get whole device pixels, round with [`ScaledPixels::round`] or convert with
/// `From<ScaledPixels> for DevicePixels`. Note that the conversion rounds up, unlike
/// [`Size::to_device_pixels`], which rounds to the nearest pixel.
///
/// # Examples
///
/// ```
/// use gpui::{DevicePixels, ScaledPixels, px};
///
/// let scaled: ScaledPixels = px(10.5).scale(2.);
/// assert_eq!(scaled, ScaledPixels::from(21.));
/// assert_eq!(scaled.round(), ScaledPixels::from(21.));
/// assert_eq!(DevicePixels::from(ScaledPixels(10.2)), DevicePixels(11));
/// ```
#[derive(Clone, Copy, Default, Add, AddAssign, Sub, SubAssign, Div, DivAssign, PartialEq)]
#[repr(transparent)]
pub struct ScaledPixels(pub f32);

impl ScaledPixels {
    /// Returns the raw `f32` value.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Returns the largest whole number that is less than or equal to this value.
    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    /// Returns this value rounded to the nearest whole number. Halfway values round away from zero.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::ScaledPixels;
    /// assert_eq!(ScaledPixels(2.4).round(), ScaledPixels(2.));
    /// ```
    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    /// Returns the smallest whole number that is greater than or equal to this value.
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

/// A length in rems, a unit based on the font size of the window.
///
/// One rem is the window's root font size, 16 pixels by default. Change it with
/// [`crate::Window::set_rem_size`]. Lengths in rems scale with the user's text size, so prefer them for
/// spacing and text. [`rems`] creates a `Rems` value.
///
/// # Examples
///
/// ```
/// use gpui::{Pixels, Rems, px, rems};
///
/// let two_rems = rems(2.0);
/// assert_eq!(two_rems, Rems(2.0));
/// assert_eq!(two_rems.to_pixels(px(16.)), Pixels::from(32.));
/// ```
#[derive(Clone, Copy, Default, Add, Sub, Mul, Div, Neg, PartialEq)]
pub struct Rems(pub f32);

impl Rems {
    /// A length of zero rems.
    pub const ZERO: Self = Self(0.0);
    /// Converts this length to pixels, given the size of one rem.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{px, rems};
    /// assert_eq!(rems(0.25).to_pixels(px(16.)), px(4.));
    /// ```
    pub fn to_pixels(self, rem_size: Pixels) -> Pixels {
        self * rem_size
    }
    /// Converts `length` to rems, using the window's current rem size.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Context, IntoElement, Render, Rems, TestApp, Window, div, px};
    /// # struct Empty;
    /// # impl Render for Empty {
    /// #     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_, _| Empty);
    /// window.update(|_, window, _| {
    ///     // The default rem size is 16 pixels.
    ///     assert_eq!(Rems::from_pixels(px(32.), window), Rems(2.0));
    /// });
    /// ```
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

/// A length given as a fraction of a reference length.
///
/// `1.0` is the full reference length and `0.5` is half of it. Fractions can be negative or
/// greater than `1.0`. Create one with [`relative`]. The reference length is supplied when the
/// value is resolved, for example by [`Relative::to_pixels`]. [`Relative`] displays as a
/// percentage, so `relative(0.5)` shows as `50%`.
///
/// # Examples
///
/// ```
/// use gpui::{AbsoluteLength, Relative, px, rems};
///
/// let half = Relative::from(0.5);
/// assert_eq!(half.to_pixels(AbsoluteLength::Rems(rems(2.)), px(16.)), px(16.));
/// let base_size = AbsoluteLength::Pixels(px(200.0));
///
/// assert_eq!(half.to_pixels(base_size, px(16.0)), px(100.0));
/// assert_eq!(half.to_string(), "50%");
/// ```
#[derive(Clone, Copy, Default, Add, Sub, Mul, Div, Neg, PartialEq)]
#[repr(transparent)]
pub struct Relative(pub(crate) f32);

impl Relative {
    /// A fraction of zero.
    pub const ZERO: Self = Self(0.0);

    /// Returns the raw fraction.
    pub fn as_f32(self) -> f32 {
        self.0
    }

    /// Resolves this fraction of `base_size` to pixels.
    ///
    /// When `base_size` is in rems, `rem_size` gives the pixel size of one rem.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AbsoluteLength, px, relative, rems};
    /// let base = AbsoluteLength::Rems(rems(2.));
    /// assert_eq!(relative(0.5).to_pixels(base, px(16.)), px(16.));
    /// ```
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

/// A length that doesn't depend on the parent: pixels or rems.
///
/// Pixels are fixed on screen. Rems depend on the window's rem size, so they track the user's
/// text size. Use [`AbsoluteLength::to_pixels`] to resolve a value for layout.
///
/// # Examples
///
/// ```
/// use gpui::{AbsoluteLength, Pixels, Rems, px, rems};
///
/// let fixed = AbsoluteLength::from(px(12.));
/// let scaled = AbsoluteLength::from(rems(0.75));
/// assert_eq!(fixed.to_pixels(px(16.)), Pixels::from(12.));
/// assert_eq!(scaled.to_pixels(px(16.)), Pixels::from(12.));
/// ```
#[derive(Clone, Copy, Neg, PartialEq)]
pub enum AbsoluteLength {
    /// A length in logical pixels.
    Pixels(Pixels),
    /// A length in rems.
    Rems(Rems),
}

impl AbsoluteLength {
    /// Returns `true` if the length is zero, whether in pixels or rems.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::{AbsoluteLength, px, rems};
    /// assert!(AbsoluteLength::from(px(0.)).is_zero());
    /// assert!(!AbsoluteLength::from(rems(0.5)).is_zero());
    /// ```
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
    /// Converts this length to pixels. Rems are multiplied by `rem_size`.
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

    /// Converts this length to rems. Pixels are divided by `rem_size`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AbsoluteLength, Pixels, Rems};
    /// let length = AbsoluteLength::Pixels(Pixels::from(32.0));
    /// assert_eq!(length.to_rems(Pixels::from(16.0)), Rems(2.0));
    /// ```
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

/// A length with a value: an absolute length, or a fraction of the parent.
///
/// Unlike [`Length`], it can't be `auto`. Layout resolves a fraction against the parent's size
/// with [`DefiniteLength::to_pixels`].
///
/// # Examples
///
/// ```
/// use gpui::{DefiniteLength, Pixels, px, relative};
///
/// let half_of_parent = DefiniteLength::from(relative(0.5));
/// let fixed = DefiniteLength::from(px(10.));
/// assert_eq!(fixed, DefiniteLength::from(Pixels::from(10.)));
/// assert_ne!(half_of_parent, fixed);
/// ```
#[derive(Clone, Copy, Neg, PartialEq)]
pub enum DefiniteLength {
    /// A fixed length in pixels or rems.
    Absolute(AbsoluteLength),
    /// A fraction of the parent's size.
    Relative(Relative),
}

impl DefiniteLength {
    /// Converts this length to pixels.
    ///
    /// An absolute length converts directly. A fraction is multiplied by `base_size`, which is the
    /// parent's size along the axis this length applies to. `rem_size` is the pixel size of one rem.
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

/// A length that can be a definite value or `auto`.
///
/// `auto` lets layout size the element from its content or its container. Style methods that
/// accept a size take a `Length`, so they accept `px`, `rems`, `relative` and `auto` values.
///
/// # Examples
///
/// ```
/// use gpui::{Length, auto, px, relative};
///
/// let fixed: Length = px(120.).into();
/// let fill: Length = relative(1.).into();
/// assert_eq!(auto(), Length::Auto);
/// assert_ne!(fixed, fill);
/// ```
#[derive(Clone, Copy, PartialEq)]
pub enum Length {
    /// A length with a value, in pixels, rems or a fraction of the parent.
    Definite(DefiniteLength),
    /// The size is chosen by the layout algorithm.
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

/// Creates a [`Relative`] length, a fraction of the reference length.
///
/// A fraction of `1.0` is the full reference length. Values outside `0.0` to `1.0` give lengths
/// below zero or above the full reference length.
///
/// # Examples
///
/// ```
/// use gpui::{AbsoluteLength, px, relative};
///
/// let full = relative(1.);
/// assert_eq!(full.as_f32(), 1.0);
/// assert_eq!(full.to_pixels(AbsoluteLength::Pixels(px(200.)), px(16.)), px(200.));
/// ```
pub const fn relative(fraction: f32) -> Relative {
    Relative(fraction)
}

/// Returns the golden ratio, about `1.618`, as a [`Relative`] length.
///
/// # Examples
///
/// ```
/// use gpui::phi;
/// let expected = (1.0 + 5f32.sqrt()) / 2.0;
/// assert!((phi().as_f32() - expected).abs() < 1e-6);
/// ```
pub const fn phi() -> Relative {
    relative(GOLDEN_RATIO)
}

/// Creates a [`Rems`] length.
///
/// # Examples
///
/// ```
/// use gpui::{Rems, rems};
/// assert_eq!(rems(1.5), Rems(1.5));
/// ```
pub const fn rems(rems: f32) -> Rems {
    Rems(rems)
}

/// Creates a [`Pixels`] length, the usual way to write a fixed size in logical pixels.
///
/// # Examples
///
/// ```
/// use gpui::{Pixels, px};
/// assert_eq!(px(12.), Pixels::from(12.));
/// ```
pub const fn px(pixels: f32) -> Pixels {
    Pixels(pixels)
}

/// Returns [`Length::Auto`], which lets layout choose the size from the context.
///
/// # Examples
///
/// ```
/// use gpui::{Length, auto};
/// assert_eq!(auto(), Length::Auto);
/// ```
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

/// The rows and columns that a grid item occupies.
///
/// Set this on a grid item to place it. Each range runs from its start placement to its end
/// placement, as in CSS `grid-row` and `grid-column`.
///
/// # Examples
///
/// ```
/// use gpui::{GridLocation, GridPlacement};
///
/// let location = GridLocation {
///     row: GridPlacement::Line(1)..GridPlacement::Span(2),
///     column: GridPlacement::Auto..GridPlacement::Auto,
/// };
/// assert_eq!(location.row.start, GridPlacement::Line(1));
/// ```
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub struct GridLocation {
    /// The rows this item uses within the grid.
    pub row: Range<GridPlacement>,
    /// The columns this item uses within the grid.
    pub column: Range<GridPlacement>,
}

/// Where a grid item starts or ends along one axis of a grid.
///
/// # Examples
///
/// ```
/// use gpui::GridPlacement;
///
/// assert_eq!(GridPlacement::default(), GridPlacement::Auto);
/// assert_eq!(GridPlacement::Span(3), GridPlacement::Span(3));
/// ```
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub enum GridPlacement {
    /// The grid line with this index, numbered as in CSS grid line placement.
    Line(i16),
    /// The number of grid tracks to span, measured from the other end of the range.
    Span(u16),
    /// Lets the layout algorithm pick the grid line.
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

/// Returns half of a value, for centering and midpoints.
///
/// Geometry code uses this trait to find midpoints and centers for each unit. For integers, the
/// result truncates toward zero, so `half` of `7` is `3`.
///
/// # Examples
///
/// ```
/// use gpui::{Half, px};
///
/// assert_eq!(7_i32.half(), 3);
/// assert_eq!(px(7.).half(), px(3.5));
/// ```
pub trait Half {
    /// Returns half of this value.
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

/// Checks whether a length or geometry value is zero.
///
/// Implemented for the length types and for [`Point`], [`Size`], [`Bounds`] and [`Corners`].
/// For a [`Size`], the check is `true` when either side is zero, because the area is then zero.
/// For a [`Point`], both coordinates must be zero. For [`Corners`], all four corners must be zero.
///
/// # Examples
///
/// ```
/// use gpui::{IsZero, px, size};
///
/// assert!(px(0.).is_zero());
/// assert!(size(px(10.), px(0.)).is_zero());
/// assert!(!size(px(10.), px(5.)).is_zero());
/// ```
pub trait IsZero {
    /// Returns `true` if this value is zero.
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
