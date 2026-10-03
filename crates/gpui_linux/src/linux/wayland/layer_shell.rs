pub use gpui::layer_shell::*;

use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

pub(crate) fn wayland_layer(layer: Layer) -> zwlr_layer_shell_v1::Layer {
    match layer {
        Layer::Background => zwlr_layer_shell_v1::Layer::Background,
        Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        Layer::Top => zwlr_layer_shell_v1::Layer::Top,
        Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

pub(crate) fn wayland_anchor(anchor: Anchor) -> zwlr_layer_surface_v1::Anchor {
    zwlr_layer_surface_v1::Anchor::from_bits_truncate(anchor.bits())
}

// The `on_demand` enum value was added in layer-shell v4.
// (https://wayland.app/protocols/wlr-layer-shell-unstable-v1#zwlr_layer_surface_v1:enum:keyboard_interactivity:entry:on_demand)
const ON_DEMAND_SINCE: u32 = 4;

/// Converts GPUI's keyboard interactivity to the layer-shell value, taking the
/// bound surface version into account. `OnDemand` is only sent on layer-shell
/// v4 or newer; older compositors get `None` rather than `Exclusive`, which
/// would request exclusive keyboard focus.
pub(crate) fn wayland_keyboard_interactivity(
    value: KeyboardInteractivity,
    version: u32,
) -> zwlr_layer_surface_v1::KeyboardInteractivity {
    match value {
        KeyboardInteractivity::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
        KeyboardInteractivity::Exclusive => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        KeyboardInteractivity::OnDemand if version >= ON_DEMAND_SINCE => {
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
        }
        KeyboardInteractivity::OnDemand => {
            log::warn!(
                "layer-shell v{version} does not support OnDemand keyboard interactivity; \
                 using None instead"
            );
            zwlr_layer_surface_v1::KeyboardInteractivity::None
        }
    }
}
