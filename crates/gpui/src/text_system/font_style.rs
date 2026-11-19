use core::fmt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display, Formatter},
    hash::Hash,
};

/// Selects an italic or oblique face when the font family has one.
///
/// Use [`crate::Font::italic`] to set [`FontStyle::Italic`].
#[derive(Clone, Copy, Eq, PartialEq, Debug, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum FontStyle {
    /// The upright face of the family.
    #[default]
    Normal,
    /// The italic face, which is usually a cursive design.
    Italic,
    /// A sloped version of the upright face.
    Oblique,
}

impl Display for FontStyle {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(self, f)
    }
}
