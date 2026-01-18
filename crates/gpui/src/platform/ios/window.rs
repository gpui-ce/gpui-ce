//! iOS Window implementation using UIWindow and UIViewController.
//!
//! iOS windows are fundamentally different from desktop windows:
//! - Always fullscreen (or split-screen on iPad)
//! - No title bar or window chrome
//! - Touch-based input
//! - Safe area insets for notch/home indicator
//!
//! The window is backed by a UIWindow containing a UIViewController
//! whose view hosts the Metal rendering layer.

use super::{IosDisplay, events::*, text_input, text_input::{create_text_position, create_text_range, get_position_index, get_range_indices}};
use crate::platform::blade;
use crate::{
    AnyWindowHandle, Bounds, DispatchEventResult, GpuSpecs, Modifiers, Pixels, PlatformAtlas,
    PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformWindow, Point, PromptButton,
    PromptLevel, RequestFrameOptions, Scene, Size, WindowAppearance, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowParams, px,
};
use anyhow::Result;
use core_graphics::{
    base::CGFloat,
    geometry::{CGPoint, CGRect, CGSize},
};
use objc::{
    class,
    declare::ClassDecl,
    msg_send, Encode, Encoding,
    runtime::{BOOL, Class, NO, Object, Sel, YES},
    sel, sel_impl,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, UiKitDisplayHandle, UiKitWindowHandle};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    ptr::{self, NonNull},
    rc::Rc,
    sync::Arc,
};

const GPUI_VIEW_IVAR: &str = "gpui_view";
const GPUI_WINDOW_IVAR: &str = "gpui_window_ptr";

/// NSRange structure for Objective-C interop
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct NSRange {
    location: u64,
    length: u64,
}

// Implement Encode for NSRange to allow it to be used in Objective-C method signatures
unsafe impl Encode for NSRange {
    fn encode() -> Encoding {
        // NSRange is a struct with two unsigned longs: {_NSRange=QQ}
        unsafe { Encoding::from_str("{_NSRange=QQ}") }
    }
}

/// Our own CGRect struct for use in Objective-C method signatures (implements Encode)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct IOSCGRect {
    origin: IOSCGPoint,
    size: IOSCGSize,
}

impl IOSCGRect {
    fn new(origin: IOSCGPoint, size: IOSCGSize) -> Self {
        Self { origin, size }
    }
}

unsafe impl Encode for IOSCGRect {
    fn encode() -> Encoding {
        // CGRect is a struct with origin (CGPoint) and size (CGSize): {CGRect={CGPoint=dd}{CGSize=dd}}
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

/// Our own CGPoint struct for use in Objective-C method signatures (implements Encode)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct IOSCGPoint {
    x: f64,
    y: f64,
}

impl IOSCGPoint {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

unsafe impl Encode for IOSCGPoint {
    fn encode() -> Encoding {
        // CGPoint is a struct with x and y doubles: {CGPoint=dd}
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}

/// Our own CGSize struct for use in Objective-C method signatures (implements Encode)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct IOSCGSize {
    width: f64,
    height: f64,
}

impl IOSCGSize {
    fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

unsafe impl Encode for IOSCGSize {
    fn encode() -> Encoding {
        // CGSize is a struct with width and height doubles: {CGSize=dd}
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}

/// NSNotFound constant (max value indicates "not found")
const NS_NOT_FOUND: u64 = u64::MAX;

static METAL_VIEW_CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

// Declare NSLog extern once at module level
unsafe extern "C" {
    fn NSLog(format: *mut Object, ...);
}

/// Helper to log to iOS system log via NSLog.
/// This ensures messages show up in `xcrun simctl spawn ... log stream`.
#[allow(unused)]
fn ios_log(message: &str) {
    unsafe {
        // Create NSString from our message
        let ns_string: *mut Object = msg_send![class!(NSString), alloc];
        let format: *mut Object = msg_send![ns_string, initWithUTF8String: message.as_ptr() as *const std::os::raw::c_char];

        NSLog(format);

        // Release the string
        let _: () = msg_send![format, release];
    }
}

/// Helper to log to iOS system log with a C string literal.
/// This is simpler and doesn't require allocation.
fn ios_log_cstr(message: &std::ffi::CStr) {
    unsafe {
        let ns_string: *mut Object = msg_send![class!(NSString), stringWithUTF8String: message.as_ptr()];
        NSLog(ns_string);
    }
}

/// Helper to log a formatted string to iOS system log.
/// Creates a proper null-terminated C string.
fn ios_log_format(message: &str) {
    let c_string = std::ffi::CString::new(message).unwrap_or_else(|_| std::ffi::CString::new("GPUI iOS: <invalid log message>").unwrap());
    unsafe {
        let ns_string: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c_string.as_ptr()];
        NSLog(ns_string);
    }
}

/// Safely access the input handler for a view.
///
/// IMPORTANT: This function takes the handler OUT of the RefCell before
/// executing the callback, then restores it afterward. This matches the
/// macOS pattern (see mac/window.rs:2492-2506) and prevents borrow conflicts
/// when iOS calls multiple UITextInput methods simultaneously during
/// keyboard initialization.
///
/// The pattern is:
/// 1. Borrow the RefCell and take() the handler out (leaving None)
/// 2. Drop the borrow (releasing the RefCell)
/// 3. Execute callback with exclusive access to the handler
/// 4. Re-borrow and restore the handler
///
/// This ensures no borrow is held during callback execution, allowing
/// re-entrant calls to succeed.
fn with_input_handler<F, R>(view: &Object, f: F) -> Option<R>
where
    F: FnOnce(&mut PlatformInputHandler) -> R,
{
    unsafe {
        let window_ptr: *mut std::ffi::c_void = *view.get_ivar(GPUI_WINDOW_IVAR);
        if window_ptr.is_null() {
            ios_log_cstr(c"GPUI iOS: with_input_handler - window_ptr is null");
            return None;
        }

        let window = &*(window_ptr as *const IosWindow);

        // Take the handler out of the RefCell in a scoped block.
        // This releases the borrow before callback execution.
        let mut handler = {
            let Ok(mut borrow) = window.input_handler.try_borrow_mut() else {
                ios_log_cstr(c"GPUI iOS: with_input_handler - BORROW CONFLICT during take!");
                return None;
            };
            let taken = borrow.take();
            if taken.is_none() {
                ios_log_cstr(c"GPUI iOS: with_input_handler - no handler set (None)");
            }
            taken
        }?;
        // Borrow is now released - handler is owned, not borrowed

        // Execute callback with exclusive access to handler
        let result = f(&mut handler);

        // Restore handler back into RefCell
        {
            let Ok(mut borrow) = window.input_handler.try_borrow_mut() else {
                // This should never happen since we released the borrow above,
                // but log an error if it does
                ios_log_cstr(c"GPUI iOS: with_input_handler - BORROW CONFLICT during restore!");
                return Some(result);
            };
            *borrow = Some(handler);
        }

        Some(result)
    }
}

/// Register a custom UIView subclass that uses CAMetalLayer as its backing layer.
/// This is required for Metal rendering on iOS.
fn register_metal_view_class() -> &'static Class {
    METAL_VIEW_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UIView);
        let mut decl = ClassDecl::new("GPUIMetalView", superclass).unwrap();

        // Add ivar to store window pointer for touch handling
        decl.add_ivar::<*mut std::ffi::c_void>(GPUI_WINDOW_IVAR);

        // CRITICAL: Declare protocol conformance for text input
        // Without this, iOS won't recognize the view as a text input view
        // and won't show the keyboard or send text input events
        // Note: UITextInput inherits from UIKeyInput and UITextInputTraits,
        // so we only need to add UITextInput
        unsafe {
            use objc::runtime::Protocol;
            // UITextInput includes UIKeyInput and UITextInputTraits
            if let Some(protocol) = Protocol::get("UITextInput") {
                decl.add_protocol(protocol);
                println!("GPUI iOS: Added UITextInput protocol to GPUIMetalView");
            } else {
                println!("GPUI iOS: Failed to get UITextInput protocol!");
            }
        }

        // Override layerClass to return CAMetalLayer
        extern "C" fn layer_class(_self: &Class, _sel: Sel) -> *const Class {
            class!(CAMetalLayer) as *const Class
        }

