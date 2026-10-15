mod app_menu;
mod keyboard;
mod keystroke;

#[cfg(all(target_os = "linux", feature = "wayland"))]
#[expect(missing_docs)]
pub mod layer_shell;

/// Types for configuring parent-anchored popup windows such as menus, dropdowns and tooltips.
pub mod popup;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
mod threaded_dispatcher;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
mod test;

#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
mod visual_test;

#[cfg(all(feature = "screen-capture", target_os = "windows"))]
pub mod screen_capture;
#[cfg(target_os = "windows")]
mod windows_screen_capture;
#[cfg(target_os = "windows")]
pub use windows_screen_capture::WindowsScreenCaptureFrame;

#[cfg(all(target_os = "windows", feature = "screen-capture"))]
pub(crate) type PlatformScreenCaptureFrame = WindowsScreenCaptureFrame;
#[cfg(not(feature = "screen-capture"))]
pub(crate) type PlatformScreenCaptureFrame = ();
#[cfg(all(target_os = "macos", feature = "screen-capture"))]
pub(crate) type PlatformScreenCaptureFrame = core_video::image_buffer::CVImageBuffer;
#[cfg(all(
    feature = "screen-capture",
    not(any(target_os = "macos", target_os = "windows"))
))]
// Screen capture currently has native frame representations only on macOS and Windows. Keep the
// cross-platform API well-formed for enabled-but-unsupported targets; source enumeration simply
// yields no platform sources there.
pub(crate) type PlatformScreenCaptureFrame = ();

use crate::util::FluentBuilder;
use crate::{
    Action, AnyWindowHandle, App, AsyncWindowContext, BackgroundExecutor, Bounds,
    DEFAULT_WINDOW_SIZE, DevicePixels, DispatchEventResult, Edges, ExternalDragPayload, Font,
    FontId, FontMetrics, ForegroundExecutor, GlyphAtlasEntry, GlyphId, GpuSpecs, ImageSource,
    InlineLayout, InlineLayoutRequest, Keymap, LineLayout, Pixels, PlatformGestures, PlatformInput,
    Point, PreparedRasterStyle, Priority, RasterStyleRequest, RasterizedGlyph,
    RasterizedGlyphFormat, RenderGlyphParams, RenderImage, RenderImageParams, RenderSvgParams,
    Scene, SharedString, Size, SvgRenderer, SystemWindowTab, Task, TextLayoutRequest,
    ValidatedRasterizedGlyph, Window, WindowControlArea, hash, point, px,
};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use anyhow::bail;
use anyhow::{Context as _, Result};
use async_task::Runnable;
use collections::FxHashMap;
use futures::channel::oneshot;
#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
use image::RgbaImage;
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder as _, DynamicImage, Frame};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use scheduler::Instant;
pub use scheduler::RunnableMeta;
use schemars::JsonSchema;
use seahash::SeaHasher;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::ops;
use std::time::Duration;
use std::{
    ffi::OsString,
    fmt::{self, Debug},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};
use strum::EnumIter;
use uuid::Uuid;

pub use app_menu::*;
pub use keyboard::*;
pub use keystroke::*;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub(crate) use test::*;

#[cfg(any(test, feature = "test-support"))]
pub use test::{TestDispatcher, TestScreenCaptureSource, TestScreenCaptureStream};

#[cfg(any(test, feature = "test-support"))]
pub use tests::TestTextSystem;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub use threaded_dispatcher::ThreadedDispatcher;

#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub use visual_test::VisualTestPlatform;

// TODO(jk): return an enum instead of a string
/// Return which compositor we're guessing we'll use.
/// Does not attempt to connect to the given compositor.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[inline]
pub fn guess_compositor() -> &'static str {
    if std::env::var_os("ZED_HEADLESS").is_some() {
        return "Headless";
    }

    #[cfg(feature = "wayland")]
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY");
    #[cfg(not(feature = "wayland"))]
    let wayland_display: Option<std::ffi::OsString> = None;

    #[cfg(feature = "x11")]
    let x11_display = std::env::var_os("DISPLAY");
    #[cfg(not(feature = "x11"))]
    let x11_display: Option<std::ffi::OsString> = None;

    let use_wayland = wayland_display.is_some_and(|display| !display.is_empty());
    let use_x11 = x11_display.is_some_and(|display| !display.is_empty());

    if use_wayland {
        "Wayland"
    } else if use_x11 {
        "X11"
    } else {
        "Headless"
    }
}

/// The activation policy for a macOS application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacActivationPolicy {
    /// The application is an ordinary app that appears in the Dock and may have a user interface.
    #[default]
    Regular,
    /// The application doesn't appear in the Dock and doesn't have a menu bar, but it may be activated programmatically or by clicking on one of its windows.
    Accessory,
    /// The application doesn't appear in the Dock and may not create windows or be activated.
    Prohibited,
}

/// The operating system or browser that GPUI runs on, as seen by the [`App`].
///
/// Each backend crate implements this trait once: `gpui_macos`, `gpui_windows`,
/// `gpui_linux` and `gpui_web`. Application code doesn't call most of these methods
/// directly. It calls the [`App`] methods that wrap them. Use this trait to
/// implement a new backend, or to reach a platform service that [`App`] doesn't
/// expose.
///
/// Methods without a default body are required. Methods with a default body are
/// optional platform services: the default supplies the behavior for a backend
/// that does not provide the service. The [platforms](crate::_documentation::platforms)
/// table lists the backends that override each default.
pub trait Platform: 'static {
    /// Returns the executor for work that runs off the main thread.
    fn background_executor(&self) -> BackgroundExecutor;
    /// Returns the executor that runs tasks on the main thread, the thread that owns windows.
    fn foreground_executor(&self) -> ForegroundExecutor;
    /// Returns the platform's text system, which shapes and rasterizes glyphs.
    fn text_system(&self) -> Arc<dyn PlatformTextSystem>;

    /// Sets the macOS activation policy. Only `gpui_macos` overrides this; other platforms ignore the call.
    fn set_mac_activation_policy(&self, _policy: MacActivationPolicy) {}
    /// Starts the event loop and calls `on_finish_launching` once the application is ready.
    ///
    /// This method blocks until the application exits. Create windows inside
    /// `on_finish_launching`.
    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>);
    /// Asks the application to quit.
    ///
    /// On macOS the request is processed asynchronously, so the event loop keeps
    /// running briefly. The web backend only logs a warning, because a page can't quit
    /// itself.
    fn quit(&self);
    /// Restarts the application by launching a new process and quitting this one.
    ///
    /// `binary_path` defaults to the current executable when `None`, and `arguments`
    /// are passed to the new process. The web backend does nothing.
    fn restart(&self, binary_path: Option<PathBuf>, arguments: Vec<OsString>);
    /// Brings the application to the front.
    ///
    /// When `ignoring_other_apps` is `true`, the application takes focus even if
    /// another application is active. The web backend ignores the call.
    fn activate(&self, ignoring_other_apps: bool);
    /// Hides all of the application's windows. macOS hides them. Windows does nothing,
    /// and Linux logs a message and ignores the call.
    fn hide(&self);
    /// Hides all other running applications. macOS does this. Linux and the web do nothing.
    /// Windows panics, because its implementation is `unimplemented!()`.
    fn hide_other_apps(&self);
    /// Shows the applications hidden by [`Platform::hide_other_apps`]. macOS does this.
    /// Linux and the web do nothing.
    fn unhide_other_apps(&self);

    /// Returns every display currently connected, in no particular order.
    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>>;
    /// Returns the display that new windows open on by default, or `None` if there is none.
    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>>;
    /// Returns the handle of the window that currently has focus, if any.
    fn active_window(&self) -> Option<AnyWindowHandle>;
    /// Returns the open windows from front to back, or `None` if the platform doesn't track stacking order.
    ///
    /// Implemented on macOS and Linux. The default returns `None`.
    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        None
    }

    /// Returns `true` if this platform can capture the screen.
    ///
    /// Returns `true` on macOS 12.3 or later and on Windows. Other platforms return
    /// `false`.
    fn is_screen_capture_supported(&self) -> bool {
        false
    }

    /// Lists the displays and windows that can be captured.
    ///
    /// The result arrives on the returned channel once enumeration finishes. The
    /// default sends an error, so backends must override it to list sources.
    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        let (sources_tx, sources_rx) = oneshot::channel();
        sources_tx
            .send(Err(anyhow::anyhow!(
                "gpui was compiled without the screen-capture feature"
            )))
            .ok();
        sources_rx
    }

    /// Creates a native window for `handle` with the given `options`.
    ///
    /// [`App::open_window`] calls this. The returned [`PlatformWindow`] represents the
    /// native window. Errors are returned for unsupported options, such as a second
    /// window on the web.
    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>>;

    /// Returns the appearance of the application's windows.
    fn window_appearance(&self) -> WindowAppearance;

    /// Overrides the appearance (light/dark) applied to the app's windows, independent
    /// of the OS-wide setting. Pass `None` to clear the override and follow the system
    /// again. The override is reflected by [`Platform::window_appearance`].
    ///
    /// Only macOS implements this, by setting `NSApplication.appearance`. This keeps
    /// the native window chrome matching a dark app theme when the system is light, or
    /// the reverse. Other platforms ignore the call.
    fn set_window_appearance(&self, _appearance: Option<WindowAppearance>) {}

    /// Returns the order of window control buttons, or `None` if the platform
    /// doesn't use the layout. Only Linux returns a value. macOS, Windows and the web return `None`.
    fn button_layout(&self) -> Option<WindowButtonLayout> {
        None
    }

    /// Opens `url` in the default handler for its scheme, such as the browser for `https`.
    fn open_url(&self, url: &str);
    /// Registers the callback that receives URLs the operating system asks this application to open.
    ///
    /// Registering replaces any earlier callback. The web backend stores the callback
    /// but never calls it.
    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>);
    /// Registers this application as the handler for `url`'s scheme, such as `myapp://`.
    ///
    /// The task resolves once registration finishes. The web backend resolves it at once and does nothing.
    fn register_url_scheme(&self, url: &str) -> Task<Result<()>>;

    /// Shows a file or folder picker and resolves to the chosen paths.
    ///
    /// The receiver yields `Ok(None)` when the user cancels. The options control whether
    /// files, directories or several items can be picked. The web backend doesn't
    /// support this, so the receiver resolves to an error.
    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>>;
    /// Shows a save dialog in `directory` and resolves to the chosen path.
    ///
    /// `suggested_name` pre-fills the file name. The receiver yields `Ok(None)` when
    /// the user cancels. The web backend doesn't support this.
    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>>;
    /// Returns whether a single file picker can select both files and directories.
    ///
    /// Only macOS returns `true`. Other platforms return `false` and need separate
    /// pickers for files and directories.
    fn can_select_mixed_files_and_dirs(&self) -> bool;
    /// Shows `path` selected in the file manager, such as Finder or Explorer.
    fn reveal_path(&self, path: &Path);
    /// Opens `path` with the application the system associates with its file type.
    fn open_with_system(&self, path: &Path);

    /// Registers the callback the platform runs when the application is asked to quit.
    ///
    /// The callback returns `true` when shutdown has completed, and `false` when it
    /// couldn't shut down synchronously. macOS calls it while terminating and ignores
    /// the return value.
    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>);
    /// Registers the callback run when the user activates the running application again, such as clicking its Dock icon on macOS.
    fn on_reopen(&self, callback: Box<dyn FnMut()>);
    /// Registers the callback run after the system wakes from sleep.
    fn on_system_wake(&self, callback: Box<dyn FnMut()>);

    // Mobile platform methods. On mobile the OS owns the application
    // lifecycle: apps are backgrounded, foregrounded, and killed at the
    // system's discretion, and must react rather than decide.

    /// Registers a callback invoked whenever the application's lifecycle
    /// phase changes. See [`AppLifecyclePhase`] for the phase vocabulary and
    /// its mapping onto iOS and Android.
    ///
    /// Desktop platforms never invoke this.
    fn on_app_lifecycle(&self, _callback: Box<dyn FnMut(AppLifecyclePhase)>) {}

    /// Registers a callback invoked when the OS signals memory pressure
    /// (iOS `didReceiveMemoryWarning`, Android `onTrimMemory`).
    ///
    /// Desktop platforms never invoke this.
    fn on_memory_warning(&self, _callback: Box<dyn FnMut()>) {}

    /// Returns the platform's gesture recognizers, or `None` to use GPUI's portable ones.
    /// See [`PlatformGestures`].
    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        None
    }

    /// Replaces the application menu with `menus`, and uses `keymap` to show each item's shortcut.
    ///
    /// Windows stores the menus but doesn't draw them. Linux and the web don't show a
    /// menu bar.
    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap);
    /// Returns the menus last set with [`Platform::set_menus`], or `None` if the platform doesn't keep them.
    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        None
    }

    /// Sets the menu shown when the user right-clicks the app's Dock icon (macOS) or taskbar entry (Windows, as a jump list).
    fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap);
    /// Runs the dock menu item at index `_action`. Windows implements this.
    fn perform_dock_menu_action(&self, _action: usize) {}
    /// Adds `path` to the system's recent documents list. Only macOS implements this.
    fn add_recent_document(&self, _path: &Path) {}
    /// Updates the Windows jump list with `_menus` and the recent-file `_entries`, and returns the entries that were kept.
    ///
    /// Other backends return an empty list.
    fn update_jump_list(
        &self,
        _menus: Vec<MenuItem>,
        _entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        Task::ready(Vec::new())
    }
    /// Registers the callback that receives actions chosen from the application menu.
    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>);
    /// Registers the callback run just before a menu opens, so the app can update its menu items.
    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>);
    /// Registers the callback that decides whether a menu item is enabled. Return `true` to enable the action.
    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>);

    /// Returns the device's current thermal state. See [`ThermalState`].
    fn thermal_state(&self) -> ThermalState;
    /// Registers the callback run when [`Platform::thermal_state`] changes. Read the new state from the platform inside the callback.
    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>);

    /// Sets the application's identifier and user-visible name.
    ///
    /// Windows sets `identifier` as the AppUserModelID, unless the process already has
    /// package identity. Linux stores `name`. macOS and the web ignore both values.
    ///
    /// Call it once early in startup, before opening windows or posting notifications.
    fn set_app_identity(&self, identifier: &str, name: &str) {
        _ = (identifier, name);
    }

    /// Posts `notification` to the operating system's notification center.
    ///
    /// Posting a notification whose [`SystemNotification::tag`] matches an
    /// earlier one replaces that notification where the platform supports it.
    /// No-op on platforms without notification support, or when delivery is
    /// unavailable (e.g. authorization was denied).
    fn show_system_notification(&self, notification: SystemNotification) {
        _ = notification;
    }

    /// Removes the delivered or pending notification with this tag.
    ///
    /// Best-effort: some platforms cannot retract a notification once shown,
    /// in which case it ages out of the notification center on its own.
    fn dismiss_system_notification(&self, tag: &str) {
        _ = tag;
    }

    /// Registers the callback invoked when the user activates a system
    /// notification, either by clicking its body or one of its action
    /// buttons.
    ///
    /// Implementations must invoke the callback on the main thread.
    fn on_system_notification_response(
        &self,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        _ = callback;
    }

    /// Returns the name of the display server or compositor, such as `"Wayland"`, `"X11"` or `"Headless"`.
    ///
    /// Returns an empty string by default.
    fn compositor_name(&self) -> &'static str {
        ""
    }
    /// Returns the path of the running application's executable.
    fn app_path(&self) -> Result<PathBuf>;
    /// Returns the path of a helper executable named `name` that ships alongside the application.
    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf>;

    /// Shows the mouse cursor `style` over this application's windows.
    fn set_cursor_style(&self, style: CursorStyle);

    /// Hides the mouse cursor until the user moves the mouse over one of
    /// this application's windows.
    fn hide_cursor_until_mouse_moves(&self);

    /// Returns whether the mouse cursor is currently visible.
    fn is_cursor_visible(&self) -> bool;

    /// Returns `true` when scrollbars should hide until the user scrolls. macOS follows
    /// the overlay-scroller setting, Windows and Linux read their UI settings, and the
    /// web returns `true`.
    fn should_auto_hide_scrollbars(&self) -> bool;

    /// Returns the clipboard's contents synchronously, or `None` if it's empty.
    ///
    /// The web backend can't read the clipboard synchronously and always returns
    /// `None`. Use [`Platform::read_from_clipboard_async`] there.
    fn read_from_clipboard(&self) -> Option<ClipboardItem>;
    /// Replaces the clipboard's contents with `item`.
    fn write_to_clipboard(&self, item: ClipboardItem);

    /// Reads the clipboard, resolving once its contents are available.
    ///
    /// Most platforms read synchronously and return a ready task. A platform with
    /// inherently asynchronous or permission-gated access, such as the browser's
    /// Async Clipboard API, overrides this method. On those platforms,
    /// [`Platform::read_from_clipboard`] cannot return the contents; callers that
    /// can await should use this method.
    fn read_from_clipboard_async(&self) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        Task::ready(Ok(self.read_from_clipboard()))
    }

    /// Returns the contents of the X11 or Wayland primary selection, the text selected with the mouse.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn read_from_primary(&self) -> Option<ClipboardItem>;
    /// Sets the primary selection to `item`. The selection is the text highlighted with the mouse, pasted with middle-click.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_to_primary(&self, item: ClipboardItem);

    /// Returns the contents of the macOS find pasteboard, which the find bar shares across apps.
    #[cfg(target_os = "macos")]
    fn read_from_find_pasteboard(&self) -> Option<ClipboardItem>;
    /// Sets the macOS find pasteboard to `item`.
    #[cfg(target_os = "macos")]
    fn write_to_find_pasteboard(&self, item: ClipboardItem);

    /// Stores `username` and `password` for `url` in the system credential store.
    ///
    /// The task resolves once the credentials are saved. Use [`Platform::read_credentials`] to read them back.
    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>>;
    /// Reads the username and password stored for `url`, resolving to `None` if there are none.
    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>>;
    /// Deletes the credentials stored for `url`.
    fn delete_credentials(&self, url: &str) -> Task<Result<()>>;

    /// Returns the current keyboard layout.
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout>;
    /// Returns the mapper that converts key codes to the characters the current layout produces.
    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper>;
    /// Registers the callback run when the keyboard layout changes.
    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>);

    /// Registers extra GPU device requirements, such as features and limits, before the first window is opened.
    ///
    /// The value must be a `gpui_wgpu::WgpuDeviceRequirements`. macOS and Linux use
    /// it. Other backends ignore it.
    fn set_gpu_requirements(&self, _requirements: Box<dyn std::any::Any>) {}

    /// Sets the label applied to credentials stored in the system keyring.
    /// Only Linux/FreeBSD use this label.
    fn set_keyring_label(&self, _label: SharedString) {}

    /// Returns `true` if [`Platform::play_haptic_feedback`] has an effect on this platform.
    ///
    /// Only macOS returns `true`, for trackpads with Force Touch.
    fn supports_haptic_feedback(&self) -> bool {
        false
    }

    /// Plays haptic feedback of the given style.
    ///
    /// No-op on platforms that don't support haptic feedback.
    fn play_haptic_feedback(&self, _style: HapticFeedbackStyle) {}
}

