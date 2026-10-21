//! Frame pacing for macOS windows through `CVDisplayLink`.
//!
//! `CVDisplayLinkStop` returns before the I/O thread exits, with no way to
//! know whether the final callback has finished. Releasing the link let the
//! I/O thread read freed internals, causing a crash in `CVHWTime::reset`.
//! Leaking the link fixed that in #32116.
//!
//! The callback also dereferenced a dispatch source through its context
//! pointer. Releasing that source let a late callback read freed memory,
//! causing a crash in `dispatch_source_merge_data`, ZED-7XR.
//!
//! A static registry keeps one lazily created, never-released link per display.
//! Windows subscribe instead of tearing down links on resize, activation, or
//! occlusion changes. The callback receives an integer display ID and only
//! dereferences the static registry, so callbacks after stop are harmless.
//!
//! Each window owns one dispatch source for its lifetime. Closing removes it
//! under the lock, making it unreachable from callbacks, then cancels and
//! releases it. Only the registry starts and stops links. A link runs exactly
//! while it has subscribers, so windows sharing a display cannot stop each
//! other's frame callbacks.
//!
//! Links survive disconnection, display-ID reuse, and mode or refresh-rate
//! changes. `CVDisplayLink` queries timing dynamically. If cached timing ever
//! fails, refresh entries on display reconfiguration. Releasing links would
//! reintroduce the teardown race.
//!
//! Never call CoreVideo while holding the registry lock. The I/O callback
//! takes that lock and may hold undocumented CoreVideo locks, creating a
//! possible cycle. Registry changes and link start/stop run only on the main
//! thread, keeping `running` consistent with the link's actual state without
//! locking across those calls.
//!
//! On macOS, `std::sync::Mutex` currently uses `os_unfair_lock`. Its priority
//! donation resolves priority inversion between the high-priority I/O thread
//! and the main thread. This is why the registry uses it instead of
//! `parking_lot`. The backing lock is an implementation detail. If it changes,
//! contention latency may increase, but correctness is unaffected.

use anyhow::Result;
use core_graphics::display::CGDirectDisplayID;
use dispatch2::{
    _dispatch_source_type_data_add, DispatchObject, DispatchQueue, DispatchRetained, DispatchSource,
};
use gpui_util::ResultExt;
use std::{
    collections::{BTreeMap, btree_map},
    ffi::c_void,
    sync::{Mutex, MutexGuard, PoisonError},
};

static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

struct Registry {
    displays: BTreeMap<CGDirectDisplayID, DisplayEntry>,
    next_subscriber_id: u64,
}

impl Registry {
    const fn new() -> Self {
        Registry {
            displays: BTreeMap::new(),
            next_subscriber_id: 0,
        }
    }
}

struct DisplayEntry {
    link: sys::DisplayLink,
    running: bool,
    subscribers: Vec<(SubscriberId, DispatchRetained<DispatchSource>)>,
}

// SAFETY: Both fields wrapping raw pointers are refcounted handles to
// thread-safe objects that are valid on any thread: `sys::DisplayLink` to a
// CoreVideo object, and each subscriber's `DispatchRetained<DispatchSource>`
// to a GCD object (which the display's io thread really does use, calling
// `merge_data` from the output callback). All mutation of the entry itself
// is serialized by the registry lock.
unsafe impl Send for DisplayEntry {}

#[derive(Copy, Clone, PartialEq, Eq)]
struct SubscriberId(u64);

fn lock_registry() -> MutexGuard<'static, Registry> {
    // Proceeding past poison is safe here (the map's invariants hold after
    // any partial mutation), and panicking instead would abort the process
    // when this is reached from the extern "C" output callback.
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

fn debug_assert_main_thread() {
    #[cfg(debug_assertions)]
    {
        use objc2_foundation::NSThread;
        let is_main_thread = NSThread::isMainThread_class();
        debug_assert!(
            is_main_thread,
            "display link registry mutations must happen on the main thread; \
             the registry's lock ordering and state consistency depend on it"
        );
    }
}

