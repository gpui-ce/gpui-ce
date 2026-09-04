use core::slice;
use std::ffi::{CStr, c_void};
use std::path::PathBuf;

use objc::{class, msg_send, rc::StrongPtr, runtime::Object, sel, sel_impl};
use smallvec::SmallVec;
use strum::IntoEnumIterator as _;

use crate::ns_string;
use gpui::{
    ClipboardEntry, ClipboardItem, ClipboardString, ExternalPaths, Image, ImageFormat, hash,
};

type Id = *mut Object;
#[cfg(test)]
const NIL: Id = std::ptr::null_mut();

pub struct Pasteboard {
    inner: StrongPtr,
    text_hash_type: StrongPtr,
    metadata_type: StrongPtr,
}

impl Pasteboard {
    pub fn general() -> Self {
        unsafe { Self::new(msg_send![class!(NSPasteboard), generalPasteboard]) }
    }

    pub fn find() -> Self {
        unsafe {
            Self::new(msg_send![class!(NSPasteboard), pasteboardWithName: NSPasteboardNameFind])
        }
    }

    #[cfg(test)]
    pub fn unique() -> Self {
        unsafe { Self::new(msg_send![class!(NSPasteboard), pasteboardWithUniqueName]) }
    }

    unsafe fn new(inner: Id) -> Self {
        // These constructors return autoreleased objects, but a Pasteboard can
        // outlive the autorelease pool in which it was created.
        Self {
            inner: unsafe { StrongPtr::retain(inner) },
            text_hash_type: unsafe { StrongPtr::retain(ns_string("zed-text-hash")) },
            metadata_type: unsafe { StrongPtr::retain(ns_string("zed-metadata")) },
        }
    }

    pub fn read(&self) -> Option<ClipboardItem> {
        unsafe {
            // Check for file paths first
            let filenames: Id = msg_send![*self.inner, propertyListForType: NSFilenamesPboardType];
            if !filenames.is_null() {
                let count: usize = msg_send![filenames, count];
                if count > 0 {
                    let mut paths = SmallVec::new();
                    for i in 0..count {
                        let file: Id = msg_send![filenames, objectAtIndex: i];
                        let f: *const std::ffi::c_char = msg_send![file, UTF8String];
                        let path = CStr::from_ptr(f).to_string_lossy().into_owned();
                        paths.push(PathBuf::from(path));
                    }
                    if !paths.is_empty() {
                        let mut entries = vec![ClipboardEntry::ExternalPaths(ExternalPaths(paths))];

                        // Also include the string representation so text editors can
                        // paste the path as text.
                        if let Some(string_item) = self.read_string_from_pasteboard() {
                            entries.push(string_item);
                        }

                        return Some(ClipboardItem { entries });
                    }
                }
            }

            // Next, check for a plain string.
            if let Some(string_entry) = self.read_string_from_pasteboard() {
                return Some(ClipboardItem {
                    entries: vec![string_entry],
                });
            }

            // Finally, try the various supported image types.
            for format in ImageFormat::iter() {
                if let Some(item) = self.read_image(format) {
                    return Some(item);
                }
            }
        }

        None
    }

    fn read_image(&self, format: ImageFormat) -> Option<ClipboardItem> {
        let ut_type: UTType = format.into();

        unsafe {
            let types: Id = msg_send![*self.inner, types];
            if msg_send![types, containsObject: ut_type.inner()] {
                self.data_for_type(ut_type.inner_mut()).map(|bytes| {
                    let bytes = bytes.to_vec();
                    let id = hash(&bytes);

                    ClipboardItem {
                        entries: vec![ClipboardEntry::Image(Image { format, bytes, id })],
                    }
                })
            } else {
                None
            }
        }
    }

    unsafe fn read_string_from_pasteboard(&self) -> Option<ClipboardEntry> {
        unsafe {
            let pasteboard_types: Id = msg_send![*self.inner, types];
            let string_type: Id = ns_string("public.utf8-plain-text");

            if !msg_send![pasteboard_types, containsObject: string_type] {
                return None;
            }

            let text_bytes = self.data_for_type(string_type)?;

            let text = String::from_utf8_lossy(&text_bytes).to_string();
            let metadata = self
                .data_for_type(*self.text_hash_type)
                .and_then(|hash_bytes| {
                    let hash_bytes = hash_bytes.as_slice().try_into().ok()?;
                    let hash = u64::from_be_bytes(hash_bytes);
                    let metadata = self.data_for_type(*self.metadata_type)?;

                    if hash == ClipboardString::text_hash(&text) {
                        String::from_utf8(metadata).ok()
                    } else {
                        None
                    }
                });

            Some(ClipboardEntry::String(ClipboardString { text, metadata }))
        }
    }