/// A display, such as a monitor or a laptop screen, as the platform reports it.
///
/// Obtain one from [`App::displays`] or [`App::primary_display`]. Its bounds are
/// in logical pixels.
pub trait PlatformDisplay: Debug {
    /// Returns the display's id for the current session. To persist a display across restarts, use [`PlatformDisplay::uuid`].
    fn id(&self) -> DisplayId;

    /// Returns a stable identifier for this display that survives restarts.
    ///
    /// Returns an error if the platform can't provide one.
    fn uuid(&self) -> Result<Uuid>;

    /// Returns the display's full bounds in logical pixels, including areas such as the taskbar.
    fn bounds(&self) -> Bounds<Pixels>;

    /// Returns the usable bounds, excluding the menu bar, Dock and taskbar.
    ///
    /// Defaults to [`PlatformDisplay::bounds`] when a backend doesn't override it.
    fn visible_bounds(&self) -> Bounds<Pixels> {
        self.bounds()
    }

    /// Returns the bounds a new window opens at on this display: the default window size, centered and clipped to the display.
    fn default_bounds(&self) -> Bounds<Pixels> {
        let bounds = self.bounds();
        let center = bounds.center();
        let clipped_window_size = DEFAULT_WINDOW_SIZE.min(&bounds.size);

        let offset = clipped_window_size / 2.0;
        let origin = point(center.x - offset.width, center.y - offset.height);
        Bounds::new(origin, clipped_window_size)
    }
}

/// A notification for the operating system's notification center, rather than
/// in-app UI.
///
/// Post one with [`App::show_system_notification`]. Responses arrive as a
/// [`SystemNotificationResponse`] with the same `tag`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemNotification {
    /// Stable identity for the notification. Posting a new notification with
    /// the same tag replaces the previous one where the platform supports it,
    /// and responses carry the tag back to the application.
    pub tag: SharedString,
    /// The notification's headline.
    pub title: SharedString,
    /// Additional text displayed below the title.
    pub body: SharedString,
    /// Buttons shown with the notification. Platforms may show fewer than given.
    pub actions: Vec<SystemNotificationAction>,
}

/// A button on a [`SystemNotification`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SystemNotificationAction {
    /// Identifies the button in a [`SystemNotificationResponse::action_id`].
    pub id: SharedString,
    /// The text shown on the button.
    pub label: SharedString,
}

/// The user's response to a [`SystemNotification`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemNotificationResponse {
    /// The [`SystemNotification::tag`] of the notification the user activated.
    pub tag: SharedString,
    /// The [`SystemNotificationAction::id`] of the button clicked, or `None` when the body was clicked.
    pub action_id: Option<SharedString>,
}

/// The device's thermal pressure, from least to most constrained.
///
/// Read it with [`App::thermal_state`]. When it changes, the callback from
/// [`App::on_thermal_state_change`] runs. Reduce discretionary work, such as
/// animation frames or background indexing, as the state rises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    /// The system has no thermal constraints.
    Nominal,
    /// The system is slightly constrained. Reduce discretionary work.
    Fair,
    /// The system is moderately constrained. Reduce CPU and GPU intensive work.
    Serious,
    /// The system is critically constrained. Minimize resource use.
    Critical,
}

/// A kind of haptic feedback to play with [`App::play_haptic_feedback`].
///
/// The styles map to `NSHapticFeedbackPattern` on macOS. Other platforms do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticFeedbackStyle {
    /// A generic tap, for most interactions.
    Generic,
    /// A sharp snap, for alignment guides, detents and snapping.
    Alignment,
    /// A distinct click, for slider steps, toggles and discrete state changes.
    LevelChange,
}

/// Describes a screen or window that can be captured.
///
/// Returned by [`ScreenCaptureSource::metadata`] and [`ScreenCaptureStream::metadata`].
#[derive(Clone)]
pub struct SourceMetadata {
    /// An opaque identifier for the source. It is stable only within one session.
    pub id: u64,
    /// A name to show in a picker, or `None` when the platform gives none.
    pub label: Option<SharedString>,
    /// Whether this is the main display, or `None` when unknown.
    pub is_main: Option<bool>,
    /// The size of the captured video, in device pixels.
    pub resolution: Size<DevicePixels>,
}

/// A screen or window that can be captured.
///
/// Get sources from [`Platform::screen_capture_sources`]. Call
/// [`ScreenCaptureSource::stream`] to start receiving frames.
pub trait ScreenCaptureSource {
    /// Returns a description of this source.
    fn metadata(&self) -> Result<SourceMetadata>;

    /// Starts capturing this source and calls `frame_callback` with each frame.
    ///
    /// The returned receiver resolves to the running [`ScreenCaptureStream`], or to an
    /// error if capture can't start. The platform chooses the thread that runs
    /// `frame_callback`, so the callback must be `Send`. Drop the stream to stop capture.
    fn stream(
        &self,
        foreground_executor: &ForegroundExecutor,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>>;
}

/// A running capture of a [`ScreenCaptureSource`].
pub trait ScreenCaptureStream {
    /// Returns a description of the captured source.
    fn metadata(&self) -> Result<SourceMetadata>;
}

/// One frame of captured video.
///
/// The wrapped value is platform-specific. On macOS it is a `CVImageBuffer`, and on
/// Windows it is a `WindowsScreenCaptureFrame`. Without the `screen-capture`
/// feature it is `()`.
pub struct ScreenCaptureFrame(pub PlatformScreenCaptureFrame);

/// An identifier for a display that is valid for the current session.
#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub struct DisplayId(pub(crate) u64);

impl DisplayId {
    /// Creates a `DisplayId` from a raw platform display identifier.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for DisplayId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

impl From<DisplayId> for u64 {
    fn from(id: DisplayId) -> Self {
        id.0
    }
}

impl Debug for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayId({})", self.0)
    }
}

/// The edge or corner of a window that a user drags to resize it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    /// The top edge
    Top,
    /// The top right corner
    TopRight,
    /// The right edge
    Right,
    /// The bottom right corner
    BottomRight,
    /// The bottom edge
    Bottom,
    /// The bottom left corner
    BottomLeft,
    /// The left edge
    Left,
    /// The top left corner
    TopLeft,
}

impl From<ResizeEdge> for CursorStyle {
    fn from(edge: ResizeEdge) -> Self {
        match edge {
            ResizeEdge::Top => Self::ResizeUp,
            ResizeEdge::Right => Self::ResizeRight,
            ResizeEdge::Bottom => Self::ResizeDown,
            ResizeEdge::Left => Self::ResizeLeft,
            ResizeEdge::TopLeft => Self::ResizeUpLeft,
            ResizeEdge::TopRight => Self::ResizeUpRight,
            ResizeEdge::BottomLeft => Self::ResizeDownLeft,
            ResizeEdge::BottomRight => Self::ResizeDownRight,
        }
    }
}

/// Whether a window draws its own titlebar and borders, or the window server does.
///
/// Set this with [`WindowOptions::window_decorations`]. Linux's Wayland and X11
/// backends honor it. The other backends ignore it.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum WindowDecorations {
    /// The window server draws the titlebar and borders.
    #[default]
    Server,
    /// GPUI draws the titlebar and borders.
    Client,
}

/// The decorations a window is currently using.
///
/// Read it with [`Window::window_decorations`] to decide whether to draw resize handles or
/// window controls yourself.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum Decorations {
    /// The window server draws the decorations.
    #[default]
    Server,
    /// GPUI draws the decorations. `tiling` says which edges are snapped to the screen.
    Client {
        /// Which edges of the window are tiled.
        tiling: Tiling,
    },
}

/// The window controls a platform supports.
///
/// The default assumes that every control is available.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct WindowControls {
    /// Whether the window can enter fullscreen.
    pub fullscreen: bool,
    /// Whether the window can be maximized.
    pub maximize: bool,
    /// Whether the window can be minimized.
    pub minimize: bool,
    /// Whether the window has a window menu.
    pub window_menu: bool,
}

impl Default for WindowControls {
    fn default() -> Self {
        // Assume that we can do anything, unless told otherwise
        Self {
            fullscreen: true,
            maximize: true,
            minimize: true,
            window_menu: true,
        }
    }
}

/// A window control button, used in [`WindowButtonLayout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowButton {
    /// The minimize button
    Minimize,
    /// The maximize button
    Maximize,
    /// The close button
    Close,
}

impl WindowButton {
    /// Returns a stable element id for rendering this button.
    ///
    /// Use the id to find the button's element in a titlebar. The ids are `"minimize"`,
    /// `"maximize"` and `"close"`.
    pub fn id(&self) -> &'static str {
        match self {
            WindowButton::Minimize => "minimize",
            WindowButton::Maximize => "maximize",
            WindowButton::Close => "close",
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn index(&self) -> usize {
        match self {
            WindowButton::Minimize => 0,
            WindowButton::Maximize => 1,
            WindowButton::Close => 2,
        }
    }
}

/// The largest number of [`WindowButton`]s on one side of the titlebar.
pub const MAX_BUTTONS_PER_SIDE: usize = 3;

/// Which [`WindowButton`]s appear on each side of the titlebar, in order.
///
/// On Linux, this is read from the desktop environment's configuration
/// (the `button-layout` key of `org.gnome.desktop.wm.preferences`) via `WindowButtonLayout::parse`.
/// Empty slots are `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowButtonLayout {
    /// The buttons on the left side of the titlebar, from the edge inwards.
    pub left: [Option<WindowButton>; MAX_BUTTONS_PER_SIDE],
    /// The buttons on the right side of the titlebar, from the edge inwards.
    pub right: [Option<WindowButton>; MAX_BUTTONS_PER_SIDE],
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl WindowButtonLayout {
    /// Returns GPUI's fallback layout for Linux titlebars: minimize, maximize and close on the right.
    pub fn linux_default() -> Self {
        Self {
            left: [None; MAX_BUTTONS_PER_SIDE],
            right: [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close),
            ],
        }
    }

    /// Parses a GNOME-style `button-layout` string such as `"close,minimize:maximize"`.
    ///
    /// The text before the `:` is the left side and the text after it is the right side.
    /// Names are `minimize`, `maximize` and `close`. Unknown names and repeated buttons
    /// are skipped, and each side keeps at most [`MAX_BUTTONS_PER_SIDE`] buttons.
    ///
    /// # Errors
    ///
    /// Returns an error when the string names no valid buttons and at least one name is unknown.
    pub fn parse(layout_string: &str) -> Result<Self> {
        fn parse_side(
            s: &str,
            seen_buttons: &mut [bool; MAX_BUTTONS_PER_SIDE],
            unrecognized: &mut Vec<String>,
        ) -> [Option<WindowButton>; MAX_BUTTONS_PER_SIDE] {
            let mut result = [None; MAX_BUTTONS_PER_SIDE];
            let mut i = 0;
            for name in s.split(',') {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let button = match trimmed {
                    "minimize" => Some(WindowButton::Minimize),
                    "maximize" => Some(WindowButton::Maximize),
                    "close" => Some(WindowButton::Close),
                    other => {
                        unrecognized.push(other.to_string());
                        None
                    }
                };
                if let Some(button) = button {
                    if seen_buttons[button.index()] {
                        continue;
                    }
                    if let Some(slot) = result.get_mut(i) {
                        *slot = Some(button);
                        seen_buttons[button.index()] = true;
                        i += 1;
                    }
                }
            }
            result
        }

        let (left_str, right_str) = layout_string.split_once(':').unwrap_or(("", layout_string));
        let mut unrecognized = Vec::new();
        let mut seen_buttons = [false; MAX_BUTTONS_PER_SIDE];
        let layout = Self {
            left: parse_side(left_str, &mut seen_buttons, &mut unrecognized),
            right: parse_side(right_str, &mut seen_buttons, &mut unrecognized),
        };

        if !unrecognized.is_empty()
            && layout.left.iter().all(Option::is_none)
            && layout.right.iter().all(Option::is_none)
        {
            bail!(
                "button layout string {:?} contains no valid buttons (unrecognized: {})",
                layout_string,
                unrecognized.join(", ")
            );
        }

        Ok(layout)
    }

    /// Formats the layout back into a GNOME-style `button-layout` string.
    #[cfg(test)]
    pub fn format(&self) -> String {
        fn format_side(buttons: &[Option<WindowButton>; MAX_BUTTONS_PER_SIDE]) -> String {
            buttons
                .iter()
                .flatten()
                .map(|button| match button {
                    WindowButton::Minimize => "minimize",
                    WindowButton::Maximize => "maximize",
                    WindowButton::Close => "close",
                })
                .collect::<Vec<_>>()
                .join(",")
        }

        format!("{}:{}", format_side(&self.left), format_side(&self.right))
    }
}

/// Which edges of a window are tiled against the screen edge.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct Tiling {
    /// Whether the top edge is tiled.
    pub top: bool,
    /// Whether the left edge is tiled.
    pub left: bool,
    /// Whether the right edge is tiled.
    pub right: bool,
    /// Whether the bottom edge is tiled.
    pub bottom: bool,
}

impl Tiling {
    /// Returns a [`Tiling`] with every edge tiled.
    pub fn tiled() -> Self {
        Self {
            top: true,
            left: true,
            right: true,
            bottom: true,
        }
    }

    /// Returns `true` if any edge is tiled.
    pub fn is_tiled(&self) -> bool {
        self.top || self.left || self.right || self.bottom
    }
}

/// The callbacks an accessibility adapter uses to talk to the application.
///
/// The platform's accessibility adapter calls these when a screen reader connects,
/// disconnects, or asks for an action. Register them with
/// [`PlatformWindow::a11y_init`].
pub struct A11yCallbacks {
    /// Returns the accessibility tree to send when a screen reader connects.
    pub activation: Box<dyn Fn() -> Option<accesskit::TreeUpdate> + Send + 'static>,
    /// Handles an action, such as focusing or activating an element, that a screen reader requested.
    pub action: Box<dyn Fn(accesskit::ActionRequest) + Send + 'static>,
    /// Runs when the screen reader disconnects.
    pub deactivation: Box<dyn Fn() + Send + 'static>,
}

/// Options for a frame the platform asks GPUI to draw. The platform passes them to
/// the callback registered with [`PlatformWindow::on_request_frame`].
#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub struct RequestFrameOptions {
    /// Whether the frame must be presented to the screen, not only drawn.
    pub require_presentation: bool,
    /// Whether to redraw everything, ignoring cached rendering state.
    pub force_render: bool,
}

/// The application's lifecycle phase, as owned and reported by a mobile OS.
///
/// `Inactive` means visible but not receiving input (a system dialog on
/// top), while `Background` means not visible at all, with process death
/// possible at any time thereafter.
///
/// | Phase        | iOS                          | Android      |
/// |--------------|------------------------------|--------------|
/// | `Active`     | `didBecomeActive`            | `onResume`   |
/// | `Inactive`   | `willResignActive`           | `onPause`    |
/// | `Background` | `didEnterBackground`         | `onStop`     |
/// | `Foreground` | `willEnterForeground`        | `onStart`    |
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum AppLifecyclePhase {
    /// Foreground and receiving input.
    Active,
    /// Foreground (visible) but not receiving input.
    Inactive,
    /// Not visible. The GPU surface may be destroyed while backgrounded and
    /// the process may be killed without further notice.
    Background,
    /// Becoming visible again, before input is restored.
    Foreground,
}

/// Regions of a window that are obscured or reserved by the system.
///
/// Mobile applications often share space in their window with system-specific
/// geometry, from keyboards to camera notches. In GPUI, all this is abstracted
/// into a single "inset" which should be overlaid on the window's bounds.
/// It is up to the application develop to determine how to handle these cases.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowInsets {
    /// Regions covered by system UI or hardware: status bar, display
    /// cutouts/notch, home indicator, navigation bars.
    /// (iOS: `safeAreaInsets`. Android: `WindowInsets` of types
    /// `systemBars() | displayCutout()`.)
    pub safe_area: Edges<Pixels>,
    /// The region covered by the keyboard, when present.
    /// (iOS: derived from `keyboardWillShow`/frame-change notifications.
    /// Android: `WindowInsets.Type.ime()`.)
    pub ime: Edges<Pixels>,
}

impl WindowInsets {
    /// Returns, on each side, the larger of the system inset and the keyboard inset.
    ///
    /// Content should stay clear of this area.
    pub fn effective(&self) -> Edges<Pixels> {
        Edges {
            top: self.safe_area.top.max(self.ime.top),
            right: self.safe_area.right.max(self.ime.right),
            bottom: self.safe_area.bottom.max(self.ime.bottom),
            left: self.safe_area.left.max(self.ime.left),
        }
    }
}

/// A change in the state of the focused text input.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum TextInputStateChange {
    /// An editable element gained focus.
    FocusGained,
    /// The focused editable element lost focus.
    FocusLost,
    /// The selection or caret moved
    SelectionChanged,
    /// The document content changed outside of platform-initiated edits.
    ContentChanged,
}

