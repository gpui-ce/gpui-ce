use gpui::WindowAppearance;
use objc::{msg_send, runtime::Object, sel, sel_impl};
use std::ffi::{CStr, c_char};

type Id = *mut Object;

pub(crate) unsafe fn window_appearance_from_native(appearance: Id) -> WindowAppearance {
    let name: Id = msg_send![appearance, name];
    unsafe {
        if name == NSAppearanceNameVibrantLight {
            WindowAppearance::VibrantLight
        } else if name == NSAppearanceNameVibrantDark {
            WindowAppearance::VibrantDark
        } else if name == NSAppearanceNameAqua {
            WindowAppearance::Light
        } else if name == NSAppearanceNameDarkAqua {
            WindowAppearance::Dark
        } else {
            println!("unknown appearance: {:?}", {
                let utf8: *const c_char = msg_send![name, UTF8String];
                CStr::from_ptr(utf8)
            });
            WindowAppearance::Light
        }
    }
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    pub static NSAppearanceNameAqua: Id;
    pub static NSAppearanceNameDarkAqua: Id;
    pub static NSAppearanceNameVibrantLight: Id;
    pub static NSAppearanceNameVibrantDark: Id;
}
