#![cfg(target_os = "macos")]
#![allow(deprecated)]
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

use gpui_apple::metal_renderer as renderer;

pub mod metal_renderer {
    pub use gpui_apple::metal_renderer::{PathRasterizationVertex, PathSprite, SurfaceBounds};

    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    pub use gpui_apple::metal_renderer::MetalHeadlessRenderer;
}

#[cfg(feature = "font-kit")]
mod open_type;

#[cfg(feature = "font-kit")]
mod text_system;

mod platform;
mod window;
mod window_appearance;

pub(crate) use gpui::collections;

use objc::{
    class, msg_send,
    runtime::{BOOL, NO, Object, YES},
    sel, sel_impl,
};
use std::{
    ffi::{CStr, c_char},
    ops::Range,
};

pub(crate) type Id = *mut Object;
pub(crate) type NSUInteger = usize;
pub(crate) const NS_NOT_FOUND: NSUInteger = NSUInteger::MAX;

pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use display_link::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use window::*;

#[cfg(feature = "font-kit")]
pub(crate) use text_system::*;

pub use platform::MacPlatform;

trait BoolExt {
    fn to_objc(self) -> BOOL;
}

impl BoolExt for bool {
    fn to_objc(self) -> BOOL {
        if self { YES } else { NO }
    }
}

trait NSStringExt {
    unsafe fn to_str(&self) -> &str;
}

impl NSStringExt for Id {
    unsafe fn to_str(&self) -> &str {
        unsafe {
            let cstr: *const c_char = msg_send![*self, UTF8String];
            if cstr.is_null() {
                ""
            } else {
                CStr::from_ptr(cstr).to_str().unwrap()
            }
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct NSRange {
    pub location: NSUInteger,
    pub length: NSUInteger,
}

impl NSRange {
    fn invalid() -> Self {
        Self {
            location: NS_NOT_FOUND,
            length: 0,
        }
    }

    fn is_valid(&self) -> bool {
        self.location != NS_NOT_FOUND
    }

    fn to_range(self) -> Option<Range<usize>> {
        if self.is_valid() {
            let start = self.location;
            let end = start + self.length;
            Some(start..end)
        } else {
            None
        }
    }
}

impl From<Range<usize>> for NSRange {
    fn from(range: Range<usize>) -> Self {
        NSRange {
            location: range.start as NSUInteger,
            length: range.len() as NSUInteger,
        }
    }
}

unsafe impl objc::Encode for NSRange {
    fn encode() -> objc::Encoding {
        let encoding = format!(
            "{{NSRange={}{}}}",
            NSUInteger::encode().as_str(),
            NSUInteger::encode().as_str()
        );
        unsafe { objc::Encoding::from_str(&encoding) }
    }
}

/// Allow NSString::alloc use here because it sets autorelease
#[allow(clippy::disallowed_methods)]
unsafe fn ns_string(string: &str) -> Id {
    unsafe {
        let value: Id = msg_send![class!(NSString), alloc];
        let value: Id =
            msg_send![value, initWithBytes: string.as_ptr() length: string.len() encoding: 4usize];
        msg_send![value, autorelease]
    }
}