/// The native window that GPUI draws into, implemented by each backend.
///
/// [`App::open_window`] creates one through [`Platform::open_window`] for each
/// [`Window`]. Application code doesn't call these methods directly. [`Window`] and
/// the [`Platform`] wrap them. Each backend implements the trait for its windowing
/// system. Methods with a default body are optional, and the default is the behavior
/// for a platform that lacks the feature.
///
/// Methods that a platform doesn't support are commented with the platform that
/// does.
pub trait PlatformWindow: HasWindowHandle + HasDisplayHandle {
    /// Returns the window's frame in screen coordinates, in logical pixels.
    fn bounds(&self) -> Bounds<Pixels>;
    /// Returns whether the window is maximized.
    fn is_maximized(&self) -> bool;
    /// Returns the window's state, and the bounds to restore when it leaves that state.
    fn window_bounds(&self) -> WindowBounds;
    /// Returns the drawable size, excluding the titlebar, in logical pixels.
    fn content_size(&self) -> Size<Pixels>;
    /// Resizes the window's content area to `size`, in logical pixels.
    fn resize(&mut self, size: Size<Pixels>);
    /// Returns the number of device pixels per logical pixel.
    fn scale_factor(&self) -> f32;
    /// Returns the window's light or dark appearance.
    fn appearance(&self) -> WindowAppearance;
    /// Returns the display the window is on, or `None` if the platform can't tell.
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>>;
    /// Returns the mouse position in window coordinates, in logical pixels.
    fn mouse_position(&self) -> Point<Pixels>;
    /// Returns the modifier keys currently held down.
    fn modifiers(&self) -> Modifiers;
    /// Returns the state of the Caps Lock key.
    fn capslock(&self) -> Capslock;
    /// Installs the handler that receives text input for the focused element.
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler);
    /// Removes and returns the installed input handler.
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler>;
    /// Apply the focused text region's [`TextInputConfiguration`] to the
    /// platform's text input session (e.g. attributes of the hidden editable
    /// element on web). Called only when the configuration changes, because
    /// reconfiguring a live input session can restart the IME connection.
    fn set_text_input_configuration(&mut self, _configuration: TextInputConfiguration) {}
    /// Shows a native alert or prompt with `answers` as its buttons.
    ///
    /// The returned receiver yields the index in `answers` of the button the user
    /// chose. Returns `None` when the platform can't show a prompt. The web backend
    /// always returns `None`.
    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>>;
    /// Request activation, optionally using an externally supplied token.
    /// Returns whether a request was sent, not whether focus was granted.
    fn activate(&self, token: Option<&str>) -> bool;
    /// Requests that the operating system draw attention to this window.
    fn request_attention(&self) {}
    /// Returns whether this window has keyboard focus.
    fn is_active(&self) -> bool;
    /// Returns whether the mouse is over this window.
    fn is_hovered(&self) -> bool;
    /// Returns how the window background is drawn.
    fn background_appearance(&self) -> WindowBackgroundAppearance;
    /// Sets the window title.
    fn set_title(&mut self, title: &str);
    /// Sets how the window background is drawn: opaque, transparent or blurred.
    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance);
    /// Show or hide the window without explicitly requesting focus.
    ///
    /// The default implementation does nothing for platforms that do not support
    /// changing window visibility at runtime.
    fn set_visible(&self, _visible: bool) {}
    /// Minimizes the window. The web backend logs a warning and does nothing.
    fn minimize(&self);
    /// Zooms the window with the platform's zoom behavior. The web backend logs a
    /// warning and does nothing.
    fn zoom(&self);
    /// Enters fullscreen, or leaves it if the window is already fullscreen.
    fn toggle_fullscreen(&self);
    /// Returns whether the window is fullscreen.
    fn is_fullscreen(&self) -> bool;
    /// Returns a function that wakes the event loop to draw a frame, or `None` if the
    /// platform doesn't provide one.
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        None
    }
    /// Registers the callback that draws a frame when the platform asks for one.
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>);
    /// Registers the callback that receives input events. Its return value tells the
    /// platform whether GPUI handled the event.
    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>);
    /// Registers the callback run when the window gains or loses focus. It receives `true` when the window is active.
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>);
    /// Registers the callback run when the mouse enters or leaves the window. It receives `true` on enter.
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>);
    /// Registers the callback run when the content size or scale factor changes. It
    /// receives the new size and the scale factor.
    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>);
    /// Registers the callback run when the window moves.
    fn on_moved(&self, callback: Box<dyn FnMut()>);
    /// Registers the callback run when the user asks to close the window. Return `false` to keep it open.
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>);
    /// Registers the callback the platform calls to find out which titlebar control, if any, the mouse is over.
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>);
    /// Registers the callback run once the window has closed.
    fn on_close(&self, callback: Box<dyn FnOnce()>);
    /// Registers the callback run when the window's appearance changes.
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>);
    /// Registers the callback run when the titlebar button layout changes. The default does nothing.
    fn on_button_layout_changed(&self, _callback: Box<dyn FnMut()>) {}
    /// Draws `scene` into the window. GPUI calls this once per frame.
    fn draw(&self, scene: &Scene);
    /// Asks the platform to request another frame soon. The default does nothing.
    fn schedule_frame(&self) {}
    /// Returns the atlas that stores the glyph and image textures for this window.
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
    /// Returns whether the renderer supports subpixel (LCD) text antialiasing. macOS returns `false`.
    fn is_subpixel_rendering_supported(&self) -> bool;

    // macOS specific methods
    /// Returns the window title. macOS and the Linux headless window override this. The default returns an empty string.
    fn get_title(&self) -> String {
        String::new()
    }
    /// Returns the windows in this window's native tab group, or `None` if the window isn't tabbed. macOS only.
    fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        None
    }
    /// Returns whether the native tab bar is visible. macOS only.
    fn tab_bar_visible(&self) -> bool {
        false
    }
    /// Marks the window's document as edited or unedited. macOS only.
    fn set_edited(&mut self, _edited: bool) {}
    /// Sets the document path that the titlebar shows for the window. macOS only.
    fn set_document_path(&self, _path: Option<&std::path::Path>) {}
    /// Toggles simple fullscreen, which fills the screen without creating a new Space. macOS only.
    fn toggle_simple_fullscreen(&self) {}
    /// Returns whether the window is in simple fullscreen. macOS only.
    fn is_simple_fullscreen(&self) -> bool {
        false
    }
    /// Moves the traffic-light buttons to `position`, in logical pixels. macOS only.
    #[cfg(target_os = "macos")]
    fn set_traffic_light_position(&self, _position: Point<Pixels>) {}
    /// Show the platform character palette.
    ///
    /// The default implementation logs a warning when the backend does not
    /// support showing a character palette.
    fn show_character_palette(&self) {
        log::warn!("show_character_palette is not implemented by this platform backend");
    }
    /// Runs the titlebar double-click action. macOS only, and the default does nothing.
    fn titlebar_double_click(&self, _is_resizable: bool, _is_minimizable: bool) {}
    /// Registers the callback for the Move Tab to New Window action. macOS only.
    fn on_move_tab_to_new_window(&self, _callback: Box<dyn FnMut()>) {}
    /// Registers the callback for the Merge All Windows action. macOS only.
    fn on_merge_all_windows(&self, _callback: Box<dyn FnMut()>) {}
    /// Registers the callback for selecting the previous tab. macOS only.
    fn on_select_previous_tab(&self, _callback: Box<dyn FnMut()>) {}
    /// Registers the callback for selecting the next tab. macOS only.
    fn on_select_next_tab(&self, _callback: Box<dyn FnMut()>) {}
    /// Registers the callback for showing or hiding the tab bar. macOS only.
    fn on_toggle_tab_bar(&self, _callback: Box<dyn FnMut()>) {}
    /// Merges all windows into one tab group. macOS only.
    fn merge_all_windows(&self) {}
    /// Moves this window's tab into a new window. macOS only.
    fn move_tab_to_new_window(&self) {}
    /// Shows or hides the tab overview. macOS only.
    fn toggle_window_tab_overview(&self) {}
    /// Sets the identifier that groups windows into one native tab group. macOS only.
    fn set_tabbing_identifier(&self, _identifier: Option<String>) {}

    /// Returns the window's Win32 `HWND`. Windows only.
    #[cfg(target_os = "windows")]
    fn get_raw_handle(&self) -> windows::Win32::Foundation::HWND;

    // Linux specific methods
    /// Returns the window's state and bounds, excluding decorations. Defaults to [`PlatformWindow::window_bounds`].
    fn inner_window_bounds(&self) -> WindowBounds {
        self.window_bounds()
    }
    /// Asks for server-side or client-side decorations. Linux only.
    fn request_decorations(&self, _decorations: WindowDecorations) {}
    /// Shows the window menu at `position`, in logical pixels. Linux only.
    fn show_window_menu(&self, _position: Point<Pixels>) {}
    /// Starts an interactive move, as if the user dragged the titlebar. Implemented on macOS, Windows and Linux; the web backend does nothing.
    fn start_window_move(&self) {}
    /// Returns whether this window can start a drag to another application.
    fn can_start_external_drag(&self) -> bool {
        false
    }
    /// Starts a drag of `payload` to another application. Returns `true` if the drag started.
    fn start_external_drag(&self, _payload: &ExternalDragPayload) -> bool {
        false
    }
    /// Starts an interactive resize from `edge`. Linux only.
    fn start_window_resize(&self, _edge: ResizeEdge) {}
    /// Reserves `zone` logical pixels at the screen edge for a layer-shell surface. Wayland only.
    fn set_exclusive_zone(&self, _zone: Pixels) {}
    /// Sets the screen edge that a layer-shell exclusive zone uses. Wayland only.
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    fn set_exclusive_edge(&self, _edge: layer_shell::Anchor) {}
    /// Limits input to `region`, in logical pixels. `None` lets the whole window take input. Linux only.
    fn set_input_region(&self, _region: Option<&[Bounds<Pixels>]>) {}
    /// Returns the decorations the window is using. The default is [`Decorations::Server`].
    fn window_decorations(&self) -> Decorations {
        Decorations::Server
    }
    /// Sets the application id that the window manager uses to group windows and find the desktop entry. Linux only.
    fn set_app_id(&mut self, _app_id: &str) {}
    /// Maps the window so the platform shows it. The default does nothing and returns `Ok(())`.
    fn map_window(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Returns which window controls the platform supports.
    fn window_controls(&self) -> WindowControls {
        WindowControls::default()
    }
    /// Sets the width of the client-side shadow that surrounds the window, in logical pixels. Linux only.
    fn set_client_inset(&self, _inset: Pixels) {}
    /// Returns the GPU's name and driver details, or `None` if they're unavailable.
    fn gpu_specs(&self) -> Option<GpuSpecs>;

    /// Returns the GPU context for this window's renderer.
    /// The returned `Box` contains `(Arc<wgpu::Device>, Arc<wgpu::Queue>)`.
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    ))]
    fn gpu_context(&self) -> Option<Box<dyn std::any::Any>> {
        None
    }

    /// Returns typed backend-specific GPU context information for custom
    /// controls. The value is intentionally type-erased in this crate so the
    /// core UI crate does not depend on a rendering backend.
    #[cfg(any(
        target_family = "wasm",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos"
    ))]
    fn gpu_context_info(&self) -> Option<Box<dyn std::any::Any>> {
        None
    }

    /// Whether this window's GPU device has been lost (the platform renderer
    /// recovers it on a subsequent draw). `None` when the backend cannot
    /// know. Safe to call mid-recovery, unlike `gpu_context`. Embedders that
    /// captured the device from `gpu_context` should stop submitting while
    /// this is `Some(true)` and re-acquire the device once it reads
    /// `Some(false)` again.
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    ))]
    fn gpu_device_lost(&self) -> Option<bool> {
        None
    }

    /// Moves the input method's candidate window next to `bounds`, in window coordinates.
    fn update_ime_position(&self, _bounds: Bounds<Pixels>);

    // Mobile platform methods.

    /// The regions of this window currently obscured or reserved by the
    /// system. Zero on platforms without such regions.
    fn insets(&self) -> WindowInsets {
        WindowInsets::default()
    }

    /// Registers a callback invoked whenever [`Self::insets`] change.
    ///
    /// Contract: fires continuously during animated transitions (Android
    /// `WindowInsetsAnimation` progress; on iOS the platform interpolates
    /// the keyboard animation curve on frame ticks) and is exact at rest.
    fn on_insets_changed(&self, _callback: Box<dyn FnMut(WindowInsets)>) {}

    /// Sets the handler for the system back action (Android back
    /// button/gesture; no source on iOS or desktop).
    fn set_back_handler(&self, _callback: Box<dyn FnMut()>) {}

    /// Declares whether the application would currently handle the system
    /// back action (e.g. navigation depth > 0).
    fn set_back_enabled(&self, _enabled: bool) {}

    /// Requests that the soft keyboard be shown. Mobile only.
    fn show_soft_keyboard(&self) {}

    /// Requests that the soft keyboard be hidden. Mobile only.
    fn hide_soft_keyboard(&self) {}

    /// Tells the platform that the focused text input changed.
    fn text_input_state_changed(&self, _change: TextInputStateChange) {}

    /// Plays the system alert sound.
    fn play_system_bell(&self) {}

    /// Initialize the accessibility adapter with callbacks.
    fn a11y_init(&self, _callbacks: A11yCallbacks) {}

    /// Provide a TreeUpdate to the accessibility adapter.
    fn a11y_tree_update(&self, _tree_update: accesskit::TreeUpdate) {}

    /// Inform the adapter of updated window bounds.
    fn a11y_update_window_bounds(&self) {}

    /// Returns this window as a `TestWindow`, if it is one. Only the test platform's windows are.
    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_test(&mut self) -> Option<&mut TestWindow> {
        None
    }

    /// Renders the given scene to a texture and returns the pixel data as an RGBA image.
    /// This does not present the frame to screen - useful for visual testing where we want
    /// to capture what would be rendered without displaying it or requiring the window to be visible.
    #[cfg(any(test, feature = "test-support"))]
    fn render_to_image(&self, _scene: &Scene) -> Result<RgbaImage> {
        anyhow::bail!("render_to_image not implemented for this platform")
    }
}

/// Renders scenes offscreen for windows that have no display, such as in tests and benchmarks.
#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub trait PlatformHeadlessRenderer {
    /// Renders `scene` at `size` and returns the pixels as an RGBA image.
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<RgbaImage>;

    /// Render a scene to an offscreen target without reading the result back.
    ///
    /// This is the headless analogue of presenting a frame: it performs the
    /// same CPU-side scene encoding and GPU submission as drawing to a real
    /// window, but doesn't block on GPU completion or copy pixels back.
    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()>;

    /// Returns the sprite atlas used by this renderer.
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
}

/// Type alias for runnables with metadata.
/// Previously an enum with a single variant, now simplified to a direct type alias.
#[doc(hidden)]
pub type RunnableVariant = Runnable<RunnableMeta>;

#[doc(hidden)]
pub type TimerResolutionGuard = gpui_util::Deferred<Box<dyn FnOnce() + Send>>;

#[doc(hidden)]
pub enum TasksIncluded {
    OnlyCompleted,
    CompletedAndRunning,
}

/// This type is public so that our test macro can generate and use it, but it should not
/// be considered part of our public API.
#[doc(hidden)]
pub trait PlatformDispatcher: Send + Sync {
    fn is_main_thread(&self) -> bool;
    fn dispatch(&self, runnable: RunnableVariant, priority: Priority);
    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority);
    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant);

    fn dispatch_on_main_thread_when_idle(
        &self,
        runnable: RunnableVariant,
        timeout: Option<Duration>,
    ) {
        let _ = timeout;
        self.dispatch_on_main_thread(runnable, Priority::Low);
    }

    fn idle_time_remaining(&self) -> Option<Duration> {
        None
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>);

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn increase_timer_resolution(&self) -> TimerResolutionGuard {
        gpui_util::defer(Box::new(|| {}))
    }

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_test(&self) -> Option<&TestDispatcher> {
        None
    }

    // This cfg must match the `threaded_dispatcher` module's, which implements
    // this method whenever it compiles.
    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_threaded(&self) -> Option<&ThreadedDispatcher> {
        None
    }
}

/// Loads fonts and lays out text for one platform. Each backend implements this trait.
///
/// Application code uses [`TextSystem`](crate::TextSystem), which calls these methods and caches the
/// results. Glyph and layout methods run on whatever thread the caller uses, so
/// implementations must be `Send + Sync`.
pub trait PlatformTextSystem: Send + Sync {
    /// Adds fonts from their file bytes, such as TTF or OTF data, so layout can find them by family name.
    ///
    /// Returns an error if any of the bytes can't be parsed as a font.
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()>;
    /// Returns the names of every font family available to layout.
    fn all_font_names(&self) -> Vec<String>;
    /// Returns a number that changes whenever the font collection changes, so caches can detect stale entries.
    ///
    /// The default returns `0`, which means the collection never changes.
    fn font_generation(&self) -> u64 {
        0
    }
    /// Returns the id of the font that best matches `descriptor`.
    fn font_id(&self, descriptor: &Font) -> Result<FontId>;
    /// Loads the caches that shaping needs for `font_ids`, so the first layout with them is fast.
    /// The default does nothing.
    fn prewarm_fonts(&self, _font_ids: &[FontId]) {}
    /// Returns the metrics, such as ascent, descent and line gap, for `font_id`.
    fn font_metrics(&self, font_id: FontId) -> FontMetrics;
    /// Returns the glyph's bounding box, relative to its origin on the baseline.
    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>>;
    /// Returns the distance the pen moves after drawing this glyph.
    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>>;
    /// Returns the glyph that `font_id` uses for `ch`, or `None` if the font has no glyph for it.
    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId>;
    /// Rasterizes a glyph, including its atlas placement and pixels.
    fn rasterize_glyph(&self, params: &RenderGlyphParams) -> Result<RasterizedGlyph>;
    /// Returns the raster style to use for `request`, normalized so that cached glyph
    /// pixels can be shared. The default uses the requested mode unchanged.
    fn prepare_raster_style(&self, request: RasterStyleRequest) -> PreparedRasterStyle {
        PreparedRasterStyle::independent(request.requested_mode)
    }
    /// Lays out one complete text document, including hard breaks and optional wrapping.
    fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout;
    /// Lays out one complete text document that contains atomic element boxes, which
    /// keep their size and sit on the text line.
    fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout;
    /// Returns the text rendering mode the platform recommends for `font_id` at `font_size`.
    ///
    /// The default returns [`TextRenderingMode::Subpixel`].
    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Subpixel
    }
}

