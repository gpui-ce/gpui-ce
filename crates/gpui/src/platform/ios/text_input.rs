//! iOS text input handling.
//!
//! This module provides keyboard input support for iOS, implementing the full
//! UITextInput protocol for proper software keyboard and IME support.
//!
//! Key components:
//! - GPUITextPosition: UITextPosition subclass wrapping a UTF-16 index
//! - GPUITextRange: UITextRange subclass wrapping start/end positions
//! - Key code mapping for hardware keyboards
//! - UITextInput protocol method implementations (in window.rs)

use crate::{KeyDownEvent, Keystroke, Modifiers, PlatformInput};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Sel, BOOL, NO, YES},
    sel, sel_impl,
};
use std::sync::Once;

// Ivar names for GPUITextPosition
const GPUI_POSITION_INDEX_IVAR: &str = "index";

// Ivar names for GPUITextRange
const GPUI_RANGE_START_IVAR: &str = "start";
const GPUI_RANGE_END_IVAR: &str = "end";

static TEXT_POSITION_CLASS_REGISTERED: Once = Once::new();
static TEXT_RANGE_CLASS_REGISTERED: Once = Once::new();

// Cache the registered classes to avoid repeated class!() lookups.
// This avoids potential race conditions where class!() is called before
// registration is complete.
static mut TEXT_POSITION_CLASS: Option<&'static Class> = None;
static mut TEXT_RANGE_CLASS: Option<&'static Class> = None;

/// Register GPUITextPosition - a UITextPosition subclass that wraps a UTF-16 index.
///
/// iOS requires UITextPosition subclasses for the UITextInput protocol.
/// Raw integers cannot be used directly.
///
/// This function caches the class reference to avoid repeated class!() lookups,
/// which could fail if called during registration.
pub fn register_text_position_class() -> &'static Class {
    TEXT_POSITION_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UITextPosition);
        let mut decl = ClassDecl::new("GPUITextPosition", superclass).unwrap();

        // Store UTF-16 index
        decl.add_ivar::<usize>(GPUI_POSITION_INDEX_IVAR);

        // Equality check - required for proper position comparison
        extern "C" fn is_equal(this: &Object, _sel: Sel, other: *mut Object) -> BOOL {
            unsafe {
                if other.is_null() {
                    return NO;
                }
                // Check if other is a GPUITextPosition - use cached class
                let other_class: *const Class = msg_send![other, class];
                let our_class = register_text_position_class();
                if other_class as *const _ != our_class as *const _ {
                    return NO;
                }
                let this_index: usize = *this.get_ivar(GPUI_POSITION_INDEX_IVAR);
                let other_index: usize = *(&*other).get_ivar(GPUI_POSITION_INDEX_IVAR);
                if this_index == other_index {
                    YES
                } else {
                    NO
                }
            }
        }

        // Hash method for dictionary keys
        extern "C" fn hash(this: &Object, _sel: Sel) -> usize {
            unsafe {
                let index: usize = *this.get_ivar(GPUI_POSITION_INDEX_IVAR);
                index
            }
        }

        unsafe {
            decl.add_method(
                sel!(isEqual:),
                is_equal as extern "C" fn(&Object, Sel, *mut Object) -> BOOL,
            );
            decl.add_method(sel!(hash), hash as extern "C" fn(&Object, Sel) -> usize);
        }

        let registered_class = decl.register();
        // Cache the class pointer - safe because registration happens only once
        // and the class lives for the entire program lifetime.
        unsafe {
            TEXT_POSITION_CLASS = Some(registered_class);
        }
    });

    // Return cached class - safe because Once::call_once guarantees
    // the closure has completed before we reach this point.
    unsafe {
        TEXT_POSITION_CLASS.expect("GPUITextPosition class should be registered")
    }
}

/// Create a GPUITextPosition with the given UTF-16 index.
pub fn create_text_position(index: usize) -> *mut Object {
    unsafe {
        let class = register_text_position_class();
        let position: *mut Object = msg_send![class, alloc];
        let position: *mut Object = msg_send![position, init];
        (*position).set_ivar(GPUI_POSITION_INDEX_IVAR, index);
        position
    }
}

/// Get the UTF-16 index from a GPUITextPosition.
pub fn get_position_index(position: *mut Object) -> Option<usize> {
    if position.is_null() {
        return None;
    }
    unsafe {
        let index: usize = *(&*position).get_ivar(GPUI_POSITION_INDEX_IVAR);
        Some(index)
    }
}

