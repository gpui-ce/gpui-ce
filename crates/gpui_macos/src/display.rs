use crate::ns_string;
use anyhow::Result;
use core_foundation::base::CFRelease;
use core_foundation::uuid::{CFUUIDGetUUIDBytes, CFUUIDRef};
use core_graphics::display::{CGDirectDisplayID, CGDisplayBounds, CGGetActiveDisplayList};
use core_graphics::geometry::CGRect;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, point, px, size};
use objc::{class, msg_send, runtime::Object, sel, sel_impl};
use std::ptr;
use uuid::Uuid;

type Id = *mut Object;

#[derive(Debug)]
pub(crate) struct MacDisplay(pub(crate) CGDirectDisplayID);

unsafe impl Send for MacDisplay {}

impl MacDisplay {
    /// Get the screen with the given [`DisplayId`].
    pub fn find_by_id(id: DisplayId) -> Option<Self> {
        Self::all().find(|screen| screen.id() == id)
    }

    /// Get the primary screen - the one with the menu bar, and whose bottom left
    /// corner is at the origin of the AppKit coordinate system.
    pub fn primary() -> Self {
        // Instead of iterating through all active systems displays via `all()` we use the first
        // NSScreen and gets its CGDirectDisplayID, because we can't be sure that `CGGetActiveDisplayList`
        // will always return a list of active displays (machine might be sleeping).
        //
        // The following is what Chromium does too:
        //
        // https://chromium.googlesource.com/chromium/src/+/66.0.3359.158/ui/display/mac/screen_mac.mm#56
        unsafe {
            let screens: Id = msg_send![class!(NSScreen), screens];
            let screen: Id = msg_send![screens, objectAtIndex: 0usize];
            let device_description: Id = msg_send![screen, deviceDescription];
            let screen_number_key: Id = ns_string("NSScreenNumber");
            let screen_number: Id = msg_send![device_description, objectForKey: screen_number_key];
            let screen_number: CGDirectDisplayID = msg_send![screen_number, unsignedIntegerValue];
            Self(screen_number)
        }
    }

    /// Obtains an iterator over all currently active system displays.
    pub fn all() -> impl Iterator<Item = Self> {
        unsafe {
            // We're assuming there aren't more than 32 displays connected to the system.
            let mut displays = Vec::with_capacity(32);
            let mut display_count = 0;
            let result = CGGetActiveDisplayList(
                displays.capacity() as u32,
                displays.as_mut_ptr(),
                &mut display_count,
            );

            if result == 0 {
                displays.set_len(display_count as usize);
                displays.into_iter().map(MacDisplay)
            } else {
                panic!("Failed to get active display list. Result: {result}");
            }
        }
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayCreateUUIDFromDisplayID(display: CGDirectDisplayID) -> CFUUIDRef;
}

impl PlatformDisplay for MacDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(self.0 as u64)
    }

    fn uuid(&self) -> Result<Uuid> {
        let cfuuid = unsafe { CGDisplayCreateUUIDFromDisplayID(self.0 as CGDirectDisplayID) };
        anyhow::ensure!(
            !cfuuid.is_null(),
            "AppKit returned a null from CGDisplayCreateUUIDFromDisplayID"
        );

        let bytes = unsafe { CFUUIDGetUUIDBytes(cfuuid) };
        unsafe { CFRelease(cfuuid as _) };
        Ok(Uuid::from_bytes([
            bytes.byte0,
            bytes.byte1,
            bytes.byte2,
            bytes.byte3,
            bytes.byte4,
            bytes.byte5,
            bytes.byte6,
            bytes.byte7,
            bytes.byte8,
            bytes.byte9,
            bytes.byte10,
            bytes.byte11,
            bytes.byte12,
            bytes.byte13,
            bytes.byte14,
            bytes.byte15,
        ]))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        unsafe {
            // CGDisplayBounds is in "global display" coordinates, where 0 is
            // the top left of the primary display.
            let bounds = CGDisplayBounds(self.0);

            Bounds {
                origin: Default::default(),
                size: size(px(bounds.size.width as f32), px(bounds.size.height as f32)),
            }
        }
    }

    fn visible_bounds(&self) -> Bounds<Pixels> {
        unsafe {
            let dominated_screen = self.get_nsscreen();

            if dominated_screen.is_null() {
                return self.bounds();
            }

            let screen_frame: CGRect = msg_send![dominated_screen, frame];
            let visible_frame: CGRect = msg_send![dominated_screen, visibleFrame];

            // Convert from bottom-left origin (AppKit) to top-left origin
            let origin_y =
                screen_frame.size.height - visible_frame.origin.y - visible_frame.size.height
                    + screen_frame.origin.y;

            Bounds {
                origin: point(
                    px(visible_frame.origin.x as f32 - screen_frame.origin.x as f32),
                    px(origin_y as f32),
                ),
                size: size(
                    px(visible_frame.size.width as f32),
                    px(visible_frame.size.height as f32),
                ),
            }
        }
    }
}

impl MacDisplay {
    /// Find the NSScreen corresponding to this display
    unsafe fn get_nsscreen(&self) -> Id {
        let screens: Id = unsafe { msg_send![class!(NSScreen), screens] };
        let count: usize = unsafe { msg_send![screens, count] };
        let screen_number_key: Id = unsafe { ns_string("NSScreenNumber") };

        for i in 0..count {
            let screen: Id = unsafe { msg_send![screens, objectAtIndex: i] };
            let device_description: Id = unsafe { msg_send![screen, deviceDescription] };
            let screen_number: Id =
                unsafe { msg_send![device_description, objectForKey: screen_number_key] };
            let screen_id: CGDirectDisplayID = msg_send![screen_number, unsignedIntegerValue];
            if screen_id == self.0 {
                return screen;
            }
        }
        ptr::null_mut()
    }
}
