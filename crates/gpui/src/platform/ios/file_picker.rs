//! iOS File Picker implementation using UIDocumentPickerViewController.
//!
//! This module provides file selection functionality for iOS, bridging
//! between GPUI's Platform trait and iOS's document picker APIs.

use crate::PathPromptOptions;
use futures::channel::oneshot;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Protocol, Sel, BOOL, NO, YES},
    sel, sel_impl,
};
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

/// Storage for pending file picker results.
/// Only one file picker can be active at a time on iOS.
struct FilePickerState {
    /// The channel to send results back to the caller.
    sender: UnsafeCell<Option<oneshot::Sender<anyhow::Result<Option<Vec<PathBuf>>>>>>,
    /// Whether we're selecting directories (vs files).
    selecting_directories: UnsafeCell<bool>,
}

// Safety: Only accessed from main thread on iOS
unsafe impl Send for FilePickerState {}
unsafe impl Sync for FilePickerState {}

static FILE_PICKER_STATE: OnceLock<FilePickerState> = OnceLock::new();
static PICKER_DELEGATE_CLASS: OnceLock<&'static Class> = OnceLock::new();

/// Initialize the file picker state.
fn init_state() -> &'static FilePickerState {
    FILE_PICKER_STATE.get_or_init(|| FilePickerState {
        sender: UnsafeCell::new(None),
        selecting_directories: UnsafeCell::new(false),
    })
}

/// Register the Objective-C delegate class for UIDocumentPickerViewController.
fn register_delegate_class() -> &'static Class {
    PICKER_DELEGATE_CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("GPUIDocumentPickerDelegate", superclass)
            .expect("Failed to create GPUIDocumentPickerDelegate class");

        // Add protocol conformance
        let picker_delegate_protocol = Protocol::get("UIDocumentPickerDelegate")
            .expect("UIDocumentPickerDelegate protocol not found");
        decl.add_protocol(picker_delegate_protocol);

        // documentPicker:didPickDocumentsAtURLs: - called when user selects files
        extern "C" fn did_pick_documents(_this: &Object, _sel: Sel, _picker: *mut Object, urls: *mut Object) {
            log::info!("GPUI iOS: File picker - user selected documents");

            unsafe {
                let mut paths: Vec<PathBuf> = Vec::new();

                // urls is an NSArray of NSURL
                let count: usize = msg_send![urls, count];
                for i in 0..count {
                    let url: *mut Object = msg_send![urls, objectAtIndex: i];

                    // Start accessing security-scoped resource
                    let _: BOOL = msg_send![url, startAccessingSecurityScopedResource];

                    let path_string: *mut Object = msg_send![url, path];
                    if !path_string.is_null() {
                        let utf8: *const i8 = msg_send![path_string, UTF8String];
                        if !utf8.is_null() {
                            let path_str = std::ffi::CStr::from_ptr(utf8)
                                .to_str()
                                .unwrap_or("");
                            if !path_str.is_empty() {
                                paths.push(PathBuf::from(path_str));
                            }
                        }
                    }
                }

                log::info!("GPUI iOS: File picker - got {} paths", paths.len());

                // Send the result
                if let Some(state) = FILE_PICKER_STATE.get() {
                    if let Some(sender) = (*state.sender.get()).take() {
                        let result = if paths.is_empty() {
                            Ok(None)
                        } else {
                            Ok(Some(paths))
                        };
                        let _ = sender.send(result);
                    }
                }
            }
        }

        // documentPickerWasCancelled: - called when user cancels
        extern "C" fn was_cancelled(_this: &Object, _sel: Sel, _picker: *mut Object) {
            log::info!("GPUI iOS: File picker - user cancelled");

            if let Some(state) = FILE_PICKER_STATE.get() {
                unsafe {
                    if let Some(sender) = (*state.sender.get()).take() {
                        let _ = sender.send(Ok(None));
                    }
                }
            }
        }

        unsafe {
            decl.add_method(
                sel!(documentPicker:didPickDocumentsAtURLs:),
                did_pick_documents as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
            );
            decl.add_method(
                sel!(documentPickerWasCancelled:),
                was_cancelled as extern "C" fn(&Object, Sel, *mut Object),
            );
        }

        decl.register()
    })
}