        // Touch handling methods
        extern "C" fn touches_began(
            this: &mut Object,
            _sel: Sel,
            touches: *mut Object,
            event: *mut Object,
        ) {
            handle_touches(this, touches, event);
        }

        extern "C" fn touches_moved(
            this: &mut Object,
            _sel: Sel,
            touches: *mut Object,
            event: *mut Object,
        ) {
            handle_touches(this, touches, event);
        }

        extern "C" fn touches_ended(
            this: &mut Object,
            _sel: Sel,
            touches: *mut Object,
            event: *mut Object,
        ) {
            handle_touches(this, touches, event);
        }

        extern "C" fn touches_cancelled(
            this: &mut Object,
            _sel: Sel,
            touches: *mut Object,
            event: *mut Object,
        ) {
            handle_touches(this, touches, event);
        }

        // Make view focusable for keyboard input
        extern "C" fn can_become_first_responder(_this: &Object, _sel: Sel) -> bool {
            true
        }

        // UITextInputTraits - keyboard type (default)
        extern "C" fn keyboard_type(_this: &Object, _sel: Sel) -> i64 {
            0 // UIKeyboardTypeDefault
        }

        // UITextInputTraits - return key type
        extern "C" fn return_key_type(_this: &Object, _sel: Sel) -> i64 {
            0 // UIReturnKeyDefault
        }

        // UITextInputTraits - autocapitalization type
        extern "C" fn autocapitalization_type(_this: &Object, _sel: Sel) -> i64 {
            0 // UITextAutocapitalizationTypeNone
        }

        // UITextInputTraits - autocorrection type
        extern "C" fn autocorrection_type(_this: &Object, _sel: Sel) -> i64 {
            1 // UITextAutocorrectionTypeNo
        }

        // UITextInputTraits - smart quotes type (disable smart quotes)
        // UITextSmartQuotesType: 0=Default, 1=No, 2=Yes
        extern "C" fn smart_quotes_type(_this: &Object, _sel: Sel) -> i64 {
            1 // UITextSmartQuotesTypeNo
        }

        // UITextInputTraits - smart dashes type (disable smart dashes)
        // UITextSmartDashesType: 0=Default, 1=No, 2=Yes
        extern "C" fn smart_dashes_type(_this: &Object, _sel: Sel) -> i64 {
            1 // UITextSmartDashesTypeNo
        }

        // UITextInputTraits - smart insert delete type (disable double-space to period)
        // UITextSmartInsertDeleteType: 0=Default, 1=No, 2=Yes
        extern "C" fn smart_insert_delete_type(_this: &Object, _sel: Sel) -> i64 {
            1 // UITextSmartInsertDeleteTypeNo
        }

        // UITextInputTraits - spell checking type (disable spell checking)
        // UITextSpellCheckingType: 0=Default, 1=No, 2=Yes
        extern "C" fn spell_checking_type(_this: &Object, _sel: Sel) -> i64 {
            1 // UITextSpellCheckingTypeNo
        }

        // Tell iOS we want to receive keyboard input
        extern "C" fn is_user_interaction_enabled(_this: &Object, _sel: Sel) -> bool {
            true
        }

        // UIKeyInput protocol - hasText
        extern "C" fn has_text(_this: &Object, _sel: Sel) -> bool {
            ios_log_cstr(c"GPUI iOS: hasText called - returning true");
            true // Always return true to receive text input
        }

