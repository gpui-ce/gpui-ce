use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use std::{
    borrow::Cow,
    fmt::{self, Display, Formatter},
    hash::{Hash, Hasher},
};

/// Font width as a positive finite percentage, with 100 representing normal width.
///
/// Selects static width variants or a variable font's `wdth` axis. Requests outside
/// a variable font's axis range are clamped to that range by the text backend.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FontWidth(f32);

impl FontWidth {
    /// Ultra-condensed width, 50% of normal.
    pub const ULTRA_CONDENSED: Self = Self(50.0);
    /// Extra-condensed width, 62.5% of normal.
    pub const EXTRA_CONDENSED: Self = Self(62.5);
    /// Condensed width, 75% of normal.
    pub const CONDENSED: Self = Self(75.0);
    /// Semi-condensed width, 87.5% of normal.
    pub const SEMI_CONDENSED: Self = Self(87.5);
    /// Normal width, 100%.
    pub const NORMAL: Self = Self(100.0);
    /// Semi-expanded width, 112.5% of normal.
    pub const SEMI_EXPANDED: Self = Self(112.5);
    /// Expanded width, 125% of normal.
    pub const EXPANDED: Self = Self(125.0);
    /// Extra-expanded width, 150% of normal.
    pub const EXTRA_EXPANDED: Self = Self(150.0);
    /// Ultra-expanded width, 200% of normal.
    pub const ULTRA_EXPANDED: Self = Self(200.0);

    /// Creates a width from a percentage of normal width.
    ///
    /// # Panics
    ///
    /// Panics if the percentage is nonfinite or not positive.
    pub fn from_percentage(percentage: f32) -> Self {
        assert!(
            percentage.is_finite() && percentage > 0.0,
            "font width must be a positive finite percentage"
        );

        Self(percentage)
    }

    /// Returns the width as a percentage of normal width.
    pub const fn percentage(self) -> f32 {
        self.0
    }
}

impl Default for FontWidth {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl Eq for FontWidth {}

impl Hash for FontWidth {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl Display for FontWidth {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}%", self.0)
    }
}

impl<'de> Deserialize<'de> for FontWidth {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let percentage = f32::deserialize(deserializer)?;

        if !percentage.is_finite() || percentage <= 0.0 {
            return Err(DeserializerType::Error::custom(
                "font width must be a positive finite percentage",
            ));
        }

        Ok(Self(percentage))
    }
}

impl JsonSchema for FontWidth {
    fn schema_name() -> Cow<'static, str> {
        "FontWidth".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "number",
            "exclusiveMinimum": 0,
            "default": Self::default(),
            "description": "Font width as a percentage of normal width (100)"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::catch_unwind;

    #[test]
    fn percentages_serialize_and_reject_invalid_cache_keys() {
        for percentage in [0.5, 75.0, 87.5, 100.0, 250.0] {
            let width = FontWidth::from_percentage(percentage);
            let json = serde_json::to_string(&width).unwrap();

            assert_eq!(serde_json::from_str::<FontWidth>(&json).unwrap(), width);
            assert_eq!(width.percentage(), percentage);
        }

        for percentage in [0.0, -0.0, -75.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(catch_unwind(|| FontWidth::from_percentage(percentage)).is_err());
            let deserializer =
                serde::de::value::F32Deserializer::<serde::de::value::Error>::new(percentage);

            assert!(FontWidth::deserialize(deserializer).is_err());
        }

        assert_eq!(FontWidth::default(), FontWidth::NORMAL);
    }
}