    unsafe fn data_for_type(&self, kind: Id) -> Option<Vec<u8>> {
        unsafe {
            let data: Id = msg_send![*self.inner, dataForType: kind];
            if data.is_null() {
                None
            } else {
                let bytes: *const c_void = msg_send![data, bytes];
                if bytes.is_null() {
                    return Some(Vec::new());
                }
                let length: usize = msg_send![data, length];
                Some(slice::from_raw_parts(bytes.cast(), length).to_vec())
            }
        }
    }

    pub fn write(&self, item: ClipboardItem) {
        unsafe {
            match item.entries.as_slice() {
                [] => {
                    // Writing an empty list of entries just clears the clipboard.
                    let _: usize = msg_send![*self.inner, clearContents];
                }
                [ClipboardEntry::String(string)] => {
                    self.write_plaintext(string);
                }
                [ClipboardEntry::Image(image)] => {
                    self.write_image(image);
                }
                [ClipboardEntry::ExternalPaths(_)] => {}
                _ => {
                    // Agus NB: We're currently only writing string entries to the clipboard when we have more than one.
                    //
                    // This was the existing behavior before I refactored the outer clipboard code:
                    // https://github.com/zed-industries/zed/blob/65f7412a0265552b06ce122655369d6cc7381dd6/crates/gpui/src/platform/mac/platform.rs#L1060-L1110
                    //
                    // Note how `any_images` is always `false`. We should fix that, but that's orthogonal to the refactor.

                    let mut combined = ClipboardString {
                        text: String::new(),
                        metadata: None,
                    };

                    for entry in item.entries {
                        match entry {
                            ClipboardEntry::String(text) => {
                                combined.text.push_str(&text.text());
                                if combined.metadata.is_none() {
                                    combined.metadata = text.metadata;
                                }
                            }
                            _ => {}
                        }
                    }

                    self.write_plaintext(&combined);
                }
            }
        }
    }

    fn write_plaintext(&self, string: &ClipboardString) {
        unsafe {
            let _: usize = msg_send![*self.inner, clearContents];

            let text_bytes: Id = msg_send![class!(NSData), dataWithBytes: string.text.as_ptr() as *const c_void length: string.text.len()];
            let _: bool =
                msg_send![*self.inner, setData: text_bytes forType: NSPasteboardTypeString];

            if let Some(metadata) = string.metadata.as_ref() {
                let hash_bytes = ClipboardString::text_hash(&string.text).to_be_bytes();
                let hash_bytes: Id = msg_send![class!(NSData), dataWithBytes: hash_bytes.as_ptr() as *const c_void length: hash_bytes.len()];
                let _: bool =
                    msg_send![*self.inner, setData: hash_bytes forType: *self.text_hash_type];

                let metadata_bytes: Id = msg_send![class!(NSData), dataWithBytes: metadata.as_ptr() as *const c_void length: metadata.len()];
                let _: bool =
                    msg_send![*self.inner, setData: metadata_bytes forType: *self.metadata_type];
            }
        }
    }

    unsafe fn write_image(&self, image: &Image) {
        unsafe {
            let _: usize = msg_send![*self.inner, clearContents];

            let bytes: Id = msg_send![class!(NSData), dataWithBytes: image.bytes.as_ptr() as *const c_void length: image.bytes.len()];

            let _: bool = msg_send![*self.inner, setData: bytes forType: Into::<UTType>::into(image.format).inner_mut()];
        }
    }
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    /// [Apple's documentation](https://developer.apple.com/documentation/appkit/nspasteboardnamefind?language=objc)
    pub static NSPasteboardNameFind: Id;
    pub static NSFilenamesPboardType: Id;
    pub static NSPasteboardTypePNG: Id;
    pub static NSPasteboardTypeString: Id;
    pub static NSPasteboardTypeTIFF: Id;
}