        // UIKeyInput protocol - insertText:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        // Note: iOS sends printable characters ONLY through insertText, not pressesBegan,
        // so we always process insertText (no hardware key blocking needed here).
        extern "C" fn insert_text(this: &mut Object, _sel: Sel, text: *mut Object) {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                ios_log_cstr(c"GPUI iOS: insertText called");
                unsafe {
                    // Get the string from the NSString first (before any handler access)
                    let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
                    if utf8.is_null() {
                        ios_log_cstr(c"GPUI iOS: insertText - UTF8String is null!");
                        return;
                    }

                    let text_str = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
                    ios_log_format(&format!("GPUI iOS: insertText - got text: {:?}", text_str));

                    // First try the input handler directly (for text fields)
                    // This is the preferred path for software keyboard input.
                    // Uses with_input_handler which releases the borrow during callback
                    // to prevent conflicts when iOS queries multiple UITextInput methods.
                    if with_input_handler(this, |handler| {
                        ios_log_cstr(c"GPUI iOS: insertText - handler found, calling replace_text_in_range");
                        handler.replace_text_in_range(None, &text_str);
                    }).is_some() {
                        ios_log_cstr(c"GPUI iOS: insertText - SUCCESS via input handler");
                        return;
                    }

                    ios_log_cstr(c"GPUI iOS: insertText - no handler, using key event fallback");
                    // Fallback: with_input_handler returned None (no handler set)
                    // Send as key events for non-input-handler scenarios
                    let window_ptr: *mut std::ffi::c_void = *this.get_ivar(GPUI_WINDOW_IVAR);
                    if window_ptr.is_null() {
                        ios_log_cstr(c"GPUI iOS: insertText - window pointer is null for fallback!");
                        return;
                    }
                    let window = &*(window_ptr as *const IosWindow);

                    for ch in text_str.chars() {
                        match ch {
                            '\n' | '\r' => {
                                window.handle_key_event(40, 0, true); // Return key code
                            }
                            _ => {
                                // Send as individual character key event
                                let keystroke = crate::Keystroke {
                                    modifiers: Modifiers::default(),
                                    key: ch.to_string(),
                                    key_char: Some(ch.to_string()),
                                };

                                let event = PlatformInput::KeyDown(crate::KeyDownEvent {
                                    keystroke,
                                    is_held: false,
                                    prefer_character_input: true,
                                });

                                if let Some(callback) = window.input_callback.borrow_mut().as_mut() {
                                    callback(event);
                                }
                            }
                        }
                    }
                    ios_log_cstr(c"GPUI iOS: insertText - sent via key event fallback");
                }
            }));
        }

        // UIKeyInput protocol - deleteBackward
        // This is the ONLY path for backspace deletion - we skip backspace in pressesBegan
        // to avoid duplicate handling.
        //
        // iOS sometimes calls deleteBackward twice in rapid succession for a single key press.
        // We use a debounce mechanism to ignore duplicate calls within a short time window.
        extern "C" fn delete_backward(this: &mut Object, _sel: Sel) {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::time::{SystemTime, UNIX_EPOCH};

            // Use a static counter to track how many times deleteBackward is called
            static CALL_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let call_id = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            // Debounce: track last deletion time to ignore rapid duplicate calls
            // This is necessary because iOS sometimes calls deleteBackward twice per key press.
            static LAST_DELETE_TIME: AtomicU64 = AtomicU64::new(0);
            const DEBOUNCE_MS: u64 = 100; // Ignore calls within 100ms of each other

            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                let last_time = LAST_DELETE_TIME.load(Ordering::SeqCst);
                let elapsed = now.saturating_sub(last_time);

                ios_log_format(&format!(
                    "GPUI iOS: deleteBackward [#{}] ENTRY - elapsed={}ms since last",
                    call_id, elapsed
                ));

                if elapsed < DEBOUNCE_MS && last_time > 0 {
                    ios_log_format(&format!(
                        "GPUI iOS: deleteBackward [#{}] SKIPPED - debounce ({}ms < {}ms)",
                        call_id, elapsed, DEBOUNCE_MS
                    ));
                    return;
                }

                // Track whether we actually performed a deletion
                let mut did_delete = false;

                // First try the input handler directly
                // This is the preferred path for software keyboard input.
                // Uses with_input_handler which releases the borrow during callback
                // to prevent conflicts when iOS queries multiple UITextInput methods.
                let handler_result = with_input_handler(this, |handler| {
                    ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - handler found", call_id));
                    // Get current selection
                    if let Some(selection) = handler.selected_text_range(false) {
                        ios_log_format(&format!(
                            "GPUI iOS: deleteBackward [#{}] - selection: {:?}, empty: {}",
                            call_id, selection.range, selection.range.is_empty()
                        ));
                        if selection.range.is_empty() {
                            // No selection - delete one character before cursor
                            if selection.range.start > 0 {
                                let delete_range = selection.range.start - 1..selection.range.start;
                                ios_log_format(&format!(
                                    "GPUI iOS: deleteBackward [#{}] - deleting range {:?} (UTF-16)",
                                    call_id, delete_range
                                ));
                                handler.replace_text_in_range(Some(delete_range), "");
                                ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - deleted one char", call_id));
                                return true; // Performed deletion
                            } else {
                                ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - at start, nothing to delete", call_id));
                            }
                        } else {
                            // Has selection - delete the selection
                            ios_log_format(&format!(
                                "GPUI iOS: deleteBackward [#{}] - deleting selection {:?}",
                                call_id, selection.range
                            ));
                            handler.replace_text_in_range(Some(selection.range), "");
                            ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - deleted selection", call_id));
                            return true; // Performed deletion
                        }
                    } else {
                        ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - no selection available", call_id));
                    }
                    false // No deletion performed
                });

                match handler_result {
                    Some(true) => {
                        did_delete = true;
                        ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - SUCCESS via input handler", call_id));
                    }
                    Some(false) => {
                        ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - handler found but no deletion needed", call_id));
                    }
                    None => {
                        ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - no handler, using key event fallback", call_id));
                        // Fallback: with_input_handler returned None (no handler set)
                        // Send as key event
                        unsafe {
                            let window_ptr: *mut std::ffi::c_void = *this.get_ivar(GPUI_WINDOW_IVAR);
                            if window_ptr.is_null() {
                                ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - window pointer is null for fallback!", call_id));
                                return;
                            }
                            let window = &*(window_ptr as *const IosWindow);
                            ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - sending backspace key event", call_id));
                            window.handle_key_event(0x2A, 0, true); // Backspace key code
                            did_delete = true;
                        }
                    }
                }

                // Update the last delete time after a successful deletion
                if did_delete {
                    LAST_DELETE_TIME.store(now, Ordering::SeqCst);
                    ios_log_format(&format!("GPUI iOS: deleteBackward [#{}] - updated last_delete_time to {}", call_id, now));
                }
            }));
        }

        // Hardware keyboard handling
        extern "C" fn presses_began(
            this: &mut Object,
            _sel: Sel,
            presses: *mut Object,
            event: *mut Object,
        ) {
            ios_log_cstr(c"GPUI iOS: pressesBegan - hardware key pressed");
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                handle_presses(this, presses, true);
            }));
            // Call super
            unsafe {
                let superclass = class!(UIView);
                let _: () = msg_send![super(this, superclass), pressesBegan: presses withEvent: event];
            }
        }

        extern "C" fn presses_ended(
            this: &mut Object,
            _sel: Sel,
            presses: *mut Object,
            event: *mut Object,
        ) {
            ios_log_cstr(c"GPUI iOS: pressesEnded - hardware key released");
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                handle_presses(this, presses, false);
            }));
            // Call super
            unsafe {
                let superclass = class!(UIView);
                let _: () = msg_send![super(this, superclass), pressesEnded: presses withEvent: event];
            }
        }

        // ============================================
        // UITextInput Protocol - Core Properties
        // ============================================

        // UITextInput - beginningOfDocument
        extern "C" fn beginning_of_document(_this: &Object, _sel: Sel) -> *mut Object {
            create_text_position(0)
        }

        // UITextInput - endOfDocument
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn end_of_document(this: &Object, _sel: Sel) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let len = with_input_handler(this, |handler| {
                    let mut adjusted = None;
                    handler.text_for_range(0..usize::MAX, &mut adjusted)
                        .map(|s| s.encode_utf16().count())
                        .unwrap_or(0)
                }).unwrap_or(0);
                create_text_position(len)
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => create_text_position(0), // Return position 0 on panic
            }
        }

        // UITextInput - selectedTextRange (CRITICAL - must not return nil!)
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn selected_text_range(this: &Object, _sel: Sel) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let range = with_input_handler(this, |handler| {
                    handler.selected_text_range(false)
                }).flatten();

                match range {
                    Some(selection) => create_text_range(
                        selection.range.start,
                        selection.range.end
                    ),
                    // Return empty range at start, never nil!
                    None => create_text_range(0, 0),
                }
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => create_text_range(0, 0), // Return empty range at start on panic
            }
        }

        // UITextInput - setSelectedTextRange:
        // iOS calls this to sync the cursor/selection position.
        // We need to be careful here:
        // - After deleteBackward: iOS sends the OLD position which would cause double-delete
        // - After insertText: iOS sends the NEW position which we need to track
        //
        // Solution: Only process if this is setting a cursor position (empty range),
        // and track it as a "pending" selection that we verify matches our internal state.
        extern "C" fn set_selected_text_range(_this: &mut Object, _sel: Sel, range: *mut Object) {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                if let Some((start, end)) = get_range_indices(range) {
                    ios_log_format(&format!(
                        "GPUI iOS: setSelectedTextRange called with {}..{} (ignored - cursor managed internally)",
                        start, end
                    ));
                    // We intentionally ignore this call.
                    // Our text operations (insertText, deleteBackward) already update the cursor.
                    // Calling replace_text_in_range here interferes with the deduplication logic
                    // and can cause issues with subsequent text insertions.
                } else {
                    ios_log_cstr(c"GPUI iOS: setSelectedTextRange called with nil range (ignored)");
                }
            }));
        }

        // UITextInput - markedTextRange (returns nil when no marked text)
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn marked_text_range(this: &Object, _sel: Sel) -> *mut Object {
            // Wrap in catch_unwind to prevent panics from unwinding through FFI boundary
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let range = with_input_handler(this, |handler| {
                    handler.marked_text_range()
                }).flatten();

                match range {
                    Some(r) => create_text_range(r.start, r.end),
                    None => std::ptr::null_mut(),
                }
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => std::ptr::null_mut(), // Return nil on panic
            }
        }

        // UITextInput - markedTextStyle (not used, return nil)
        extern "C" fn marked_text_style(_this: &Object, _sel: Sel) -> *mut Object {
            std::ptr::null_mut()
        }

        // UITextInput - setMarkedTextStyle: (not used)
        extern "C" fn set_marked_text_style(_this: &mut Object, _sel: Sel, _style: *mut Object) {
            // No-op
        }

        // UITextInput - inputDelegate (store reference to delegate)
        extern "C" fn input_delegate(_this: &Object, _sel: Sel) -> *mut Object {
            std::ptr::null_mut()
        }

        // UITextInput - setInputDelegate:
        extern "C" fn set_input_delegate(_this: &mut Object, _sel: Sel, _delegate: *mut Object) {
            // Could store delegate for notifications if needed
        }

        // UITextInput - tokenizer (use default string tokenizer)
        extern "C" fn tokenizer(this: &Object, _sel: Sel) -> *mut Object {
            unsafe {
                // Use UITextInputStringTokenizer as default
                let tokenizer: *mut Object = msg_send![class!(UITextInputStringTokenizer), alloc];
                let tokenizer: *mut Object = msg_send![tokenizer, initWithTextInput: this];
                tokenizer
            }
        }

        // ============================================
        // UITextInput Protocol - Text Manipulation
        // ============================================

        // UITextInput - textInRange:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn text_in_range(this: &Object, _sel: Sel, range: *mut Object) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some((start, end)) = get_range_indices(range) else {
                    return std::ptr::null_mut();
                };

                let text = with_input_handler(this, |handler| {
                    let mut adjusted = None;
                    handler.text_for_range(start..end, &mut adjusted)
                }).flatten();

                match text {
                    Some(s) => unsafe {
                        let c_str = std::ffi::CString::new(s).unwrap_or_default();
                        let ns_string: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c_str.as_ptr()];
                        ns_string
                    },
                    None => std::ptr::null_mut(),
                }
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => std::ptr::null_mut(),
            }
        }

        // UITextInput - replaceRange:withText:
        // This is called by iOS for smart punctuation and autocorrect
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn replace_range_with_text(this: &mut Object, _sel: Sel, range: *mut Object, text: *mut Object) {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some((start, end)) = get_range_indices(range) else {
                    ios_log_cstr(c"GPUI iOS: replaceRange:withText: - no valid range");
                    return;
                };

                unsafe {
                    let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
                    if utf8.is_null() {
                        ios_log_cstr(c"GPUI iOS: replaceRange:withText: - null text");
                        return;
                    }
                    let text_str = std::ffi::CStr::from_ptr(utf8).to_string_lossy();

                    ios_log_format(&format!(
                        "GPUI iOS: replaceRange:withText: range={}..{}, text={:?}",
                        start, end, text_str
                    ));

                    with_input_handler(this, |handler| {
                        handler.replace_text_in_range(Some(start..end), &text_str);
                    });
                }
            }));
        }

        // UITextInput - setMarkedText:selectedRange:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn set_marked_text(
            this: &mut Object,
            _sel: Sel,
            marked_text: *mut Object,
            selected_range: NSRange,
        ) {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                unsafe {
                    if marked_text.is_null() {
                        ios_log_cstr(c"GPUI iOS: setMarkedText - unmarking (null text)");
                        // Unmark text
                        with_input_handler(this, |handler| {
                            handler.unmark_text();
                        });
                        return;
                    }

                    // Check if it's NSAttributedString
                    let is_attributed: BOOL = msg_send![marked_text, isKindOfClass: class!(NSAttributedString)];
                    let text_obj: *mut Object = if is_attributed == YES {
                        msg_send![marked_text, string]
                    } else {
                        marked_text
                    };

                    let utf8: *const std::os::raw::c_char = msg_send![text_obj, UTF8String];
                    if utf8.is_null() {
                        ios_log_cstr(c"GPUI iOS: setMarkedText - null UTF8 string");
                        return;
                    }
                    let text_str = std::ffi::CStr::from_ptr(utf8).to_string_lossy();

                    ios_log_format(&format!(
                        "GPUI iOS: setMarkedText - text={:?}, selected_range={}..{}",
                        text_str, selected_range.location, selected_range.location + selected_range.length
                    ));

                    let selected = if selected_range.location != NS_NOT_FOUND {
                        Some(selected_range.location as usize..(selected_range.location + selected_range.length) as usize)
                    } else {
                        None
                    };

                    with_input_handler(this, |handler| {
                        handler.replace_and_mark_text_in_range(None, &text_str, selected);
                    });
                }
            }));
        }

        // UITextInput - unmarkText
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn unmark_text(this: &mut Object, _sel: Sel) {
            let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                with_input_handler(this, |handler| {
                    handler.unmark_text();
                });
            }));
        }

        // UITextInput - attributedSubstringFromRange: (for copy/paste preview)
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn attributed_substring_from_range(this: &Object, _sel: Sel, range: *mut Object) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some((start, end)) = get_range_indices(range) else {
                    return std::ptr::null_mut();
                };

                let text = with_input_handler(this, |handler| {
                    let mut adjusted = None;
                    handler.text_for_range(start..end, &mut adjusted)
                }).flatten();

                match text {
                    Some(s) => unsafe {
                        let c_str = std::ffi::CString::new(s).unwrap_or_default();
                        let ns_string: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c_str.as_ptr()];
                        let attributed: *mut Object = msg_send![class!(NSAttributedString), alloc];
                        let attributed: *mut Object = msg_send![attributed, initWithString: ns_string];
                        attributed
                    },
                    None => std::ptr::null_mut(),
                }
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => std::ptr::null_mut(),
            }
        }

        // ============================================
        // UITextInput Protocol - Position/Range Calculation
        // ============================================

        // UITextInput - positionFromPosition:offset:
        extern "C" fn position_from_position_offset(
            _this: &Object,
            _sel: Sel,
            position: *mut Object,
            offset: isize,
        ) -> *mut Object {
            let Some(index) = get_position_index(position) else {
                return std::ptr::null_mut();
            };

            let new_index = if offset >= 0 {
                index.saturating_add(offset as usize)
            } else {
                index.saturating_sub((-offset) as usize)
            };

            create_text_position(new_index)
        }

        // UITextInput - positionFromPosition:inDirection:offset:
        extern "C" fn position_from_position_in_direction(
            _this: &Object,
            _sel: Sel,
            position: *mut Object,
            direction: i64, // UITextLayoutDirection
            offset: isize,
        ) -> *mut Object {
            let Some(index) = get_position_index(position) else {
                return std::ptr::null_mut();
            };

            // Direction: 0=right, 1=left, 2=up, 3=down
            // For now, treat up/down same as left/right (simplified)
            let effective_offset = match direction {
                1 | 2 => -offset.abs(), // left/up = negative
                _ => offset.abs(),      // right/down = positive
            };

            let new_index = if effective_offset >= 0 {
                index.saturating_add(effective_offset as usize)
            } else {
                index.saturating_sub((-effective_offset) as usize)
            };

            create_text_position(new_index)
        }

        // UITextInput - textRangeFromPosition:toPosition:
        extern "C" fn text_range_from_position_to_position(
            _this: &Object,
            _sel: Sel,
            from: *mut Object,
            to: *mut Object,
        ) -> *mut Object {
            let Some(start) = get_position_index(from) else {
                return std::ptr::null_mut();
            };
            let Some(end) = get_position_index(to) else {
                return std::ptr::null_mut();
            };

            create_text_range(start.min(end), start.max(end))
        }

        // UITextInput - comparePosition:toPosition:
        extern "C" fn compare_position(
            _this: &Object,
            _sel: Sel,
            position: *mut Object,
            other: *mut Object,
        ) -> i64 { // NSComparisonResult
            let Some(a) = get_position_index(position) else {
                return 0; // NSOrderedSame
            };
            let Some(b) = get_position_index(other) else {
                return 0;
            };

            match a.cmp(&b) {
                std::cmp::Ordering::Less => -1,    // NSOrderedAscending
                std::cmp::Ordering::Equal => 0,    // NSOrderedSame
                std::cmp::Ordering::Greater => 1,  // NSOrderedDescending
            }
        }

        // UITextInput - offsetFromPosition:toPosition:
        extern "C" fn offset_from_position(
            _this: &Object,
            _sel: Sel,
            from: *mut Object,
            to: *mut Object,
        ) -> isize {
            let Some(start) = get_position_index(from) else {
                return 0;
            };
            let Some(end) = get_position_index(to) else {
                return 0;
            };

            (end as isize) - (start as isize)
        }

        // UITextInput - positionWithinRange:farthestInDirection:
        extern "C" fn position_within_range_farthest(
            _this: &Object,
            _sel: Sel,
            range: *mut Object,
            direction: i64,
        ) -> *mut Object {
            let Some((start, end)) = get_range_indices(range) else {
                return std::ptr::null_mut();
            };

            // Direction: 0=right, 1=left, 2=up, 3=down
            let index = match direction {
                1 | 2 => start, // left/up = start
                _ => end,       // right/down = end
            };

            create_text_position(index)
        }

        // UITextInput - characterRangeByExtendingPosition:inDirection:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn character_range_by_extending(
            this: &Object,
            _sel: Sel,
            position: *mut Object,
            direction: i64,
        ) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some(index) = get_position_index(position) else {
                    return std::ptr::null_mut();
                };

                // Get document length
                let doc_end = with_input_handler(this, |handler| {
                    let mut adjusted = None;
                    handler.text_for_range(0..usize::MAX, &mut adjusted)
                        .map(|s| s.encode_utf16().count())
                        .unwrap_or(0)
                }).unwrap_or(0);

                match direction {
                    1 | 2 => create_text_range(0, index),           // left/up - to beginning
                    _ => create_text_range(index, doc_end),         // right/down - to end
                }
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => std::ptr::null_mut(),
            }
        }

        // ============================================
        // UITextInput Protocol - Geometry Methods
        // ============================================

        // UITextInput - caretRectForPosition: (CRITICAL for keyboard to appear!)
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn caret_rect_for_position(this: &Object, _sel: Sel, position: *mut Object) -> IOSCGRect {
            let default_rect = IOSCGRect::new(IOSCGPoint::new(20.0, 100.0), IOSCGSize::new(2.0, 20.0));

            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some(index) = get_position_index(position) else {
                    return default_rect;
                };

                let bounds = with_input_handler(this, |handler| {
                    handler.bounds_for_range(index..index)
                }).flatten();

                match bounds {
                    Some(b) => IOSCGRect::new(
                        IOSCGPoint::new(b.origin.x.0 as f64, b.origin.y.0 as f64),
                        IOSCGSize::new(2.0, b.size.height.0 as f64), // 2px wide caret
                    ),
                    None => default_rect,
                }
            }));

            match result {
                Ok(rect) => rect,
                Err(_) => default_rect,
            }
        }

        // UITextInput - firstRectForRange:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn first_rect_for_range(this: &Object, _sel: Sel, range: *mut Object) -> IOSCGRect {
            let default_rect = IOSCGRect::new(IOSCGPoint::new(20.0, 100.0), IOSCGSize::new(100.0, 20.0));

            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let Some((start, end)) = get_range_indices(range) else {
                    return default_rect;
                };

                let bounds = with_input_handler(this, |handler| {
                    handler.bounds_for_range(start..end)
                }).flatten();

                match bounds {
                    Some(b) => IOSCGRect::new(
                        IOSCGPoint::new(b.origin.x.0 as f64, b.origin.y.0 as f64),
                        IOSCGSize::new(b.size.width.0 as f64, b.size.height.0 as f64),
                    ),
                    None => default_rect,
                }
            }));

            match result {
                Ok(rect) => rect,
                Err(_) => default_rect,
            }
        }

        // UITextInput - selectionRectsForRange: (for selection handles)
        extern "C" fn selection_rects_for_range(_this: &Object, _sel: Sel, _range: *mut Object) -> *mut Object {
            // Return empty array - we handle selection rendering ourselves
            unsafe {
                msg_send![class!(NSArray), array]
            }
        }

        // UITextInput - closestPositionToPoint:
        // IMPORTANT: Uses catch_unwind because panics cannot unwind through extern "C"
        extern "C" fn closest_position_to_point(this: &Object, _sel: Sel, point: IOSCGPoint) -> *mut Object {
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                let index = with_input_handler(this, |handler| {
                    handler.character_index_for_point(Point::new(
                        px(point.x as f32),
                        px(point.y as f32),
                    ))
                }).flatten();

                create_text_position(index.unwrap_or(0))
            }));

            match result {
                Ok(ptr) => ptr,
                Err(_) => create_text_position(0),
            }
        }

        // UITextInput - closestPositionToPoint:withinRange:
        extern "C" fn closest_position_to_point_within_range(
            this: &Object,
            _sel: Sel,
            point: IOSCGPoint,
            range: *mut Object,
        ) -> *mut Object {
            let pos = closest_position_to_point(this, _sel, point);

            // Clamp to range if valid
            if let (Some(index), Some((start, end))) = (get_position_index(pos), get_range_indices(range)) {
                let clamped = index.clamp(start, end);
                return create_text_position(clamped);
            }

            pos
        }

        // UITextInput - characterRangeAtPoint:
        extern "C" fn character_range_at_point(this: &Object, _sel: Sel, point: IOSCGPoint) -> *mut Object {
            let pos = closest_position_to_point(this, _sel, point);
            let Some(index) = get_position_index(pos) else {
                return std::ptr::null_mut();
            };

            // Return single character range
            create_text_range(index, index.saturating_add(1))
        }

        // UITextInput - baseWritingDirectionForPosition:inDirection:
        extern "C" fn base_writing_direction(
            _this: &Object,
            _sel: Sel,
            _position: *mut Object,
            _direction: i64,
        ) -> i64 {
            0 // UITextWritingDirectionNatural / LeftToRight
        }

        // UITextInput - setBaseWritingDirection:forRange:
        extern "C" fn set_base_writing_direction(
            _this: &mut Object,
            _sel: Sel,
            _direction: i64,
            _range: *mut Object,
        ) {
            // No-op - we only support LTR for now
        }

        unsafe {
            // Add class method for layerClass
            decl.add_class_method(
                sel!(layerClass),
                layer_class as extern "C" fn(&Class, Sel) -> *const Class,
            );

            // Add touch handling instance methods
            decl.add_method(
                sel!(touchesBegan:withEvent:),
                touches_began as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(touchesMoved:withEvent:),
                touches_moved as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(touchesEnded:withEvent:),
                touches_ended as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(touchesCancelled:withEvent:),
                touches_cancelled as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );

            // Add keyboard handling methods
            decl.add_method(
                sel!(canBecomeFirstResponder),
                can_become_first_responder as extern "C" fn(&Object, Sel) -> bool,
            );

            // Add UITextInputTraits protocol methods
            decl.add_method(
                sel!(keyboardType),
                keyboard_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(returnKeyType),
                return_key_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(autocapitalizationType),
                autocapitalization_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(autocorrectionType),
                autocorrection_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(smartQuotesType),
                smart_quotes_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(smartDashesType),
                smart_dashes_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(smartInsertDeleteType),
                smart_insert_delete_type as extern "C" fn(&Object, Sel) -> i64,
            );
            decl.add_method(
                sel!(spellCheckingType),
                spell_checking_type as extern "C" fn(&Object, Sel) -> i64,
            );

            // Add UIKeyInput protocol methods for text input
            decl.add_method(
                sel!(hasText),
                has_text as extern "C" fn(&Object, Sel) -> bool,
            );
            decl.add_method(
                sel!(insertText:),
                insert_text as extern "C" fn(&mut Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(deleteBackward),
                delete_backward as extern "C" fn(&mut Object, Sel),
            );

            // Add hardware keyboard press handling
            decl.add_method(
                sel!(pressesBegan:withEvent:),
                presses_began as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(pressesEnded:withEvent:),
                presses_ended as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );

            // ============================================
            // UITextInput Protocol - Core Properties
            // ============================================
            decl.add_method(
                sel!(beginningOfDocument),
                beginning_of_document as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(endOfDocument),
                end_of_document as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(selectedTextRange),
                selected_text_range as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(setSelectedTextRange:),
                set_selected_text_range as extern "C" fn(&mut Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(markedTextRange),
                marked_text_range as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(markedTextStyle),
                marked_text_style as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(setMarkedTextStyle:),
                set_marked_text_style as extern "C" fn(&mut Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(inputDelegate),
                input_delegate as extern "C" fn(&Object, Sel) -> *mut Object,
            );
            decl.add_method(
                sel!(setInputDelegate:),
                set_input_delegate as extern "C" fn(&mut Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(tokenizer),
                tokenizer as extern "C" fn(&Object, Sel) -> *mut Object,
            );

            // ============================================
            // UITextInput Protocol - Text Manipulation
            // ============================================
            decl.add_method(
                sel!(textInRange:),
                text_in_range as extern "C" fn(&Object, Sel, *mut Object) -> *mut Object,
            );
            decl.add_method(
                sel!(replaceRange:withText:),
                replace_range_with_text as extern "C" fn(&mut Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(setMarkedText:selectedRange:),
                set_marked_text as extern "C" fn(&mut Object, Sel, *mut Object, NSRange),
            );
            decl.add_method(
                sel!(unmarkText),
                unmark_text as extern "C" fn(&mut Object, Sel),
            );
            decl.add_method(
                sel!(attributedSubstringFromRange:),
                attributed_substring_from_range as extern "C" fn(&Object, Sel, *mut Object) -> *mut Object,
            );

            // ============================================
            // UITextInput Protocol - Position/Range Calculation
            // ============================================
            decl.add_method(
                sel!(positionFromPosition:offset:),
                position_from_position_offset as extern "C" fn(&Object, Sel, *mut Object, isize) -> *mut Object,
            );
            decl.add_method(
                sel!(positionFromPosition:inDirection:offset:),
                position_from_position_in_direction as extern "C" fn(&Object, Sel, *mut Object, i64, isize) -> *mut Object,
            );
            decl.add_method(
                sel!(textRangeFromPosition:toPosition:),
                text_range_from_position_to_position as extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> *mut Object,
            );
            decl.add_method(
                sel!(comparePosition:toPosition:),
                compare_position as extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> i64,
            );
            decl.add_method(
                sel!(offsetFromPosition:toPosition:),
                offset_from_position as extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> isize,
            );
            decl.add_method(
                sel!(positionWithinRange:farthestInDirection:),
                position_within_range_farthest as extern "C" fn(&Object, Sel, *mut Object, i64) -> *mut Object,
            );
            decl.add_method(
                sel!(characterRangeByExtendingPosition:inDirection:),
                character_range_by_extending as extern "C" fn(&Object, Sel, *mut Object, i64) -> *mut Object,
            );

            // ============================================
            // UITextInput Protocol - Geometry Methods
            // ============================================
            decl.add_method(
                sel!(caretRectForPosition:),
                caret_rect_for_position as extern "C" fn(&Object, Sel, *mut Object) -> IOSCGRect,
            );
            decl.add_method(
                sel!(firstRectForRange:),
                first_rect_for_range as extern "C" fn(&Object, Sel, *mut Object) -> IOSCGRect,
            );
            decl.add_method(
                sel!(selectionRectsForRange:),
                selection_rects_for_range as extern "C" fn(&Object, Sel, *mut Object) -> *mut Object,
            );
            decl.add_method(
                sel!(closestPositionToPoint:),
                closest_position_to_point as extern "C" fn(&Object, Sel, IOSCGPoint) -> *mut Object,
            );
            decl.add_method(
                sel!(closestPositionToPoint:withinRange:),
                closest_position_to_point_within_range as extern "C" fn(&Object, Sel, IOSCGPoint, *mut Object) -> *mut Object,
            );
            decl.add_method(
                sel!(characterRangeAtPoint:),
                character_range_at_point as extern "C" fn(&Object, Sel, IOSCGPoint) -> *mut Object,
            );
            decl.add_method(
                sel!(baseWritingDirectionForPosition:inDirection:),
                base_writing_direction as extern "C" fn(&Object, Sel, *mut Object, i64) -> i64,
            );
            decl.add_method(
                sel!(setBaseWritingDirection:forRange:),
                set_base_writing_direction as extern "C" fn(&mut Object, Sel, i64, *mut Object),
            );
        }

        decl.register();
    });

    class!(GPUIMetalView)
}

/// Handle touch events from the GPUIMetalView
fn handle_touches(view: &mut Object, touches: *mut Object, event: *mut Object) {
    unsafe {
        // Get the window pointer from the view's ivar
        let window_ptr: *mut std::ffi::c_void = *view.get_ivar(GPUI_WINDOW_IVAR);
        if window_ptr.is_null() {
            log::warn!("GPUI iOS: Touch event but no window pointer set");
            return;
        }

        let window = &*(window_ptr as *const IosWindow);

        // Get all touches from the set
        let all_touches: *mut Object = msg_send![touches, allObjects];
        let count: usize = msg_send![all_touches, count];

        for i in 0..count {
            let touch: *mut Object = msg_send![all_touches, objectAtIndex: i];
            window.handle_touch(touch, event);
        }
    }
}

/// Handle hardware keyboard press events from the GPUIMetalView
fn handle_presses(view: &mut Object, presses: *mut Object, is_key_down: bool) {
    unsafe {
        // Get the window pointer from the view's ivar
        let window_ptr: *mut std::ffi::c_void = *view.get_ivar(GPUI_WINDOW_IVAR);
        if window_ptr.is_null() {
            return;
        }

        let window = &*(window_ptr as *const IosWindow);

        // Get all presses from the set
        let all_presses: *mut Object = msg_send![presses, allObjects];
        let count: usize = msg_send![all_presses, count];

        for i in 0..count {
            let press: *mut Object = msg_send![all_presses, objectAtIndex: i];

            // Get the UIKey from the press
            let key: *mut Object = msg_send![press, key];
            if key.is_null() {
                continue;
            }

            // Get key code
            let key_code: i64 = msg_send![key, keyCode];

            // Get modifier flags
            let modifier_flags: u64 = msg_send![key, modifierFlags];

            ios_log_format(&format!(
                "GPUI iOS: handle_presses - key_code=0x{:02X} ({}), modifiers=0x{:X}, is_down={}",
                key_code, key_code, modifier_flags, is_key_down
            ));

            // Convert modifier flags to our format
            let mut modifiers: u32 = 0;
            if modifier_flags & (1 << 17) != 0 {
                modifiers |= 1 << 0;
            } // Shift
            if modifier_flags & (1 << 18) != 0 {
                modifiers |= 1 << 1;
            } // Control
            if modifier_flags & (1 << 19) != 0 {
                modifiers |= 1 << 2;
            } // Alt/Option
            if modifier_flags & (1 << 20) != 0 {
                modifiers |= 1 << 3;
            } // Command

            // Skip backspace (0x2A) and delete (0x4C) - let iOS handle them entirely
            // through deleteBackward. This prevents duplicate deletion.
            if key_code == 0x2A || key_code == 0x4C {
                ios_log_cstr(c"GPUI iOS: handle_presses - skipping backspace/delete, handled by deleteBackward");
                continue;
            }

            window.handle_key_event(key_code as u32, modifiers, is_key_down);
        }
    }
}

/// iOS Window backed by UIWindow + UIViewController.
pub(crate) struct IosWindow {
    /// Handle used by GPUI to identify this window
    handle: AnyWindowHandle,
    /// The UIWindow object
    window: *mut Object,
    /// The UIViewController
    view_controller: *mut Object,
    /// The Metal-backed UIView (also handles UITextInput)
    view: *mut Object,
    /// Current bounds in pixels
    bounds: Cell<Bounds<Pixels>>,
    /// Scale factor
    scale_factor: Cell<f32>,
    /// Appearance (light/dark mode)
    appearance: Cell<WindowAppearance>,
    /// Input handler for text input
    input_handler: RefCell<Option<PlatformInputHandler>>,
    /// Whether the keyboard is currently shown (tracked to avoid show/hide flicker)
    keyboard_shown: Cell<bool>,
    /// Callback for frame requests
    /// Note: pub(super) to allow ffi.rs to access this for the display link callback
    pub(super) request_frame_callback: RefCell<Option<Box<dyn FnMut(RequestFrameOptions)>>>,
    /// Callback for input events
    input_callback: RefCell<Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>>,
    /// Callback for active status changes
    active_status_callback: RefCell<Option<Box<dyn FnMut(bool)>>>,
    /// Callback for hover status changes (not really applicable on iOS)
    hover_status_callback: RefCell<Option<Box<dyn FnMut(bool)>>>,
    /// Callback for resize events
    resize_callback: RefCell<Option<Box<dyn FnMut(Size<Pixels>, f32)>>>,
    /// Callback for move events (not applicable on iOS)
    moved_callback: RefCell<Option<Box<dyn FnMut()>>>,
    /// Callback for should close
    should_close_callback: RefCell<Option<Box<dyn FnMut() -> bool>>>,
    /// Callback for hit test
    hit_test_callback: RefCell<Option<Box<dyn FnMut() -> Option<WindowControlArea>>>>,
    /// Callback for close
    close_callback: RefCell<Option<Box<dyn FnOnce()>>>,
    /// Callback for appearance changes
    appearance_changed_callback: RefCell<Option<Box<dyn FnMut()>>>,
    /// Current mouse position (from touch)
    mouse_position: Cell<Point<Pixels>>,
    /// Current modifiers
    modifiers: Cell<Modifiers>,
    /// Blade renderer for GPU rendering
    renderer: RefCell<blade::Renderer>,
    /// Track if a touch is currently pressed
    touch_pressed: Cell<bool>,
}

// Required for raw_window_handle
unsafe impl Send for IosWindow {}
unsafe impl Sync for IosWindow {}

impl IosWindow {
    pub fn new(
        handle: AnyWindowHandle,
        _params: WindowParams,
        renderer_context: blade::Context,
    ) -> Result<Self> {
        // Pre-register text input classes early to avoid race conditions.
        // iOS may query UITextInput methods (markedTextRange, selectedTextRange)
        // immediately when the keyboard is shown, before our classes are ready.
        text_input::ensure_text_input_classes_registered();

        // Create the window on the main screen
        let screen = IosDisplay::main();
        let screen_bounds = screen.bounds();
        let scale_factor = screen.scale();

        unsafe {
            // Create UIWindow
            let screen_obj: *mut Object = msg_send![class!(UIScreen), mainScreen];
            let screen_bounds_cg: CGRect = msg_send![screen_obj, bounds];
            let window: *mut Object = msg_send![class!(UIWindow), alloc];
            let window: *mut Object = msg_send![window, initWithFrame: screen_bounds_cg];

            // Create UIViewController
            let view_controller: *mut Object = msg_send![class!(UIViewController), alloc];
            let view_controller: *mut Object = msg_send![view_controller, init];

            // Create our custom Metal view using the registered class
            let metal_view_class = register_metal_view_class();
            let view: *mut Object = msg_send![metal_view_class, alloc];
            let view: *mut Object = msg_send![view, initWithFrame: screen_bounds_cg];

            // Configure the Metal layer
            let layer: *mut Object = msg_send![view, layer];

            // Get the Metal device using the Metal framework function
            #[link(name = "Metal", kind = "framework")]
            unsafe extern "C" {
                fn MTLCreateSystemDefaultDevice() -> *mut Object;
            }
            let device = MTLCreateSystemDefaultDevice();
            if !device.is_null() {
                let _: () = msg_send![layer, setDevice: device];
            }
            let _: () = msg_send![layer, setPixelFormat: 80_u64]; // MTLPixelFormatBGRA8Unorm
            let _: () = msg_send![layer, setFramebufferOnly: NO];
            let scale: CGFloat = msg_send![screen_obj, scale];
            let _: () = msg_send![layer, setContentsScale: scale];
            let drawable_size = CGSize {
                width: screen_bounds_cg.size.width * scale,
                height: screen_bounds_cg.size.height * scale,
            };
            let _: () = msg_send![layer, setDrawableSize: drawable_size];

            // Enable user interaction on the Metal view for touch handling
            let _: () = msg_send![view, setUserInteractionEnabled: YES];
            let _: () = msg_send![view, setMultipleTouchEnabled: YES];

            // Set the view as the view controller's view
            let _: () = msg_send![view_controller, setView: view];

            // Set the root view controller
            let _: () = msg_send![window, setRootViewController: view_controller];

            // Make the window visible
            let _: () = msg_send![window, makeKeyAndVisible];

            // Make the Metal view first responder for keyboard input
            // The GPUIMetalView implements UITextInput protocol for text input
            let _: () = msg_send![view, becomeFirstResponder];

            // Create the blade renderer
            // Note: Blade expects size in pixels (device pixels), not points
            let renderer = blade::new_renderer(
                renderer_context,
                window as *mut c_void,
                view as *mut c_void,
                crate::Size {
                    width: drawable_size.width as f32,
                    height: drawable_size.height as f32,
                },
                false, // not transparent
            );

            let ios_window = Self {
                handle,
                window,
                view_controller,
                view,
                bounds: Cell::new(screen_bounds),
                scale_factor: Cell::new(scale_factor),
                appearance: Cell::new(WindowAppearance::Light),
                input_handler: RefCell::new(None),
                keyboard_shown: Cell::new(false),
                request_frame_callback: RefCell::new(None),
                input_callback: RefCell::new(None),
                active_status_callback: RefCell::new(None),
                hover_status_callback: RefCell::new(None),
                resize_callback: RefCell::new(None),
                moved_callback: RefCell::new(None),
                should_close_callback: RefCell::new(None),
                hit_test_callback: RefCell::new(None),
                close_callback: RefCell::new(None),
                appearance_changed_callback: RefCell::new(None),
                mouse_position: Cell::new(Point::default()),
                modifiers: Cell::new(Modifiers::default()),
                renderer: RefCell::new(renderer),
                touch_pressed: Cell::new(false),
            };

            Ok(ios_window)
        }
    }

    /// Register this window with the FFI layer after it's been stored.
    /// This must be called after the window is placed at a stable address
    /// (e.g., in a Box or Arc).
    pub(crate) fn register_with_ffi(&self) {
        super::ffi::register_window(self as *const Self);

        // Set the window pointer on the view so touch events can find us
        unsafe {
            let window_ptr = self as *const Self as *mut std::ffi::c_void;
            (*self.view).set_ivar(GPUI_WINDOW_IVAR, window_ptr);
            log::info!(
                "GPUI iOS: Set window pointer {:p} on view {:p}",
                window_ptr,
                self.view
            );
        }
    }

    /// Handle a touch event from UIKit
    /// Get the UIWindow pointer for this window.
    pub fn ui_window(&self) -> *mut Object {
        self.window
    }

    pub fn handle_touch(&self, touch: *mut Object, _event: *mut Object) {
        let position = touch_location_in_view(touch, self.view);
        let phase = touch_phase(touch);
        let tap_count = touch_tap_count(touch);
        let modifiers = self.modifiers.get();

        self.mouse_position.set(position);

        let platform_input = match phase {
            UITouchPhase::Began => {
                self.touch_pressed.set(true);
                touch_began_to_mouse_down(position, tap_count, modifiers)
            }
            UITouchPhase::Moved => {
                touch_moved_to_mouse_move(position, modifiers, Some(crate::MouseButton::Left))
            }
            UITouchPhase::Ended | UITouchPhase::Cancelled => {
                self.touch_pressed.set(false);
                touch_ended_to_mouse_up(position, tap_count, modifiers)
            }
            UITouchPhase::Stationary => return,
        };

        if let Some(callback) = self.input_callback.borrow_mut().as_mut() {
            callback(platform_input);
        }
    }

    /// Get the safe area insets
    pub fn safe_area_insets(&self) -> (f32, f32, f32, f32) {
        unsafe {
            // UIEdgeInsets struct
            #[repr(C)]
            struct UIEdgeInsets {
                top: f64,
                left: f64,
                bottom: f64,
                right: f64,
            }

            let insets: UIEdgeInsets = msg_send![self.view, safeAreaInsets];
            (
                insets.top as f32,
                insets.left as f32,
                insets.bottom as f32,
                insets.right as f32,
            )
        }
    }

    /// Show the software keyboard
    pub fn show_keyboard(&self) {
        unsafe {
            // Log using NSLog so it shows in Xcode console
            let msg = objc::runtime::Sel::register("UTF8String");
            let log_str = "GPUI iOS: show_keyboard called, calling becomeFirstResponder";
            let ns_str: *mut Object = msg_send![class!(NSString), stringWithUTF8String: log_str.as_ptr() as *const std::ffi::c_char];
            let _: () = msg_send![class!(NSObject), performSelector: objc::runtime::Sel::register("class") withObject: std::ptr::null::<Object>()];

            // Make the Metal view (which implements UITextInput) become first responder
            // This will trigger iOS to show the software keyboard
            let is_first_responder: BOOL = msg_send![self.view, isFirstResponder];
            let can_become: BOOL = msg_send![self.view, canBecomeFirstResponder];
            println!("GPUI iOS: show_keyboard - isFirstResponder={}, canBecomeFirstResponder={}", is_first_responder != NO, can_become != NO);

            let result: BOOL = msg_send![self.view, becomeFirstResponder];
            println!("GPUI iOS: becomeFirstResponder returned: {}", result != NO);
        }
    }

    /// Hide the software keyboard
    pub fn hide_keyboard(&self) {
        log::info!("GPUI iOS: Hiding keyboard");
        unsafe {
            // Resign first responder to hide keyboard
            let _: BOOL = msg_send![self.view, resignFirstResponder];
        }
    }

    /// Handle text input from the software keyboard
    /// Note: This is a fallback path. Primary text input goes through insert_text.
    pub fn handle_text_input(&self, text: *mut Object) {
        if text.is_null() {
            return;
        }

        unsafe {
            let utf8: *const i8 = msg_send![text, UTF8String];
            if utf8.is_null() {
                return;
            }

            let text_str = std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned();

            log::info!("GPUI iOS: handle_text_input (fallback): {:?}", text_str);

            // Send as key events
            for c in text_str.chars() {
                let keystroke = crate::Keystroke {
                    modifiers: Modifiers::default(),
                    key: c.to_string(),
                    key_char: Some(c.to_string()),
                };

                let event = PlatformInput::KeyDown(crate::KeyDownEvent {
                    keystroke,
                    is_held: false,
                    prefer_character_input: true,
                });

                if let Some(callback) = self.input_callback.borrow_mut().as_mut() {
                    callback(event);
                }
            }
        }
    }

    /// Handle a key event from an external keyboard
    pub fn handle_key_event(&self, key_code: u32, modifier_flags: u32, is_key_down: bool) {
        use super::text_input::{
            key_code_to_key_down, key_code_to_key_up, key_code_to_string,
            modifier_flags_to_modifiers,
        };

        let key = key_code_to_string(key_code);
        let modifiers = modifier_flags_to_modifiers(modifier_flags);

        // Enhanced logging for debugging arrow key and other key mapping issues
        ios_log_format(&format!(
            "GPUI iOS: handle_key_event - code=0x{:02X}, key={}, modifiers={:?}, down={}",
            key_code, key, modifiers, is_key_down
        ));

        // Handle arrow keys directly via input handler for cursor navigation
        // This bypasses GPUI's key event dispatch which may not work correctly on iOS
        if is_key_down {
            let handled = self.handle_arrow_key(&key, modifiers.shift);
            if handled {
                ios_log_format(&format!("GPUI iOS: handle_key_event - arrow key '{}' handled directly", key));
                return;
            }
        }

        let event = if is_key_down {
            key_code_to_key_down(key_code, modifier_flags)
        } else {
            key_code_to_key_up(key_code, modifier_flags)
        };

        if let Some(callback) = self.input_callback.borrow_mut().as_mut() {
            ios_log_format(&format!("GPUI iOS: handle_key_event - calling input_callback with key={}", key));
            callback(event);
            ios_log_cstr(c"GPUI iOS: handle_key_event - callback returned");
        } else {
            ios_log_cstr(c"GPUI iOS: handle_key_event - NO input_callback set!");
        }
    }

    /// Handle arrow key navigation directly via the input handler.
    /// Returns true if the key was handled, false otherwise.
    fn handle_arrow_key(&self, key: &str, _shift: bool) -> bool {
        // Only handle arrow keys
        let delta: i64 = match key {
            "left" => -1,
            "right" => 1,
            "up" => -1,   // For now, up/down also move by 1 char (proper line nav needs layout info)
            "down" => 1,
            _ => return false,
        };

        // Use with_input_handler via the view, just like insertText does
        // This properly takes/restores the handler from the RefCell
        unsafe {
            let view = &*(self.view as *const Object);

            let handled = with_input_handler(view, |handler| {
                // Get current selection
                let Some(selection) = handler.selected_text_range(false) else {
                    ios_log_cstr(c"GPUI iOS: handle_arrow_key - couldn't get selection");
                    return false;
                };

                ios_log_format(&format!(
                    "GPUI iOS: handle_arrow_key - selection: {:?}, delta: {}",
                    selection.range, delta
                ));

                // Calculate new cursor position
                let current_pos = if selection.reversed {
                    selection.range.start
                } else {
                    selection.range.end
                };

                let new_pos = if delta < 0 {
                    current_pos.saturating_sub((-delta) as usize)
                } else {
                    current_pos.saturating_add(delta as usize)
                };

                ios_log_format(&format!(
                    "GPUI iOS: handle_arrow_key - moving from {} to {}",
                    current_pos, new_pos
                ));

                // Set new cursor position using replace_text_in_range with empty text
                handler.replace_text_in_range(Some(new_pos..new_pos), "");

                ios_log_cstr(c"GPUI iOS: handle_arrow_key - cursor moved");
                true
            });

            handled.unwrap_or(false)
        }
    }

    /// Notify the window of active status changes (foreground/background).
    ///
    /// This is called by the FFI layer when the app transitions between
    /// foreground and background states.
    pub fn notify_active_status_change(&self, is_active: bool) {
        log::info!("GPUI iOS: Window active status changed to: {}", is_active);

        if let Some(callback) = self.active_status_callback.borrow_mut().as_mut() {
            callback(is_active);
        }
    }
}

impl HasWindowHandle for IosWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        let view = NonNull::new(self.view as *mut c_void)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = UiKitWindowHandle::new(view);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for IosWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        let handle = UiKitDisplayHandle::new();
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(handle.into()) })
    }
}