#[cfg(any(test, feature = "test-support"))]
mod tests {
    use crate::{
        Bounds, CaretAffinity, CaretMovement, CaretPosition, Font, FontId, FontMetrics, GlyphId,
        InlineLayout, InlineLayoutRequest, InlineVisualLine, LineLayout, PaintFragment, PaintStyle,
        Pixels, PlatformTextLayout, PlatformTextSystem, Point, PositionedInlineBox,
        RasterizedGlyph, RasterizedGlyphFormat, RenderGlyphParams, ResolvedDirection, ShapedGlyph,
        Size, TextAlign, TextBoundary, TextDirection, TextLayoutRequest, TextMovement,
        TextRenderingMode, TextSelectionKind, VisualDirection, VisualLine, align_inline_boxes,
        point, px, size,
    };
    use anyhow::Result;
    use std::{borrow::Cow, ops::Range, sync::Arc};

    #[expect(missing_docs)]
    pub struct TestTextSystem;

    #[derive(Debug)]
    /// A deterministic test layout with exactly one visual and hard line.
    struct TestPlatformTextLayout {
        /// Source text.
        text: String,
        /// Ordered (byte index, x position) caret stops.
        stops: Vec<(usize, Pixels)>,
        /// Size of the single visual line.
        size: Size<Pixels>,
    }

    impl TestPlatformTextLayout {
        fn caret_stop(&self, byte_offset: usize) -> (usize, Pixels) {
            let stop_index = self
                .stops
                .partition_point(|(index, _position)| *index <= byte_offset)
                .saturating_sub(1);

            self.stops[stop_index]
        }
    }

    impl PlatformTextLayout for TestPlatformTextLayout {
        fn len(&self) -> usize {
            self.stops.last().map_or(0, |(index, _)| *index)
        }

        fn line_count(&self) -> usize {
            // This test layout deliberately reports its single visual line.
            1
        }

        fn size(&self) -> Size<Pixels> {
            self.size
        }

        fn byte_index_from_pixel_point(
            &self,
            pixel_point: Point<Pixels>,
            line_height: Pixels,
        ) -> Result<usize, usize> {
            let closest = self
                .caret_from_pixel_point(pixel_point, line_height)
                .unwrap_or_else(|caret| caret)
                .index;
            self.stops
                .windows(2)
                .find_map(|stops| {
                    let [(start_index, start_x), (_end_index, end_x)] = stops else {
                        return None;
                    };

                    (pixel_point.x >= *start_x && pixel_point.x < *end_x)
                        .then_some(Ok(*start_index))
                })
                .unwrap_or(Err(closest))
        }

        fn caret_from_pixel_point(
            &self,
            pixel_point: Point<Pixels>,
            line_height: Pixels,
        ) -> Result<CaretPosition, CaretPosition> {
            let index = self
                .stops
                .iter()
                .min_by(|(_, left), (_, right)| {
                    (f32::from(*left) - f32::from(pixel_point.x))
                        .abs()
                        .total_cmp(&(f32::from(*right) - f32::from(pixel_point.x)).abs())
                })
                .map_or(0, |(index, _)| *index);
            let caret = self.normalized_caret(CaretPosition {
                index,
                affinity: CaretAffinity::Downstream,
            });

            if pixel_point.y >= Pixels::ZERO
                && pixel_point.y < line_height
                && pixel_point.x >= Pixels::ZERO
                && pixel_point.x < self.size.width
            {
                Ok(caret)
            } else {
                Err(caret)
            }
        }

        fn caret_bounds(
            &self,
            caret: CaretPosition,
            line_height: Pixels,
        ) -> Option<Bounds<Pixels>> {
            let caret = self.normalized_caret(caret);
            let (_index, position) = self.caret_stop(caret.index);

            Some(Bounds::new(
                point(position, Pixels::ZERO),
                size(Pixels::ZERO, line_height),
            ))
        }

        fn normalized_caret(&self, caret: CaretPosition) -> CaretPosition {
            let (index, _position) = self.caret_stop(caret.index);
            let affinity = if index == self.len() && index != 0 {
                CaretAffinity::Upstream
            } else {
                caret.affinity
            };

            CaretPosition { index, affinity }
        }

        fn adjacent_visual_caret(
            &self,
            caret: CaretPosition,
            direction: VisualDirection,
        ) -> Option<CaretPosition> {
            let caret = self.normalized_caret(caret);
            let position = self
                .stops
                .iter()
                .position(|(index, _)| *index == caret.index)?;
            let position = match direction {
                VisualDirection::Left => position.checked_sub(1)?,
                VisualDirection::Right => position.checked_add(1)?,
            };

            let index = self.stops.get(position)?.0;
            Some(self.normalized_caret(CaretPosition {
                index,
                affinity: CaretAffinity::Downstream,
            }))
        }

        fn selection_bounds(
            &self,
            byte_range: Range<usize>,
            line_height: Pixels,
        ) -> Vec<Bounds<Pixels>> {
            if byte_range.is_empty() {
                return Vec::new();
            }

            let (_start_index, start) = self.caret_stop(byte_range.start);
            let (_end_index, end) = self.caret_stop(byte_range.end);

            // The platform contract returns one bound per visual line. Since this test
            // layout has exactly one visual line, every non-empty selection has one bound.
            vec![Bounds::from_corners(
                point(start.min(end), Pixels::ZERO),
                point(start.max(end), line_height),
            )]
        }

        fn logical_cluster_before(&self, caret: CaretPosition) -> Option<Range<usize>> {
            self.stops
                .windows(2)
                .rev()
                .find(|stops| stops[1].0 <= caret.index)
                .map(|stops| stops[0].0..stops[1].0)
        }

        fn logical_cluster_after(&self, caret: CaretPosition) -> Option<Range<usize>> {
            self.stops
                .windows(2)
                .find(|stops| stops[0].0 >= caret.index)
                .map(|stops| stops[0].0..stops[1].0)
        }

        fn caret_movement(
            &self,
            caret: CaretPosition,
            movement: TextMovement,
            vertical_navigation_x: Option<Pixels>,
        ) -> CaretMovement {
            use TextBoundary::*;
            use TextDirection::*;

            let index = match (movement.direction, movement.boundary) {
                (Left, Cluster) => {
                    self.adjacent_visual_caret(caret, VisualDirection::Left)
                        .unwrap_or(caret)
                        .index
                }
                (Right, Cluster) => {
                    self.adjacent_visual_caret(caret, VisualDirection::Right)
                        .unwrap_or(caret)
                        .index
                }
                (Left, Word) => {
                    let prefix = &self.text[..caret.index.min(self.text.len())];
                    let trimmed = prefix.trim_end_matches(char::is_whitespace);
                    trimmed.rfind(char::is_whitespace).map_or(0, |index| {
                        index + trimmed[index..].chars().next().unwrap().len_utf8()
                    })
                }
                (Right, Word) => {
                    let start = caret.index.min(self.text.len());
                    let suffix = &self.text[start..];
                    let word_end = suffix.find(char::is_whitespace).unwrap_or(suffix.len());
                    let rest = &suffix[word_end..];
                    start
                        + word_end
                        + rest
                            .find(|character: char| !character.is_whitespace())
                            .unwrap_or(rest.len())
                }
                // This test layout has one visual and hard line. Without wrapping or
                // additional hard lines, these movements resolve to the document endpoints.
                (Up | Start, VisualLine) | (Start, HardLine | Document) => 0,
                (Down | End, VisualLine) | (End, HardLine | Document) => self.len(),
                _ => caret.index,
            };

            let vertical_navigation_x =
                matches!(movement.direction, TextDirection::Up | TextDirection::Down).then(|| {
                    vertical_navigation_x.unwrap_or_else(|| {
                        self.caret_bounds(caret, self.size.height)
                            .map_or(Pixels::ZERO, |bounds| bounds.origin.x)
                    })
                });

            CaretMovement {
                result: self.normalized_caret(CaretPosition {
                    index,
                    affinity: CaretAffinity::Downstream,
                }),
                vertical_navigation_x,
            }
        }

        fn selection_from_pixel_point(
            &self,
            pixel_point: Point<Pixels>,
            line_height: Pixels,
            kind: TextSelectionKind,
        ) -> Range<usize> {
            if !matches!(kind, TextSelectionKind::Word) {
                return 0..self.len();
            }

            let index = self
                .caret_from_pixel_point(pixel_point, line_height)
                .unwrap_or_else(|caret| caret)
                .index
                .min(self.text.len());
            let start = self.text[..index]
                .rfind(char::is_whitespace)
                .map_or(0, |offset| {
                    offset + self.text[offset..].chars().next().unwrap().len_utf8()
                });
            let end = self.text[index..]
                .find(char::is_whitespace)
                .map_or(self.text.len(), |offset| index + offset);
            start..end
        }
    }

    #[expect(missing_docs)]
    impl TestTextSystem {
        #[allow(dead_code)]
        pub fn new() -> Self {
            Self
        }

        fn em_width(&self, font_size: Pixels) -> Pixels {
            let metrics = self.font_metrics(FontId(0));
            let advance = self
                .advance(FontId(0), self.glyph_for_char(FontId(0), 'm').unwrap())
                .unwrap();

            font_size * advance.width / metrics.units_per_em as f32
        }
    }

    fn position_test_inline_boxes(
        request: InlineLayoutRequest<'_>,
        em_width: Pixels,
        baseline: Pixels,
    ) -> Vec<PositionedInlineBox> {
        let mut preceding_width = Pixels::ZERO;
        request
            .boxes
            .iter()
            .map(|inline_box| {
                let text_width = em_width
                    * request.text[..inline_box.index]
                        .chars()
                        .map(|character| character.len_utf16() as f32)
                        .sum::<f32>();
                let positioned = PositionedInlineBox {
                    id: inline_box.id,
                    line_index: 0,
                    bounds: Bounds::new(
                        point(
                            text_width + preceding_width,
                            baseline - inline_box.size.height,
                        ),
                        inline_box.size,
                    ),
                };

                preceding_width += inline_box.size.width;
                positioned
            })
            .collect()
    }

    fn add_test_inline_box_advances(layout: &mut LineLayout, request: InlineLayoutRequest<'_>) {
        for fragment in &mut layout.paint_fragments {
            // This layout owns its freshly built glyphs. If they become shared,
            // make_mut copies the slice before changing their positions.
            for (glyph, (index, _)) in Arc::make_mut(&mut fragment.glyphs)
                .iter_mut()
                .zip(request.text.char_indices())
            {
                glyph.position.x += request
                    .boxes
                    .iter()
                    .filter(|inline_box| inline_box.index <= index)
                    .map(|inline_box| inline_box.size.width)
                    .sum::<Pixels>();
            }
        }

        let box_width = request
            .boxes
            .iter()
            .map(|inline_box| inline_box.size.width)
            .sum::<Pixels>();
        for fragment in &mut layout.paint_fragments {
            fragment.x_range.end += box_width;
        }

        layout.width += box_width;

        if let Some(line) = layout.visual_lines.first_mut() {
            line.advance_width += box_width;
        }
    }

    impl PlatformTextSystem for TestTextSystem {
        fn add_fonts(&self, _fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
            Ok(())
        }

        fn all_font_names(&self) -> Vec<String> {
            Vec::new()
        }

        fn font_id(&self, _descriptor: &Font) -> Result<FontId> {
            Ok(FontId(1))
        }

        fn font_metrics(&self, _font_id: FontId) -> FontMetrics {
            FontMetrics {
                units_per_em: 1000,
                ascent: 1025.0,
                descent: -275.0,
                line_gap: 0.0,
                underline_position: -95.0,
                underline_thickness: 60.0,
                cap_height: 698.0,
                x_height: 516.0,
                bounding_box: Bounds {
                    origin: Point {
                        x: -260.0,
                        y: -245.0,
                    },
                    size: Size {
                        width: 1501.0,
                        height: 1364.0,
                    },
                },
            }
        }

        fn typographic_bounds(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<Bounds<f32>> {
            Ok(Bounds {
                origin: Point { x: 54.0, y: 0.0 },
                size: size(392.0, 528.0),
            })
        }

        fn advance(&self, _font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
            Ok(size(600.0 * glyph_id.0 as f32, 0.0))
        }

        fn glyph_for_char(&self, _font_id: FontId, ch: char) -> Option<GlyphId> {
            Some(GlyphId(ch.len_utf16() as u32))
        }

        fn rasterize_glyph(&self, _params: &RenderGlyphParams) -> Result<RasterizedGlyph> {
            Ok(RasterizedGlyph {
                bounds: Default::default(),
                size: Default::default(),
                format: RasterizedGlyphFormat::AlphaMask,
                pixels: Vec::new(),
            })
        }

        fn layout_text(&self, request: TextLayoutRequest<'_>) -> LineLayout {
            let text = request.text;
            let font_size = request.font_size;
            let shaping_runs = request.runs;
            let mut position = px(0.);
            let metrics = self.font_metrics(FontId(0));
            let em_width = self.em_width(font_size);
            let mut glyphs = Vec::new();
            let mut stops = vec![(0, Pixels::ZERO)];

            for (idx, character) in text.char_indices() {
                let glyph_id = GlyphId(character.len_utf16() as u32);
                glyphs.push(ShapedGlyph {
                    id: glyph_id,
                    position: point(position, Pixels::ZERO),
                    is_emoji: glyph_id.0 == 2,
                });

                position += em_width * glyph_id.0 as f32;
                stops.push((idx + character.len_utf8(), position));
            }

            let mut tracking = px(0.);
            let mut tracking_covered = 0usize;
            for run in shaping_runs {
                let start = tracking_covered.min(text.len());
                let end = start.saturating_add(run.len).min(text.len()).max(start);
                let slice = text.get(start..end).unwrap_or("");
                let n = slice.chars().count();
                if n > 1 {
                    if let Some(spacing) = run.letter_spacing {
                        tracking += spacing * (n - 1) as f32;
                    }
                }
                tracking_covered = end;
            }

            let direction = match request.options.direction {
                crate::ParagraphDirection::LeftToRight => ResolvedDirection::LeftToRight,
                crate::ParagraphDirection::RightToLeft => ResolvedDirection::RightToLeft,
                crate::ParagraphDirection::Auto => {
                    ResolvedDirection::from_first_strong(text).unwrap_or_default()
                }
            };
            let advance = position + tracking;
            let alignment_width = request.options.alignment_width.unwrap_or(advance);
            let offset = match request.options.text_align {
                TextAlign::Start if direction.is_rtl() => alignment_width - advance,
                TextAlign::Start | TextAlign::Left => Pixels::ZERO,
                TextAlign::Center => (alignment_width - advance) / 2.0,
                TextAlign::End if direction.is_rtl() => Pixels::ZERO,
                TextAlign::End | TextAlign::Right => alignment_width - advance,
            };

            for (_idx, position) in &mut stops {
                *position += offset;
            }

            let visual_lines = [VisualLine {
                text_range: 0..text.len(),
                paint_fragment_range: 0..usize::from(!glyphs.is_empty()),
                advance_width: advance,
                offset,
                direction,
            }]
            .into_iter()
            .collect();
            let paint_fragments = (!glyphs.is_empty())
                .then(|| PaintFragment {
                    source_run: 0,
                    font_id: FontId(0),
                    font_size,
                    glyphs: glyphs.into(),
                    x_range: Pixels::ZERO..advance,
                    style: shaping_runs
                        .first()
                        .map_or_else(PaintStyle::default, PaintStyle::from),
                    underline_offset: Some(font_size * 0.1),
                    strikethrough_offset: Some(-font_size * 0.3),
                })
                .into_iter()
                .collect();
            LineLayout {
                font_size,
                width: advance,
                ascent: font_size * (metrics.ascent / metrics.units_per_em as f32),
                descent: font_size * (metrics.descent / metrics.units_per_em as f32),
                visual_lines,
                paint_fragments,
                len: text.len(),
                platform_layout: Arc::new(TestPlatformTextLayout {
                    text: text.to_owned(),
                    stops,
                    size: size(advance, font_size),
                }),
            }
        }

        fn layout_inline(&self, request: InlineLayoutRequest<'_>) -> InlineLayout {
            let mut layout = self.layout_text(TextLayoutRequest {
                text: request.text,
                font_size: request.font_size,
                runs: request.runs,
                options: request.options,
            });

            let em_width = self.em_width(request.font_size);
            let baseline = request
                .boxes
                .iter()
                .map(|inline_box| inline_box.size.height)
                .fold(request.line_height, Pixels::max);
            let positioned_boxes = position_test_inline_boxes(request, em_width, baseline);
            add_test_inline_box_advances(&mut layout, request);
            let line_width = request
                .options
                .wrap_width
                .unwrap_or(Pixels::MAX)
                .min(layout.width);
            let line_offset = layout.visual_lines[0].offset;
            let mut inline = InlineLayout {
                size: size(layout.width, baseline),
                layout: Arc::new(layout),
                lines: [InlineVisualLine {
                    origin: point(line_offset, Pixels::ZERO),
                    size: size(line_width, baseline),
                    baseline,
                }]
                .into_iter()
                .collect(),
                boxes: positioned_boxes,
                alignment_offset: Pixels::ZERO,
            };

            align_inline_boxes(
                &mut inline.lines,
                &mut inline.boxes,
                &mut inline.size,
                request.boxes,
                &[request.text_metrics],
                &[],
                request.text_metrics,
                request.line_height,
            );
            inline
        }

        fn recommended_rendering_mode(
            &self,
            _font_id: FontId,
            _font_size: Pixels,
        ) -> TextRenderingMode {
            TextRenderingMode::Grayscale
        }
    }
}

// Adapted from https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.cpp
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
/// Returns the gamma correction ratios for subpixel text at `gamma`.
///
/// `gamma` is rounded to one decimal place and clamped to the range `1.0` to `2.2`.
#[allow(dead_code)]
pub fn get_gamma_correction_ratios(gamma: f32) -> [f32; 4] {
    const GAMMA_INCORRECT_TARGET_RATIOS: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0], // gamma = 1.0
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0], // gamma = 1.1
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0], // gamma = 1.2
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0], // gamma = 1.3
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0], // gamma = 1.4
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0], // gamma = 1.5
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0], // gamma = 1.6
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0], // gamma = 1.7
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0], // gamma = 1.8
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0], // gamma = 1.9
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0], // gamma = 2.0
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0], // gamma = 2.1
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0], // gamma = 2.2
    ];

    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / (255.0) * 4.0) as f32;

    let index = ((gamma * 10.0).round() as usize).clamp(10, 22) - 10;
    let ratios = GAMMA_INCORRECT_TARGET_RATIOS[index];

    [
        ratios[0] * NORM13,
        ratios[1] * NORM24,
        ratios[2] * NORM13,
        ratios[3] * NORM24,
    ]
}