/// Register GPUITextRange - a UITextRange subclass wrapping start/end UTF-16 indices.
///
/// iOS requires UITextRange subclasses for the UITextInput protocol.
///
/// This function caches the class reference to avoid repeated class!() lookups,
/// which could fail if called during registration.
pub fn register_text_range_class() -> &'static Class {
    // Ensure position class is registered first
    register_text_position_class();

    TEXT_RANGE_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UITextRange);
        let mut decl = ClassDecl::new("GPUITextRange", superclass).unwrap();

        decl.add_ivar::<usize>(GPUI_RANGE_START_IVAR);
        decl.add_ivar::<usize>(GPUI_RANGE_END_IVAR);

        // start property - returns GPUITextPosition
        extern "C" fn get_start(this: &Object, _sel: Sel) -> *mut Object {
            unsafe {
                let start: usize = *this.get_ivar(GPUI_RANGE_START_IVAR);
                create_text_position(start)
            }
        }

        // end property - returns GPUITextPosition
        extern "C" fn get_end(this: &Object, _sel: Sel) -> *mut Object {
            unsafe {
                let end: usize = *this.get_ivar(GPUI_RANGE_END_IVAR);
                create_text_position(end)
            }
        }

        // isEmpty property - required by UITextRange
        extern "C" fn is_empty(this: &Object, _sel: Sel) -> BOOL {
            unsafe {
                let start: usize = *this.get_ivar(GPUI_RANGE_START_IVAR);
                let end: usize = *this.get_ivar(GPUI_RANGE_END_IVAR);
                if start == end {
                    YES
                } else {
                    NO
                }
            }
        }

        unsafe {
            decl.add_method(
                sel!(start),
                get_start as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(end),
                get_end as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(isEmpty),
                is_empty as extern "C" fn(&Object, Sel) -> BOOL,
            );
        }

        let registered_class = decl.register();
        // Cache the class pointer - safe because registration happens only once
        // and the class lives for the entire program lifetime.
        unsafe {
            TEXT_RANGE_CLASS = Some(registered_class);
        }
    });

    // Return cached class - safe because Once::call_once guarantees
    // the closure has completed before we reach this point.
    unsafe {
        TEXT_RANGE_CLASS.expect("GPUITextRange class should be registered")
    }
}

/// Pre-register all text input classes.
///
/// Call this early during window initialization to ensure classes are registered
/// before iOS tries to use them. This avoids race conditions where iOS queries
/// UITextInput methods (like markedTextRange) before classes are ready.
pub fn ensure_text_input_classes_registered() {
    register_text_position_class();
    register_text_range_class();
}

/// Create a GPUITextRange with start and end UTF-16 indices.
pub fn create_text_range(start: usize, end: usize) -> *mut Object {
    unsafe {
        let class = register_text_range_class();
        let range: *mut Object = msg_send![class, alloc];
        let range: *mut Object = msg_send![range, init];
        (*range).set_ivar(GPUI_RANGE_START_IVAR, start);
        (*range).set_ivar(GPUI_RANGE_END_IVAR, end);
        range
    }
}

/// Get the start and end indices from a GPUITextRange.
pub fn get_range_indices(range: *mut Object) -> Option<(usize, usize)> {
    if range.is_null() {
        return None;
    }
    unsafe {
        let start: usize = *(&*range).get_ivar(GPUI_RANGE_START_IVAR);
        let end: usize = *(&*range).get_ivar(GPUI_RANGE_END_IVAR);
        Some((start, end))
    }
}

