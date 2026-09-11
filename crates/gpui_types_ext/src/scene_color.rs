//! Renderer-facing color representation used by the scene and GPU buffers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Internal representation of [`palette::Hsla`] which is layout sensitive, as
/// it's provided to the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub struct SceneHsla {
    /// Hue, in a range from 0 to 1
    pub h: f32,
    /// Saturation, in a range from 0 to 1
    pub s: f32,
    /// Lightness, in a range from 0 to 1
    pub l: f32,
    /// Alpha, in a range from 0 to 1
    pub a: f32,
}

impl From<palette::Hsla> for SceneHsla {
    fn from(hsla: palette::Hsla) -> Self {
        Self {
            h: hsla.hue.into_positive_degrees() / 360.0,
            s: hsla.saturation,
            l: hsla.lightness,
            a: hsla.alpha,
        }
    }
}

impl From<SceneHsla> for palette::Hsla {
    fn from(hsla: SceneHsla) -> Self {
        palette::Hsla::new(hsla.h * 360.0, hsla.s, hsla.l, hsla.a)
    }
}

impl From<gpui_types::color::Hsla> for SceneHsla {
    fn from(hsla: gpui_types::color::Hsla) -> Self {
        Self {
            h: hsla.h,
            s: hsla.s,
            l: hsla.l,
            a: hsla.a,
        }
    }
}

impl From<SceneHsla> for gpui_types::color::Hsla {
    fn from(hsla: SceneHsla) -> Self {
        gpui_types::color::Hsla {
            h: hsla.h,
            s: hsla.s,
            l: hsla.l,
            a: hsla.a,
        }
    }
}