/// Identifies one entry in a [`PlatformAtlas`]: a glyph, an SVG or a raster image.
#[derive(PartialEq, Eq, Hash, Clone)]
pub enum AtlasKey {
    /// A rasterized glyph, with the format it was rasterized in.
    Glyph {
        /// The glyph and how it is rendered.
        params: RenderGlyphParams,
        /// The pixel format it was rasterized in.
        format: RasterizedGlyphFormat,
    },
    /// An SVG rendered at specific parameters.
    Svg(RenderSvgParams),
    /// A raster image, identified by its render parameters.
    Image(RenderImageParams),
}

impl AtlasKey {
    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    /// Returns the texture kind for this atlas key.
    pub fn texture_kind(&self) -> AtlasTextureKind {
        match self {
            AtlasKey::Glyph { format, .. } => match format {
                RasterizedGlyphFormat::AlphaMask => AtlasTextureKind::Monochrome,
                RasterizedGlyphFormat::BgraSubpixelMask => AtlasTextureKind::Subpixel,
                RasterizedGlyphFormat::BgraColor => AtlasTextureKind::Polychrome,
            },
            AtlasKey::Svg(_) => AtlasTextureKind::Monochrome,
            AtlasKey::Image(_) => AtlasTextureKind::Polychrome,
        }
    }
}

impl From<(RenderGlyphParams, RasterizedGlyphFormat)> for AtlasKey {
    fn from((params, format): (RenderGlyphParams, RasterizedGlyphFormat)) -> Self {
        Self::Glyph { params, format }
    }
}

impl From<RenderSvgParams> for AtlasKey {
    fn from(params: RenderSvgParams) -> Self {
        Self::Svg(params)
    }
}

impl From<RenderImageParams> for AtlasKey {
    fn from(params: RenderImageParams) -> Self {
        Self::Image(params)
    }
}

#[doc(hidden)]
#[derive(Default)]
pub struct GlyphAtlasCache {
    entries: FxHashMap<RenderGlyphParams, GlyphAtlasEntry>,
}

impl GlyphAtlasCache {
    pub fn get(&self, params: &RenderGlyphParams) -> Option<GlyphAtlasEntry> {
        self.entries.get(params).copied()
    }

    pub fn insert(
        &mut self,
        params: &RenderGlyphParams,
        glyph: &RasterizedGlyph,
        tile: Option<AtlasTile>,
    ) -> GlyphAtlasEntry {
        let entry = GlyphAtlasEntry {
            tile,
            bounds: glyph.bounds,
            format: glyph.format,
        };
        self.entries.insert(params.clone(), entry);

        entry
    }

    pub fn remove(&mut self, key: &AtlasKey) {
        let AtlasKey::Glyph { params, format } = key else {
            return;
        };

        if self
            .entries
            .get(params)
            .is_some_and(|entry| entry.format == *format)
        {
            self.entries.remove(params);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A texture atlas that packs glyphs, SVGs and images into shared GPU textures.
///
/// Each window's renderer owns one atlas. Methods that insert tiles build the pixels
/// only on a cache miss, so callers don't pay for rasterizing a tile they already
/// have.
pub trait PlatformAtlas {
    /// Returns the tile for `key`, calling `build` on a cache miss to produce the pixels.
    ///
    /// `build` returns `None` when there is nothing to draw, and then this returns
    /// `Ok(None)`. Returns an error if the pixels fail validation or the atlas is full.
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>>;
    /// Returns the atlas entry for the glyph `params`, calling `build` on a cache miss.
    ///
    /// `build` returns a [`ValidatedRasterizedGlyph`], which is inserted into the atlas.
    fn get_or_insert_glyph_with(
        &self,
        params: &RenderGlyphParams,
        build: &mut dyn FnMut() -> Result<ValidatedRasterizedGlyph>,
    ) -> Result<GlyphAtlasEntry>;
    /// Removes the tile for `key`, freeing its space in the atlas.
    fn remove(&self, key: &AtlasKey);

    /// Returns `true` if the atlas has a tile for `key`. Atlases that don't track this return `false`.
    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn contains(&self, _key: &AtlasKey) -> bool {
        false
    }
}

#[doc(hidden)]
pub struct AtlasTextureList<T> {
    pub textures: Vec<Option<T>>,
    pub free_list: Vec<usize>,
}

impl<T> Default for AtlasTextureList<T> {
    fn default() -> Self {
        Self {
            textures: Vec::default(),
            free_list: Vec::default(),
        }
    }
}

impl<T> ops::Index<usize> for AtlasTextureList<T> {
    type Output = Option<T>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.textures[index]
    }
}

impl<T> AtlasTextureList<T> {
    #[allow(unused)]
    pub fn drain(&mut self) -> std::vec::Drain<'_, Option<T>> {
        self.free_list.clear();
        self.textures.drain(..)
    }

    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        self.textures.iter_mut().flatten()
    }
}

/// Where a glyph, SVG or image sits in an atlas texture.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AtlasTile {
    /// The texture this tile belongs to.
    pub texture_id: AtlasTextureId,
    /// The unique ID of this tile within its texture.
    pub tile_id: TileId,
    /// Padding around the tile content in pixels.
    pub padding: u32,
    /// The bounds of this tile within the texture.
    pub bounds: Bounds<DevicePixels>,
}

/// Identifies one texture in an atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct AtlasTextureId {
    // We use u32 instead of usize for Metal Shader Language compatibility
    /// The index of this texture in the atlas.
    pub index: u32,
    /// The kind of content stored in this texture.
    pub kind: AtlasTextureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
/// The kind of pixels a texture in an atlas stores.
pub enum AtlasTextureKind {
    /// One coverage byte per pixel, used for alpha-masked glyphs and SVGs.
    Monochrome = 0,
    /// Four bytes per pixel, used for full-color images and emoji.
    Polychrome = 1,
    /// Four bytes per pixel, used for subpixel (LCD) antialiased glyphs.
    Subpixel = 2,
}

impl AtlasTextureKind {
    /// Validates a tightly packed bitmap before an atlas allocates or caches its tile.
    /// Monochrome tiles contain one coverage byte; color and LCD tiles contain four bytes.
    pub fn validate_upload(self, size: Size<DevicePixels>, bytes: &[u8]) -> Result<()> {
        anyhow::ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "{self:?} atlas upload requires positive dimensions, got {size:?}"
        );
        let channels = match self {
            Self::Monochrome => 1,
            Self::Polychrome | Self::Subpixel => 4,
        };
        let expected = (size.width.0 as usize)
            .checked_mul(size.height.0 as usize)
            .and_then(|pixels| pixels.checked_mul(channels))
            .ok_or_else(|| anyhow::anyhow!("atlas upload byte count overflow for {size:?}"))?;
        anyhow::ensure!(
            bytes.len() == expected,
            "{self:?} atlas upload for {size:?} requires {expected} bytes, got {}",
            bytes.len()
        );
        Ok(())
    }
}

/// Identifies a tile within its atlas texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct TileId(pub u32);

impl From<etagere::AllocId> for TileId {
    fn from(id: etagere::AllocId) -> Self {
        Self(id.serialize())
    }
}

impl From<TileId> for etagere::AllocId {
    fn from(id: TileId) -> Self {
        Self::deserialize(id.0)
    }
}

/// Connects the platform's input method (IME) to the focused [`InputHandler`].
///
/// GPUI creates one for the focused element while it's focused. Each method
/// runs the matching [`InputHandler`] method inside an update of the window. If the
/// window has closed, the methods return `None` or do nothing.
pub struct PlatformInputHandler {
    cx: AsyncWindowContext,
    handler: Box<dyn InputHandler>,
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
impl PlatformInputHandler {
    /// Wraps `handler` so the platform can call it through the window context `cx`.
    pub fn new(cx: AsyncWindowContext, handler: Box<dyn InputHandler>) -> Self {
        Self { cx, handler }
    }

    /// Returns the selection, in UTF-16 units, or `None` if the handler has no selection.
    pub fn selected_text_range(&mut self, ignore_disabled_input: bool) -> Option<UTF16Selection> {
        self.cx
            .update(|window, cx| {
                self.handler
                    .selected_text_range(ignore_disabled_input, window, cx)
            })
            .ok()
            .flatten()
    }

    /// Returns the range of composing (marked) text in UTF-16 units, or `None` when no text is being composed.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn marked_text_range(&mut self) -> Option<Range<usize>> {
        self.cx
            .update(|window, cx| self.handler.marked_text_range(window, cx))
            .ok()
            .flatten()
    }

    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    /// Returns the document text in `range_utf16`, or `None` if the handler has none.
    ///
    /// `adjusted` receives the range the handler actually used, which may differ from the request.
    pub fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> Option<String> {
        self.cx
            .update(|window, cx| {
                self.handler
                    .text_for_range(range_utf16, adjusted, window, cx)
            })
            .ok()
            .flatten()
    }

    /// Replaces `replacement_range`, in UTF-16 units, with `text`. `None` replaces the selection.
    pub fn replace_text_in_range(&mut self, replacement_range: Option<Range<usize>>, text: &str) {
        self.cx
            .update(|window, cx| {
                self.handler
                    .replace_text_in_range(replacement_range, text, window, cx);
            })
            .ok();
    }

    /// Replaces `range_utf16` with `new_text` and marks the new text as composing, as an
    /// IME does while the user types. `new_selected_range` sets the selection inside the composing text.
    pub fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
    ) {
        self.cx
            .update(|window, cx| {
                self.handler.replace_and_mark_text_in_range(
                    range_utf16,
                    new_text,
                    new_selected_range,
                    window,
                    cx,
                )
            })
            .ok();
    }

    /// Ends the composition and keeps the marked text.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn unmark_text(&mut self) {
        self.cx
            .update(|window, cx| self.handler.unmark_text(window, cx))
            .ok();
    }

    /// Pastes `item` at the selection through [`InputHandler::paste`].
    pub fn paste(&mut self, item: ClipboardItem) {
        self.cx
            .update(|window, cx| self.handler.paste(item, window, cx))
            .ok();
    }

    /// Returns the screen bounds of `range_utf16`, in logical pixels. The platform uses these to place the IME candidate window.
    pub fn bounds_for_range(&mut self, range_utf16: Range<usize>) -> Option<Bounds<Pixels>> {
        self.cx
            .update(|window, cx| self.handler.bounds_for_range(range_utf16, window, cx))
            .ok()
            .flatten()
    }

    /// Returns whether macOS press-and-hold should stay enabled for this handler.
    #[allow(dead_code)]
    pub fn apple_press_and_hold_enabled(&mut self) -> bool {
        self.handler.apple_press_and_hold_enabled()
    }

    /// Replaces the selection with `input`, using the handler's [`InputHandler::replace_text_in_range`].
    pub fn dispatch_input(&mut self, input: &str, window: &mut Window, cx: &mut App) {
        self.handler.replace_text_in_range(None, input, window, cx);
    }

    /// Returns where the IME candidate window should appear.
    ///
    /// While text is composing, this is the start of the composing text's visual line,
    /// found by walking back from the caret until the line changes. Otherwise it's the
    /// caret position. `bounds_for_range` returns the bounds of each position it's asked for.
    pub fn compute_ime_candidate_bounds(
        marked_range: Option<Range<usize>>,
        selection: &UTF16Selection,
        mut bounds_for_range: impl FnMut(Range<usize>) -> Option<Bounds<Pixels>>,
    ) -> Option<Bounds<Pixels>> {
        // The active end of the selection, which is `start` when it is reversed.
        let caret = if selection.reversed {
            selection.range.start
        } else {
            selection.range.end
        };

        if let Some(marked_range) = marked_range {
            // Default to the start of the marked (composing) range.
            let mut line_start = marked_range.start;

            // Walk backward from the caret looking for a line break. A change in
            // the Y coordinate means we crossed into the previous visual line, so
            // the line start is one position after the break point.
            if let Some(caret_bounds) = bounds_for_range(caret..caret) {
                for i in (marked_range.start..caret).rev() {
                    if let Some(b) = bounds_for_range(i..i) {
                        if (b.origin.y - caret_bounds.origin.y).abs() > px(0.1) {
                            line_start = i + 1;
                            break;
                        }
                    }
                }
            }
            bounds_for_range(line_start..line_start)
        } else {
            // No active composition — use the selection endpoint.
            bounds_for_range(caret..caret)
        }
    }

    /// Returns the IME candidate position for the current selection.
    pub fn selected_bounds(&mut self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let marked_range = self.handler.marked_text_range(window, cx);
        let selection = self.handler.selected_text_range(true, window, cx)?;
        Self::compute_ime_candidate_bounds(marked_range, &selection, |range| {
            self.handler.bounds_for_range(range, window, cx)
        })
    }

    /// Returns the IME candidate position for the current selection, through the window context.
    pub fn ime_candidate_bounds(&mut self) -> Option<Bounds<Pixels>> {
        let marked_range = self.marked_text_range();
        let selection = self.selected_text_range(true)?;
        Self::compute_ime_candidate_bounds(marked_range, &selection, |range| {
            self.bounds_for_range(range)
        })
    }

    /// Returns the UTF-16 index of the character at `point`, in window coordinates, or `None` if there is none.
    #[allow(unused)]
    pub fn character_index_for_point(&mut self, point: Point<Pixels>) -> Option<usize> {
        self.cx
            .update(|window, cx| self.handler.character_index_for_point(point, window, cx))
            .ok()
            .flatten()
    }

    /// See [`InputHandler::set_selected_text_range`].
    pub fn set_selected_text_range(&mut self, range_utf16: Range<usize>) {
        self.cx
            .update(|window, cx| {
                self.handler
                    .set_selected_text_range(range_utf16, window, cx)
            })
            .ok();
    }

    /// See [`InputHandler::element_bounds`].
    pub fn element_bounds(&mut self) -> Option<Bounds<Pixels>> {
        self.cx
            .update(|window, cx| self.handler.element_bounds(window, cx))
            .ok()
            .flatten()
    }

    /// See [`InputHandler::text_length_utf16`].
    pub fn text_length_utf16(&mut self) -> Option<usize> {
        self.cx
            .update(|window, cx| self.handler.text_length_utf16(window, cx))
            .ok()
            .flatten()
    }

    /// Returns whether the handler accepts text input right now.
    #[allow(dead_code)]
    pub fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.handler.accepts_text_input(window, cx)
    }

    /// Returns whether the handler accepts text input. Returns `true` if the window is gone.
    #[allow(dead_code)]
    pub fn query_accepts_text_input(&mut self) -> bool {
        self.cx
            .update(|window, cx| self.handler.accepts_text_input(window, cx))
            .unwrap_or(true)
    }

    /// See [`InputHandler::prefers_ime_for_printable_keys`].
    ///
    /// This is not a pure delegation to the handler: while a multi-stroke binding is pending this
    /// returns `false` regardless of the handler's preference, because the next printable key may
    /// complete a binding whose prefix already bypassed the IME.
    pub fn query_prefers_ime_for_printable_keys(&mut self) -> bool {
        self.cx
            .update(|window, cx| {
                // The next printable key may complete a chord whose prefix bypassed the IME.
                !window.has_pending_keystrokes()
                    && self.handler.prefers_ime_for_printable_keys(window, cx)
            })
            .unwrap_or(false)
    }

    /// Returns the text input configuration the handler requests.
    pub fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> TextInputConfiguration {
        self.handler.text_input_configuration(window, cx)
    }

    /// See [`InputHandler::text_input_editable_range`].
    pub fn text_input_editable_range(&mut self) -> Option<Range<usize>> {
        self.cx
            .update(|window, cx| self.handler.text_input_editable_range(window, cx))
            .ok()
            .flatten()
    }
}

/// A struct representing a selection in a text buffer, in UTF16 characters.
/// This is different from a range because the head may be before the tail.
#[derive(Debug)]
pub struct UTF16Selection {
    /// The range of text in the document this selection corresponds to
    /// in UTF16 characters.
    pub range: Range<usize>,
    /// Whether the head of this selection is at the start (true), or end (false)
    /// of the range
    pub reversed: bool,
}