/// Convert a key code from UIKeyboardHIDUsage to a GPUI key string.
///
/// UIKeyboardHIDUsage values are based on the USB HID specification.
pub fn key_code_to_string(code: u32) -> String {
    match code {
        // Letters (0x04-0x1D = a-z)
        0x04..=0x1D => {
            let letter = (b'a' + (code - 0x04) as u8) as char;
            letter.to_string()
        }
        // Numbers (0x1E-0x27 = 1-9, 0)
        0x1E..=0x26 => {
            let num = ((code - 0x1E + 1) % 10) as u8 + b'0';
            (num as char).to_string()
        }
        0x27 => "0".to_string(),
        // Special keys
        0x28 => "enter".to_string(),
        0x29 => "escape".to_string(),
        0x2A => "backspace".to_string(),
        0x2B => "tab".to_string(),
        0x2C => " ".to_string(),
        0x2D => "-".to_string(),
        0x2E => "=".to_string(),
        0x2F => "[".to_string(),
        0x30 => "]".to_string(),
        0x31 => "\\".to_string(),
        0x33 => ";".to_string(),
        0x34 => "'".to_string(),
        0x35 => "`".to_string(),
        0x36 => ",".to_string(),
        0x37 => ".".to_string(),
        0x38 => "/".to_string(),
        // Arrow keys
        0x4F => "right".to_string(),
        0x50 => "left".to_string(),
        0x51 => "down".to_string(),
        0x52 => "up".to_string(),
        // Function keys
        0x3A => "f1".to_string(),
        0x3B => "f2".to_string(),
        0x3C => "f3".to_string(),
        0x3D => "f4".to_string(),
        0x3E => "f5".to_string(),
        0x3F => "f6".to_string(),
        0x40 => "f7".to_string(),
        0x41 => "f8".to_string(),
        0x42 => "f9".to_string(),
        0x43 => "f10".to_string(),
        0x44 => "f11".to_string(),
        0x45 => "f12".to_string(),
        // Other special keys
        0x49 => "insert".to_string(),
        0x4A => "home".to_string(),
        0x4B => "pageup".to_string(),
        0x4C => "delete".to_string(),
        0x4D => "end".to_string(),
        0x4E => "pagedown".to_string(),
        // Default
        _ => format!("unknown-{:02x}", code),
    }
}

/// Convert UIKeyModifierFlags to GPUI Modifiers.
///
/// UIKeyModifierFlags:
/// - alphaShift (caps lock): 1 << 16
/// - shift: 1 << 17
/// - control: 1 << 18
/// - alternate (option): 1 << 19
/// - command: 1 << 20
/// - numericPad: 1 << 21
pub fn modifier_flags_to_modifiers(flags: u32) -> Modifiers {
    const SHIFT: u32 = 1 << 17;
    const CONTROL: u32 = 1 << 18;
    const ALT: u32 = 1 << 19;
    const COMMAND: u32 = 1 << 20;

    Modifiers {
        control: flags & CONTROL != 0,
        alt: flags & ALT != 0,
        shift: flags & SHIFT != 0,
        platform: flags & COMMAND != 0,
        function: false,
    }
}

/// Create a key down event from a character.
pub fn character_to_key_down(c: char) -> PlatformInput {
    let keystroke = Keystroke {
        modifiers: Modifiers::default(),
        key: c.to_string(),
        key_char: Some(c.to_string()),
    };

    PlatformInput::KeyDown(KeyDownEvent {
        keystroke,
        is_held: false,
        prefer_character_input: true,
    })
}

/// Create a backspace key down event.
pub fn backspace_key_down() -> PlatformInput {
    let keystroke = Keystroke {
        modifiers: Modifiers::default(),
        key: "backspace".to_string(),
        key_char: None,
    };

    PlatformInput::KeyDown(KeyDownEvent {
        keystroke,
        is_held: false,
        prefer_character_input: false,
    })
}

/// Create a key down event from a key code and modifiers.
pub fn key_code_to_key_down(key_code: u32, modifier_flags: u32) -> PlatformInput {
    let modifiers = modifier_flags_to_modifiers(modifier_flags);
    let key = key_code_to_string(key_code);

    let key_char = if key.len() == 1 {
        Some(key.clone())
    } else {
        None
    };

    let keystroke = Keystroke {
        modifiers,
        key,
        key_char,
    };

    PlatformInput::KeyDown(KeyDownEvent {
        keystroke,
        is_held: false,
        prefer_character_input: false,
    })
}

/// Create a key up event from a key code and modifiers.
pub fn key_code_to_key_up(key_code: u32, modifier_flags: u32) -> PlatformInput {
    let modifiers = modifier_flags_to_modifiers(modifier_flags);
    let key = key_code_to_string(key_code);

    let key_char = if key.len() == 1 {
        Some(key.clone())
    } else {
        None
    };

    let keystroke = Keystroke {
        modifiers,
        key,
        key_char,
    };

    PlatformInput::KeyUp(crate::KeyUpEvent { keystroke })
}
