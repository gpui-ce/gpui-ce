use crate::{App, Global, Window, WindowAppearance, rgb};
use palette::rgb::Rgba;
use std::ops::Deref;
use std::sync::Arc;

/// A pair of light and dark base palettes for application components to share.
/// GPUI's built-in elements do not read this value automatically.
///
/// Choose a palette for a window with [`Colors::for_appearance`]. For one app-wide palette,
/// call [`App::init_colors`] and read it with [`DefaultColors::default_colors`].
///
/// # Examples
///
/// ```
/// use gpui::{colors::Colors, rgb};
///
/// let light = Colors::light();
/// assert_eq!(light.background, rgb(0xffffff));
///
/// let dark = Colors::dark();
/// assert_eq!(dark.text, rgb(0xffffff));
/// ```
#[derive(Clone, Debug)]
pub struct Colors {
    /// The default color for text.
    pub text: Rgba,
    /// The color for text on a [`Colors::selected`] background.
    pub selected_text: Rgba,
    /// The color of the window background.
    pub background: Rgba,
    /// The color for disabled controls and their text.
    pub disabled: Rgba,
    /// The color for selected items and highlighted controls.
    pub selected: Rgba,
    /// The color for borders around controls.
    pub border: Rgba,
    /// The color for dividers between items.
    pub separator: Rgba,
    /// The color for raised surfaces, such as cards and panels, drawn over [`Colors::background`].
    pub container: Rgba,
}

impl Default for Colors {
    /// Returns [`Colors::light`].
    fn default() -> Self {
        Self::light()
    }
}

impl Colors {
    /// Returns [`Colors::light`] or [`Colors::dark`] for the window's [`WindowAppearance`].
    ///
    /// The returned palette is a snapshot. Ask again after the window appearance changes.
    pub fn for_appearance(window: &Window) -> Self {
        match window.appearance() {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    /// Returns the colors for dark appearances.
    pub fn dark() -> Self {
        Self {
            text: rgb(0xffffff),
            selected_text: rgb(0xffffff),
            disabled: rgb(0x565656),
            selected: rgb(0x2457ca),
            background: rgb(0x222222),
            border: rgb(0x000000),
            separator: rgb(0xd9d9d9),
            container: rgb(0x262626),
        }
    }

    /// Returns the colors for light appearances.
    ///
    /// This is the set that [`Colors::default`] returns.
    pub fn light() -> Self {
        Self {
            text: rgb(0x252525),
            selected_text: rgb(0xffffff),
            background: rgb(0xffffff),
            disabled: rgb(0xb0b0b0),
            selected: rgb(0x2a63d9),
            border: rgb(0xd9d9d9),
            separator: rgb(0xe6e6e6),
            container: rgb(0xf4f5f5),
        }
    }

    /// Returns the app-wide colors that [`App::init_colors`] set.
    ///
    /// # Panics
    ///
    /// Panics if [`App::init_colors`] has not been called.
    pub fn get_global(cx: &App) -> &Arc<Colors> {
        &cx.global::<GlobalColors>().0
    }
}

/// The app-wide [`Colors`], stored as a global on the [`App`].
///
/// [`App::init_colors`] sets it. Read the colors through [`DefaultColors::default_colors`]
/// or [`Colors::get_global`] rather than this type. It dereferences to the shared [`Colors`].
#[derive(Clone, Debug)]
pub struct GlobalColors(pub Arc<Colors>);

impl Deref for GlobalColors {
    type Target = Arc<Colors>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Global for GlobalColors {}

/// Gives access to the app-wide [`Colors`] through `cx.default_colors()`.
///
/// [`App`] implements this trait. The colors exist only after [`App::init_colors`] runs.
///
/// # Panics
///
/// Calling [`DefaultColors::default_colors`] panics if [`App::init_colors`] has not been called.
pub trait DefaultColors {
    /// Returns the app-wide [`Colors`] that [`App::init_colors`] set.
    fn default_colors(&self) -> &Arc<Colors>;
}

impl DefaultColors for App {
    fn default_colors(&self) -> &Arc<Colors> {
        &self.global::<GlobalColors>().0
    }
}

/// Whether GPUI's base colors use the light or the dark set.
///
/// This is a two-value form of [`WindowAppearance`], which also has vibrant variants. The
/// conversion from [`WindowAppearance`] maps each vibrant variant to its base set. Use
/// [`Colors::light`] for `Light` and [`Colors::dark`] for `Dark`.
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