/// Receives text from the platform's input method (IME) on behalf of a focused element.
///
/// Implement this on the element that accepts text, and return it from
/// [`Window::handle_input`]. The methods mirror the macOS `NSTextInputClient`
/// protocol. All ranges are in UTF-16 code units, because that is what platforms
/// use, not bytes or characters.
///
/// Methods without a default body must be implemented. The others have defaults, so
/// override only what your text needs. See [text input](crate::_documentation::guides::text_input).
pub trait InputHandler: 'static {
    /// Get the range of the user's currently selected text, if any
    /// Corresponds to [selectedRange()](https://developer.apple.com/documentation/appkit/nstextinputclient/1438242-selectedrange)
    ///
    /// Return value is in terms of UTF-16 characters, from 0 to the length of the document
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection>;

    /// Get the range of the currently marked text, if any
    /// Corresponds to [markedRange()](https://developer.apple.com/documentation/appkit/nstextinputclient/1438250-markedrange)
    ///
    /// Return value is in terms of UTF-16 characters, from 0 to the length of the document
    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>>;

    /// Get the text for the given document range in UTF-16 characters
    /// Corresponds to [attributedSubstring(forProposedRange: actualRange:)](https://developer.apple.com/documentation/appkit/nstextinputclient/1438238-attributedsubstring)
    ///
    /// range_utf16 is in terms of UTF-16 characters
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String>;

    /// Replace the text in the given document range with the given text
    /// Corresponds to [insertText(_:replacementRange:)](https://developer.apple.com/documentation/appkit/nstextinputclient/1438258-inserttext)
    ///
    /// replacement_range is in terms of UTF-16 characters
    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    );

    /// Replace the text in the given document range with the given text,
    /// and mark the given text as part of an IME 'composing' state
    /// Corresponds to [setMarkedText(_:selectedRange:replacementRange:)](https://developer.apple.com/documentation/appkit/nstextinputclient/1438246-setmarkedtext)
    ///
    /// range_utf16 is in terms of UTF-16 characters
    /// new_selected_range is in terms of UTF-16 characters
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    );

    /// Remove the IME 'composing' state from the document
    /// Corresponds to [unmarkText()](https://developer.apple.com/documentation/appkit/nstextinputclient/1438239-unmarktext)
    fn unmark_text(&mut self, window: &mut Window, cx: &mut App);

    /// Insert a platform-initiated paste at the current selection.
    ///
    /// Platforms that deliver paste as an input event rather than through an
    /// application-defined action (e.g. the DOM `paste` event on web) call
    /// this with the full clipboard contents. The default implementation
    /// inserts only the plain-text portion of the item.
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        if let Some(text) = item.text() {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    /// Get the bounds of the given document range in screen coordinates
    /// Corresponds to [firstRect(forCharacterRange:actualRange:)](https://developer.apple.com/documentation/appkit/nstextinputclient/1438240-firstrect)
    ///
    /// This is used for positioning the IME candidate window
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>>;

    /// Get the character offset for the given point in terms of UTF16 characters
    ///
    /// Corresponds to [characterIndexForPoint:](https://developer.apple.com/documentation/appkit/nstextinputclient/characterindex(for:))
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize>;

    /// Set the range of the user's currently selected text.
    ///
    /// This is the reverse data-flow direction from [`Self::selected_text_range`]:
    /// platforms call it when the system text machinery moves the selection on the
    /// application's behalf — e.g. the user drags a system selection handle or
    /// invokes Select All from system UI (iOS `UITextInput setSelectedTextRange:`,
    /// Android `InputConnection.setSelection`).
    ///
    /// range_utf16 is in terms of UTF-16 characters, from 0 to the length of the document
    fn set_selected_text_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    /// Get the bounds of the focused text element in window coordinates, if known.
    ///
    /// This is the pull counterpart to the [`PlatformWindow::update_ime_position`]
    /// push: mobile platforms ask for the focused element's geometry when they
    /// need it (e.g. to frame system text-interaction UI overlaid on the focused
    /// element).
    fn element_bounds(&mut self, _window: &mut Window, _cx: &mut App) -> Option<Bounds<Pixels>> {
        None
    }

    /// Get the length of the document in UTF-16 characters, if known.
    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut App) -> Option<usize> {
        None
    }

    /// Allows a given input context to opt into getting raw key repeats instead of
    /// sending these to the platform.
    /// TODO: Ideally we should be able to set ApplePressAndHoldEnabled in NSUserDefaults
    /// (which is how iTerm does it) but it doesn't seem to work for me.
    #[allow(dead_code)]
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        true
    }

    /// Returns whether this handler is accepting text input to be inserted.
    fn accepts_text_input(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        true
    }

    /// The contiguous range of text, in UTF-16 code units, that platform text
    /// input may read and edit around the current selection.
    ///
    /// Platforms that mirror document text into an IME-editable buffer clamp
    /// the mirrored window to this range, so multi-step IME edit gestures
    /// (word deletion, autocorrect rewrites, suggestion picks) cannot reach
    /// content outside it. The range should contain the current selection;
    /// when it cannot (a selection spanning a region boundary), platforms
    /// degrade the mirrored IME context rather than widening the range.
    /// `None` places no bound.
    fn text_input_editable_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Range<usize>> {
        None
    }

    /// Returns whether printable keys should be routed to the IME before keybinding
    /// matching when a non-ASCII input source (e.g. Japanese, Korean, Chinese IME)
    /// is active. This prevents multi-stroke keybindings like `jj` from intercepting
    /// keys that the IME should compose.
    ///
    /// Defaults to `false`. The editor overrides this based on whether it expects
    /// character input (e.g. Vim insert mode returns `true`, normal mode returns `false`).
    /// The terminal keeps the default `false` so that raw keys reach the terminal process.
    fn prefers_ime_for_printable_keys(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        false
    }

    /// Get this handler's preferences for platform text assistance.
    ///
    /// GPUI re-queries this every frame and forwards it to the platform window
    /// only when it changes, so implementations must be cheap and may vary the
    /// result with application state (e.g. with the cursor's position).
    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> TextInputConfiguration {
        TextInputConfiguration::default()
    }
}

/// Platform text-assistance preferences for the focused text region.
///
/// Returned by [`InputHandler::text_input_configuration`] and forwarded to the
/// platform whenever it changes; the platform maps the fields onto its native
/// input-session attributes (on web, DOM attributes of the hidden editable
/// element such as `autocorrect` and `enterkeyhint`).
///
/// The default disables all text assistance and requests no particular action
/// key presentation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInputConfiguration {
    /// Whether the platform may automatically correct entered text.
    pub autocorrect: bool,
    /// How software keyboards automatically capitalize entered text.
    pub autocapitalize: Autocapitalize,
    /// Whether software keyboards may offer word suggestions and spellcheck.
    pub suggestions: bool,
    /// The action advertised on a software keyboard's confirm ("enter") key.
    pub input_action: TextInputAction,
}

/// Automatic capitalization applied by software keyboards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Autocapitalize {
    /// No automatic capitalization.
    #[default]
    None,
    /// Capitalize the first letter of each word.
    Words,
    /// Capitalize the first letter of each sentence.
    Sentences,
    /// Capitalize every letter.
    Characters,
}

/// The action a software keyboard advertises on its confirm ("enter") key.
///
/// This affects only how the key is presented (icon or label); pressing it is
/// still delivered as ordinary input.
///
/// The variants are the HTML `enterkeyhint` attribute's value set
/// (<https://html.spec.whatwg.org/multipage/interaction.html#input-modalities:-the-enterkeyhint-attribute>),
/// which also maps onto Android's `IME_ACTION_*` constants and iOS's
/// `UIReturnKeyType`; [`TextInputAction::Unspecified`] means "emit no hint".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextInputAction {
    /// Let the platform choose its default presentation.
    #[default]
    Unspecified,
    /// Inserting a line break.
    Enter,
    /// Committing the field's value.
    Done,
    /// Navigating to the typed target.
    Go,
    /// Moving to the next field.
    Next,
    /// Moving to the previous field.
    Previous,
    /// Executing a search.
    Search,
    /// Sending a message.
    Send,
}

/// Options for creating a window, passed to [`App::open_window`].
///
/// Start from [`WindowOptions::default`] (or [`WindowOptions::new`], which is the same)
/// and chain setters for the fields you want to change. Each setter takes the value
/// itself, or an `Option` to clear the field. The defaults give a focused, shown,
/// resizable, movable normal window with a titlebar, at the platform's default size
/// and position.
///
/// Not every platform reads every field. The web backend uses [`WindowOptions::kind`],
/// [`WindowOptions::show`], and `web_window_background`. The page's canvas is
/// the window, so bounds, the titlebar, the minimum size and the resize options are ignored.
/// Each field's docs say which
/// platforms use it. See [platforms](crate::_documentation::platforms).
///
/// # Examples
///
/// Start from the defaults and change only what you need:
///
/// ```
/// use gpui::{Bounds, WindowBounds, WindowOptions, point, px, size};
///
/// let bounds = Bounds::new(point(px(100.), px(100.)), size(px(800.), px(600.)));
/// let options = WindowOptions::default()
///     .window_bounds(WindowBounds::Windowed(bounds))
///     .is_resizable(false)
///     .titlebar(None);
///
/// assert_eq!(options.window_bounds, Some(WindowBounds::Windowed(bounds)));
/// assert!(!options.is_resizable);
/// assert!(options.titlebar.is_none());
/// assert!(options.focus);
/// ```
///
/// Create a window that doesn't take focus and opens hidden, so you can show it later:
///
/// ```
/// use gpui::WindowOptions;
///
/// let options = WindowOptions::new().focus(false).show(false);
///
/// assert!(!options.focus);
/// assert!(!options.show);
/// ```
#[derive(Debug, derive_setters::Setters)]
#[setters(into)]
pub struct WindowOptions {
    /// The window's state and bounds in screen coordinates, in logical pixels.
    ///
    /// `None` inherits the bounds from the platform, and `Some` opens the window in the
    /// given state. A `Windowed` window opens at the rectangle, and `Maximized` and
    /// `Fullscreen` restore to the rectangle when left. Ignored on the web.
    pub window_bounds: Option<WindowBounds>,

    /// The titlebar, or `None` for a window without one. Ignored on the web.
    ///
    /// The default is [`TitlebarOptions::default`], which shows the platform's titlebar with no title.
    pub titlebar: Option<TitlebarOptions>,

    /// Whether the window takes keyboard focus when it opens. Defaults to `true`.
    pub focus: bool,

    /// Whether the window appears when it opens. Set it to `false` to show the window later
    /// with [`PlatformWindow::set_visible`].
    pub show: bool,

    /// The kind of window to create. Defaults to [`WindowKind::Normal`]. On the web, only `Normal` works.
    pub kind: WindowKind,

    /// Whether the user can drag the window. Defaults to `true`. macOS and Windows use this
    /// value. Linux and the web ignore it. Programmatic moves still work.
    pub is_movable: bool,

    /// Whether the application owns dragging of the (custom) titlebar, rather than
    /// AppKit. Only has an effect on macOS.
    ///
    /// Set this to `true` for windows that draw their own titlebar and move the window
    /// themselves via [`Window::start_window_move`]. It marks the whole content view as
    /// app-owned titlebar content, so AppKit neither drags the window from the titlebar
    /// nor delays titlebar clicks while disambiguating double-clicks (a delay first
    /// observed on macOS 27). It is independent of `is_movable`, so such windows stay
    /// user-movable (via their own drag) and keep the Window-menu tiling items enabled.
    ///
    /// Leave this `false` for windows that rely on AppKit's native titlebar dragging.
    pub app_owns_titlebar_drag: bool,

    /// The minimum time between animation frames while the window is inactive.
    ///
    /// Defaults to about 30 frames per second. Set `None` to draw inactive windows at the
    /// full frame rate.
    pub inactive_frame_interval: Option<Duration>,

    /// Whether the user can resize the window. Defaults to `true`. Ignored on the web.
    pub is_resizable: bool,

    /// Whether the user can minimize the window. Defaults to `true`. Ignored on the web.
    pub is_minimizable: bool,

    /// The display to open the window on, or `None` for the primary display. Ignored on the web.
    pub display_id: Option<DisplayId>,

    /// The background appearance of a macOS window.
    #[cfg(target_os = "macos")]
    pub macos_window_background: MacosWindowBackground,

    /// The background appearance of a Windows window.
    #[cfg(target_os = "windows")]
    pub windows_window_background: WindowsWindowBackground,

    /// The background appearance of a Linux window.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub linux_window_background: LinuxWindowBackground,

    /// The background appearance of a web window.
    #[cfg(target_family = "wasm")]
    pub web_window_background: WebWindowBackground,

    /// The application id that desktop environments use to group windows and find the desktop entry.
    ///
    /// Linux only. Other platforms ignore it.
    pub app_id: Option<String>,

    /// The smallest size the user can resize the window to, in logical pixels. `None` sets no minimum.
    /// Ignored on the web.
    pub window_min_size: Option<Size<Pixels>>,

    /// Whether the window draws its own decorations (client-side) or the window server does (server-side).
    ///
    /// `None` uses the server's default. Linux's Wayland and X11 backends honor it, and
    /// the other backends ignore it. The platform may ignore requests it can't satisfy.
    pub window_decorations: Option<WindowDecorations>,

    /// The window's icon, from an RGBA image. Linux only, and Wayland needs the
    /// `xdg-toplevel-icon` protocol to show it.
    pub icon: Option<Arc<image::RgbaImage>>,

    /// The name of the native tab group. Windows with the same identifier open as tabs of one window.
    /// macOS only.
    pub tabbing_identifier: Option<String>,
}

impl FluentBuilder for WindowOptions {}

/// The variables that can be configured when creating a new window
#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
/// The platform-level options for one window, passed to [`Platform::open_window`].
///
/// [`WindowOptions`] is the public form. GPUI resolves its fields into these parameters,
/// with `bounds` already computed.
#[allow(missing_docs)]
pub struct WindowParams {
    /// The window's initial bounds, in logical pixels.
    pub bounds: Bounds<Pixels>,

    /// The titlebar configuration of the window
    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub titlebar: Option<TitlebarOptions>,

    /// The kind of window to create
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub kind: WindowKind,

    /// Whether the window should be movable by the user
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_movable: bool,

    /// Whether the application owns dragging of the (custom) titlebar (macOS only)
    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    /// Whether the application owns dragging of the custom titlebar. macOS only. See [`WindowOptions::app_owns_titlebar_drag`].
    pub app_owns_titlebar_drag: bool,

    /// Whether the window should be resizable by the user
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_resizable: bool,

    /// Whether the window should be minimized by the user
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_minimizable: bool,

    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    /// Whether the window takes keyboard focus when it opens.
    pub focus: bool,

    /// Whether the window appears when it opens.
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub show: bool,

    /// An image to set as the window icon (X11 and Wayland only)
    /// Wayland requires the xdg-toplevel-icon protocol to be available
    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub icon: Option<Arc<image::RgbaImage>>,

    /// The display to open the window on, or `None` for the primary display.
    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub display_id: Option<DisplayId>,

    /// The application id for desktop environments to group windows. Linux only.
    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub app_id: Option<String>,

    /// The smallest size the user can resize the window to, in logical pixels.
    pub window_min_size: Option<Size<Pixels>>,

    /// The name of the native tab group. macOS only.
    #[cfg(target_os = "macos")]
    pub tabbing_identifier: Option<String>,
}

/// The state a window opens in, with the bounds it uses for that state.
///
/// Set it with [`WindowOptions::window_bounds`]. The bounds are in screen coordinates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum WindowBounds {
    /// Indicates that the window should open in a windowed state with the given bounds.
    Windowed(Bounds<Pixels>),
    /// Indicates that the window should open in a maximized state.
    /// The bounds provided here represent the restore size of the window.
    Maximized(Bounds<Pixels>),
    /// Indicates that the window should open in fullscreen mode.
    /// The bounds provided here represent the restore size of the window.
    Fullscreen(Bounds<Pixels>),
}

impl Default for WindowBounds {
    fn default() -> Self {
        WindowBounds::Windowed(Bounds::default())
    }
}

impl WindowBounds {
    /// Returns the bounds, whatever the state. For a maximized or fullscreen window these are the restore bounds.
    pub fn get_bounds(&self) -> Bounds<Pixels> {
        match self {
            WindowBounds::Windowed(bounds) => *bounds,
            WindowBounds::Maximized(bounds) => *bounds,
            WindowBounds::Fullscreen(bounds) => *bounds,
        }
    }

    /// Returns `WindowBounds::Windowed` with `size`, centered on the primary display.
    ///
    /// Use [`Bounds::centered`] to center on another display.
    pub fn centered(size: Size<Pixels>, cx: &App) -> Self {
        WindowBounds::Windowed(Bounds::centered(None, size, cx))
    }
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            window_bounds: None,
            titlebar: Some(TitlebarOptions {
                title: Default::default(),
                appears_transparent: Default::default(),
                traffic_light_position: Default::default(),
            }),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            app_owns_titlebar_drag: false,
            inactive_frame_interval: Some(Duration::from_micros(33_333)),
            is_resizable: true,
            is_minimizable: true,
            display_id: None,
            #[cfg(target_os = "macos")]
            macos_window_background: MacosWindowBackground::default(),
            #[cfg(target_os = "windows")]
            windows_window_background: WindowsWindowBackground::default(),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            linux_window_background: LinuxWindowBackground::default(),
            #[cfg(target_family = "wasm")]
            web_window_background: WebWindowBackground::default(),
            icon: None,
            app_id: None,
            window_min_size: None,
            window_decorations: None,
            tabbing_identifier: None,
        }
    }
}

impl WindowOptions {
    /// Returns window options with the platform's defaults: a focused, shown,
    /// resizable, minimizable, movable normal window with a titlebar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the window's background appearance for the current platform,
    /// erased into the renderer-facing [`WindowBackgroundAppearance`].
    pub fn background_appearance(&self) -> WindowBackgroundAppearance {
        #[cfg(target_os = "macos")]
        {
            self.macos_window_background.into()
        }
        #[cfg(target_os = "windows")]
        {
            self.windows_window_background.into()
        }
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            self.linux_window_background.into()
        }
        #[cfg(target_family = "wasm")]
        {
            self.web_window_background.into()
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "windows",
            any(target_os = "linux", target_os = "freebsd"),
            target_family = "wasm",
        )))]
        {
            WindowBackgroundAppearance::default()
        }
    }
}

