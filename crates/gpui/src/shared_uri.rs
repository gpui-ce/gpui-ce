use derive_more::{Deref, DerefMut};

use crate::SharedString;

/// A URI held in a [`SharedString`], so cloning the handle does not copy its text.
///
/// The text is not validated. Converting a `SharedUri` into an [`ImageSource`](crate::ImageSource)
/// always treats it as a URL, bypassing the scheme check applied to a plain `&str`. See
/// [`ImageSource`](crate::ImageSource) for the corresponding string conversions.
#[derive(Deref, DerefMut, Default, PartialEq, Eq, Hash, Clone)]
pub struct SharedUri(SharedString);

impl std::fmt::Debug for SharedUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Display for SharedUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.as_ref())
    }
}

impl<T: Into<SharedString>> From<T> for SharedUri {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}