unsafe extern "C" fn display_link_output_callback(
    _display_link_out: *mut sys::CVDisplayLink,
    _current_time: *const sys::CVTimeStamp,
    _output_time: *const sys::CVTimeStamp,
    _flags_in: i64,
    _flags_out: *mut i64,
    display_id: *mut c_void,
) -> i32 {
    let display_id = display_id as usize as CGDirectDisplayID;
    let registry = lock_registry();
    if let Some(entry) = registry.displays.get(&display_id) {
        for (_, frame_requests) in &entry.subscribers {
            frame_requests.merge_data(1);
        }
    }
    0
}

fn subscribe(
    display_id: CGDirectDisplayID,
    frame_requests: DispatchRetained<DispatchSource>,
) -> Result<SubscriberId> {
    debug_assert_main_thread();

    let needs_link = !lock_registry().displays.contains_key(&display_id);
    let new_link = if needs_link {
        // Created outside the registry lock; see the lock ordering note above.
        Some(unsafe {
            sys::DisplayLink::new(
                display_id,
                display_link_output_callback,
                display_id as usize as *mut c_void,
            )?
        })
    } else {
        None
    };

    let (subscriber_id, link_to_start) = {
        let mut registry = lock_registry();
        let registry = &mut *registry;
        let subscriber_id = SubscriberId(registry.next_subscriber_id);
        registry.next_subscriber_id += 1;
        let entry = match (registry.displays.entry(display_id), new_link) {
            // If an entry appeared since the check above, dropping `new_link`
            // is safe: a never-started link has no io thread, so releasing it
            // doesn't race.
            (btree_map::Entry::Occupied(entry), _) => entry.into_mut(),
            (btree_map::Entry::Vacant(vacant), Some(link)) => vacant.insert(DisplayEntry {
                link,
                running: false,
                subscribers: Vec::new(),
            }),
            (btree_map::Entry::Vacant(_), None) => {
                // Entries are never removed, so an entry observed above cannot
                // have disappeared while subscriptions stay on the main thread.
                anyhow::bail!("display link registry entry vanished for display {display_id}");
            }
        };
        entry.subscribers.push((subscriber_id, frame_requests));
        let link_to_start = if entry.running {
            None
        } else {
            entry.running = true;
            // Clone the refcounted handle so the CVDisplayLinkStart call can
            // happen after the lock is released.
            Some(entry.link.clone())
        };
        (subscriber_id, link_to_start)
    };

    if let Some(mut link) = link_to_start {
        if let Err(error) = unsafe { link.start() } {
            let mut registry = lock_registry();
            if let Some(entry) = registry.displays.get_mut(&display_id) {
                entry.running = false;
                entry.subscribers.retain(|(id, _)| *id != subscriber_id);
            }
            return Err(error);
        }
    }

    Ok(subscriber_id)
}

fn unsubscribe(display_id: CGDirectDisplayID, subscriber_id: SubscriberId) {
    debug_assert_main_thread();

    let link_to_stop = {
        let mut registry = lock_registry();
        let Some(entry) = registry.displays.get_mut(&display_id) else {
            return;
        };
        entry.subscribers.retain(|(id, _)| *id != subscriber_id);
        if entry.subscribers.is_empty() && entry.running {
            entry.running = false;
            Some(entry.link.clone())
        } else {
            None
        }
    };

    if let Some(mut link) = link_to_stop {
        // A final output callback can still fire after this returns; it finds
        // no subscribers for this display and does nothing.
        unsafe { link.stop().log_err() };
    }
}

/// A per-window source of frame requests, paced by the display the window is
/// on. The wrapped dispatch source coalesces vsync ticks from the display's
/// io thread and invokes `callback(data)` on the main queue.
pub struct WindowFrameSource {
    frame_requests: DispatchRetained<DispatchSource>,
    registration: Option<(CGDirectDisplayID, SubscriberId)>,
}

impl WindowFrameSource {
    pub fn new(data: *mut c_void, callback: extern "C" fn(*mut c_void)) -> Self {
        let frame_requests = unsafe {
            let frame_requests = DispatchSource::new(
                &raw const _dispatch_source_type_data_add as *mut _,
                0,
                0,
                Some(DispatchQueue::main()),
            );
            frame_requests.set_context(data);
            frame_requests.set_event_handler_f(callback);
            // Resume before this source can ever be dropped: destroying a
            // suspended dispatch source is undefined behavior (#50875).
            frame_requests.resume();
            frame_requests
        };
        Self {
            frame_requests,
            registration: None,
        }
    }