impl PlatformWindow for IosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    fn is_maximized(&self) -> bool {
        true // iOS windows are always "maximized"
    }

    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Fullscreen(self.bounds.get())
    }

    fn content_size(&self) -> Size<Pixels> {
        self.bounds.get().size
    }

    fn resize(&mut self, _size: Size<Pixels>) {
        // iOS windows cannot be resized programmatically
    }

    fn scale_factor(&self) -> f32 {
        self.scale_factor.get()
    }

    fn appearance(&self) -> WindowAppearance {
        unsafe {
            let trait_collection: *mut Object = msg_send![self.view, traitCollection];
            let style: i64 = msg_send![trait_collection, userInterfaceStyle];
            match style {
                2 => WindowAppearance::Dark,
                _ => WindowAppearance::Light,
            }
        }
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(IosDisplay::main()))
    }

    fn mouse_position(&self) -> Point<Pixels> {
        self.mouse_position.get()
    }

    fn modifiers(&self) -> Modifiers {
        self.modifiers.get()
    }

    fn capslock(&self) -> crate::Capslock {
        // Would need to check UIKeyModifierFlags
        crate::Capslock { on: false }
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.input_handler.borrow_mut() = Some(input_handler);
        // Only show keyboard if we haven't already shown it.
        // Note: set_input_handler and take_input_handler are called every frame
        // as part of GPUI's rendering cycle, so we use keyboard_shown flag to
        // track state and avoid show/hide flicker.
        if !self.keyboard_shown.get() {
            ios_log_cstr(c"GPUI iOS: set_input_handler - showing keyboard (first time)");
            self.keyboard_shown.set(true);
            self.show_keyboard();
        }
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        let handler = self.input_handler.borrow_mut().take();
        // Note: Don't hide keyboard here! take_input_handler is called every frame
        // as part of GPUI's rendering cycle (see window.rs line 2006).
        // The keyboard should only be hidden when the view resigns first responder,
        // not on every frame. The keyboard_shown flag stays true.
        handler
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<futures::channel::oneshot::Receiver<usize>> {
        // Would use UIAlertController
        let (_tx, rx) = futures::channel::oneshot::channel();

        unsafe {
            // Create UIAlertController
            let title = msg;
            let message = detail.unwrap_or("");

            let alert_style: i64 = 1; // UIAlertControllerStyleAlert

            let title_str: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: title.as_ptr()];
            let message_str: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: message.as_ptr()];

            let alert: *mut Object = msg_send![
                class!(UIAlertController),
                alertControllerWithTitle: title_str
                message: message_str
                preferredStyle: alert_style
            ];

            // Add buttons
            for (_index, button) in answers.iter().enumerate() {
                let button_title: *mut Object = msg_send![
                    class!(NSString),
                    stringWithUTF8String: button.label().as_str().as_ptr()
                ];

                let action_style: i64 = if button.is_cancel() { 1 } else { 0 }; // UIAlertActionStyleCancel or Default

                // Note: In production, this would need a block that calls tx.send(index)
                let action: *mut Object = msg_send![
                    class!(UIAlertAction),
                    actionWithTitle: button_title
                    style: action_style
                    handler: ptr::null::<Object>()
                ];

                let _: () = msg_send![alert, addAction: action];
            }

            // Present the alert
            let _: () = msg_send![
                self.view_controller,
                presentViewController: alert
                animated: YES
                completion: ptr::null::<Object>()
            ];
        }

        Some(rx)
    }

    fn activate(&self) {
        unsafe {
            let _: () = msg_send![self.window, makeKeyAndVisible];
        }
    }

    fn is_active(&self) -> bool {
        unsafe {
            let app: *mut Object = msg_send![class!(UIApplication), sharedApplication];
            let key_window: *mut Object = msg_send![app, keyWindow];
            self.window == key_window
        }
    }

    fn is_hovered(&self) -> bool {
        // Hover isn't really applicable on iOS
        false
    }

    fn set_title(&mut self, _title: &str) {
        // iOS apps don't have window titles
    }

    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {
        // Could adjust view background color
    }

    fn minimize(&self) {
        // iOS apps cannot be minimized
    }

    fn zoom(&self) {
        // iOS apps cannot be zoomed
    }

    fn toggle_fullscreen(&self) {
        // iOS apps are always fullscreen
    }

    fn is_fullscreen(&self) -> bool {
        true
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        *self.request_frame_callback.borrow_mut() = Some(callback);
    }

    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        *self.input_callback.borrow_mut() = Some(callback);
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        *self.active_status_callback.borrow_mut() = Some(callback);
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        *self.hover_status_callback.borrow_mut() = Some(callback);
    }

    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        *self.resize_callback.borrow_mut() = Some(callback);
    }

    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        *self.moved_callback.borrow_mut() = Some(callback);
    }

    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        *self.should_close_callback.borrow_mut() = Some(callback);
    }

    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        *self.hit_test_callback.borrow_mut() = Some(callback);
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        *self.close_callback.borrow_mut() = Some(callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        *self.appearance_changed_callback.borrow_mut() = Some(callback);
    }

    fn draw(&self, scene: &Scene) {
        self.renderer.borrow_mut().draw(scene);
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.renderer.borrow().sprite_atlas().clone()
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        // Would query Metal device capabilities
        None
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {
        // iOS handles IME positioning automatically
    }
}
