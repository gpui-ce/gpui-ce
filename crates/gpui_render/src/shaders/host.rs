use super::*;

impl From<bool> for common::ShaderBool {
    fn from(value: bool) -> Self {
        if value {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

impl From<gpui::Bounds<gpui::ScaledPixels>> for common::Bounds {
    fn from(bounds: gpui::Bounds<gpui::ScaledPixels>) -> Self {
        Self {
            origin: wgsl_rs::std::vec2f(bounds.origin.x.0, bounds.origin.y.0),
            size: wgsl_rs::std::vec2f(bounds.size.width.0, bounds.size.height.0),
        }
    }
}

impl From<gpui::Corners<gpui::ScaledPixels>> for common::Corners {
    fn from(corners: gpui::Corners<gpui::ScaledPixels>) -> Self {
        Self {
            top_left: corners.top_left.0,
            top_right: corners.top_right.0,
            bottom_right: corners.bottom_right.0,
            bottom_left: corners.bottom_left.0,
        }
    }
}