    pub fn start(&mut self, display_id: CGDirectDisplayID) -> Result<()> {
        self.stop();
        let subscriber_id = subscribe(display_id, self.frame_requests.clone())?;
        self.registration = Some((display_id, subscriber_id));
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some((display_id, subscriber_id)) = self.registration.take() {
            unsubscribe(display_id, subscriber_id);
        }
    }
}

impl Drop for WindowFrameSource {
    fn drop(&mut self) {
        self.stop();
        // Unsubscribing prevents the output callback from reaching this source,
        // fixing ZED-7XR. Cancel before releasing to prevent further handler calls.
        // The handler's context points to the window's native view, which may
        // be deallocated after this.
        self.frame_requests.cancel();
    }
}

mod sys {
    //! Derived from display-link crate under the following license:
    //! <https://github.com/BrainiumLLC/display-link/blob/master/LICENSE-MIT>
    //! Apple docs: [CVDisplayLink](https://developer.apple.com/documentation/corevideo/cvdisplaylinkoutputcallback?language=objc)
    #![allow(dead_code, non_upper_case_globals)]

    use anyhow::Result;
    use core_graphics::display::CGDirectDisplayID;
    use foreign_types::{ForeignType, foreign_type};
    use std::{
        ffi::c_void,
        fmt::{self, Debug, Formatter},
    };

    #[derive(Debug)]
    pub enum CVDisplayLink {}

    foreign_type! {
        pub unsafe type DisplayLink {
            type CType = CVDisplayLink;
            fn drop = CVDisplayLinkRelease;
            fn clone = CVDisplayLinkRetain;
        }
    }

    impl Debug for DisplayLink {
        fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
            formatter
                .debug_tuple("DisplayLink")
                .field(&self.as_ptr())
                .finish()
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(crate) struct CVTimeStamp {
        pub version: u32,
        pub video_time_scale: i32,
        pub video_time: i64,
        pub host_time: u64,
        pub rate_scalar: f64,
        pub video_refresh_period: i64,
        pub smpte_time: CVSMPTETime,
        pub flags: u64,
        pub reserved: u64,
    }

    pub type CVTimeStampFlags = u64;

    pub const kCVTimeStampVideoTimeValid: CVTimeStampFlags = 1 << 0;
    pub const kCVTimeStampHostTimeValid: CVTimeStampFlags = 1 << 1;
    pub const kCVTimeStampSMPTETimeValid: CVTimeStampFlags = 1 << 2;
    pub const kCVTimeStampVideoRefreshPeriodValid: CVTimeStampFlags = 1 << 3;
    pub const kCVTimeStampRateScalarValid: CVTimeStampFlags = 1 << 4;
    pub const kCVTimeStampTopField: CVTimeStampFlags = 1 << 16;
    pub const kCVTimeStampBottomField: CVTimeStampFlags = 1 << 17;
    pub const kCVTimeStampVideoHostTimeValid: CVTimeStampFlags =
        kCVTimeStampVideoTimeValid | kCVTimeStampHostTimeValid;
    pub const kCVTimeStampIsInterlaced: CVTimeStampFlags =
        kCVTimeStampTopField | kCVTimeStampBottomField;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(crate) struct CVSMPTETime {
        pub subframes: i16,
        pub subframe_divisor: i16,
        pub counter: u32,
        pub time_type: u32,
        pub flags: u32,
        pub hours: i16,
        pub minutes: i16,
        pub seconds: i16,
        pub frames: i16,
    }

    pub type CVSMPTETimeType = u32;

    pub const kCVSMPTETimeType24: CVSMPTETimeType = 0;
    pub const kCVSMPTETimeType25: CVSMPTETimeType = 1;
    pub const kCVSMPTETimeType30Drop: CVSMPTETimeType = 2;
    pub const kCVSMPTETimeType30: CVSMPTETimeType = 3;
    pub const kCVSMPTETimeType2997: CVSMPTETimeType = 4;
    pub const kCVSMPTETimeType2997Drop: CVSMPTETimeType = 5;
    pub const kCVSMPTETimeType60: CVSMPTETimeType = 6;
    pub const kCVSMPTETimeType5994: CVSMPTETimeType = 7;

