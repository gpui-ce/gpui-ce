#![cfg(target_os = "macos")]
//! macOS platform implementation for GPUI.
//!
//! macOS screens have a y axis that goes up from the bottom of the screen and
//! an origin at the bottom left of the main display.

mod dispatcher;
mod display;
mod display_link;
mod events;
mod haptic_feedback;
mod keyboard;
mod pasteboard;
mod system_notifications;

#[cfg(feature = "screen-capture")]
mod screen_capture;

use gpui_apple::metal_renderer as renderer;

#[cfg(feature = "font-kit")]
mod open_type;

#[cfg(feature = "font-kit")]
mod text_system;

mod platform;
mod window;
mod window_appearance;

pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use display_link::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use window::*;

#[cfg(feature = "font-kit")]
pub(crate) use text_system::*;

pub use platform::MacPlatform;
