use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Font families to try, in order, when a [`Font`](crate::Font) lacks a glyph.
///
/// Set the list on [`Font::fallbacks`](crate::Font::fallbacks). The text system
/// uses these families in order when the primary family cannot supply the needed
/// glyphs. Shaping chooses fonts for clusters, which may contain combining marks
/// or an emoji sequence; it does not choose a font independently for every code
/// point. Names use the same [`TextSystem`](crate::TextSystem) font database as
/// the primary family. See the [text guide](crate::_documentation::guides::text)
/// for loading fonts and working with cluster boundaries.
///
/// # Examples
///
/// ```
/// # use gpui::{FontFallbacks, font};
/// let mut body = font("Lilex");
/// body.fallbacks = Some(FontFallbacks::from_fonts(vec![
///     "Noto Sans Arabic".to_string(),
/// ]));
/// assert_eq!(body.fallbacks.unwrap().fallback_list(), ["Noto Sans Arabic"]);
/// ```
#[derive(Default, Clone, Eq, PartialEq, Hash, Debug, Deserialize, Serialize, JsonSchema)]
pub struct FontFallbacks(pub Arc<Vec<String>>);

impl FontFallbacks {
    /// Returns the fallback families in the order the text system tries them.
    pub fn fallback_list(&self) -> &[String] {
        self.0.as_slice()
    }

    /// Creates a fallback list from family names, tried in the order given.
    pub fn from_fonts(fonts: Vec<String>) -> Self {
        FontFallbacks(Arc::new(fonts))
    }
}