/// Present the iOS file picker.
///
/// This function is called from the Platform trait implementation.
/// It returns a receiver that will be fulfilled when the user
/// selects files or cancels.
pub(crate) fn prompt_for_paths(
    options: PathPromptOptions,
) -> oneshot::Receiver<anyhow::Result<Option<Vec<PathBuf>>>> {
    let (tx, rx) = oneshot::channel();

    let state = init_state();

    // Store the sender for the callback
    unsafe {
        *state.sender.get() = Some(tx);
        *state.selecting_directories.get() = options.directories;
    }

    // Register delegate class if not already done
    let delegate_class = register_delegate_class();

    unsafe {
        // Create the delegate
        let delegate: *mut Object = msg_send![delegate_class, new];

        // Create UTType array for allowed content types
        let ut_types: *mut Object = if options.directories {
            // For directories, use UTTypeFolder
            let folder_type: *mut Object = msg_send![class!(UTType), folderType];
            msg_send![class!(NSArray), arrayWithObject: folder_type]
        } else {
            // For files, allow all types (user can filter in picker)
            let item_type: *mut Object = msg_send![class!(UTType), itemType];
            msg_send![class!(NSArray), arrayWithObject: item_type]
        };

        // Create UIDocumentPickerViewController
        // iOS 14+: initForOpeningContentTypes:asCopy:
        let picker: *mut Object = msg_send![class!(UIDocumentPickerViewController), alloc];
        let picker: *mut Object = msg_send![picker, initForOpeningContentTypes: ut_types asCopy: NO];

        // Configure the picker
        let _: () = msg_send![picker, setDelegate: delegate];
        let _: () = msg_send![picker, setAllowsMultipleSelection: options.multiple];

        // For directories, we need to set this mode
        if options.directories {
            // UIDocumentPickerMode.open = 0
            // Actually for folders we use a different approach - the UTTypeFolder
            // should be enough with iOS 14+
        }

        // Get the root view controller to present from
        let app: *mut Object = msg_send![class!(UIApplication), sharedApplication];
        let key_window: *mut Object = msg_send![app, keyWindow];

        if key_window.is_null() {
            log::error!("GPUI iOS: No key window available for file picker");
            if let Some(state) = FILE_PICKER_STATE.get() {
                if let Some(sender) = (*state.sender.get()).take() {
                    let _ = sender.send(Err(anyhow::anyhow!("No key window available")));
                }
            }
            return rx;
        }

        let root_vc: *mut Object = msg_send![key_window, rootViewController];
        if root_vc.is_null() {
            log::error!("GPUI iOS: No root view controller for file picker");
            if let Some(state) = FILE_PICKER_STATE.get() {
                if let Some(sender) = (*state.sender.get()).take() {
                    let _ = sender.send(Err(anyhow::anyhow!("No root view controller")));
                }
            }
            return rx;
        }

        // Present the picker
        log::info!("GPUI iOS: Presenting file picker");
        let _: () = msg_send![root_vc, presentViewController: picker animated: YES completion: std::ptr::null::<c_void>()];
    }

    rx
}

/// Present a save dialog.
///
/// On iOS, this uses UIDocumentPickerViewController in export mode.
pub(crate) fn prompt_for_new_path(
    directory: &std::path::Path,
    suggested_name: Option<&str>,
) -> oneshot::Receiver<anyhow::Result<Option<PathBuf>>> {
    let (tx, rx) = oneshot::channel();

    // For save dialogs, we need to create a temporary file first,
    // then use UIDocumentPickerViewController to let the user choose
    // where to save it. This is more complex on iOS.
    //
    // For MVP, we'll save to the app's Documents directory with the suggested name.

    unsafe {
        let file_manager: *mut Object = msg_send![class!(NSFileManager), defaultManager];
        let urls: *mut Object = msg_send![file_manager,
            URLsForDirectory: 9u64 // NSDocumentDirectory = 9
            inDomains: 1u64 // NSUserDomainMask = 1
        ];

        let count: usize = msg_send![urls, count];
        if count == 0 {
            let _ = tx.send(Err(anyhow::anyhow!("No Documents directory available")));
            return rx;
        }

        let docs_url: *mut Object = msg_send![urls, objectAtIndex: 0usize];
        let docs_path: *mut Object = msg_send![docs_url, path];
        let utf8: *const i8 = msg_send![docs_path, UTF8String];

        if utf8.is_null() {
            let _ = tx.send(Err(anyhow::anyhow!("Failed to get Documents path")));
            return rx;
        }

        let docs_str = std::ffi::CStr::from_ptr(utf8).to_str().unwrap_or("");
        let mut save_path = PathBuf::from(docs_str);

        // Add the suggested filename or a default
        let filename = suggested_name.unwrap_or("Untitled");
        save_path.push(filename);

        log::info!("GPUI iOS: Save path: {:?}", save_path);
        let _ = tx.send(Ok(Some(save_path)));
    }

    rx
}

/// FFI function to receive file picker results from Objective-C.
///
/// This is called by the Objective-C delegate when the user selects files.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_file_picker_did_pick(urls: *mut Object) {
    log::info!("GPUI iOS: gpui_ios_file_picker_did_pick called");

    if urls.is_null() {
        // User cancelled
        if let Some(state) = FILE_PICKER_STATE.get() {
            unsafe {
                if let Some(sender) = (*state.sender.get()).take() {
                    let _ = sender.send(Ok(None));
                }
            }
        }
        return;
    }

    unsafe {
        let mut paths: Vec<PathBuf> = Vec::new();
        let count: usize = msg_send![urls, count];

        for i in 0..count {
            let url: *mut Object = msg_send![urls, objectAtIndex: i];
            let path_string: *mut Object = msg_send![url, path];

            if !path_string.is_null() {
                let utf8: *const i8 = msg_send![path_string, UTF8String];
                if !utf8.is_null() {
                    let path_str = std::ffi::CStr::from_ptr(utf8).to_str().unwrap_or("");
                    if !path_str.is_empty() {
                        paths.push(PathBuf::from(path_str));
                    }
                }
            }
        }

        if let Some(state) = FILE_PICKER_STATE.get() {
            if let Some(sender) = (*state.sender.get()).take() {
                let result = if paths.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(paths))
                };
                let _ = sender.send(result);
            }
        }
    }
}

/// FFI function called when the file picker is cancelled.
#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_file_picker_cancelled() {
    log::info!("GPUI iOS: File picker cancelled");

    if let Some(state) = FILE_PICKER_STATE.get() {
        unsafe {
            if let Some(sender) = (*state.sender.get()).take() {
                let _ = sender.send(Ok(None));
            }
        }
    }
}
