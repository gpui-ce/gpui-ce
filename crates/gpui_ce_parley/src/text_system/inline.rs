use anyhow::{Context as _, Result};
use gpui::Font;
use parley::{FontFeature, setting::Tag};

pub(super) fn parley_font_features(font: &Font) -> Result<Vec<FontFeature>> {
    font.features
        .tag_value_list()
        .iter()
        .map(|(tag, value)| {
            let tag =
                Tag::parse(tag).with_context(|| format!("invalid OpenType feature tag '{tag}'"))?;

            let value = (*value)
                .try_into()
                .with_context(|| format!("OpenType feature '{tag}' value is larger than u16"))?;

            Ok(FontFeature::new(tag, value))
        })
        .collect()
}
