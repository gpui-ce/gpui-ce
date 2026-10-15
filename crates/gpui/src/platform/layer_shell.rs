use bitflags::bitflags;
use thiserror::Error;

use crate::{Pixels, util::FluentBuilder};

/// The layer the surface is rendered on. Multiple surfaces can share a layer, and ordering within
/// a single layer is undefined.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    /// The background layer, typically used for wallpapers.
    Background,

    /// The bottom layer.
    Bottom,

    /// The top layer, typically used for fullscreen windows.
    Top,

    /// The overlay layer, used for surfaces that should always be on top.
    #[default]
    Overlay,
}

bitflags! {
    /// Screen anchor point for layer_shell surfaces. These can be used in any combination, e.g.
    /// specifying `Anchor::LEFT | Anchor::RIGHT` will stretch the surface across the width of the
    /// screen.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct Anchor: u32 {
        /// Anchor to the top edge of the screen.
        const TOP = 1;
        /// Anchor to the bottom edge of the screen.
        const BOTTOM = 2;
        /// Anchor to the left edge of the screen.
        const LEFT = 4;
        /// Anchor to the right edge of the screen.
        const RIGHT = 8;
    }
}

/// Keyboard interactivity mode for the layer_shell surfaces.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum KeyboardInteractivity {
    /// No keyboard inputs will be delivered to the surface and it won't be able to receive
    /// keyboard focus.
    None,

    /// The surface will receive exclusive keyboard focus as long as it is above the shell surface
    /// layer, and no other layer_shell surfaces are above it.
    Exclusive,

    /// The surface can be focused similarly to a normal window.
    #[default]
    OnDemand,
}

/// Configures a Wayland layer-shell surface, such as a panel, dock, or wallpaper.
///
/// Set this on [`WindowKind::LayerShell`](crate::WindowKind::LayerShell) when you open a window.
/// Chain setters on [`LayerShellOptions::default`] to configure the surface. Pass `None` to
/// clear the exclusive zone, exclusive edge, or margins.
///
/// The compositor applies the namespace, layer, anchor and margins. Opening the window fails
/// with [`LayerShellNotSupportedError`] when the compositor doesn't support the layer-shell
/// protocol. Layer shell exists only on Wayland, so this module is available only on Linux
/// builds with the `wayland` feature.
///
/// # Examples
///
/// ```
/// # #[cfg(all(target_os = "linux", feature = "wayland"))]
/// # fn main() {
/// use gpui::{
///     layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
///     px,
/// };
///
/// // A 40 px bar along the bottom edge, stretched across the screen.
/// let options = LayerShellOptions::default()
///     .namespace("panel")
///     .layer(Layer::Top)
///     .anchor(Anchor::LEFT | Anchor::RIGHT | Anchor::BOTTOM)
///     .exclusive_zone(px(40.0))
///     .keyboard_interactivity(KeyboardInteractivity::None);
///
/// assert_eq!(options.namespace, "panel");
/// assert_eq!(options.layer, Layer::Top);
/// assert_eq!(options.exclusive_zone, Some(px(40.0)));
/// # }
/// # #[cfg(not(all(target_os = "linux", feature = "wayland")))]
/// # fn main() {}
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, derive_setters::Setters)]
#[setters(into)]
pub struct LayerShellOptions {
    /// The namespace for the surface, mostly used by compositors to apply rules, can not be
    /// changed after the surface is created.
    pub namespace: String,
    /// The layer the surface is rendered on.
    pub layer: Layer,
    /// The anchor point of the surface.
    pub anchor: Anchor,
    /// Requests that the compositor avoids occluding an area with other surfaces.
    pub exclusive_zone: Option<Pixels>,
    /// The anchor point of the exclusive zone, will be determined using the anchor if left
    /// unspecified.
    pub exclusive_edge: Option<Anchor>,
    /// Margins between the surface and its anchor point(s).
    /// Specified in CSS order: top, right, bottom, left.
    pub margin: Option<(Pixels, Pixels, Pixels, Pixels)>,
    /// How keyboard events should be delivered to the surface.
    pub keyboard_interactivity: KeyboardInteractivity,
}

impl FluentBuilder for LayerShellOptions {}

/// An error indicating that an action failed because the compositor doesn't support the required
/// layer_shell protocol.
#[derive(Debug, Error)]
#[error("Compositor doesn't support zwlr_layer_shell_v1")]
pub struct LayerShellNotSupportedError;