    pub type CVSMPTETimeFlags = u32;

    pub const kCVSMPTETimeValid: CVSMPTETimeFlags = 1 << 0;
    pub const kCVSMPTETimeRunning: CVSMPTETimeFlags = 1 << 1;

    pub type CVDisplayLinkOutputCallback = unsafe extern "C" fn(
        display_link_out: *mut CVDisplayLink,
        // A pointer to the current timestamp. This represents the timestamp when the callback is called.
        current_time: *const CVTimeStamp,
        // A pointer to the output timestamp. This represents the timestamp for when the frame will be displayed.
        output_time: *const CVTimeStamp,
        // Unused
        flags_in: i64,
        // Unused
        flags_out: *mut i64,
        // A pointer to app-defined data.
        display_link_context: *mut c_void,
    ) -> i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "CoreVideo", kind = "framework")]
    #[allow(improper_ctypes, unknown_lints, clippy::duplicated_attributes)]
    unsafe extern "C" {
        pub fn CVDisplayLinkCreateWithActiveCGDisplays(
            display_link_out: *mut *mut CVDisplayLink,
        ) -> i32;
        pub fn CVDisplayLinkSetCurrentCGDisplay(
            display_link: &mut DisplayLinkRef,
            display_id: u32,
        ) -> i32;
        pub fn CVDisplayLinkSetOutputCallback(
            display_link: &mut DisplayLinkRef,
            callback: CVDisplayLinkOutputCallback,
            user_info: *mut c_void,
        ) -> i32;
        pub fn CVDisplayLinkStart(display_link: &mut DisplayLinkRef) -> i32;
        pub fn CVDisplayLinkStop(display_link: &mut DisplayLinkRef) -> i32;
        pub fn CVDisplayLinkRelease(display_link: *mut CVDisplayLink);
        pub fn CVDisplayLinkRetain(display_link: *mut CVDisplayLink) -> *mut CVDisplayLink;
    }

    impl DisplayLink {
        /// Apple docs: [CVDisplayLinkCreateWithCGDisplay](https://developer.apple.com/documentation/corevideo/1456981-cvdisplaylinkcreatewithcgdisplay?language=objc)
        pub unsafe fn new(
            display_id: CGDirectDisplayID,
            callback: CVDisplayLinkOutputCallback,
            user_info: *mut c_void,
        ) -> Result<Self> {
            unsafe {
                let mut display_link: *mut CVDisplayLink = 0 as _;

                let code = CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link);
                anyhow::ensure!(code == 0, "could not create display link, code: {}", code);

                let mut display_link = DisplayLink::from_ptr(display_link);

                let code = CVDisplayLinkSetOutputCallback(&mut display_link, callback, user_info);
                anyhow::ensure!(code == 0, "could not set output callback, code: {}", code);

                let code = CVDisplayLinkSetCurrentCGDisplay(&mut display_link, display_id);
                anyhow::ensure!(
                    code == 0,
                    "could not assign display to display link, code: {}",
                    code
                );

                Ok(display_link)
            }
        }
    }

    impl DisplayLinkRef {
        /// Apple docs: [CVDisplayLinkStart](https://developer.apple.com/documentation/corevideo/1457193-cvdisplaylinkstart?language=objc)
        pub unsafe fn start(&mut self) -> Result<()> {
            unsafe {
                let code = CVDisplayLinkStart(self);
                anyhow::ensure!(code == 0, "could not start display link, code: {}", code);
                Ok(())
            }
        }

        /// Apple docs: [CVDisplayLinkStop](https://developer.apple.com/documentation/corevideo/1457281-cvdisplaylinkstop?language=objc)
        pub unsafe fn stop(&mut self) -> Result<()> {
            unsafe {
                let code = CVDisplayLinkStop(self);
                anyhow::ensure!(code == 0, "could not stop display link, code: {}", code);
                Ok(())
            }
        }
    }
}