impl From<ImageFormat> for UTType {
    fn from(value: ImageFormat) -> Self {
        match value {
            ImageFormat::Png => Self::png(),
            ImageFormat::Jpeg => Self::jpeg(),
            ImageFormat::Tiff => Self::tiff(),
            ImageFormat::Webp => Self::webp(),
            ImageFormat::Gif => Self::gif(),
            ImageFormat::Bmp => Self::bmp(),
            ImageFormat::Svg => Self::svg(),
            ImageFormat::Ico => Self::ico(),
            ImageFormat::Pnm => Self::pnm(),
        }
    }
}

// See https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/
pub struct UTType(Id);

impl UTType {
    pub fn png() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/png
        Self(unsafe { NSPasteboardTypePNG }) // This is a rare case where there's a built-in NSPasteboardType
    }

    pub fn jpeg() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/jpeg
        Self(unsafe { ns_string("public.jpeg") })
    }

    pub fn gif() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/gif
        Self(unsafe { ns_string("com.compuserve.gif") })
    }

    pub fn webp() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/webp
        Self(unsafe { ns_string("org.webmproject.webp") })
    }

    pub fn bmp() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/bmp
        Self(unsafe { ns_string("com.microsoft.bmp") })
    }

    pub fn svg() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/svg
        Self(unsafe { ns_string("public.svg-image") })
    }

    pub fn ico() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/ico
        Self(unsafe { ns_string("com.microsoft.ico") })
    }

    pub fn tiff() -> Self {
        // https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/tiff
        Self(unsafe { NSPasteboardTypeTIFF }) // This is a rare case where there's a built-in NSPasteboardType
    }

    pub fn pnm() -> Self {
        //https://en.wikipedia.org/w/index.php?title=Netpbm&oldid=1336679433 under Uniform Type Identifier
        Self(unsafe { ns_string("public.pbm") })
    }

    fn inner(&self) -> *const Object {
        self.0
    }

    pub fn inner_mut(&self) -> *mut Object {
        self.0 as *mut _
    }
}

#[cfg(test)]
mod tests {
    use objc::rc::autoreleasepool;
    use std::ffi::c_void;
    use std::sync::Mutex;

    use gpui::{ClipboardEntry, ClipboardItem, ClipboardString, ImageFormat};

    use super::*;

    static PASTEBOARD_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn run_pasteboard_test<T>(test: impl FnOnce() -> T) -> T {
        let _guard = PASTEBOARD_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        autoreleasepool(test)
    }

    unsafe fn simulate_external_file_copy(pasteboard: &Pasteboard, paths: &[&str]) {
        unsafe {
            let ns_paths: Vec<Id> = paths.iter().map(|p| ns_string(p)).collect();
            let ns_array: Id = msg_send![class!(NSArray), arrayWithObjects: ns_paths.as_ptr() count: ns_paths.len()];

            let _: usize = msg_send![*pasteboard.inner, clearContents];

            let _: bool = msg_send![*pasteboard.inner, setPropertyList: ns_array forType: NSFilenamesPboardType];

            let joined = paths.join("\n");
            let bytes: Id = msg_send![class!(NSData), dataWithBytes: joined.as_ptr() as *const c_void length: joined.len()];
            let _: bool =
                msg_send![*pasteboard.inner, setData: bytes forType: NSPasteboardTypeString];
        }
    }

    #[test]
    fn test_string() {
        run_pasteboard_test(|| {
            let pasteboard = Pasteboard::unique();
            assert_eq!(pasteboard.read(), None);

            let item = ClipboardItem::new_string("1".to_string());
            pasteboard.write(item.clone());
            assert_eq!(pasteboard.read(), Some(item));

            let item = ClipboardItem {
                entries: vec![ClipboardEntry::String(
                    ClipboardString::new("2".to_string()).with_json_metadata(vec![3, 4]),
                )],
            };
            pasteboard.write(item.clone());
            assert_eq!(pasteboard.read(), Some(item));

            let text_from_other_app = "text from other app";
            unsafe {
                let bytes: Id = msg_send![class!(NSData), dataWithBytes: text_from_other_app.as_ptr() as *const c_void length: text_from_other_app.len()];
                let _: bool =
                    msg_send![*pasteboard.inner, setData: bytes forType: NSPasteboardTypeString];
            }
            assert_eq!(
                pasteboard.read(),
                Some(ClipboardItem::new_string(text_from_other_app.to_string()))
            );
        });
    }

