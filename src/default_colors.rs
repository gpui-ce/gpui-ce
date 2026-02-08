use crate::{App, Global, Rgba, Window, WindowAppearance, rgb};
use std::ops::Deref;
use std::sync::Arc;

/// The default set of colors for gpui.
///
/// These are used for styling base components, examples and more.
#[derive(Clone, Debug)]
pub struct Colors {
    /// Primary text color
    pub text: Rgba,
    /// Muted/secondary text color
    pub text_muted: Rgba,
    /// Selected text color
    pub selected_text: Rgba,
    /// Background color (root level)
    pub background: Rgba,
    /// Surface color (cards, panels, elevated containers)
    pub surface: Rgba,
    /// Surface color on hover
    pub surface_hover: Rgba,
    /// Disabled color
    pub disabled: Rgba,
    /// Selected color
    pub selected: Rgba,
    /// Border color
    pub border: Rgba,
    /// Separator color
    pub separator: Rgba,
    /// Container color
    pub container: Rgba,
    /// Accent/primary action color (macOS blue)
    pub accent: Rgba,
    /// Accent color on hover
    pub accent_hover: Rgba,
    /// Accent color when active/pressed
    pub accent_active: Rgba,
    /// Success/positive color
    pub success: Rgba,
    /// Success color on hover
    pub success_hover: Rgba,
    /// Warning/caution color
    pub warning: Rgba,
    /// Warning color on hover
    pub warning_hover: Rgba,
    /// Error/destructive color
    pub error: Rgba,
    /// Error color on hover
    pub error_hover: Rgba,
}

impl Default for Colors {
    fn default() -> Self {
        Self::light()
    }
}

impl Colors {
    /// Returns the default colors for the given window appearance.
    pub fn for_appearance(window: &Window) -> Self {
        match window.appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    /// Returns the default dark colors
    pub fn dark() -> Self {
        Self {
            // Text
            text: rgb(0xffffff),
            text_muted: rgb(0x98989d),
            selected_text: rgb(0xffffff),
            disabled: rgb(0x565656),

            // Backgrounds
            background: rgb(0x1e1e1e),
            surface: rgb(0x2d2d2d),
            surface_hover: rgb(0x3d3d3d),
            container: rgb(0x262626),

            // Borders
            border: rgb(0x3d3d3d),
            separator: rgb(0x3d3d3d),

            // Selection
            selected: rgb(0x0058d0),

            // Accent (macOS blue)
            accent: rgb(0x0a84ff),
            accent_hover: rgb(0x409cff),
            accent_active: rgb(0x0071e3),

            // Success (green)
            success: rgb(0x30d158),
            success_hover: rgb(0x28cd52),

            // Warning (yellow/orange)
            warning: rgb(0xffd60a),
            warning_hover: rgb(0xffcc00),

            // Error (red)
            error: rgb(0xff453a),
            error_hover: rgb(0xff6961),
        }
    }

    /// Returns the default light colors
    pub fn light() -> Self {
        Self {
            // Text
            text: rgb(0x1d1d1f),
            text_muted: rgb(0x86868b),
            selected_text: rgb(0xffffff),
            disabled: rgb(0xb0b0b0),

            // Backgrounds
            background: rgb(0xffffff),
            surface: rgb(0xf5f5f7),
            surface_hover: rgb(0xe8e8ed),
            container: rgb(0xf5f5f7),

            // Borders
            border: rgb(0xd2d2d7),
            separator: rgb(0xd2d2d7),

            // Selection
            selected: rgb(0x0066cc),

            // Accent (macOS blue)
            accent: rgb(0x007aff),
            accent_hover: rgb(0x0071e3),
            accent_active: rgb(0x0058d0),

            // Success (green)
            success: rgb(0x28cd41),
            success_hover: rgb(0x23b839),

            // Warning (yellow/orange)
            warning: rgb(0xff9f0a),
            warning_hover: rgb(0xe68f09),

            // Error (red)
            error: rgb(0xff3b30),
            error_hover: rgb(0xe6352b),
        }
    }

    /// Get [Colors] from the global state
    pub fn get_global(cx: &App) -> &Arc<Colors> {
        &cx.global::<GlobalColors>().0
    }
}

/// Get [Colors] from the global state
#[derive(Clone, Debug)]
pub struct GlobalColors(pub Arc<Colors>);

impl Deref for GlobalColors {
    type Target = Arc<Colors>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Global for GlobalColors {}

/// Implement this trait to allow global [Colors] access via `cx.default_colors()`.
pub trait DefaultColors {
    /// Returns the default [`Colors`]
    fn default_colors(&self) -> &Arc<Colors>;
}

impl DefaultColors for App {
    fn default_colors(&self) -> &Arc<Colors> {
        &self.global::<GlobalColors>().0
    }
}

/// The appearance of the base GPUI colors, used to style GPUI elements
///
/// Varies based on the system's current [`WindowAppearance`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DefaultAppearance {
    /// Use the set of colors for light appearances.
    #[default]
    Light,
    /// Use the set of colors for dark appearances.
    Dark,
}

impl From<WindowAppearance> for DefaultAppearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
        }
    }
}