/// Options for a window's titlebar.
///
/// Chain setters on [`TitlebarOptions::default`] and pass the result to
/// [`WindowOptions::titlebar`]. Pass `None` to clear the title or traffic light position.
///
/// ```
/// use gpui::{SharedString, TitlebarOptions, point, px};
///
/// let titlebar = TitlebarOptions::default()
///     .title(SharedString::from("My app"))
///     .appears_transparent(true)
///     .traffic_light_position(point(px(16.0), px(16.0)));
///
/// assert_eq!(titlebar.title, Some(SharedString::from("My app")));
/// assert!(titlebar.appears_transparent);
/// ```
#[derive(Debug, Default, derive_setters::Setters)]
#[setters(into)]
pub struct TitlebarOptions {
    /// The window's initial title. The web backend ignores it.
    pub title: Option<SharedString>,

    /// Whether to hide the system titlebar so you can draw your own. macOS and Windows only.
    /// On Linux, use [`WindowOptions::window_decorations`] instead.
    pub appears_transparent: bool,

    /// The position of the macOS traffic-light buttons, in logical pixels. macOS only.
    pub traffic_light_position: Option<Point<Pixels>>,
}

impl FluentBuilder for TitlebarOptions {}

/// The kind of window to create, set with [`WindowOptions::kind`].
///
/// On the web, only `Normal` works. The other kinds return an error from
/// [`App::open_window`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// A normal application window
    Normal,

    /// A window that appears above all other windows, usually used for alerts or popups.
    /// Use it sparingly.
    PopUp,

    /// A parent-anchored, platform-native popup window for menus, comboboxes, context menus and
    /// tooltips. Unlike [`WindowKind::PopUp`], it is positioned relative to a parent window.
    ///
    /// The popup's size comes from [`WindowOptions::window_bounds`], whose origin is ignored.
    /// See [`popup::PopupOptions`] for the placement options. Platforms without a native
    /// implementation reject it with [`popup::PopupNotSupportedError`].
    AnchoredPopup(popup::PopupOptions),

    /// A floating window that appears on top of its parent window.
    Floating,

    /// A Wayland LayerShell window, used to draw overlays or backgrounds for applications such as
    /// docks, notifications or wallpapers.
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    LayerShell(layer_shell::LayerShellOptions),

    /// A window that appears on top of its parent window and blocks interaction with it
    /// until the modal window is closed.
    Dialog,
}

/// The light or dark appearance of a window, as the operating system defines it.
///
/// Read it with [`Window::appearance`]. [`Platform::set_window_appearance`] can override the system's setting on macOS.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WindowAppearance {
    /// A light appearance.
    ///
    /// On macOS, this corresponds to the `aqua` appearance.
    #[default]
    Light,

    /// A light appearance with vibrant colors.
    ///
    /// On macOS, this corresponds to the `NSAppearanceNameVibrantLight` appearance.
    VibrantLight,

    /// A dark appearance.
    ///
    /// On macOS, this corresponds to the `darkAqua` appearance.
    Dark,

    /// A dark appearance with vibrant colors.
    ///
    /// On macOS, this corresponds to the `NSAppearanceNameVibrantDark` appearance.
    VibrantDark,
}

/// How a window's background is drawn when there is no content or the content is transparent.
///
/// The variants a platform supports depend on the target. The Mica backdrop materials
/// are supported only on Windows 11, and are ignored elsewhere. Set the appearance
/// with the `macos_window_background`, `windows_window_background`,
/// `linux_window_background` or `web_window_background` field of [`WindowOptions`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum WindowBackgroundAppearance {
    /// Opaque.
    ///
    /// This lets the window manager know that content behind this
    /// window does not need to be drawn.
    ///
    /// Actual color depends on the system and themes should define a fully
    /// opaque background color instead.
    #[default]
    Opaque,
    /// Plain alpha transparency.
    Transparent,
    /// Transparency, but the contents behind the window are blurred.
    ///
    /// Not always supported.
    Blurred,
    /// The Mica backdrop material, supported on Windows 11.
    MicaBackdrop,
    /// The Mica Alt backdrop material, supported on Windows 11.
    MicaAltBackdrop,
}

impl WindowBackgroundAppearance {
    /// Whether the window's background hides everything behind the window,
    /// letting the platform skip compositing the content it covers.
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }

    /// Whether the window's background lets content behind the window show
    /// through, either directly, blurred, or via a system backdrop material.
    pub fn is_transparent(&self) -> bool {
        !self.is_opaque()
    }
}

#[cfg(test)]
mod window_background_appearance_tests {
    use super::WindowBackgroundAppearance;

    #[test]
    fn only_opaque_hides_the_content_behind_the_window() {
        assert!(WindowBackgroundAppearance::Opaque.is_opaque());
        assert!(!WindowBackgroundAppearance::Opaque.is_transparent());

        for appearance in [
            WindowBackgroundAppearance::Transparent,
            WindowBackgroundAppearance::Blurred,
            WindowBackgroundAppearance::MicaBackdrop,
            WindowBackgroundAppearance::MicaAltBackdrop,
        ] {
            assert!(appearance.is_transparent());
            assert!(!appearance.is_opaque());
        }
    }
}

/// The background appearance of a macOS window, set through
/// [`WindowOptions::macos_window_background`].
#[cfg(target_os = "macos")]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum MacosWindowBackground {
    /// Hides everything behind the window. Themes should define a fully
    /// opaque background color instead of relying on the system's.
    #[default]
    Opaque,
    /// Plain alpha transparency.
    Transparent,
    /// Transparency with the contents behind the window blurred, via the
    /// system's vibrancy materials.
    Blurred,
}

/// The background appearance of a Windows window, set through
/// [`WindowOptions::windows_window_background`].
#[cfg(target_os = "windows")]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum WindowsWindowBackground {
    /// Hides everything behind the window. Themes should define a fully
    /// opaque background color instead of relying on the system's.
    #[default]
    Opaque,
    /// Plain alpha transparency.
    Transparent,
    /// Transparency with the contents behind the window blurred.
    Blurred,
    /// The Mica backdrop material, supported on Windows 11.
    MicaBackdrop,
    /// The Mica Alt backdrop material, supported on Windows 11.
    MicaAltBackdrop,
}

/// The background appearance of a Linux window, set through
/// [`WindowOptions::linux_window_background`]. Whether blurring is honored
/// depends on the compositor; X11 treats it as plain transparency.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum LinuxWindowBackground {
    /// Hides everything behind the window. Themes should define a fully
    /// opaque background color instead of relying on the system's.
    #[default]
    Opaque,
    /// Plain alpha transparency.
    Transparent,
    /// Transparency with the contents behind the window blurred, when the
    /// compositor supports it.
    Blurred,
}

/// The background appearance of a web window, set through
/// `web_window_background`. Web windows are always opaque at
/// the platform level.
#[cfg(target_family = "wasm")]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum WebWindowBackground {
    /// The window background is always opaque.
    #[default]
    Opaque,
}

#[cfg(target_os = "macos")]
impl From<MacosWindowBackground> for WindowBackgroundAppearance {
    fn from(background: MacosWindowBackground) -> Self {
        match background {
            MacosWindowBackground::Opaque => Self::Opaque,
            MacosWindowBackground::Transparent => Self::Transparent,
            MacosWindowBackground::Blurred => Self::Blurred,
        }
    }
}

#[cfg(target_os = "windows")]
impl From<WindowsWindowBackground> for WindowBackgroundAppearance {
    fn from(background: WindowsWindowBackground) -> Self {
        match background {
            WindowsWindowBackground::Opaque => Self::Opaque,
            WindowsWindowBackground::Transparent => Self::Transparent,
            WindowsWindowBackground::Blurred => Self::Blurred,
            WindowsWindowBackground::MicaBackdrop => Self::MicaBackdrop,
            WindowsWindowBackground::MicaAltBackdrop => Self::MicaAltBackdrop,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl From<LinuxWindowBackground> for WindowBackgroundAppearance {
    fn from(background: LinuxWindowBackground) -> Self {
        match background {
            LinuxWindowBackground::Opaque => Self::Opaque,
            LinuxWindowBackground::Transparent => Self::Transparent,
            LinuxWindowBackground::Blurred => Self::Blurred,
        }
    }
}

#[cfg(target_family = "wasm")]
impl From<WebWindowBackground> for WindowBackgroundAppearance {
    fn from(background: WebWindowBackground) -> Self {
        match background {
            WebWindowBackground::Opaque => Self::Opaque,
        }
    }
}

#[cfg(test)]
mod platform_window_background_tests {
    use super::WindowBackgroundAppearance;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_backgrounds_map_to_appearances() {
        use super::MacosWindowBackground;
        assert_eq!(
            WindowBackgroundAppearance::from(MacosWindowBackground::Opaque),
            WindowBackgroundAppearance::Opaque
        );
        assert_eq!(
            WindowBackgroundAppearance::from(MacosWindowBackground::Blurred),
            WindowBackgroundAppearance::Blurred
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_backgrounds_map_to_appearances() {
        use super::WindowsWindowBackground;
        assert_eq!(
            WindowBackgroundAppearance::from(WindowsWindowBackground::MicaAltBackdrop),
            WindowBackgroundAppearance::MicaAltBackdrop
        );
        assert_eq!(
            WindowBackgroundAppearance::from(WindowsWindowBackground::default()),
            WindowBackgroundAppearance::Opaque
        );
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    #[test]
    fn linux_backgrounds_map_to_appearances() {
        use super::LinuxWindowBackground;
        assert_eq!(
            WindowBackgroundAppearance::from(LinuxWindowBackground::Transparent),
            WindowBackgroundAppearance::Transparent
        );
    }

    #[cfg(target_family = "wasm")]
    #[test]
    fn web_backgrounds_map_to_appearances() {
        use super::WebWindowBackground;
        assert_eq!(
            WindowBackgroundAppearance::from(WebWindowBackground::Opaque),
            WindowBackgroundAppearance::Opaque
        );
    }
}

/// The text rendering mode for glyphs.
///
/// [`PlatformTextSystem::recommended_rendering_mode`] returns the platform's choice.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextRenderingMode {
    /// Use the platform's default text rendering mode.
    #[default]
    PlatformDefault,
    /// Use subpixel (ClearType-style) text rendering.
    Subpixel,
    /// Use grayscale text rendering.
    Grayscale,
}

/// Options for [`Platform::prompt_for_paths`], which shows a file or folder picker.
#[derive(Clone, Debug)]
pub struct PathPromptOptions {
    /// Whether the user can pick files.
    pub files: bool,
    /// Whether the user can pick directories.
    pub directories: bool,
    /// Whether the user can pick more than one item.
    pub multiple: bool,
    /// Text that explains the request in the picker, on platforms that display it.
    pub prompt: Option<SharedString>,
}

/// How serious a prompt is. The platform uses it to choose the alert's icon or style.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PromptLevel {
    /// A prompt that is shown when the user should be notified of something
    Info,

    /// A prompt that is shown when the user needs to be warned of a potential problem
    Warning,

    /// A prompt that is shown when a critical problem has occurred
    Critical,
}

/// A button on a prompt shown with [`PlatformWindow::prompt`].
///
/// The variant tells the platform the button's role, such as the default or cancel action.
/// Use [`PromptButton::from`] to get a role from a label: `"ok"` and `"cancel"`,
/// in any case, become `Ok` and `Cancel`, and any other text becomes `Other`.
///
/// ```
/// use gpui::PromptButton;
///
/// assert_eq!(PromptButton::from("ok"), PromptButton::Ok("OK".into()));
/// assert_eq!(PromptButton::from("Delete"), PromptButton::Other("Delete".into()));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum PromptButton {
    /// Ok button
    Ok(SharedString),
    /// Cancel button
    Cancel(SharedString),
    /// Other button
    Other(SharedString),
}

impl PromptButton {
    /// Creates a button with `label` and no special role.
    pub fn new(label: impl Into<SharedString>) -> Self {
        PromptButton::Other(label.into())
    }

    /// Creates a button with the [`PromptButton::Ok`] role.
    pub fn ok(label: impl Into<SharedString>) -> Self {
        PromptButton::Ok(label.into())
    }

    /// Creates a button with the [`PromptButton::Cancel`] role.
    pub fn cancel(label: impl Into<SharedString>) -> Self {
        PromptButton::Cancel(label.into())
    }

    /// Returns `true` if this is the cancel button.
    #[allow(dead_code)]
    pub fn is_cancel(&self) -> bool {
        matches!(self, PromptButton::Cancel(_))
    }

    /// Returns the button's label.
    pub fn label(&self) -> &SharedString {
        match self {
            PromptButton::Ok(label) => label,
            PromptButton::Cancel(label) => label,
            PromptButton::Other(label) => label,
        }
    }
}

impl From<&str> for PromptButton {
    fn from(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "ok" => PromptButton::Ok("OK".into()),
            "cancel" => PromptButton::Cancel("Cancel".into()),
            _ => PromptButton::Other(SharedString::from(value.to_owned())),
        }
    }
}

/// The shape of the mouse cursor. Each variant names the CSS cursor it maps to on the web.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum CursorStyle {
    /// The default cursor, corresponds to the CSS cursor value `default`
    #[default]
    Arrow,

    /// A text input cursor
    /// corresponds to the CSS cursor value `text`
    IBeam,

    /// A crosshair cursor
    /// corresponds to the CSS cursor value `crosshair`
    Crosshair,

    /// A closed hand cursor
    /// corresponds to the CSS cursor value `grabbing`
    ClosedHand,

    /// An open hand cursor
    /// corresponds to the CSS cursor value `grab`
    OpenHand,

    /// A pointing hand cursor
    /// corresponds to the CSS cursor value `pointer`
    PointingHand,

    /// A resize left cursor
    /// corresponds to the CSS cursor value `ew-resize`
    ResizeLeft,

    /// A resize right cursor
    /// corresponds to the CSS cursor value `ew-resize`
    ResizeRight,

    /// A resize cursor to the left and right
    /// corresponds to the CSS cursor value `ew-resize`
    ResizeLeftRight,

    /// A resize up cursor
    /// corresponds to the CSS cursor value `ns-resize`
    ResizeUp,

    /// A resize down cursor
    /// corresponds to the CSS cursor value `ns-resize`
    ResizeDown,

    /// A resize cursor directing up and down
    /// corresponds to the CSS cursor value `ns-resize`
    ResizeUpDown,

    /// A cursor for the upper-left resize edge (`nw-resize`).
    ResizeUpLeft,

    /// A cursor for the upper-right resize edge (`ne-resize`).
    ResizeUpRight,

    /// A cursor for the lower-left resize edge (`sw-resize`).
    ResizeDownLeft,

    /// A cursor for the lower-right resize edge (`se-resize`).
    ResizeDownRight,

    /// A resize cursor directing up-left and down-right
    /// corresponds to the CSS cursor value `nwse-resize`
    ResizeUpLeftDownRight,

    /// A resize cursor directing up-right and down-left
    /// corresponds to the CSS cursor value `nesw-resize`
    ResizeUpRightDownLeft,

    /// A cursor indicating that the item/column can be resized horizontally.
    /// corresponds to the CSS cursor value `col-resize`
    ResizeColumn,

    /// A cursor indicating that the item/row can be resized vertically.
    /// corresponds to the CSS cursor value `row-resize`
    ResizeRow,

    /// A text input cursor for vertical layout
    /// corresponds to the CSS cursor value `vertical-text`
    IBeamCursorForVerticalLayout,

    /// A cursor indicating that the operation is not allowed
    /// corresponds to the CSS cursor value `not-allowed`
    OperationNotAllowed,

    /// A cursor indicating that the operation will result in a link
    /// corresponds to the CSS cursor value `alias`
    DragLink,

    /// A cursor indicating that the operation will result in a copy
    /// corresponds to the CSS cursor value `copy`
    DragCopy,

    /// A cursor indicating that the operation will result in a context menu
    /// corresponds to the CSS cursor value `context-menu`
    ContextualMenu,
}

/// Something to put on the clipboard, or something read from it.
///
/// An item holds one or more [`ClipboardEntry`] values, such as text, an image or file
/// paths. Use [`App::write_to_clipboard`] to copy an item, and
/// [`App::read_from_clipboard`] to read one.
///
/// # Examples
///
/// ```
/// use gpui::{ClipboardEntry, ClipboardItem};
///
/// let item = ClipboardItem::new_string("hello".to_string());
/// assert_eq!(item.text().as_deref(), Some("hello"));
/// assert_eq!(item.metadata(), None);
///
/// let item = ClipboardItem::new_string_with_json_metadata("1, 2, 3".to_string(), vec![1, 2, 3]);
/// assert_eq!(item.metadata().map(String::as_str), Some("[1,2,3]"));
///
/// let ClipboardEntry::String(entry) = &item.entries()[0] else {
///     panic!("expected a string entry");
/// };
/// assert_eq!(entry.metadata_json::<Vec<i32>>(), Some(vec![1, 2, 3]));
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardItem {
    /// The entries in this clipboard item.
    pub entries: Vec<ClipboardEntry>,
}

/// An error from [`Platform::read_from_clipboard_async`]. Each variant calls for different guidance to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardReadError {
    /// The platform clipboard is not available in this context, e.g. the
    /// browser does not expose the async clipboard API or the page is not a
    /// secure context.
    Unavailable,
    /// The platform refused access, e.g. the user declined the browser's
    /// clipboard permission prompt or paste confirmation.
    Denied(String),
    /// The clipboard contents could not be converted into a
    /// [`ClipboardItem`].
    UnsupportedContent,
}

impl std::fmt::Display for ClipboardReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("the clipboard is unavailable"),
            Self::Denied(message) => {
                write!(formatter, "clipboard access was denied: {message}")
            }
            Self::UnsupportedContent => {
                formatter.write_str("the clipboard contents are unsupported")
            }
        }
    }
}

impl std::error::Error for ClipboardReadError {}

/// One part of a [`ClipboardItem`]: a string, an image or a list of file paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardEntry {
    /// A string entry
    String(ClipboardString),
    /// An image entry
    Image(Image),
    /// A file entry
    ExternalPaths(crate::ExternalPaths),
}

