#![cfg(target_os = "macos")]
//! macOS platform implementation for GPUI.
//!
//! macOS screens have a y axis that goes up from the bottom of the screen and
//! an origin at the bottom left of the main display.

mod dispatcher;
mod display;
mod display_link;
mod events;
mod font_rasterizer;
mod haptic_feedback;
mod keyboard;
mod pasteboard;
mod system_notifications;

#[cfg(feature = "screen-capture")]
mod screen_capture;

use gpui_apple::metal_renderer as renderer;

mod platform;
mod window;
mod window_appearance;

pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use display_link::*;
pub(crate) use font_rasterizer::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use window::*;

pub use platform::MacPlatform;
