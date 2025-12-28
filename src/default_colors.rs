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
    /// Container color (deprecated, use surface)
    pub container: Rgba,
    /// Accent/primary action color
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

    /// Returns the default dark colors.
    pub fn dark() -> Self {
        Self {
            // Text
            text: rgb(0xffffff),
            text_muted: rgb(0x94a3b8),
            selected_text: rgb(0xffffff),
            disabled: rgb(0x565656),

            // Backgrounds
            background: rgb(0x0f172a),
            surface: rgb(0x1e293b),
            surface_hover: rgb(0x334155),
            container: rgb(0x262626),

            // Borders
            border: rgb(0x334155),
            separator: rgb(0x334155),

            // Selection
            selected: rgb(0x2457ca),

            // Accent (blue)
            accent: rgb(0x3b82f6),
            accent_hover: rgb(0x2563eb),
            accent_active: rgb(0x1d4ed8),

            // Success (green)
            success: rgb(0x22c55e),
            success_hover: rgb(0x16a34a),

            // Warning (yellow/amber)
            warning: rgb(0xeab308),
            warning_hover: rgb(0xca8a04),

            // Error (red)
            error: rgb(0xef4444),
            error_hover: rgb(0xdc2626),
        }
    }

    /// Returns the default light colors.
    pub fn light() -> Self {
        Self {
            // Text
            text: rgb(0x0f172a),
            text_muted: rgb(0x64748b),
            selected_text: rgb(0xffffff),
            disabled: rgb(0xb0b0b0),

            // Backgrounds
            background: rgb(0xffffff),
            surface: rgb(0xf1f5f9),
            surface_hover: rgb(0xe2e8f0),
            container: rgb(0xf4f5f5),

            // Borders
            border: rgb(0xe2e8f0),
            separator: rgb(0xe2e8f0),

            // Selection
            selected: rgb(0x2a63d9),

            // Accent (blue)
            accent: rgb(0x2563eb),
            accent_hover: rgb(0x1d4ed8),
            accent_active: rgb(0x1e40af),

            // Success (green)
            success: rgb(0x16a34a),
            success_hover: rgb(0x15803d),

            // Warning (yellow/amber)
            warning: rgb(0xca8a04),
            warning_hover: rgb(0xa16207),

            // Error (red)
            error: rgb(0xdc2626),
            error_hover: rgb(0xb91c1c),
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