impl ClipboardItem {
    /// Creates an item holding `text`, with no metadata.
    pub fn new_string(text: String) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(ClipboardString::new(text))],
        }
    }

    /// Creates an item holding `text` and a raw `metadata` string.
    ///
    /// Use [`ClipboardItem::new_string_with_json_metadata`] to store structured data.
    pub fn new_string_with_metadata(text: String, metadata: String) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(ClipboardString {
                text,
                metadata: Some(metadata),
            })],
        }
    }

    /// Creates an item holding `text` and `metadata`, serialized as JSON.
    ///
    /// Read the metadata back with [`ClipboardString::metadata_json`].
    ///
    /// # Panics
    ///
    /// Panics if `metadata` can't be serialized to JSON, such as a map with non-string keys.
    pub fn new_string_with_json_metadata<T: Serialize>(text: String, metadata: T) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(
                ClipboardString::new(text).with_json_metadata(metadata),
            )],
        }
    }

    /// Creates an item holding a copy of `image`.
    pub fn new_image(image: &Image) -> Self {
        Self {
            entries: vec![ClipboardEntry::Image(image.clone())],
        }
    }

    /// Returns the text of all string entries joined together, or `None` if the item has no text.
    ///
    /// If the string entries contain no text, the file paths of any [`ClipboardEntry::ExternalPaths`]
    /// entries are joined with no separator instead. The result is `None` if there are no paths either.
    pub fn text(&self) -> Option<String> {
        let mut answer = String::new();

        for entry in self.entries.iter() {
            if let ClipboardEntry::String(ClipboardString { text, metadata: _ }) = entry {
                answer.push_str(text);
            }
        }

        if answer.is_empty() {
            for entry in self.entries.iter() {
                if let ClipboardEntry::ExternalPaths(paths) = entry {
                    for path in &paths.0 {
                        use std::fmt::Write as _;
                        _ = write!(answer, "{}", path.display());
                    }
                }
            }
        }

        if !answer.is_empty() {
            Some(answer)
        } else {
            None
        }
    }

    /// Returns the metadata of the item's string entry, or `None`.
    ///
    /// Returns `None` unless the item has exactly one entry and that entry is a string.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn metadata(&self) -> Option<&String> {
        match self.entries().first() {
            Some(ClipboardEntry::String(clipboard_string)) if self.entries.len() == 1 => {
                clipboard_string.metadata.as_ref()
            }
            _ => None,
        }
    }

    /// Returns the item's entries.
    pub fn entries(&self) -> &[ClipboardEntry] {
        &self.entries
    }

    /// Consumes the item and returns its entries.
    pub fn into_entries(self) -> impl Iterator<Item = ClipboardEntry> {
        self.entries.into_iter()
    }
}

impl From<ClipboardString> for ClipboardEntry {
    fn from(value: ClipboardString) -> Self {
        Self::String(value)
    }
}

impl From<String> for ClipboardEntry {
    fn from(value: String) -> Self {
        Self::from(ClipboardString::from(value))
    }
}

impl From<Image> for ClipboardEntry {
    fn from(value: Image) -> Self {
        Self::Image(value)
    }
}

impl From<ClipboardEntry> for ClipboardItem {
    fn from(value: ClipboardEntry) -> Self {
        Self {
            entries: vec![value],
        }
    }
}

impl From<String> for ClipboardItem {
    fn from(value: String) -> Self {
        Self::from(ClipboardEntry::from(value))
    }
}

impl From<Image> for ClipboardItem {
    fn from(value: Image) -> Self {
        Self::from(ClipboardEntry::from(value))
    }
}

/// An image format GPUI recognizes in clipboard data and in [`Image`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, EnumIter, Hash)]
pub enum ImageFormat {
    // Sorted from most to least likely to be pasted into an editor,
    // which matters when we iterate through them trying to see if
    // clipboard content matches them.
    /// .png
    Png,
    /// .jpeg or .jpg
    Jpeg,
    /// .webp
    Webp,
    /// .gif
    Gif,
    /// .svg
    Svg,
    /// .bmp
    Bmp,
    /// .tif or .tiff
    Tiff,
    /// .ico
    Ico,
    /// Netpbm image formats (.pbm, .ppm, .pgm).
    Pnm,
}

impl ImageFormat {
    /// Returns the canonical MIME type, such as `image/png`.
    pub const fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Svg => "image/svg+xml",
            ImageFormat::Bmp => "image/bmp",
            ImageFormat::Tiff => "image/tiff",
            ImageFormat::Ico => "image/ico",
            ImageFormat::Pnm => "image/x-portable-anymap",
        }
    }

    /// Returns the file extension for this image format (without leading dot).
    pub const fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Ico => "ico",
            ImageFormat::Pnm => "pnm",
        }
    }

    /// Returns the format for `mime_type`, or `None` if the type is unknown.
    ///
    /// The aliases `image/jpg` and `image/tif` map to [`ImageFormat::Jpeg`] and [`ImageFormat::Tiff`].
    ///
    /// ```
    /// use gpui::ImageFormat;
    ///
    /// assert_eq!(ImageFormat::from_mime_type("image/jpg"), Some(ImageFormat::Jpeg));
    /// assert_eq!(ImageFormat::Png.extension(), "png");
    /// assert_eq!(ImageFormat::from_mime_type("text/plain"), None);
    /// ```
    pub fn from_mime_type(mime_type: &str) -> Option<Self> {
        use strum::IntoEnumIterator;
        Self::iter()
            .find(|format| format.mime_type() == mime_type)
            .or_else(|| Self::from_mime_type_alias(mime_type))
    }

    /// Non-canonical mime types that some producers use in the wild.
    /// Unlike `mime_type()` which returns the single canonical form,
    /// these are legacy or shortened variants we still need to recognize.
    fn from_mime_type_alias(mime_type: &str) -> Option<Self> {
        match mime_type {
            "image/jpg" => Some(Self::Jpeg),
            "image/tif" => Some(Self::Tiff),
            _ => None,
        }
    }
}

/// Encoded image data and the format it is encoded in.
///
/// Use [`Image::from_bytes`] to create one. GPUI decodes it for display through
/// [`Image::to_image_data`] or the asset APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The format the bytes are encoded in, such as PNG.
    pub format: ImageFormat,
    /// The encoded image bytes.
    pub bytes: Vec<u8>,
    /// A key for the image in GPUI's asset cache. [`Image::from_bytes`] sets it to a hash of `bytes`.
    pub id: u64,
}

pub(crate) fn decode_static_image(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<SmallVec<[Frame; 1]>> {
    let decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .context("creating image decoder")?;
    decode_static_image_from_decoder(decoder)
}

pub(crate) fn decode_static_image_from_decoder(
    mut decoder: impl image::ImageDecoder,
) -> Result<SmallVec<[Frame; 1]>> {
    let orientation = decoder
        .orientation()
        .context("reading decoder's orientation")?;
    let mut image = DynamicImage::from_decoder(decoder).context("decoding image")?;
    image.apply_orientation(orientation);

    let mut data = image.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    Ok(SmallVec::from_elem(Frame::new(data), 1))
}

impl Hash for Image {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.id);
    }
}

impl Image {
    /// Returns a PNG image with no bytes. It can't be decoded.
    pub fn empty() -> Self {
        Self::from_bytes(ImageFormat::Png, Vec::new())
    }

    /// Creates an image from `format` and encoded `bytes`.
    ///
    /// The [`Image::id`] is a hash of `bytes` alone, so two images with the same bytes but different formats share an id.
    pub fn from_bytes(format: ImageFormat, bytes: Vec<u8>) -> Self {
        Self {
            id: hash(&bytes),
            format,
            bytes,
        }
    }

    /// Returns the image's cache key. [`Image::from_bytes`] sets it from a hash of the bytes.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns the decoded image, starting the load if it isn't cached yet.
    ///
    /// Returns `None` while the image is loading, or if it can't be decoded. Call it from a render method, so the window redraws when the load finishes.
    pub fn use_render_image(
        self: Arc<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        ImageSource::Image(self)
            .use_data(None, window, cx)
            .and_then(|result| result.ok())
    }

    /// Returns the decoded image if it's already cached. Doesn't start a load.
    pub fn get_render_image(
        self: Arc<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        ImageSource::Image(self)
            .get_data(None, window, cx)
            .and_then(|result| result.ok())
    }

    /// Removes the decoded image from GPUI's asset cache.
    pub fn remove_asset(self: Arc<Self>, cx: &mut App) {
        ImageSource::Image(self).remove_asset(cx);
    }

    /// Check whether this image is present in GPUI's asset cache (loading or
    /// loaded), without fetching it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn is_asset_cached(self: &Arc<Self>, cx: &App) -> bool {
        ImageSource::Image(self.clone()).is_asset_cached(cx)
    }

    /// Decodes the image into a [`RenderImage`] that you can paint.
    ///
    /// SVG images are rendered at scale `1.0`.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes can't be decoded. For a GIF, frames that fail to
    /// decode are skipped and logged, and an error is returned only when every frame fails.
    pub fn to_image_data(&self, svg_renderer: SvgRenderer) -> Result<Arc<RenderImage>> {
        let frames = match self.format {
            ImageFormat::Gif => {
                let decoder = GifDecoder::new(Cursor::new(&self.bytes))?;
                let mut frames = SmallVec::new();

                for frame in decoder.into_frames() {
                    match frame {
                        Ok(mut frame) => {
                            // Convert from RGBA to BGRA.
                            for pixel in frame.buffer_mut().chunks_exact_mut(4) {
                                pixel.swap(0, 2);
                            }
                            frames.push(frame);
                        }
                        Err(err) => {
                            log::debug!("Skipping GIF frame due to decode error: {err}");
                        }
                    }
                }

                if frames.is_empty() {
                    anyhow::bail!("GIF could not be decoded: all frames failed");
                }

                frames
            }
            ImageFormat::Png => decode_static_image(&self.bytes, image::ImageFormat::Png)?,
            ImageFormat::Jpeg => decode_static_image(&self.bytes, image::ImageFormat::Jpeg)?,
            ImageFormat::Webp => decode_static_image(&self.bytes, image::ImageFormat::WebP)?,
            ImageFormat::Bmp => decode_static_image(&self.bytes, image::ImageFormat::Bmp)?,
            ImageFormat::Tiff => decode_static_image(&self.bytes, image::ImageFormat::Tiff)?,
            ImageFormat::Ico => decode_static_image(&self.bytes, image::ImageFormat::Ico)?,
            ImageFormat::Svg => {
                return svg_renderer
                    .render_single_frame(&self.bytes, 1.0)
                    .map_err(Into::into);
            }
            ImageFormat::Pnm => decode_static_image(&self.bytes, image::ImageFormat::Pnm)?,
        };

        Ok(Arc::new(RenderImage::new(frames)))
    }

    /// Returns the image's format.
    pub fn format(&self) -> ImageFormat {
        self.format
    }

    /// Returns the encoded image bytes.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

/// A string on the clipboard, with optional metadata.
///
/// Metadata lets an application record more about copied text, such as where it came
/// from. The metadata is stored with the text and read back with [`ClipboardString::metadata_json`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardString {
    /// The text content.
    pub text: String,
    /// Optional metadata associated with this clipboard string.
    pub metadata: Option<String>,
}

impl ClipboardString {
    /// Creates a clipboard string with `text` and no metadata.
    pub fn new(text: String) -> Self {
        Self {
            text,
            metadata: None,
        }
    }

    /// Sets the metadata to `metadata`, serialized as JSON. Returns the updated string.
    ///
    /// # Panics
    ///
    /// Panics if `metadata` can't be serialized to JSON.
    pub fn with_json_metadata<T: Serialize>(mut self, metadata: T) -> Self {
        self.metadata = Some(serde_json::to_string(&metadata).unwrap());
        self
    }

    /// Returns the text.
    pub fn text(&self) -> &String {
        &self.text
    }

    /// Consumes the string and returns its text.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Returns the metadata deserialized as `T`.
    ///
    /// Returns `None` if there is no metadata or it doesn't deserialize as `T`.
    pub fn metadata_json<T>(&self) -> Option<T>
    where
        T: for<'a> Deserialize<'a>,
    {
        self.metadata
            .as_ref()
            .and_then(|m| serde_json::from_str(m).ok())
    }

    /// Returns a hash of `text`. Use it to detect whether the clipboard text has changed.
    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub fn text_hash(text: &str) -> u64 {
        let mut hasher = SeaHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }
}

impl From<String> for ClipboardString {
    fn from(value: String) -> Self {
        Self {
            text: value,
            metadata: None,
        }
    }
}

#[cfg(test)]
mod ime_candidate_bounds_tests {
    use super::*;
    use crate::size;

    // Offsets 0..3 sit on the first visual line and the rest wrap onto the second.
    fn bounds_at(offset: usize) -> Bounds<Pixels> {
        let line = if offset < 3 { 0. } else { 20. };
        Bounds::new(point(px(offset as f32), px(line)), size(px(1.), px(20.)))
    }

    fn candidate_bounds(
        marked_range: Option<Range<usize>>,
        selection: UTF16Selection,
    ) -> Option<Bounds<Pixels>> {
        PlatformInputHandler::compute_ime_candidate_bounds(marked_range, &selection, |range| {
            Some(bounds_at(range.start))
        })
    }

    #[test]
    fn composition_follows_the_active_end_of_a_reversed_selection() {
        // The caret is at offset 2 on the first line, so the candidate window
        // stays at the start of the composition instead of the wrapped line.
        let bounds = candidate_bounds(
            Some(0..8),
            UTF16Selection {
                range: 2..7,
                reversed: true,
            },
        );
        assert_eq!(bounds, Some(bounds_at(0)));
    }

    #[test]
    fn composition_uses_the_start_of_the_caret_line() {
        let bounds = candidate_bounds(
            Some(0..8),
            UTF16Selection {
                range: 2..7,
                reversed: false,
            },
        );
        assert_eq!(bounds, Some(bounds_at(3)));
    }

    #[test]
    fn without_composition_uses_the_active_end() {
        let bounds = candidate_bounds(
            None,
            UTF16Selection {
                range: 2..7,
                reversed: true,
            },
        );
        assert_eq!(bounds, Some(bounds_at(2)));
    }
}

#[cfg(test)]
mod image_tests {
    use crate::{AssetRegistry, size};

    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_image_to_image_data_applies_exif_orientation() {
        let image = Image::from_bytes(
            ImageFormat::Jpeg,
            include_bytes!("../tests/fixtures/exif-orientation-rotate-180.jpg").to_vec(),
        );

        let render_image = image
            .to_image_data(SvgRenderer::new(Arc::new(AssetRegistry::default())))
            .unwrap();

        assert_eq!(render_image.size(0), size(16.into(), 32.into()));

        let bytes = render_image.as_bytes(0).unwrap();
        assert_eq!(&bytes[..4], &[255, 255, 255, 255]);
        assert_eq!(&bytes[(16 * 32 - 1) * 4..], &[0, 0, 0, 255]);
    }

    #[test]
    fn test_svg_image_to_image_data_converts_to_bgra() {
        let image = Image::from_bytes(
            ImageFormat::Svg,
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">
<rect width="1" height="1" fill="#38BDF8"/>
</svg>"##
                .to_vec(),
        );

        let render_image = image
            .to_image_data(SvgRenderer::new(Arc::new(AssetRegistry::default())))
            .unwrap();
        let bytes = render_image.as_bytes(0).unwrap();

        for pixel in bytes.chunks_exact(4) {
            assert_eq!(pixel, &[0xF8, 0xBD, 0x38, 0xFF]);
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod window_button_layout_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_window_button_layout_parse_standard() {
        let layout = WindowButtonLayout::parse("close,minimize:maximize").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_right_only() {
        let layout = WindowButtonLayout::parse("minimize,maximize,close").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close)
            ]
        );
    }

    #[test]
    fn test_window_button_layout_parse_left_only() {
        let layout = WindowButtonLayout::parse("close,minimize,maximize:").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize)
            ]
        );
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_with_whitespace() {
        let layout = WindowButtonLayout::parse(" close , minimize : maximize ").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_empty() {
        let layout = WindowButtonLayout::parse("").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_intentionally_empty() {
        let layout = WindowButtonLayout::parse(":").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_invalid_buttons() {
        let layout = WindowButtonLayout::parse("close,invalid,minimize:maximize,foo").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_deduplicates_same_side_buttons() {
        let layout = WindowButtonLayout::parse("close,close,minimize").unwrap();
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.format(), ":close,minimize");
    }

    #[test]
    fn test_window_button_layout_parse_deduplicates_buttons_across_sides() {
        let layout = WindowButtonLayout::parse("close:maximize,close,minimize").unwrap();
        assert_eq!(layout.left, [Some(WindowButton::Close), None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Maximize),
                Some(WindowButton::Minimize),
                None
            ]
        );

        let button_ids: Vec<_> = layout
            .left
            .iter()
            .chain(layout.right.iter())
            .flatten()
            .map(WindowButton::id)
            .collect();
        let unique_button_ids = button_ids.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique_button_ids.len(), button_ids.len());
        assert_eq!(layout.format(), "close:maximize,minimize");
    }

    #[test]
    fn test_window_button_layout_parse_gnome_style() {
        let layout = WindowButtonLayout::parse("close").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [Some(WindowButton::Close), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_elementary_style() {
        let layout = WindowButtonLayout::parse("close:maximize").unwrap();
        assert_eq!(layout.left, [Some(WindowButton::Close), None, None]);
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_round_trip() {
        let cases = [
            "close:minimize,maximize",
            "minimize,maximize,close:",
            ":close",
            "close:",
            "close:maximize",
            ":",
        ];

        for case in cases {
            let layout = WindowButtonLayout::parse(case).unwrap();
            assert_eq!(layout.format(), case, "Round-trip failed for: {}", case);
        }
    }

    #[test]
    fn test_window_button_layout_linux_default() {
        let layout = WindowButtonLayout::linux_default();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close)
            ]
        );

        let round_tripped = WindowButtonLayout::parse(&layout.format()).unwrap();
        assert_eq!(round_tripped, layout);
    }

    #[test]
    fn test_window_button_layout_parse_all_invalid() {
        assert!(WindowButtonLayout::parse("asdfghjkl").is_err());
    }
}
