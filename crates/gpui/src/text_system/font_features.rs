use std::borrow::Cow;
use std::sync::Arc;

use schemars::{JsonSchema, json_schema};

/// OpenType feature settings applied while shaping a [`Font`](crate::Font).
///
/// Each entry is a four-character feature tag, such as `"liga"` or `"tnum"`, and
/// a value. A boolean feature is `1` when enabled and `0` when disabled. Some
/// features take other numbers, such as a stylistic set number. The default is
/// empty, which leaves every feature at the font's own setting.
///
/// Set features with [`Font::features`](crate::Font::features). In JSON
/// configuration, write a map from tag to `true`, `false`, or a number. When you
/// deserialize, tags that are not four ASCII alphanumeric characters, and
/// numbers that are not unsigned integers, are logged and skipped.
///
/// # Examples
///
/// ```
/// # use gpui::{FontFeatures, font};
/// let mut code = font("Lilex");
/// code.features = FontFeatures::disable_ligatures();
/// assert_eq!(code.features.is_calt_enabled(), Some(false));
///
/// let json = r#"{ "tnum": true, "calt": false }"#;
/// let features: FontFeatures = serde_json::from_str(json).unwrap();
/// assert_eq!(
///     features.tag_value_list(),
///     [("tnum".to_string(), 1), ("calt".to_string(), 0)]
/// );
/// ```
#[derive(Default, Clone, Eq, PartialEq, Hash)]
pub struct FontFeatures(pub Arc<Vec<(String, u32)>>);

impl FontFeatures {
    /// Returns features that turn off `calt`, which many fonts use for programming ligatures.
    ///
    /// Other features, such as `liga`, keep the font's default.
    pub fn disable_ligatures() -> Self {
        Self(Arc::new(vec![("calt".into(), 0)]))
    }

    /// Returns the feature tags and values, in the order they were set.
    ///
    /// Only features that were set are listed. A feature that isn't listed
    /// keeps the font's default.
    pub fn tag_value_list(&self) -> &[(String, u32)] {
        self.0.as_slice()
    }

    /// Returns whether `calt` is enabled, or `None` if this set doesn't mention `calt`.
    ///
    /// Only the value `1` counts as enabled. Any other value, including `2`, returns `Some(false)`.
    pub fn is_calt_enabled(&self) -> Option<bool> {
        self.0
            .iter()
            .find(|(feature, _)| feature == "calt")
            .map(|(_, value)| *value == 1)
    }
}

impl std::fmt::Debug for FontFeatures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("FontFeatures");
        for (tag, value) in self.tag_value_list() {
            debug.field(tag, value);
        }

        debug.finish()
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum FeatureValue {
    Bool(bool),
    Number(serde_json::Number),
}

impl<'de> serde::Deserialize<'de> for FontFeatures {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{MapAccess, Visitor};
        use std::fmt;

        struct FontFeaturesVisitor;

        impl<'de> Visitor<'de> for FontFeaturesVisitor {
            type Value = FontFeatures;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map of font features")
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut feature_list = Vec::new();

                while let Some((key, value)) =
                    access.next_entry::<String, Option<FeatureValue>>()?
                {
                    if !is_valid_feature_tag(&key) {
                        log::error!("Incorrect font feature tag: {}", key);
                        continue;
                    }
                    if let Some(value) = value {
                        match value {
                            FeatureValue::Bool(enable) => {
                                if enable {
                                    feature_list.push((key, 1));
                                } else {
                                    feature_list.push((key, 0));
                                }
                            }
                            FeatureValue::Number(value) => {
                                if value.is_u64() {
                                    feature_list.push((key, value.as_u64().unwrap() as u32));
                                } else {
                                    log::error!(
                                        "Incorrect font feature value {} for feature tag {}",
                                        value,
                                        key
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                }

                Ok(FontFeatures(Arc::new(feature_list)))
            }
        }

        let features = deserializer.deserialize_map(FontFeaturesVisitor)?;
        Ok(features)
    }
}

impl serde::Serialize for FontFeatures {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;

        for (tag, value) in self.tag_value_list() {
            map.serialize_entry(tag, value)?;
        }

        map.end()
    }
}

impl JsonSchema for FontFeatures {
    fn schema_name() -> Cow<'static, str> {
        "FontFeatures".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "type": "object",
            "patternProperties": {
                "[0-9a-zA-Z]{4}$": {
                    "type": ["boolean", "integer"],
                    "minimum": 0,
                    "multipleOf": 1
                }
            },
            "additionalProperties": false
        })
    }
}

fn is_valid_feature_tag(tag: &str) -> bool {
    tag.len() == 4 && tag.chars().all(|c| c.is_ascii_alphanumeric())
}