    #[test]
    fn test_custom_types_survive_creation_autorelease_pool() {
        let pasteboard = autoreleasepool(|| unsafe { Pasteboard::new(NIL) });

        unsafe {
            let text_hash_type = CStr::from_ptr(msg_send![*pasteboard.text_hash_type, UTF8String]);
            let metadata_type = CStr::from_ptr(msg_send![*pasteboard.metadata_type, UTF8String]);
            assert_eq!(text_hash_type.to_bytes(), b"zed-text-hash");
            assert_eq!(metadata_type.to_bytes(), b"zed-metadata");
        }
    }

    #[test]
    fn test_read_external_path() {
        run_pasteboard_test(|| {
            let pasteboard = Pasteboard::unique();

            unsafe {
                simulate_external_file_copy(&pasteboard, &["/test.txt"]);
            }

            let item = pasteboard.read().expect("should read clipboard item");

            // Test both ExternalPaths and String entries exist
            assert_eq!(item.entries.len(), 2);

            // Test first entry is ExternalPaths
            match &item.entries[0] {
                ClipboardEntry::ExternalPaths(ep) => {
                    assert_eq!(ep.paths(), &[PathBuf::from("/test.txt")]);
                }
                other => panic!("expected ExternalPaths, got {:?}", other),
            }

            // Test second entry is String
            match &item.entries[1] {
                ClipboardEntry::String(s) => {
                    assert_eq!(s.text(), "/test.txt");
                }
                other => panic!("expected String, got {:?}", other),
            }
        });
    }

    #[test]
    fn test_read_external_paths_with_spaces() {
        run_pasteboard_test(|| {
            let pasteboard = Pasteboard::unique();
            let paths = ["/some file with spaces.txt"];

            unsafe {
                simulate_external_file_copy(&pasteboard, &paths);
            }

            let item = pasteboard.read().expect("should read clipboard item");

            match &item.entries[0] {
                ClipboardEntry::ExternalPaths(ep) => {
                    assert_eq!(ep.paths(), &[PathBuf::from("/some file with spaces.txt")]);
                }
                other => panic!("expected ExternalPaths, got {:?}", other),
            }
        });
    }

    #[test]
    fn test_read_multiple_external_paths() {
        run_pasteboard_test(|| {
            let pasteboard = Pasteboard::unique();
            let paths = ["/file.txt", "/image.png"];

            unsafe {
                simulate_external_file_copy(&pasteboard, &paths);
            }

            let item = pasteboard.read().expect("should read clipboard item");
            assert_eq!(item.entries.len(), 2);

            // Test both ExternalPaths and String entries exist
            match &item.entries[0] {
                ClipboardEntry::ExternalPaths(ep) => {
                    assert_eq!(
                        ep.paths(),
                        &[PathBuf::from("/file.txt"), PathBuf::from("/image.png"),]
                    );
                }
                other => panic!("expected ExternalPaths, got {:?}", other),
            }

            match &item.entries[1] {
                ClipboardEntry::String(s) => {
                    assert_eq!(s.text(), "/file.txt\n/image.png");
                    assert_eq!(s.metadata, None);
                }
                other => panic!("expected String, got {:?}", other),
            }
        });
    }

    #[test]
    fn test_read_image() {
        run_pasteboard_test(|| {
            let pasteboard = Pasteboard::unique();

            // Smallest valid PNG: 1x1 transparent pixel
            let png_bytes: &[u8] = &[
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78,
                0x9C, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0x7A, 0x5E, 0xAB, 0x3F,
                0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
            ];

            unsafe {
                let ns_png_type = NSPasteboardTypePNG;
                let _: usize = msg_send![*pasteboard.inner, clearContents];

                let data: Id = msg_send![class!(NSData), dataWithBytes: png_bytes.as_ptr() as *const c_void length: png_bytes.len()];
                let _: bool = msg_send![*pasteboard.inner, setData: data forType: ns_png_type];
            }

            let item = pasteboard.read().expect("should read PNG image");

            // Test Image entry exists
            assert_eq!(item.entries.len(), 1);
            match &item.entries[0] {
                ClipboardEntry::Image(img) => {
                    assert_eq!(img.format, ImageFormat::Png);
                    assert_eq!(img.bytes, png_bytes);
                }
                other => panic!("expected Image, got {:?}", other),
            }
        });
    }
}
