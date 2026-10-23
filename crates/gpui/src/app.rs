use crate::{
    Action, ActionBuildError, ActionRegistry, Any, AnyView, AnyWindowHandle, AppContext, Arena,
    ArenaBox, Asset, AssetRegistry, BackgroundExecutor, Bounds, ClipboardItem, ClipboardReadError,
    CursorStyle, DispatchPhase, DisplayId, EventEmitter, ExternalDragPayload, FocusHandle,
    FocusMap, ForegroundExecutor, Global, HapticFeedbackStyle, KeyBinding, KeyContext, Keymap,
    Keystroke, LayoutId, Menu, MenuItem, OwnedMenu, PathPromptOptions, Pixels, Platform,
    PlatformDisplay, PlatformKeyboardLayout, PlatformKeyboardMapper, Point, Priority,
    PromptBuilder, PromptButton, PromptHandle, PromptLevel, Render, RenderImage,
    RenderablePromptHandle, Reservation, ScreenCaptureSource, SharedString, SubscriberSet,
    Subscription, SvgRenderer, SystemNotification, SystemNotificationResponse, Task,
    TextRenderingMode, TextSystem, ThermalState, Window, WindowAppearance, WindowButtonLayout,
    WindowHandle, WindowId, WindowInvalidator,
    colors::{Colors, GlobalColors},
    hash,
    http_client::{HttpClient, NullHttpClient},
    init_app_menus,
};
use anyhow::{Context as _, Result, anyhow};
use collections::{FxHashMap, FxHashSet, HashMap, TypeIdHashMap, TypeIdHashSet, VecDeque};
use derive_more::{Deref, DerefMut};
use futures::{
    Future, FutureExt,
    channel::oneshot,
    future::{LocalBoxFuture, Shared},
};
use gpui_util::{ResultExt, debug_panic};
use itertools::Itertools;
use parking_lot::RwLock;
use scheduler::Instant;
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{
    any::{TypeId, type_name},
    cell::{BorrowMutError, Cell, Ref, RefCell, RefMut},
    ffi::OsString,
    marker::PhantomData,
    mem,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::{Arc, atomic::Ordering::SeqCst},
    time::Duration,
};

mod async_context;
#[cfg(feature = "bench-support")]
mod bench_context;
mod context;
mod entity_map;
#[cfg(any(test, feature = "test-support"))]
mod headless_app_context;
#[cfg(any(test, feature = "test-support"))]
mod test_app;
#[cfg(any(test, feature = "test-support"))]
mod test_context;
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
mod visual_test_context;

pub use async_context::*;
#[cfg(feature = "bench-support")]
pub use bench_context::{BenchAppContext, BenchReport, BenchWindowContext, bench_platform};
pub use context::*;
pub use entity_map::*;
#[cfg(any(test, feature = "test-support"))]
pub use headless_app_context::*;
#[cfg(any(test, feature = "test-support"))]
pub use test_app::*;
#[cfg(any(test, feature = "test-support"))]
pub use test_context::*;
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub use visual_test_context::*;

#[cfg(any(feature = "inspector", debug_assertions))]
use crate::InspectorElementRegistry;
#[cfg(target_os = "macos")]
use crate::MacActivationPolicy;

/// The duration for which native applications wait for futures returned from
/// [Context::on_app_quit] before fully quitting.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(200);

/// Temporary(?) wrapper around [`RefCell<App>`] to help us debug any double borrows.
/// Strongly consider removing after stabilization.
#[doc(hidden)]
pub struct AppCell {
    app: RefCell<App>,
}

impl AppCell {
    #[doc(hidden)]
    #[track_caller]
    pub fn borrow(&self) -> AppRef<'_> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        AppRef(self.app.borrow())
    }

    #[doc(hidden)]
    #[track_caller]
    pub fn borrow_mut(&self) -> AppRefMut<'_> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        AppRefMut(self.app.borrow_mut())
    }

    #[doc(hidden)]
    #[track_caller]
    pub fn try_borrow_mut(&self) -> Result<AppRefMut<'_>, BorrowMutError> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        Ok(AppRefMut(self.app.try_borrow_mut()?))
    }
}

#[doc(hidden)]
#[derive(Deref, DerefMut)]
pub struct AppRef<'a>(Ref<'a, App>);

impl Drop for AppRef<'_> {
    fn drop(&mut self) {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("dropped borrow from {thread_id:?}");
        }
    }
}

#[doc(hidden)]
#[derive(Deref, DerefMut)]
pub struct AppRefMut<'a>(RefMut<'a, App>);

impl Drop for AppRefMut<'_> {
    fn drop(&mut self) {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("dropped {thread_id:?}");
        }
    }
}

/// The entry point for a GPUI application. Configure it, then call [`Application::run`].
///
/// Get one from `gpui_platform::application()`, which picks the platform for the
/// current target. Configure it with the `with_*` and `on_*` builder methods, then call
/// [`Application::run`] from `main`. The callback you pass to `run` receives the
/// [`App`] once the platform has finished launching. That is where you open windows,
/// bind keys and set globals.
///
/// The `with_*` methods take the application by value and return it, so you can chain
/// them. Call them before [`Application::run`].
///
/// `run` blocks until the app quits on native platforms, so anything after it in
/// `main` runs only after the app exits. Once the app is running, you use [`App`]
/// instead of `Application`.
///
/// # Examples
///
/// ```no_run
/// use gpui::{App, Window, WindowOptions, div, prelude::*};
///
/// struct HelloWorld;
///
/// impl Render for HelloWorld {
///     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
///         div().child("Hello, world!")
///     }
/// }
///
/// fn main() {
///     gpui_platform::application().run(|cx: &mut App| {
///         cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| HelloWorld))
///             .unwrap();
///     });
/// }
/// ```
///
/// See [the start a project guide](crate::_documentation::guides::start_a_project) for a
/// complete app. `TestApp` runs app code without a display, and is available with the
/// `test-support` feature.
pub struct Application(Rc<AppCell>);

/// A strong handle to an [`Application`] started with [`Application::run_embedded`].
///
/// Dropping this handle releases the app, so an embedder must hold it for as long as the
/// app should run. While held, it is the embedder's entry point back into GPUI each time
/// the external run loop gives it control.
pub struct ApplicationHandle {
    app: Rc<AppCell>,
}

impl ApplicationHandle {
    /// Runs `f` with the app context, from outside the platform's run loop.
    ///
    /// Call this each time the external run loop gives GPUI control. Effects queued by `f`,
    /// such as deferred callbacks, run before this returns. Don't call it from
    /// inside an update on the same app: the app state is a `RefCell`, so a re-entrant
    /// call panics with a double borrow.
    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let cx = &mut *self.app.borrow_mut();
        cx.update(f)
    }

    /// Returns an [`AsyncApp`] for use across await points.
    ///
    /// The [`AsyncApp`] holds the app weakly. This handle must stay alive for the
    /// [`AsyncApp`] to do anything useful.
    pub fn to_async(&self) -> AsyncApp {
        self.update(|cx| cx.to_async())
    }
}

impl Application {
    /// Creates an application that uses a platform implementation you provide.
    ///
    /// Most apps should use `gpui_platform::application()`, which chooses the platform
    /// for you. Use this when you want a custom or test platform. The app starts with
    /// an empty [`AssetRegistry`], an HTTP client that fails every request, and
    /// [`QuitMode::Default`].
    ///
    /// # Panics
    ///
    /// Panics if called from a thread other than the main thread.
    pub fn with_platform(platform: Rc<dyn Platform>) -> Self {
        Self(App::new_app(
            platform,
            AssetRegistry::default().into(),
            Arc::new(NullHttpClient),
        ))
    }

    /// Builds an app with accessibility (AccessKit) integration forcibly
    /// disabled.
    ///
    /// In this mode, accessibility APIs (e.g.
    /// [`div().role()`][crate::StatefulInteractiveElement::role]) silently
    /// no-op.
    ///
    /// See the [accessibility guide](crate::_documentation::guides::accessibility) for the
    /// features this disables.
    pub fn new_inaccessible(platform: Rc<dyn Platform>) -> Self {
        let this = Self::with_platform(platform);
        this.0.borrow_mut().accessibility_force_disabled = true;
        this
    }

    /// Build every window's accessibility tree every frame, even when no
    /// assistive technology is connected.
    ///
    /// For automation and tree inspection: [`crate::Window::debug_a11y_tree_json`]
    /// otherwise reports nothing until a screen reader activates the platform
    /// adapter. [`Application::new_inaccessible`] still wins when both are set.
    pub fn with_accessibility_forced(self) -> Self {
        self.0.borrow_mut().accessibility_forced = true;
        self
    }

    /// Sets the source of assets (images, fonts and SVGs) that the app loads by path.
    ///
    /// The registry is returned by [`App::assets`], and the SVG renderer is rebuilt to
    /// read from it. Set it before you open windows that load assets.
    pub fn with_assets(self, assets: impl Into<AssetRegistry>) -> Self {
        let mut context_lock = self.0.borrow_mut();
        let asset_registry = Arc::new(assets.into());
        context_lock.asset_registry = asset_registry.clone();
        context_lock.svg_renderer = SvgRenderer::new(asset_registry);
        drop(context_lock);
        self
    }

    /// Sets the command-line arguments that [`App::restart`] passes to the new process.
    pub fn with_restart_arguments(self, arguments: Vec<OsString>) -> Self {
        self.0.borrow_mut().restart_arguments = arguments;
        self
    }

    /// Sets the HTTP client that [`App::http_client`] returns.
    ///
    /// The default fails every request. Pass a real client to let the app fetch URLs,
    /// for example for remote images.
    pub fn with_http_client(self, http_client: Arc<dyn HttpClient>) -> Self {
        let mut context_lock = self.0.borrow_mut();
        context_lock.http_client = http_client;
        drop(context_lock);
        self
    }

    /// Sets when the app quits on its own. See [`QuitMode`] for the choices.
    ///
    /// Defaults to [`QuitMode::Default`]. You can also change it at runtime with
    /// [`App::set_quit_mode`].
    pub fn with_quit_mode(self, mode: QuitMode) -> Self {
        self.0.borrow_mut().quit_mode = mode;
        self
    }

    /// Sets the activation policy for the application (macOS only).
    ///
    /// This determines how the application appears in the system:
    /// - `Regular`: Normal app with Dock icon and menu bar
    /// - `Accessory`: Background app without Dock icon (LSUIElement)
    /// - `Prohibited`: Runs in background, no UI allowed
    ///
    /// This must be called before the application finishes launching to take effect.
    /// The method exists only on macOS, so cross-platform code calls it behind
    /// `#[cfg(target_os = "macos")]`.
    #[cfg(target_os = "macos")]
    pub fn with_activation_policy(self, policy: MacActivationPolicy) -> Self {
        self.0.borrow().platform.set_mac_activation_policy(policy);
        self
    }

    /// Starts the event loop and calls `on_finish_launching` once the app has launched.
    ///
    /// The callback runs on the main thread with the [`App`], and runs once. Open your
    /// first window there. On native platforms this method blocks until the app quits,
    /// so code after it runs only after the app exits. On the web it returns right away,
    /// and the app keeps running in the page.
    ///
    /// See the [`Application`] example for a complete `main`.
    pub fn run<F>(self, on_finish_launching: F)
    where
        F: 'static + FnOnce(&mut App),
    {
        #[cfg(all(
            feature = "hot-patching",
            debug_assertions,
            not(target_family = "wasm")
        ))]
        dioxus_devtools::connect_subsecond();

        let this = self.0.clone();
        let platform = self.0.borrow().platform.clone();
        platform.run(Box::new(move || {
            let cx = &mut *this.borrow_mut();
            on_finish_launching(cx);
        }));

        // Native platform run loops block until shutdown, keeping `self` alive for the
        // application's lifetime. The web platform schedules startup and returns
        // immediately, so retain the application until the browser tears down the page.
        #[cfg(target_family = "wasm")]
        std::mem::forget(self);
    }

    /// Start the application for an embedder that drives the run loop itself.
    ///
    /// On ordinary platforms `Platform::run` blocks for the lifetime of the app, and the
    /// app state is kept alive by [`Application::run`]'s stack frame. Embedded platforms —
    /// where the run loop belongs to someone else, e.g. GPUI compiled into a Wasm guest,
    /// or a GPUI view hosted inside a foreign native application — implement
    /// `Platform::run` to invoke the launch callback and return immediately. This method
    /// supports that shape: it returns an [`ApplicationHandle`] that keeps the app alive
    /// and lets the embedder re-enter it whenever the external run loop yields control.
    pub fn run_embedded<F>(self, on_finish_launching: F) -> ApplicationHandle
    where
        F: 'static + FnOnce(&mut App),
    {
        let this = self.0.clone();
        let platform = self.0.borrow().platform.clone();
        platform.run(Box::new(move || {
            let cx = &mut *this.borrow_mut();
            on_finish_launching(cx);
        }));
        ApplicationHandle { app: self.0 }
    }

    /// Registers a handler for URLs that the user asks the app to open.
    ///
    /// The handler receives the URLs as strings. Register a scheme with
    /// [`App::register_url_scheme`] so the platform routes links with that scheme to
    /// the app.
    pub fn on_open_urls<F>(&self, mut callback: F) -> &Self
    where
        F: 'static + FnMut(Vec<String>),
    {
        self.0.borrow().platform.on_open_urls(Box::new(callback));
        self
    }

    /// Registers a handler for a second launch while the app is already running.
    ///
    /// On macOS this happens when the user clicks the app icon in the Dock or in
    /// Finder. A common response is to open a window if none is visible. The handler
    /// receives the [`App`].
    pub fn on_reopen<F>(&self, mut callback: F) -> &Self
    where
        F: 'static + FnMut(&mut App),
    {
        let this = Rc::downgrade(&self.0);
        self.0.borrow_mut().platform.on_reopen(Box::new(move || {
            if let Some(app) = this.upgrade() {
                callback(&mut app.borrow_mut());
            }
        }));
        self
    }

    /// Returns the executor for `Send` futures. See [`BackgroundExecutor`] for platform behavior.
    pub fn background_executor(&self) -> BackgroundExecutor {
        self.0.borrow().background_executor.clone()
    }

    /// Returns the executor for futures that run on the foreground thread.
    pub fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.borrow().foreground_executor.clone()
    }

    /// Returns a shared handle to the app's [`TextSystem`], which shapes and rasterizes text.
    pub fn text_system(&self) -> Arc<TextSystem> {
        self.0.borrow().text_system.clone()
    }

    /// Returns the path to a helper executable named `name` that ships with the app.
    ///
    /// Use this to launch a companion binary, such as a CLI or a helper process. It
    /// returns an error if the platform cannot find the executable.
    pub fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.0.borrow().path_for_auxiliary_executable(name)
    }
}

type Handler = Box<dyn FnMut(&mut App) -> bool + 'static>;
type Listener = Box<dyn FnMut(&dyn Any, &mut App) -> bool + 'static>;
pub(crate) type KeystrokeObserver =
    Box<dyn FnMut(&KeystrokeEvent, &mut Window, &mut App) -> bool + 'static>;
type QuitHandler = Box<dyn FnOnce(&mut App) -> LocalBoxFuture<'static, ()> + 'static>;
type WindowClosedHandler = Box<dyn FnMut(&mut App, WindowId)>;
type ReleaseListener = Box<dyn FnOnce(&mut dyn Any, &mut App) + 'static>;
type NewEntityListener = Box<dyn FnMut(AnyEntity, &mut Option<&mut Window>, &mut App) + 'static>;

/// Controls whether the app quits on its own when its last window closes.
///
/// The check runs each time a window closes. With [`QuitMode::LastWindowClosed`], the
/// app quits as soon as no windows remain. With [`QuitMode::Explicit`], closing the
/// last window leaves the app running until you call [`App::quit`].
///
/// # Examples
///
/// ```
/// # use gpui::QuitMode;
/// assert_eq!(QuitMode::default(), QuitMode::Default);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuitMode {
    /// Behaves like [`QuitMode::LastWindowClosed`] on Linux and Windows, and like
    /// [`QuitMode::Explicit`] on macOS, where closing the last window is normal.
    #[default]
    Default,
    /// Quit automatically when the last window is closed.
    LastWindowClosed,
    /// Quit only when requested via [`App::quit`].
    Explicit,
}

/// Controls when GPUI hides the mouse cursor in response to keyboard input.
///
/// GPUI decides when to hide the cursor. The platform shows it again on mouse motion.
/// Set the mode with [`App::set_cursor_hide_mode`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorHideMode {
    /// Never hide the cursor automatically.
    Never,
    /// Hide on character-producing key presses (typing).
    OnTyping,
    /// Hide on character-producing key presses, *and* when a key binding
    /// resolves to an action that consumes the keystroke.
    #[default]
    OnTypingAndAction,
}

/// A tab in a group of native window tabs. Used by [`SystemWindowTabController`].
#[doc(hidden)]
#[derive(Clone, PartialEq, Eq)]
pub struct SystemWindowTab {
    /// The ID of the window this tab shows.
    pub id: WindowId,
    /// The title shown on the tab.
    pub title: SharedString,
    /// The handle to the window.
    pub handle: AnyWindowHandle,
    /// When the tab was last active. Used to choose which tab in a group to show.
    pub last_active_at: Instant,
}

impl SystemWindowTab {
    /// Creates a tab for a window, with its current time as the last-active time.
    pub fn new(title: SharedString, handle: AnyWindowHandle) -> Self {
        Self {
            id: handle.id,
            title,
            handle,
            last_active_at: Instant::now(),
        }
    }
}

/// A global that tracks which windows are grouped into native window tabs.
///
/// Stored as a [`Global`]. Call [`SystemWindowTabController::init`] once at startup.
/// The methods that take `cx` read and update this global.
#[derive(Default)]
pub struct SystemWindowTabController {
    visible: Option<bool>,
    tab_groups: FxHashMap<usize, Vec<SystemWindowTab>>,
}

impl Global for SystemWindowTabController {}

impl SystemWindowTabController {
    /// Creates an empty controller with no tab groups and tabs hidden.
    pub fn new() -> Self {
        Self {
            visible: None,
            tab_groups: FxHashMap::default(),
        }
    }

    /// Sets the global [`SystemWindowTabController`] to a new, empty controller.
    pub fn init(cx: &mut App) {
        cx.set_global(SystemWindowTabController::new());
    }

    /// Returns every tab group, keyed by group ID.
    pub fn tab_groups(&self) -> &FxHashMap<usize, Vec<SystemWindowTab>> {
        &self.tab_groups
    }

    /// Returns the handle of the window in the tab group that follows the group containing `id`
    /// in the iteration order of `tab_groups`. That order is arbitrary.
    ///
    /// The window chosen is the most recently active tab in that group. Returns `None`
    /// if `id` is not in a group.
    pub fn get_next_tab_group_window(cx: &mut App, id: WindowId) -> Option<&AnyWindowHandle> {
        let controller = cx.global::<SystemWindowTabController>();
        let current_group = controller
            .tab_groups
            .iter()
            .find_map(|(group, tabs)| tabs.iter().find(|tab| tab.id == id).map(|_| group));

        let current_group = current_group?;
        // TODO: `.keys()` returns arbitrary order, what does "next" mean?
        let mut group_ids: Vec<_> = controller.tab_groups.keys().collect();
        let idx = group_ids.iter().position(|g| *g == current_group)?;
        let next_idx = (idx + 1) % group_ids.len();

        controller
            .tab_groups
            .get(group_ids[next_idx])
            .and_then(|tabs| {
                tabs.iter()
                    .max_by_key(|tab| tab.last_active_at)
                    .or_else(|| tabs.first())
                    .map(|tab| &tab.handle)
            })
    }

    /// Returns the handle of the window in the tab group that precedes the group containing `id`
    /// in the iteration order of `tab_groups`. That order is arbitrary.
    ///
    /// The window chosen is the most recently active tab in that group. Returns `None`
    /// if `id` is not in a group.
    pub fn get_prev_tab_group_window(cx: &mut App, id: WindowId) -> Option<&AnyWindowHandle> {
        let controller = cx.global::<SystemWindowTabController>();
        let current_group = controller
            .tab_groups
            .iter()
            .find_map(|(group, tabs)| tabs.iter().find(|tab| tab.id == id).map(|_| group));

        let current_group = current_group?;
        // TODO: `.keys()` returns arbitrary order, what does "previous" mean?
        let mut group_ids: Vec<_> = controller.tab_groups.keys().collect();
        let idx = group_ids.iter().position(|g| *g == current_group)?;
        let prev_idx = if idx == 0 {
            group_ids.len() - 1
        } else {
            idx - 1
        };

        controller
            .tab_groups
            .get(group_ids[prev_idx])
            .and_then(|tabs| {
                tabs.iter()
                    .max_by_key(|tab| tab.last_active_at)
                    .or_else(|| tabs.first())
                    .map(|tab| &tab.handle)
            })
    }

    /// Returns the tabs in the group that contains the window `id`, or `None` if it has none.
    pub fn tabs(&self, id: WindowId) -> Option<&Vec<SystemWindowTab>> {
        self.tab_groups
            .values()
            .find(|tabs| tabs.iter().any(|tab| tab.id == id))
    }

    /// Sets tab visibility to `visible`, but only if it has not been set yet.
    pub fn init_visible(cx: &mut App, visible: bool) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        if controller.visible.is_none() {
            controller.visible = Some(visible);
        }
    }

    /// Returns whether window tabs are shown. Defaults to `false` until set.
    pub fn is_visible(&self) -> bool {
        self.visible.unwrap_or(false)
    }

    /// Shows or hides window tabs.
    pub fn set_visible(cx: &mut App, visible: bool) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        controller.visible = Some(visible);
    }

    /// Records that the window `id` is now the active tab, for ordering tab groups.
    pub fn update_last_active(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for windows in controller.tab_groups.values_mut() {
            for tab in windows.iter_mut() {
                if tab.id == id {
                    tab.last_active_at = Instant::now();
                }
            }
        }
    }

    /// Moves the tab for window `id` to index `ix` within its group.
    ///
    /// Does nothing if `ix` is out of range or the tab is already there.
    pub fn update_tab_position(cx: &mut App, id: WindowId, ix: usize) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for (_, windows) in controller.tab_groups.iter_mut() {
            if let Some(current_pos) = windows.iter().position(|tab| tab.id == id) {
                if ix < windows.len() && current_pos != ix {
                    let window_tab = windows.remove(current_pos);
                    windows.insert(ix, window_tab);
                }
                break;
            }
        }
    }

    /// Sets the title of the tab for window `id`. Does nothing if the title is unchanged.
    pub fn update_tab_title(cx: &mut App, id: WindowId, title: SharedString) {
        let controller = cx.global::<SystemWindowTabController>();
        let tab = controller
            .tab_groups
            .values()
            .flat_map(|windows| windows.iter())
            .find(|tab| tab.id == id);

        if tab.map_or(true, |t| t.title == title) {
            return;
        }

        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for windows in controller.tab_groups.values_mut() {
            for tab in windows.iter_mut() {
                if tab.id == id {
                    tab.title = title;
                    return;
                }
            }
        }
    }

    /// Adds the window `id` to the group that holds the other tabs in `tabs`.
    ///
    /// If no existing group holds exactly the other tabs, `tabs` becomes a new group.
    /// Does nothing if `tabs` does not contain `id`.
    pub fn add_tab(cx: &mut App, id: WindowId, tabs: Vec<SystemWindowTab>) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tab) = tabs.iter().find(|tab| tab.id == id).cloned() else {
            return;
        };

        let mut expected_tab_ids: Vec<_> = tabs
            .iter()
            .filter(|tab| tab.id != id)
            .map(|tab| tab.id)
            .sorted()
            .collect();

        let mut tab_group_id = None;
        for (group_id, group_tabs) in &controller.tab_groups {
            let tab_ids: Vec<_> = group_tabs.iter().map(|tab| tab.id).sorted().collect();
            if tab_ids == expected_tab_ids {
                tab_group_id = Some(*group_id);
                break;
            }
        }

        if let Some(tab_group_id) = tab_group_id {
            if let Some(tabs) = controller.tab_groups.get_mut(&tab_group_id) {
                tabs.push(tab);
            }
        } else {
            let new_group_id = controller.tab_groups.keys().max().map_or(0, |id| id + 1);
            controller.tab_groups.insert(new_group_id, tabs);
        }
    }

    /// Removes the tab for window `id` from its group and returns it.
    ///
    /// A group that becomes empty is removed. Returns `None` if `id` is in no group.
    pub fn remove_tab(cx: &mut App, id: WindowId) -> Option<SystemWindowTab> {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let mut removed_tab = None;

        controller.tab_groups.retain(|_, tabs| {
            if let Some(pos) = tabs.iter().position(|tab| tab.id == id) {
                removed_tab = Some(tabs.remove(pos));
            }
            !tabs.is_empty()
        });

        removed_tab
    }

    /// Moves the tab for window `id` out of its group into a new group of its own.
    pub fn move_tab_to_new_window(cx: &mut App, id: WindowId) {
        let mut removed_tab = Self::remove_tab(cx, id);
        let mut controller = cx.global_mut::<SystemWindowTabController>();

        if let Some(tab) = removed_tab {
            let new_group_id = controller.tab_groups.keys().max().map_or(0, |k| k + 1);
            controller.tab_groups.insert(new_group_id, vec![tab]);
        }
    }

    /// Merges every tab group into one group, starting with the group that holds `id`.
    pub fn merge_all_windows(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(initial_tabs) = controller.tabs(id) else {
            return;
        };

        let initial_tabs_len = initial_tabs.len();
        let mut all_tabs = initial_tabs.clone();

        for (_, mut tabs) in controller.tab_groups.drain() {
            tabs.retain(|tab| !all_tabs[..initial_tabs_len].contains(tab));
            all_tabs.extend(tabs);
        }

        controller.tab_groups.insert(0, all_tabs);
    }

    /// Activates the tab after window `id`'s tab in its group, wrapping around at the end.
    pub fn select_next_tab(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tabs) = controller.tabs(id) else {
            return;
        };

        let current_index = tabs.iter().position(|tab| tab.id == id).unwrap();
        let next_index = (current_index + 1) % tabs.len();

        let _ = &tabs[next_index].handle.update(cx, |_, window, _| {
            window.activate();
        });
    }

    /// Activates the tab before window `id`'s tab in its group, wrapping around at the start.
    pub fn select_previous_tab(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tabs) = controller.tabs(id) else {
            return;
        };

        let current_index = tabs.iter().position(|tab| tab.id == id).unwrap();
        let previous_index = if current_index == 0 {
            tabs.len() - 1
        } else {
            current_index - 1
        };

        let _ = &tabs[previous_index].handle.update(cx, |_, window, _| {
            window.activate();
        });
    }
}

pub(crate) enum GpuiMode {
    #[cfg(any(test, feature = "test-support"))]
    Test {
        skip_drawing: bool,
    },
    Production,
}

impl GpuiMode {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> Self {
        GpuiMode::Test {
            skip_drawing: false,
        }
    }

    #[inline]
    pub(crate) fn skip_drawing(&self) -> bool {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            GpuiMode::Test { skip_drawing } => *skip_drawing,
            GpuiMode::Production => false,
        }
    }
}

struct PlatformOwnedDrag {
    source_window: WindowId,
    state: PlatformOwnedDragState,
}

enum PlatformOwnedDragState {
    Suspended(AnyDrag),
    // A source-window drop consumes `active_drag` before AppKit ends the dragging session, so this
    // marker can outlive the active drag and is cleaned up by `FileDropEvent::Ended`.
    RestoredInSourceWindow,
}

/// The state of a running application: its entities, windows, globals and key bindings.
///
/// You get a `&mut App` from the callback passed to [`Application::run`], from
/// `TestApp::update`, and from any [`Context`], which derefs to `App`. Read an
/// entity with [`Entity::read`], and create one with [`AppContext::new`]. Callbacks that
/// run after an `await` can't hold the `App`, so they use an [`AsyncApp`] instead.
///
/// `App` is not `Clone`. Keep one per app, and use the references you are given. It
/// is not `Send`, so it only runs on the main thread.
///
/// # Examples
///
/// ```
/// # use gpui::{AppContext, TestApp};
/// let mut app = TestApp::new();
/// let count = app.update(|cx| cx.new(|_| 1));
/// app.read(|cx| assert_eq!(*count.read(cx), 1));
/// ```
///
/// See [contexts](crate::_documentation::concepts::contexts) for how `App` relates to the
/// other context types.
pub struct App {
    pub(crate) this: Weak<AppCell>,
    pub(crate) platform: Rc<dyn Platform>,
    text_system: Arc<TextSystem>,

    pub(crate) actions: Rc<ActionRegistry>,
    pub(crate) active_drag: Option<AnyDrag>,
    platform_owned_drag: Option<PlatformOwnedDrag>,
    pub(crate) background_executor: BackgroundExecutor,
    pub(crate) foreground_executor: ForegroundExecutor,
    #[cfg(feature = "profiler")]
    foreground_journal: crate::profiler::journal::ForegroundJournal,
    pub(crate) entities: EntityMap,
    pub(crate) new_entity_observers: SubscriberSet<TypeId, NewEntityListener>,
    pub(crate) windows: SlotMap<WindowId, Option<Box<Window>>>,
    pub(crate) window_handles: FxHashMap<WindowId, AnyWindowHandle>,
    pub(crate) focus_handles: Arc<FocusMap>,
    pub(crate) keymap: Rc<RefCell<Keymap>>,
    pub(crate) keyboard_layout: Box<dyn PlatformKeyboardLayout>,
    pub(crate) keyboard_mapper: Rc<dyn PlatformKeyboardMapper>,
    pub(crate) global_action_listeners:
        TypeIdHashMap<Vec<Rc<dyn Fn(&dyn Any, DispatchPhase, &mut Self)>>>,
    pending_effects: VecDeque<Effect>,

    pub(crate) observers: SubscriberSet<EntityId, Handler>,
    pub(crate) event_listeners: SubscriberSet<EntityId, (TypeId, Listener)>,
    pub(crate) keystroke_observers: SubscriberSet<(), KeystrokeObserver>,
    pub(crate) keystroke_interceptors: SubscriberSet<(), KeystrokeObserver>,
    pub(crate) keyboard_layout_observers: SubscriberSet<(), Handler>,
    pub(crate) thermal_state_observers: SubscriberSet<(), Handler>,
    pub(crate) system_wake_observers: SubscriberSet<(), Handler>,
    pub(crate) release_listeners: SubscriberSet<EntityId, ReleaseListener>,
    pub(crate) global_observers: SubscriberSet<TypeId, Handler>,
    pub(crate) quit_observers: SubscriberSet<(), QuitHandler>,
    pub(crate) restart_observers: SubscriberSet<(), Handler>,
    pub(crate) window_closed_observers: SubscriberSet<(), WindowClosedHandler>,

    /// Per-App element arena. This isolates element allocations between different
    /// App instances (important for tests where multiple Apps run concurrently).
    pub(crate) element_arena: RefCell<Arena>,
    /// Per-App event arena.
    pub(crate) event_arena: Arena,

    // Drop globals last. We need to ensure all tasks owned by entities and
    // callbacks are marked cancelled at this point as this will also shutdown
    // the tokio runtime. As any task attempting to spawn a blocking tokio task,
    // might panic.
    pub(crate) globals_by_type: TypeIdHashMap<Box<dyn Any>>,
    // Entities which are retained until explicitly removed from the app or dropped at end of life-cycle.
    // This is an alternative way to define "global" data which uses entities.
    // There can be multiple entities of a given type & each `Entity<T>` is stored as `Box<dyn Any>`.
    global_entities: Vec<AnyEntity>,

    // assets
    pub(crate) loading_assets: FxHashMap<(TypeId, u64), Box<dyn Any>>,
    asset_registry: Arc<AssetRegistry>,
    pub(crate) svg_renderer: SvgRenderer,
    http_client: Arc<dyn HttpClient>,

    // below is plain data, the drop order is insignificant here
    pub(crate) pending_notifications: FxHashSet<EntityId>,
    pub(crate) pending_global_notifications: TypeIdHashSet,
    pub(crate) restart_path: Option<PathBuf>,
    pub(crate) restart_arguments: Vec<OsString>,
    pub(crate) layout_id_buffer: Vec<LayoutId>, // We recycle this memory across layout requests.
    pub(crate) propagate_event: bool,
    pub(crate) prompt_builder: Option<PromptBuilder>,
    pub(crate) window_invalidators_by_entity:
        FxHashMap<EntityId, FxHashMap<WindowId, WindowInvalidator>>,
    pub(crate) tracked_entities: FxHashMap<WindowId, FxHashSet<EntityId>>,
    pub(crate) current_window_by_entity: FxHashMap<EntityId, WindowId>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) inspector_renderer: Option<crate::InspectorRenderer>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) inspector_element_registry: InspectorElementRegistry,
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    pub(crate) name: Option<&'static str>,
    pub(crate) text_rendering_mode: Rc<Cell<TextRenderingMode>>,

    pub(crate) window_update_stack: Vec<WindowId>,
    pub(crate) mode: GpuiMode,
    pub(crate) cursor_hide_mode: CursorHideMode,
    pub(crate) reduce_motion: bool,
    /// Origin of the shared clock that phase-locks synced repeating animations.
    pub(crate) synced_animation_epoch: Instant,
    /// Whether the app was created by [`Application::new_inaccessible`]. No
    /// accesskit APIs will be called when this flag is set.
    pub(crate) accessibility_force_disabled: bool,
    /// Whether windows build their accessibility tree every frame without an
    /// assistive-technology client. See [`Application::with_accessibility_forced`].
    pub(crate) accessibility_forced: bool,
    flushing_effects: bool,
    pending_updates: usize,
    quit_mode: QuitMode,
    quitting: bool,

    // We need to ensure the leak detector drops last, after all tasks, callbacks and things have been dropped.
    // Otherwise it may report false positives.
    #[cfg(any(test, feature = "leak-detection"))]
    _ref_counts: Arc<RwLock<EntityRefCounts>>,
}

impl App {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new_app(
        platform: Rc<dyn Platform>,
        asset_registry: Arc<AssetRegistry>,
        http_client: Arc<dyn HttpClient>,
    ) -> Rc<AppCell> {
        let background_executor = platform.background_executor();
        let foreground_executor = platform.foreground_executor();
        assert!(
            background_executor.is_main_thread(),
            "must construct App on main thread"
        );
        #[cfg(feature = "profiler")]
        let foreground_journal = crate::profiler::journal::install_foreground_journal();
        let synced_animation_epoch = background_executor.now();

        let text_system = Arc::new(TextSystem::new(platform.text_system()));
        let entities = EntityMap::new();
        let keyboard_layout = platform.keyboard_layout();
        let keyboard_mapper = platform.keyboard_mapper();

        #[cfg(any(test, feature = "leak-detection"))]
        let _ref_counts = entities.ref_counts_drop_handle();

        let app = Rc::new_cyclic(|this| AppCell {
            app: RefCell::new(App {
                this: this.clone(),
                platform: platform.clone(),
                text_system,
                text_rendering_mode: Rc::new(Cell::new(TextRenderingMode::default())),
                mode: GpuiMode::Production,
                actions: Rc::new(ActionRegistry::default()),
                flushing_effects: false,
                pending_updates: 0,
                active_drag: None,
                platform_owned_drag: None,
                background_executor,
                foreground_executor,
                #[cfg(feature = "profiler")]
                foreground_journal,
                svg_renderer: SvgRenderer::new(asset_registry.clone()),
                loading_assets: Default::default(),
                asset_registry,
                http_client,
                globals_by_type: Default::default(),
                global_entities: Default::default(),
                entities,
                new_entity_observers: SubscriberSet::new(),
                windows: SlotMap::with_key(),
                window_update_stack: Vec::new(),
                window_handles: FxHashMap::default(),
                focus_handles: Arc::new(RwLock::new(SlotMap::with_key())),
                keymap: Rc::new(RefCell::new(Keymap::default())),
                keyboard_layout,
                keyboard_mapper,
                global_action_listeners: Default::default(),
                pending_effects: VecDeque::new(),
                pending_notifications: FxHashSet::default(),
                pending_global_notifications: Default::default(),
                observers: SubscriberSet::new(),
                tracked_entities: FxHashMap::default(),
                window_invalidators_by_entity: FxHashMap::default(),
                current_window_by_entity: FxHashMap::default(),
                event_listeners: SubscriberSet::new(),
                release_listeners: SubscriberSet::new(),
                keystroke_observers: SubscriberSet::new(),
                keystroke_interceptors: SubscriberSet::new(),
                keyboard_layout_observers: SubscriberSet::new(),
                thermal_state_observers: SubscriberSet::new(),
                system_wake_observers: SubscriberSet::new(),
                global_observers: SubscriberSet::new(),
                quit_observers: SubscriberSet::new(),
                restart_observers: SubscriberSet::new(),
                restart_path: None,
                restart_arguments: Vec::new(),
                window_closed_observers: SubscriberSet::new(),
                layout_id_buffer: Default::default(),
                propagate_event: true,
                prompt_builder: Some(PromptBuilder::Default),
                #[cfg(any(feature = "inspector", debug_assertions))]
                inspector_renderer: None,
                #[cfg(any(feature = "inspector", debug_assertions))]
                inspector_element_registry: InspectorElementRegistry::default(),
                quit_mode: QuitMode::default(),
                quitting: false,
                cursor_hide_mode: CursorHideMode::default(),
                reduce_motion: false,
                synced_animation_epoch,
                accessibility_force_disabled: false,
                accessibility_forced: false,

                #[cfg(any(test, feature = "test-support", debug_assertions))]
                name: None,
                element_arena: RefCell::new(Arena::new(1024 * 1024)),
                event_arena: Arena::new(1024 * 1024),

                #[cfg(any(test, feature = "leak-detection"))]
                _ref_counts,
            }),
        });

        init_app_menus(platform.as_ref(), &app.borrow());
        SystemWindowTabController::init(&mut app.borrow_mut());

        platform.on_keyboard_layout_change(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.keyboard_layout = cx.platform.keyboard_layout();
                    cx.keyboard_mapper = cx.platform.keyboard_mapper();
                    cx.keyboard_layout_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_thermal_state_change(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.thermal_state_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_system_wake(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.system_wake_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_quit(Box::new({
            let cx = Rc::downgrade(&app);
            move || {
                let Some(cx) = cx.upgrade() else {
                    return true;
                };
                match cx.try_borrow_mut() {
                    Ok(mut cx) => {
                        cx.shutdown();
                        true
                    }
                    Err(_) => {
                        // Quit was requested while the AppCell was borrowed, so we can't shut down synchronously.
                        // The platform decides how to proceed.
                        false
                    }
                }
            }
        }));

        app
    }

    #[doc(hidden)]
    pub fn ref_counts_drop_handle(&self) -> impl Sized + use<> {
        self.entities.ref_counts_drop_handle()
    }

    /// Captures a snapshot of all entities that currently have alive handles.
    ///
    /// The returned [`LeakDetectorSnapshot`] can later be passed to
    /// [`assert_no_new_leaks`](Self::assert_no_new_leaks) to verify that no
    /// entities created after the snapshot are still alive.
    #[cfg(any(test, feature = "leak-detection"))]
    pub fn leak_detector_snapshot(&self) -> LeakDetectorSnapshot {
        self.entities.leak_detector_snapshot()
    }

    /// Asserts that no entities created after `snapshot` still have alive handles.
    ///
    /// Entities that were already tracked at the time of the snapshot are ignored,
    /// even if they still have handles. Only *new* entities (those whose
    /// `EntityId` was not present in the snapshot) are considered leaks.
    ///
    /// # Panics
    ///
    /// Panics if any new entity handles exist. The panic message lists every
    /// leaked entity with its type name, and includes allocation-site backtraces
    /// when `LEAK_BACKTRACE` is set.
    #[cfg(any(test, feature = "leak-detection"))]
    pub fn assert_no_new_leaks(&self, snapshot: &LeakDetectorSnapshot) {
        self.entities.assert_no_new_leaks(snapshot)
    }

    /// Runs the quit handlers, closes every window and flushes pending effects.
    ///
    /// The platform calls this when the app quits. Handlers registered with
    /// [`Context::on_app_quit`] run first. Native apps wait up to [`SHUTDOWN_TIMEOUT`]
    /// for them to finish and log an error if they don't. On the web the handlers run
    /// asynchronously as best-effort cleanup, because the event loop cannot block.
    ///
    /// To quit from your own code, call [`App::quit`] instead.
    pub fn shutdown(&mut self) {
        let mut futures = Vec::new();

        for observer in self.quit_observers.remove(&()) {
            futures.push(observer(self));
        }

        self.windows.clear();
        self.window_handles.clear();
        self.flush_effects();
        self.quitting = true;

        let futures = futures::future::join_all(futures);
        #[cfg(not(target_family = "wasm"))]
        if self
            .foreground_executor
            .block_with_timeout(SHUTDOWN_TIMEOUT, futures)
            .is_err()
        {
            log::error!("timed out waiting on app_will_quit");
        }
        #[cfg(target_family = "wasm")]
        self.foreground_executor.spawn(futures).detach();

        self.quitting = false;
    }

    /// Returns the keyboard layout that is active now.
    ///
    /// Use it to translate between keystrokes and the characters they produce. The
    /// value is updated when the layout changes, which [`App::on_keyboard_layout_change`]
    /// reports.
    pub fn keyboard_layout(&self) -> &dyn PlatformKeyboardLayout {
        self.keyboard_layout.as_ref()
    }

    /// Returns the keyboard mapper for the active layout.
    pub fn keyboard_mapper(&self) -> &Rc<dyn PlatformKeyboardMapper> {
        &self.keyboard_mapper
    }

    /// Calls `callback` each time the user switches the keyboard layout.
    ///
    /// The subscription stays active while you hold it. Drop it to stop receiving
    /// calls. By the time the callback runs, [`App::keyboard_layout`] already returns
    /// the new layout.
    pub fn on_keyboard_layout_change<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.keyboard_layout_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Asks the platform to quit the app, the same as choosing Quit from the app menu.
    ///
    /// The quit happens later, not inside this call. The platform then calls
    /// [`App::shutdown`], which runs the [`Context::on_app_quit`] handlers and closes
    /// the windows. Use this to quit from your own code. [`QuitMode`] controls quits
    /// that happen when the last window closes.
    pub fn quit(&self) {
        self.platform.quit();
    }

    /// Returns the policy that decides when GPUI hides the cursor while typing.
    ///
    /// Defaults to [`CursorHideMode::OnTypingAndAction`].
    pub fn cursor_hide_mode(&self) -> CursorHideMode {
        self.cursor_hide_mode
    }

    /// Sets the policy that decides when GPUI hides the cursor while typing.
    ///
    /// Takes effect for the next keystroke. The cursor reappears when the mouse moves.
    pub fn set_cursor_hide_mode(&mut self, mode: CursorHideMode) {
        self.cursor_hide_mode = mode;
    }

    /// Returns whether the cursor is currently visible according to the
    /// platform. This will report `false` after a keyboard input has hidden
    /// the cursor and the user has not yet moved the mouse to restore it.
    ///
    /// See [`App::set_cursor_hide_mode`].
    pub fn is_cursor_visible(&self) -> bool {
        self.platform.is_cursor_visible()
    }

    /// Returns whether non-essential animations should render in a static state.
    ///
    /// Check this before you start a decorative animation, such as a spinner that only
    /// looks busy. The app sets it, and it defaults to `false`.
    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// Sets whether non-essential animations should render in a static state.
    ///
    /// Setting a different value redraws all windows. Animations that are needed to
    /// show state, such as a progress bar, should keep running.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        if self.reduce_motion != reduce_motion {
            self.reduce_motion = reduce_motion;
            self.refresh_windows();
        }
    }

    /// Redraws every window once the current update finishes.
    ///
    /// Calling it several times in one update still causes a single redraw per window.
    /// Use [`Context::notify`] on the entity that changed instead when you can. Use this
    /// when state outside any entity changes what is drawn.
    pub fn refresh_windows(&mut self) {
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    pub(crate) fn update<R>(&mut self, update: impl FnOnce(&mut Self) -> R) -> R {
        self.start_update();
        let result = update(self);
        self.finish_update();
        result
    }

    pub(crate) fn start_update(&mut self) {
        self.pending_updates += 1;
    }

    pub(crate) fn finish_update(&mut self) {
        if !self.flushing_effects && self.pending_updates == 1 {
            self.flushing_effects = true;
            self.flush_effects();
            self.flushing_effects = false;
        }
        self.pending_updates -= 1;
    }

    /// Calls `on_notify` each time `entity` calls [`Context::notify`].
    ///
    /// The callback receives the entity and the [`App`]. It runs when the update that
    /// called `notify` finishes, not during that call. Keep the returned [`Subscription`]
    /// alive, because dropping it ends the observation.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// # use std::{cell::Cell, rc::Rc};
    /// # let mut app = TestApp::new();
    /// let seen = Rc::new(Cell::new(0));
    /// let counter = app.update(|cx| cx.new(|_| 0));
    /// let _subscription = app.update(|cx| {
    ///     cx.observe(&counter, {
    ///         let seen = seen.clone();
    ///         move |counter, cx| seen.set(*counter.read(cx))
    ///     })
    /// });
    /// app.update(|cx| {
    ///     counter.update(cx, |count, cx| {
    ///         *count = 2;
    ///         cx.notify();
    ///     })
    /// });
    /// assert_eq!(seen.get(), 2);
    /// ```
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership).
    pub fn observe<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(Entity<W>, &mut App) + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        self.observe_internal(entity, move |e, cx| {
            on_notify(e, cx);
            true
        })
    }

    pub(crate) fn detect_accessed_entities<R>(
        &mut self,
        callback: impl FnOnce(&mut App) -> R,
    ) -> (R, FxHashSet<EntityId>) {
        let accessed_entities_start = self.entities.accessed_entities.get_mut().clone();
        let result = callback(self);
        let entities_accessed_in_callback = self
            .entities
            .accessed_entities
            .get_mut()
            .difference(&accessed_entities_start)
            .copied()
            .collect::<FxHashSet<EntityId>>();
        (result, entities_accessed_in_callback)
    }

    pub(crate) fn record_entities_accessed(
        &mut self,
        window_handle: AnyWindowHandle,
        invalidator: WindowInvalidator,
        entities: &FxHashSet<EntityId>,
    ) {
        let mut tracked_entities =
            std::mem::take(self.tracked_entities.entry(window_handle.id).or_default());
        for entity in tracked_entities.iter() {
            self.window_invalidators_by_entity
                .entry(*entity)
                .and_modify(|windows| {
                    windows.remove(&window_handle.id);
                });
        }
        for entity in entities.iter() {
            self.window_invalidators_by_entity
                .entry(*entity)
                .or_default()
                .insert(window_handle.id, invalidator.clone());
            self.current_window_by_entity
                .insert(*entity, window_handle.id);
        }
        tracked_entities.clear();
        tracked_entities.extend(entities.iter().copied());
        self.tracked_entities
            .insert(window_handle.id, tracked_entities);
    }

    pub(crate) fn new_observer(&mut self, key: EntityId, value: Handler) -> Subscription {
        let (subscription, activate) = self.observers.insert(key, value);
        self.defer(move |_| activate());
        subscription
    }

    pub(crate) fn observe_internal<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(Entity<W>, &mut App) -> bool + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        let entity_id = entity.entity_id();
        let handle = entity.downgrade();
        self.new_observer(
            entity_id,
            Box::new(move |cx| {
                if let Some(entity) = handle.upgrade() {
                    on_notify(entity, cx)
                } else {
                    false
                }
            }),
        )
    }

    /// Calls `on_event` each time `entity` emits an event of type `Event`.
    ///
    /// The callback receives the entity, a reference to the event and the [`App`]. The
    /// entity emits with [`Context::emit`]. Callbacks run when the update that emitted
    /// the event finishes. Keep the returned [`Subscription`] alive, because dropping it
    /// ends the subscription. Use [`App::observe`] when the entity only calls
    /// [`Context::notify`] and doesn't send a payload.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, EventEmitter, TestApp};
    /// # use std::{cell::Cell, rc::Rc};
    /// struct Counter;
    /// struct Incremented(usize);
    /// impl EventEmitter<Incremented> for Counter {}
    ///
    /// # let mut app = TestApp::new();
    /// let counter = app.update(|cx| cx.new(|_| Counter));
    /// let total = Rc::new(Cell::new(0));
    /// let _subscription = app.update(|cx| {
    ///     cx.subscribe(&counter, {
    ///         let total = total.clone();
    ///         move |_, event: &Incremented, _| total.set(total.get() + event.0)
    ///     })
    /// });
    /// app.update(|cx| counter.update(cx, |_, cx| cx.emit(Incremented(3))));
    /// assert_eq!(total.get(), 3);
    /// ```
    pub fn subscribe<T, Event>(
        &mut self,
        entity: &Entity<T>,
        mut on_event: impl FnMut(Entity<T>, &Event, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Event>,
        Event: 'static,
    {
        self.subscribe_internal(entity, move |entity, event, cx| {
            on_event(entity, event, cx);
            true
        })
    }

    pub(crate) fn new_subscription(
        &mut self,
        key: EntityId,
        value: (TypeId, Listener),
    ) -> Subscription {
        let (subscription, activate) = self.event_listeners.insert(key, value);
        self.defer(move |_| activate());
        subscription
    }
    pub(crate) fn subscribe_internal<T, Evt>(
        &mut self,
        entity: &Entity<T>,
        mut on_event: impl FnMut(Entity<T>, &Evt, &mut App) -> bool + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let entity_id = entity.entity_id();
        let handle = entity.downgrade();
        self.new_subscription(
            entity_id,
            (
                TypeId::of::<Evt>(),
                Box::new(move |event, cx| {
                    let event: &Evt = event.downcast_ref().expect("invalid event type");
                    if let Some(entity) = handle.upgrade() {
                        on_event(entity, event, cx)
                    } else {
                        false
                    }
                }),
            ),
        )
    }

    /// Returns handles to every open window.
    ///
    /// Each handle is untyped. [`AnyWindowHandle::downcast`] checks the root type recorded in
    /// the handle. If the root may have been replaced, use [`AnyWindowHandle::update`] to
    /// inspect the live root; the recorded type does not change with it.
    pub fn windows(&self) -> Vec<AnyWindowHandle> {
        self.windows
            .keys()
            .flat_map(|window_id| self.window_handles.get(&window_id).copied())
            .collect()
    }

    /// Returns the window handles ordered by their appearance on screen, front to back.
    ///
    /// The first window in the returned list is the active/topmost window of the application.
    ///
    /// This method returns None if the platform doesn't implement the method yet.
    pub fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        self.platform.window_stack()
    }

    /// Sets the GPU device requirements (extra features and limits) for the renderer.
    /// Only macOS uses them.
    ///
    /// Call this before opening any window. The argument must be a
    /// `gpui_wgpu::WgpuDeviceRequirements` boxed as `Box<dyn Any>`.
    pub fn set_gpu_requirements(&self, requirements: Box<dyn std::any::Any>) {
        self.platform.set_gpu_requirements(requirements);
    }

    /// Sets the label applied to credentials stored in the system keyring.
    /// Call before writing credentials. Only Linux/FreeBSD apply the label.
    pub fn set_keyring_label(&self, label: impl Into<SharedString>) {
        self.platform.set_keyring_label(label.into());
    }

    /// Returns the handle of the window that the platform reports as active, if any.
    ///
    /// Returns `None` when no window of this app has focus, for example when another
    /// application is in front.
    pub fn active_window(&self) -> Option<AnyWindowHandle> {
        self.platform.active_window()
    }

    /// Opens a window and builds its root view.
    ///
    /// `build_root_view` runs once, with the new [`Window`] and the [`App`], and returns
    /// the root entity. The window is drawn once before this method returns. Use the
    /// returned [`WindowHandle`] to update or read the root view later.
    ///
    /// In tests, `TestApp::open_window` opens a window without a display.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform cannot create the window. In that case
    /// `build_root_view` is not called.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Context, Render, TestApp, Window, WindowOptions, div, prelude::*};
    /// struct Hello;
    ///
    /// impl Render for Hello {
    ///     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    ///         div().child("Hello")
    ///     }
    /// }
    ///
    /// # let mut app = TestApp::new();
    /// let window = app.update(|cx| {
    ///     cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Hello))
    ///         .unwrap()
    /// });
    /// app.read(|cx| assert_eq!(cx.windows().len(), 1));
    /// # let _ = window;
    /// ```
    pub fn open_window<V: 'static + Render>(
        &mut self,
        options: crate::WindowOptions,
        build_root_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> anyhow::Result<WindowHandle<V>> {
        self.update(|cx| {
            let id = cx.windows.insert(None);
            let handle = WindowHandle::new(id);
            match Window::new(handle.into(), options, cx) {
                Ok(mut window) => {
                    cx.window_update_stack.push(id);
                    let root_view = build_root_view(&mut window, cx);
                    cx.window_update_stack.pop();
                    window.root.replace(root_view.into());
                    window.defer(cx, |window: &mut Window, cx| window.appearance_changed(cx));

                    // allow a window to draw at least once before returning
                    // this didn't cause any issues on non windows platforms as it seems we always won the race to on_request_frame
                    // on windows we quite frequently lose the race and return a window that has never rendered, which leads to a crash
                    // where DispatchTree::root_node_id asserts on empty nodes
                    let clear = window.draw(cx);
                    clear.clear(cx);

                    cx.window_handles.insert(id, window.handle);
                    cx.windows.get_mut(id).unwrap().replace(Box::new(window));
                    Ok(handle)
                }
                Err(e) => {
                    cx.windows.remove(id);
                    Err(e)
                }
            }
        })
    }

    /// Brings the app to the front.
    ///
    /// Pass `ignoring_other_apps: true` to activate the app even when another app is in front.
    /// Use it sparingly, because it moves the user's focus.
    pub fn activate(&self, ignoring_other_apps: bool) {
        self.platform.activate(ignoring_other_apps);
    }

    /// Hides the app, the same as choosing Hide from the app menu.
    pub fn hide(&self) {
        self.platform.hide();
    }

    /// Hides all other applications.
    pub fn hide_other_apps(&self) {
        self.platform.hide_other_apps();
    }

    /// Shows the applications hidden by [`App::hide_other_apps`].
    pub fn unhide_other_apps(&self) {
        self.platform.unhide_other_apps();
    }

    /// Returns the connected displays.
    pub fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.platform.displays()
    }

    /// Returns the primary display, or `None` if the platform reports none.
    pub fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.platform.primary_display()
    }

    /// Returns whether [`App::screen_capture_sources`] can return sources on this platform.
    pub fn is_screen_capture_supported(&self) -> bool {
        self.platform.is_screen_capture_supported()
    }

    /// Asks the platform for the screens and windows that can be captured.
    ///
    /// The list arrives on the returned channel, so the call doesn't block. The channel
    /// yields an error if the platform can't list sources. Check
    /// [`App::is_screen_capture_supported`] first.
    pub fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        self.platform.screen_capture_sources()
    }

    /// Returns the display with the given [`DisplayId`], or `None` if it is not connected.
    pub fn find_display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        self.displays()
            .iter()
            .find(|display| display.id() == id)
            .cloned()
    }

    /// Returns the system's current [`ThermalState`].
    ///
    /// Check it before heavy work, such as a long animation or a background
    /// computation, and reduce the work when the system is hot.
    pub fn thermal_state(&self) -> ThermalState {
        self.platform.thermal_state()
    }

    /// Calls `callback` when the system's thermal state changes.
    ///
    /// The subscription stays active while you hold it.
    pub fn on_thermal_state_change<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.thermal_state_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Calls `callback` when the system wakes from sleep.
    ///
    /// Use it to reconnect to network services or refresh data that went stale
    /// while the machine slept. The subscription stays active while you hold it.
    pub fn on_system_wake<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.system_wake_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Returns whether the windows use the light or dark appearance.
    ///
    /// This is the system setting unless [`App::set_window_appearance`] overrides it.
    pub fn window_appearance(&self) -> WindowAppearance {
        self.platform.window_appearance()
    }

    /// Overrides the appearance (light/dark) applied to the app's windows, independent of
    /// the OS-wide setting. Pass `None` to clear the override and follow the system again.
    /// The current value is reported by [`App::window_appearance`].
    ///
    /// On macOS this sets the underlying `NSApplication.appearance`, which controls the
    /// native window chrome (the window border and titlebar) of every window. Use this
    /// when the app uses a dark theme while the system is in light mode (or vice versa)
    /// so the window edges render to match the theme. While an appearance is forced,
    /// windows stop tracking system light/dark changes; pass `None` to resume following
    /// the system. On other platforms this is a no-op.
    pub fn set_window_appearance(&self, appearance: Option<WindowAppearance>) {
        self.platform.set_window_appearance(appearance);
    }

    /// Returns whether [`App::play_haptic_feedback`] does anything on this platform.
    pub fn supports_haptic_feedback(&self) -> bool {
        self.platform.supports_haptic_feedback()
    }

    /// Plays a haptic feedback pulse of the given style.
    ///
    /// Call it from the main thread, for example in response to a click or a drop that
    /// the user should feel. It does nothing on platforms without haptics. Check
    /// [`App::supports_haptic_feedback`] to decide whether to show a haptic option.
    /// Styles correspond to
    /// [`NSHapticFeedbackPattern`](https://developer.apple.com/documentation/appkit/nshapticfeedbackmanager/feedbackpattern)
    /// values on macOS.
    pub fn play_haptic_feedback(&self, style: HapticFeedbackStyle) {
        self.platform.play_haptic_feedback(style)
    }

    /// Returns the window button layout (the position of close, minimize and maximize), if the platform reports one.
    ///
    /// Returns `None` when the platform has no custom layout. Use it to draw your own
    /// window controls in the same place as the native ones.
    pub fn button_layout(&self) -> Option<WindowButtonLayout> {
        self.platform.button_layout()
    }

    /// Reads the current contents of the system clipboard.
    ///
    /// Returns `None` if the clipboard is empty or holds nothing GPUI can read. On
    /// the web, use [`App::read_from_clipboard_async`] instead, because this method
    /// cannot wait for permission.
    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_clipboard()
    }

    /// Reads the system clipboard and resolves once the contents are available.
    ///
    /// Prefer this over [`App::read_from_clipboard`] in code that can await. On
    /// platforms where clipboard access is asynchronous and permission-gated (such as
    /// the web), the synchronous read always returns `None` while this method performs
    /// a real read. The task resolves to an error if the read fails.
    pub fn read_from_clipboard_async(
        &self,
    ) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        self.platform.read_from_clipboard_async()
    }

    /// Sets how text is rasterized for every window in the app.
    ///
    /// The default is [`TextRenderingMode::PlatformDefault`]. The new mode applies to
    /// text drawn after this call.
    pub fn set_text_rendering_mode(&mut self, mode: TextRenderingMode) {
        self.text_rendering_mode.set(mode);
    }

    /// Returns how text is currently rasterized. See [`App::set_text_rendering_mode`].
    pub fn text_rendering_mode(&self) -> TextRenderingMode {
        self.text_rendering_mode.get()
    }

    /// Replaces the contents of the system clipboard with `item`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, ClipboardItem, TestApp};
    /// # let mut app = TestApp::new();
    /// app.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("copied".into())));
    /// app.read(|cx| {
    ///     let item = cx.read_from_clipboard().unwrap();
    ///     assert_eq!(item.text().as_deref(), Some("copied"));
    /// });
    /// ```
    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.platform.write_to_clipboard(item)
    }

    /// Reads data from the primary selection buffer.
    /// Only available on Linux and FreeBSD.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.platform.read_from_primary()
    }

    /// Writes data to the primary selection buffer.
    /// Only available on Linux and FreeBSD.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn write_to_primary(&self, item: ClipboardItem) {
        self.platform.write_to_primary(item)
    }

    /// Reads data from macOS's "Find" pasteboard.
    ///
    /// Used to share the current search string between apps.
    ///
    /// <https://developer.apple.com/documentation/appkit/nspasteboard/name-swift.struct/find>
    #[cfg(target_os = "macos")]
    pub fn read_from_find_pasteboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_find_pasteboard()
    }

    /// Writes data to macOS's "Find" pasteboard.
    ///
    /// Used to share the current search string between apps.
    ///
    /// <https://developer.apple.com/documentation/appkit/nspasteboard/name-swift.struct/find>
    #[cfg(target_os = "macos")]
    pub fn write_to_find_pasteboard(&self, item: ClipboardItem) {
        self.platform.write_to_find_pasteboard(item)
    }

    /// Stores a username and password for `url` in the platform keychain.
    ///
    /// The task resolves once the store finishes, or to an error if it fails. Await it
    /// from a task spawned with [`App::spawn`].
    pub fn write_credentials(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> Task<Result<()>> {
        self.platform.write_credentials(url, username, password)
    }

    /// Reads the username and password stored for `url`.
    ///
    /// The task resolves to `None` when nothing is stored for `url`. Use the same `url`
    /// that you passed to [`App::write_credentials`].
    pub fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        self.platform.read_credentials(url)
    }

    /// Removes the credentials stored for `url` from the platform keychain.
    pub fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        self.platform.delete_credentials(url)
    }

    /// Opens `url` in the platform's default browser or handler.
    ///
    /// The call returns immediately. In tests, `TestApp::opened_url` reports the last
    /// URL passed here.
    pub fn open_url(&self, url: &str) {
        self.platform.open_url(url);
    }

    /// Registers `scheme` (for example `zed` for `zed://` links) so that links with it open this app.
    ///
    /// Use [`Application::on_open_urls`] to receive the links. On some platforms you
    /// declare schemes when you package the app. This method registers them at runtime.
    pub fn register_url_scheme(&self, scheme: &str) -> Task<Result<()>> {
        self.platform.register_url_scheme(scheme)
    }

    /// Sets the app's identifier and user-visible name for the operating system.
    ///
    /// The identifier is used for platform identity mechanisms such as the Windows
    /// AppUserModelID. The name is shown wherever the operating system presents the app.
    pub fn set_app_identity(&self, identifier: &str, name: &str) {
        self.platform.set_app_identity(identifier, name);
    }

    /// Posts a notification to the operating system's notification center.
    ///
    /// Posting a notification whose [`SystemNotification::tag`] matches an
    /// earlier one replaces that notification where the platform supports it.
    /// No-op on platforms without notification support, or when delivery is
    /// unavailable (e.g. authorization was denied).
    pub fn show_system_notification(&self, notification: SystemNotification) {
        self.platform.show_system_notification(notification);
    }

    /// Removes the delivered or pending notification with this tag.
    ///
    /// Best-effort: some platforms cannot retract a notification once shown,
    /// in which case it ages out of the notification center on its own.
    pub fn dismiss_system_notification(&self, tag: &str) {
        self.platform.dismiss_system_notification(tag);
    }

    /// Registers the handler invoked when the user activates a system
    /// notification, either by clicking its body or one of its action
    /// buttons. Subsequent registrations replace the handler.
    pub fn on_system_notification_response<F>(&self, mut callback: F)
    where
        F: 'static + FnMut(SystemNotificationResponse, &mut App),
    {
        let this = self.this.clone();
        self.platform
            .on_system_notification_response(Box::new(move |response| {
                if let Some(app) = this.upgrade() {
                    callback(response, &mut app.borrow_mut());
                }
            }));
    }

    /// Returns the full pathname of the current app bundle.
    ///
    /// Returns an error if the app is not being run from a bundle.
    pub fn app_path(&self) -> Result<PathBuf> {
        self.platform.app_path()
    }

    /// On Linux, returns the name of the compositor in use. On the web, returns `"Web"`.
    ///
    /// Returns an empty string on other platforms.
    pub fn compositor_name(&self) -> &'static str {
        self.platform.compositor_name()
    }

    /// Returns the path to a helper executable named `name` that ships with the app.
    ///
    /// See [`Application::path_for_auxiliary_executable`].
    pub fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.platform.path_for_auxiliary_executable(name)
    }

    /// Shows the platform's file or folder picker and returns the user's choice.
    ///
    /// The result arrives on the returned channel once the user picks paths or cancels.
    /// Cancelling yields `Ok(None)`. The picker is shown without blocking the caller.
    /// On Linux the channel yields an error if the picker can't be opened.
    pub fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        self.platform.prompt_for_paths(options)
    }

    /// Shows the platform's save dialog and returns the path the user chose.
    ///
    /// `directory` is the folder the dialog opens in, and `suggested_name` pre-fills the
    /// file name. The result arrives on the returned channel. Cancelling yields
    /// `Ok(None)`. On Linux the channel yields an error if the dialog can't be opened.
    pub fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        self.platform.prompt_for_new_path(directory, suggested_name)
    }

    /// Shows `path` in the platform's file manager, such as Finder on macOS.
    pub fn reveal_path(&self, path: &Path) {
        self.platform.reveal_path(path)
    }

    /// Opens `path` with the system's default application for its file type.
    pub fn open_with_system(&self, path: &Path) {
        self.platform.open_with_system(path)
    }

    /// Returns whether the user has configured scrollbars to auto-hide at the platform level.
    pub fn should_auto_hide_scrollbars(&self) -> bool {
        self.platform.should_auto_hide_scrollbars()
    }

    /// Restarts the app. Runs the handlers from [`App::on_app_restart`], then asks the
    /// platform to relaunch it.
    ///
    /// The relaunch uses the path from [`App::set_restart_path`] and the arguments from
    /// [`Application::with_restart_arguments`].
    pub fn restart(&mut self) {
        self.restart_observers
            .clone()
            .retain(&(), |observer| observer(self));
        self.platform.restart(
            self.restart_path.take(),
            std::mem::take(&mut self.restart_arguments),
        )
    }

    /// Sets the executable path that [`App::restart`] launches. The path is used once, by the next restart.
    pub fn set_restart_path(&mut self, path: PathBuf) {
        self.restart_path = Some(path);
    }

    /// Returns the app's HTTP client. See [`Application::with_http_client`].
    pub fn http_client(&self) -> Arc<dyn HttpClient> {
        self.http_client.clone()
    }

    /// Replaces the HTTP client that [`App::http_client`] returns.
    pub fn set_http_client(&mut self, new_client: Arc<dyn HttpClient>) {
        self.http_client = new_client;
    }

    /// Sets when the app quits on its own. See [`QuitMode`].
    ///
    /// Defaults to [`QuitMode::Default`].
    pub fn set_quit_mode(&mut self, mode: QuitMode) {
        self.quit_mode = mode;
    }

    /// Returns the SVG renderer that draws the app's SVG assets.
    pub fn svg_renderer(&self) -> SvgRenderer {
        self.svg_renderer.clone()
    }

    pub(crate) fn push_effect(&mut self, effect: Effect) {
        match &effect {
            Effect::Notify { emitter } => {
                if !self.pending_notifications.insert(*emitter) {
                    return;
                }
            }
            Effect::NotifyGlobalObservers { global_type } => {
                if !self.pending_global_notifications.insert(*global_type) {
                    return;
                }
            }
            _ => {}
        };

        self.pending_effects.push_back(effect);
    }

    /// Called at the end of [`App::update`] to complete any side effects
    /// such as notifying observers, emitting events, etc. Effects can themselves
    /// cause effects, so we continue looping until all effects are processed.
    fn flush_effects(&mut self) {
        loop {
            self.release_dropped_entities();
            self.release_dropped_focus_handles();
            if let Some(effect) = self.pending_effects.pop_front() {
                match effect {
                    Effect::Notify { emitter } => {
                        self.apply_notify_effect(emitter);
                    }

                    Effect::Emit {
                        emitter,
                        event_type,
                        event,
                    } => self.apply_emit_effect(emitter, event_type, &*event),

                    Effect::RefreshWindows => {
                        self.apply_refresh_effect();
                    }

                    Effect::NotifyGlobalObservers { global_type } => {
                        self.apply_notify_global_observers_effect(global_type);
                    }

                    Effect::Defer { callback } => {
                        self.apply_defer_effect(callback);
                    }
                    Effect::EntityCreated {
                        entity,
                        tid,
                        window,
                    } => {
                        self.apply_entity_created_effect(entity, tid, window);
                    }
                }
            } else {
                #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
                for window in self
                    .windows
                    .values()
                    .filter_map(|window| {
                        let window = window.as_deref()?;
                        window.invalidator.is_dirty().then_some(window.handle)
                    })
                    .collect::<Vec<_>>()
                {
                    self.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
                        .unwrap();
                }

                if self.pending_effects.is_empty() {
                    for window in self.windows.values().filter_map(|window| window.as_deref()) {
                        if window.invalidator.is_dirty()
                            || window.needs_present.get()
                            || !window.next_frame_callbacks.borrow().is_empty()
                        {
                            window.platform_window.schedule_frame();
                        }
                    }

                    self.event_arena.clear();
                    break;
                }
            }
        }
    }

    /// Repeatedly called during `flush_effects` to release any entities whose
    /// reference count has become zero. We invoke any release observers before dropping
    /// each entity.
    fn release_dropped_entities(&mut self) {
        loop {
            let dropped = self.entities.take_dropped();
            if dropped.is_empty() {
                break;
            }

            for (entity_id, mut entity) in dropped {
                self.observers.remove(&entity_id);
                self.event_listeners.remove(&entity_id);
                self.window_invalidators_by_entity.remove(&entity_id);
                self.current_window_by_entity.remove(&entity_id);
                for release_callback in self.release_listeners.remove(&entity_id) {
                    release_callback(entity.as_mut(), self);
                }
            }
        }
    }

    /// Repeatedly called during `flush_effects` to handle a focused handle being dropped.
    fn release_dropped_focus_handles(&mut self) {
        self.focus_handles
            .clone()
            .write()
            .retain(|handle_id, focus| {
                if focus.ref_count.load(SeqCst) == 0 {
                    for window_handle in self.windows() {
                        window_handle
                            .update(self, |_, window, cx| {
                                if window.focus == Some(handle_id) {
                                    window.blur(cx);
                                }
                            })
                            .unwrap();
                    }
                    false
                } else {
                    true
                }
            });
    }

    fn apply_notify_effect(&mut self, emitter: EntityId) {
        self.pending_notifications.remove(&emitter);

        self.observers
            .clone()
            .retain(&emitter, |handler| handler(self));
    }

    fn apply_emit_effect(&mut self, emitter: EntityId, event_type: TypeId, event: &dyn Any) {
        self.event_listeners
            .clone()
            .retain(&emitter, |(stored_type, handler)| {
                if *stored_type == event_type {
                    handler(event, self)
                } else {
                    true
                }
            });
    }

    fn apply_refresh_effect(&mut self) {
        for window in self.windows.values_mut() {
            if let Some(window) = window.as_deref_mut() {
                window.refreshing = true;
                window.invalidator.set_dirty(true);
            }
        }
    }

    fn apply_notify_global_observers_effect(&mut self, type_id: TypeId) {
        self.pending_global_notifications.remove(&type_id);
        self.global_observers
            .clone()
            .retain(&type_id, |observer| observer(self));
    }

    fn apply_defer_effect(&mut self, callback: Box<dyn FnOnce(&mut Self) + 'static>) {
        callback(self);
    }

    fn apply_entity_created_effect(
        &mut self,
        entity: AnyEntity,
        tid: TypeId,
        window: Option<WindowId>,
    ) {
        // Seed the entity's current window from its creation context so
        // `with_window` resolves correctly before the entity has ever been
        // rendered.
        if let Some(id) = window {
            self.current_window_by_entity.insert(entity.entity_id(), id);
        }

        self.new_entity_observers.clone().retain(&tid, |observer| {
            if let Some(id) = window {
                self.update_window_id(id, {
                    let entity = entity.clone();
                    |_, window, cx| (observer)(entity, &mut Some(window), cx)
                })
                .expect("All windows should be off the stack when flushing effects");
            } else {
                (observer)(entity.clone(), &mut None, self)
            }
            true
        });
    }

    /// Runs `f` with the window that most recently drew the entity `entity_id`.
    ///
    /// If the entity has not been drawn yet, the window it was created in is used. The
    /// callback receives the [`Window`] and the [`App`]. Returns `None` if the entity has
    /// no current window, if that window has closed, or if the window is already being
    /// updated higher up the call stack.
    pub fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        let window_id = *self.current_window_by_entity.get(&entity_id)?;
        self.update_window_id(window_id, |_, window, cx| f(window, cx))
            .ok()
    }

    fn ensure_window(&mut self, entity_id: EntityId, window: WindowId) {
        self.current_window_by_entity
            .entry(entity_id)
            .or_insert(window);
    }

    pub(crate) fn update_window_id<T, F>(&mut self, id: WindowId, update: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.update(|cx| {
            let mut window = cx.windows.get_mut(id)?.take()?;

            let root_view = window.root.clone().unwrap();

            cx.window_update_stack.push(window.handle.id);
            let result = update(root_view, &mut window, cx);
            fn trail(id: WindowId, window: Box<Window>, cx: &mut App) -> Option<()> {
                cx.window_update_stack.pop();

                if window.removed {
                    cx.end_platform_drag(id);
                    cx.window_handles.remove(&id);
                    cx.windows.remove(id);
                    if let Some(tracked) = cx.tracked_entities.remove(&id) {
                        for entity_id in tracked {
                            if let Some(windows) =
                                cx.window_invalidators_by_entity.get_mut(&entity_id)
                            {
                                windows.remove(&id);
                            }
                            if cx.current_window_by_entity.get(&entity_id) == Some(&id) {
                                cx.current_window_by_entity.remove(&entity_id);
                            }
                        }
                    }

                    cx.window_closed_observers.clone().retain(&(), |callback| {
                        callback(cx, id);
                        true
                    });

                    let quit_on_empty = match cx.quit_mode {
                        QuitMode::Explicit => false,
                        QuitMode::LastWindowClosed => true,
                        QuitMode::Default => cfg!(not(target_os = "macos")),
                    };

                    if quit_on_empty && cx.windows.is_empty() {
                        cx.quit();
                    }
                } else {
                    cx.windows.get_mut(id)?.replace(window);
                }
                Some(())
            }
            trail(id, window, cx)?;

            Some(result)
        })
        .context("window not found")
    }

    /// Returns an [`AsyncApp`] that can be held across `await` points.
    ///
    /// The [`AsyncApp`] holds the app weakly, so it does not keep the app alive. Use
    /// it in tasks that outlive the current update.
    pub fn to_async(&self) -> AsyncApp {
        AsyncApp {
            app: self.this.clone(),
            background_executor: self.background_executor.clone(),
            foreground_executor: self.foreground_executor.clone(),
        }
    }

    /// Returns the [`BackgroundExecutor`] for `Send` futures.
    ///
    /// Native backends use worker threads; single-threaded web builds use the main thread.
    pub fn background_executor(&self) -> &BackgroundExecutor {
        &self.background_executor
    }

    /// Returns the [`ForegroundExecutor`], which runs futures on the main thread.
    ///
    /// # Panics
    ///
    /// Panics if called after the quit handlers have started running (from
    /// [`App::on_app_quit`]).
    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        if self.quitting {
            panic!("Can't spawn on main thread after on_app_quit")
        };
        &self.foreground_executor
    }

    /// Returns the foreground work journal for this app's foreground thread.
    /// Apps constructed on the same thread share the stream.
    #[cfg(feature = "profiler")]
    pub fn foreground_journal(&self) -> crate::profiler::journal::ForegroundJournal {
        self.foreground_journal.clone()
    }

    /// Runs an async block on the main thread, with an [`AsyncApp`] for access to app state.
    ///
    /// The closure receives `&mut AsyncApp`. Use it to update entities and windows after
    /// an `await`. The future doesn't need to be `Send`, because it stays on the main
    /// thread. Dropping the returned [`Task`] cancels it, so store it or call
    /// [`Task::detach`] to let it run to completion in the background.
    ///
    /// Use [`App::background_executor`] for CPU-heavy or blocking work, and send the
    /// result back to the main thread with a `spawn` task.
    ///
    /// # Panics
    ///
    /// In debug builds this panics if called after the quit handlers have started
    /// running. Release builds log a debug message and continue.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// # use std::{cell::Cell, rc::Rc};
    /// # let mut app = TestApp::new();
    /// let ran = Rc::new(Cell::new(false));
    /// app.update(|cx| {
    ///     let ran = ran.clone();
    ///     cx.spawn(async move |_cx| ran.set(true)).detach();
    /// });
    /// app.read(|_| assert!(ran.get()));
    /// ```
    ///
    /// See [async work](crate::_documentation::guides::async_work) for the executors and
    /// how to share state across tasks.
    #[track_caller]
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        if self.quitting {
            debug_panic!("Can't spawn on main thread after on_app_quit")
        };

        let mut cx = self.to_async();

        self.foreground_executor
            .spawn(async move { f(&mut cx).await }.boxed_local())
    }

    /// Like [`App::spawn`], but schedules the task with the given [`Priority`].
    pub fn spawn_with_priority<AsyncFn, R>(&self, priority: Priority, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        if self.quitting {
            debug_panic!("Can't spawn on main thread after on_app_quit")
        };

        let mut cx = self.to_async();

        self.foreground_executor
            .spawn_with_priority(priority, async move { f(&mut cx).await }.boxed_local())
    }

    /// Runs `f` after the current effect cycle finishes, with the [`App`].
    ///
    /// Use it to act on state that is borrowed right now. For example, you can change
    /// an entity from inside its own `update`, which would otherwise be a double borrow.
    /// The callback runs once the outermost update returns.
    ///
    /// ```
    /// # use gpui::{AppContext, TestApp};
    /// let mut app = TestApp::new();
    /// let count = app.update(|cx| cx.new(|_| 0));
    /// app.update(|cx| {
    ///     let count = count.clone();
    ///     cx.defer(move |cx| {
    ///         count.update(cx, |count, _| *count += 1);
    ///     });
    /// });
    /// app.read(|cx| assert_eq!(*count.read(cx), 1));
    /// ```
    pub fn defer(&mut self, f: impl FnOnce(&mut App) + 'static) {
        self.push_effect(Effect::Defer {
            callback: Box::new(f),
        });
    }

    /// Returns the asset registry that the app loads assets from. See [`Application::with_assets`].
    pub fn assets(&self) -> &Arc<AssetRegistry> {
        &self.asset_registry
    }

    /// Returns the app's [`TextSystem`], which shapes and rasterizes text.
    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    /// Returns whether a global of type `G` has been set.
    pub fn has_global<G: Global>(&self) -> bool {
        self.globals_by_type.contains_key(&TypeId::of::<G>())
    }

    /// Returns the global of type `G`.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set. Use [`App::try_global`] when the
    /// global may be missing.
    #[track_caller]
    pub fn global<G: Global>(&self) -> &G {
        self.globals_by_type
            .get(&TypeId::of::<G>())
            .map(|any_state| any_state.downcast_ref::<G>().unwrap())
            .unwrap_or_else(|| panic!("no state of type {} exists", type_name::<G>()))
    }

    /// Returns the global of type `G`, or `None` if it has not been set.
    pub fn try_global<G: Global>(&self) -> Option<&G> {
        self.globals_by_type
            .get(&TypeId::of::<G>())
            .map(|any_state| any_state.downcast_ref::<G>().unwrap())
    }

    /// Returns the global of type `G` for mutation, and notifies global observers.
    ///
    /// Observers registered with [`App::observe_global`] run when the current update
    /// finishes. The notification happens on every call, even if you don't change the
    /// value.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set. Use [`App::default_global`] to
    /// create one on first use.
    #[track_caller]
    pub fn global_mut<G: Global>(&mut self) -> &mut G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type
            .get_mut(&global_type)
            .and_then(|any_state| any_state.downcast_mut::<G>())
            .unwrap_or_else(|| panic!("no state of type {} exists", type_name::<G>()))
    }

    /// Returns the global of type `G` for mutation, first setting `G::default()` if it is missing.
    ///
    /// Like [`App::global_mut`], this notifies global observers.
    pub fn default_global<G: Global + Default>(&mut self) -> &mut G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type
            .entry(global_type)
            .or_insert_with(|| Box::<G>::default())
            .downcast_mut::<G>()
            .unwrap()
    }

    /// Sets the global of type `G` to `global`, replacing any previous value.
    ///
    /// Notifies global observers when the current update finishes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Global, TestApp};
    /// struct Theme {
    ///     name: &'static str,
    /// }
    /// impl Global for Theme {}
    ///
    /// # let mut app = TestApp::new();
    /// app.update(|cx| cx.set_global(Theme { name: "dark" }));
    /// app.read(|cx| assert_eq!(cx.global::<Theme>().name, "dark"));
    /// ```
    pub fn set_global<G: Global>(&mut self, global: G) {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type.insert(global_type, Box::new(global));
    }

    /// Clear all stored globals. Does not notify global observers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn clear_globals(&mut self) {
        self.globals_by_type.drain();
    }

    /// Removes the global of type `G` and returns it.
    ///
    /// Notifies global observers when the current update finishes.
    ///
    /// # Panics
    ///
    /// Panics if no global of type `G` has been set.
    pub fn remove_global<G: Global>(&mut self) -> G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        *self
            .globals_by_type
            .remove(&global_type)
            .unwrap_or_else(|| panic!("no global added for {}", type_name::<G>()))
            .downcast()
            .unwrap()
    }

    /// Calls `f` each time the global of type `G` is set, changed or removed.
    ///
    /// The observer is active once the current update finishes. Keep the returned
    /// [`Subscription`] alive, because dropping it stops the observer.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{AppContext, Global, TestApp};
    /// # use std::{cell::Cell, rc::Rc};
    /// struct Count(usize);
    /// impl Global for Count {}
    ///
    /// # let mut app = TestApp::new();
    /// let calls = Rc::new(Cell::new(0));
    /// let _subscription = app.update(|cx| {
    ///     cx.set_global(Count(0));
    ///     cx.observe_global::<Count>({
    ///         let calls = calls.clone();
    ///         move |_| calls.set(calls.get() + 1)
    ///     })
    /// });
    /// app.update(|cx| cx.global_mut::<Count>().0 += 1);
    /// assert_eq!(calls.get(), 1);
    /// ```
    pub fn observe_global<G: Global>(
        &mut self,
        mut f: impl FnMut(&mut Self) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.global_observers.insert(
            TypeId::of::<G>(),
            Box::new(move |cx| {
                f(cx);
                true
            }),
        );
        self.defer(move |_| activate());
        subscription
    }

    /// Move the global of the given type to the stack.
    #[track_caller]
    pub(crate) fn lease_global<G: Global>(&mut self) -> GlobalLease<G> {
        GlobalLease::new(
            self.globals_by_type
                .remove(&TypeId::of::<G>())
                .with_context(|| format!("no global registered of type {}", type_name::<G>()))
                .unwrap(),
        )
    }

    /// Restore the global of the given type after it is moved to the stack.
    pub(crate) fn end_global_lease<G: Global>(&mut self, lease: GlobalLease<G>) {
        let global_type = TypeId::of::<G>();

        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type.insert(global_type, lease.global);
    }

    pub(crate) fn new_entity_observer(
        &self,
        key: TypeId,
        value: NewEntityListener,
    ) -> Subscription {
        let (subscription, activate) = self.new_entity_observers.insert(key, value);
        activate();
        subscription
    }

    /// Calls `on_new` each time an entity of type `T` is created.
    ///
    /// The callback receives the new state, the [`Window`] if the entity was created with
    /// one, and a [`Context`] for it. Use it to set up behavior for every instance of a
    /// type, such as registering an observer. Keep the returned [`Subscription`] alive
    /// for the callback to keep running.
    pub fn observe_new<T: 'static>(
        &self,
        on_new: impl 'static + Fn(&mut T, Option<&mut Window>, &mut Context<T>),
    ) -> Subscription {
        self.new_entity_observer(
            TypeId::of::<T>(),
            Box::new(
                move |any_entity: AnyEntity, window: &mut Option<&mut Window>, cx: &mut App| {
                    any_entity
                        .downcast::<T>()
                        .unwrap()
                        .update(cx, |entity_state, cx| {
                            on_new(entity_state, window.as_deref_mut(), cx)
                        })
                },
            ),
        )
    }

    /// Calls `on_release` when the entity behind `handle` is released.
    ///
    /// The callback runs after the entity's last strong handle is dropped, and before the
    /// entity's state is dropped. It receives the state as `&mut T`. Keep the returned
    /// [`Subscription`] alive for the callback to run.
    pub fn observe_release<T>(
        &self,
        handle: &Entity<T>,
        on_release: impl FnOnce(&mut T, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let (subscription, activate) = self.release_listeners.insert(
            handle.entity_id(),
            Box::new(move |entity, cx| {
                let entity = entity.downcast_mut().expect("invalid entity type");
                on_release(entity, cx)
            }),
        );
        activate();
        subscription
    }

    /// Calls `on_release` when the entity behind `handle` is released, inside `window`.
    ///
    /// Like [`App::observe_release`], but the callback also receives `window`. The
    /// callback doesn't run if `window` has already closed when the entity is released.
    pub fn observe_release_in<T>(
        &self,
        handle: &Entity<T>,
        window: &Window,
        on_release: impl FnOnce(&mut T, &mut Window, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let window_handle = window.handle;
        self.observe_release(handle, move |entity, cx| {
            let _ = window_handle.update(cx, |_, window, cx| on_release(entity, window, cx));
        })
    }

    /// Calls `f` for every key-down that reaches any window of the app.
    ///
    /// The callback runs after action and event handling has finished, so it sees the
    /// keystrokes that were not handled earlier. Use [`App::intercept_keystrokes`] to
    /// see keystrokes before they are dispatched. Keep the returned [`Subscription`] alive.
    pub fn observe_keystrokes(
        &mut self,
        mut f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        fn inner(
            keystroke_observers: &SubscriberSet<(), KeystrokeObserver>,
            handler: KeystrokeObserver,
        ) -> Subscription {
            let (subscription, activate) = keystroke_observers.insert((), handler);
            activate();
            subscription
        }

        inner(
            &self.keystroke_observers,
            Box::new(move |event, window, cx| {
                f(event, window, cx);
                true
            }),
        )
    }

    /// Calls `f` for every key-down that reaches any window, before it is dispatched.
    ///
    /// Unlike [`App::observe_keystrokes`], which runs after handling, the interceptor runs
    /// first. Calling [`App::stop_propagation`] inside it stops the keystroke from reaching
    /// action dispatch. Keep the returned [`Subscription`] alive.
    pub fn intercept_keystrokes(
        &mut self,
        mut f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        fn inner(
            keystroke_interceptors: &SubscriberSet<(), KeystrokeObserver>,
            handler: KeystrokeObserver,
        ) -> Subscription {
            let (subscription, activate) = keystroke_interceptors.insert((), handler);
            activate();
            subscription
        }

        inner(
            &self.keystroke_interceptors,
            Box::new(move |event, window, cx| {
                f(event, window, cx);
                true
            }),
        )
    }

    /// Adds key bindings to the app's keymap and redraws the windows.
    ///
    /// A binding maps a keystroke, optionally limited to a [`KeyContext`], to an action.
    /// Bindings are matched during key dispatch. Calling this again adds to the existing
    /// bindings, and [`App::clear_key_bindings`] removes them all.
    ///
    /// # Examples
    ///
    /// ```standalone_crate
    /// use gpui::{AppContext, KeyBinding, Keystroke, TestApp, actions};
    ///
    /// actions!(app_bind_keys, [Save]);
    ///
    /// let mut app = TestApp::new();
    /// app.update(|cx| cx.bind_keys([KeyBinding::new("ctrl-s", Save, None)]));
    /// app.read(|cx| {
    ///     let bindings = cx.all_bindings_for_input(&[Keystroke::parse("ctrl-s").unwrap()]);
    ///     assert_eq!(bindings.len(), 1);
    ///     assert_eq!(bindings[0].action().name(), "app_bind_keys::Save");
    /// });
    /// ```
    ///
    /// See [keyboard](crate::_documentation::guides::keyboard) for context rules and
    /// [input dispatch](crate::_documentation::concepts::input_dispatch) for how a keystroke
    /// picks an action.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        self.keymap.borrow_mut().add_bindings(bindings);
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    /// Removes every key binding from the app's keymap.
    pub fn clear_key_bindings(&mut self) {
        self.keymap.borrow_mut().clear();
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    /// Returns the app's keymap, shared with the app.
    ///
    /// Borrow it to inspect bindings. Don't hold the borrow across a call that changes
    /// bindings, such as [`App::bind_keys`], because the borrow panics.
    pub fn key_bindings(&self) -> Rc<RefCell<Keymap>> {
        self.keymap.clone()
    }

    /// Registers a handler that runs when an action is dispatched. Global handlers run in
    /// the bubble phase, after the handlers in the element tree, and only if no earlier
    /// handler stopped the propagation. Use it for app-wide
    /// commands such as Quit, which should work with no window focused. Dispatch an action
    /// with [`App::dispatch_action`].
    ///
    /// # Examples
    ///
    /// ```standalone_crate
    /// use gpui::{AppContext, TestApp, actions};
    /// use std::{cell::Cell, rc::Rc};
    ///
    /// actions!(app_on_action, [Ping]);
    ///
    /// let mut app = TestApp::new();
    /// let pings = Rc::new(Cell::new(0));
    /// app.update(|cx| {
    ///     let pings = pings.clone();
    ///     cx.on_action(move |_: &Ping, _| pings.set(pings.get() + 1));
    /// });
    /// app.update(|cx| cx.dispatch_action(&Ping));
    /// assert_eq!(pings.get(), 1);
    /// ```
    pub fn on_action<A: Action>(
        &mut self,
        listener: impl Fn(&A, &mut Self) + 'static,
    ) -> &mut Self {
        self.global_action_listeners
            .entry(TypeId::of::<A>())
            .or_default()
            .push(Rc::new(move |action, phase, cx| {
                if phase == DispatchPhase::Bubble {
                    let action = action.downcast_ref().unwrap();
                    listener(action, cx)
                }
            }));
        self
    }

    /// Stops the current event or action from reaching later handlers.
    ///
    /// Handlers for mouse events further down the z-order, and keyboard handlers higher
    /// in the tree, don't run after this. Call [`App::propagate`] to undo it before the
    /// effects are flushed.
    pub fn stop_propagation(&mut self) {
        self.propagate_event = false;
    }

    /// Lets the current event or action continue to later handlers.
    ///
    /// Use it in a handler to pass the event on to the next handler, for example an
    /// action listener on a parent element or a global handler from [`App::on_action`].
    /// It cancels an earlier [`App::stop_propagation`] in the same dispatch.
    pub fn propagate(&mut self) {
        self.propagate_event = true;
    }

    /// Builds an action by its registered name, from JSON data such as a keymap entry.
    ///
    /// `data` is the action's parameters, or `None` for an action with no parameters.
    /// Returns an [`ActionBuildError`] if the name is unknown or the data doesn't match
    /// the action's type. Use it to load actions from a keymap file.
    pub fn build_action(
        &self,
        name: &str,
        data: Option<serde_json::Value>,
    ) -> std::result::Result<Box<dyn Action>, ActionBuildError> {
        self.actions.build_action(name, data)
    }

    /// Returns the names of every action type that has been registered.
    ///
    /// Registration makes an action buildable by name with [`App::build_action`]. It
    /// does not bind the action to a key or a handler. For that, use [`App::bind_keys`]
    /// and [`App::on_action`].
    pub fn all_action_names(&self) -> &[&'static str] {
        self.actions.all_action_names()
    }

    /// Returns the bindings that match `input`, ignoring key context.
    ///
    /// The bindings come back in the order they were added. When you show one to the user,
    /// use the last one, because it takes precedence.
    pub fn all_bindings_for_input(&self, input: &[Keystroke]) -> Vec<KeyBinding> {
        RefCell::borrow(&self.keymap).all_bindings_for_input(input)
    }

    /// Returns each registered action name with its JSON schema, if it has one.
    ///
    /// Use it to generate editor completions or to validate keymap files.
    pub fn action_schemas(
        &self,
        generator: &mut schemars::SchemaGenerator,
    ) -> Vec<(&'static str, Option<schemars::Schema>)> {
        self.actions.action_schemas(generator)
    }

    /// Returns the JSON schema for the action named `name`.
    ///
    /// Returns `None` if no action has that name. Returns `Some(None)` if the action
    /// exists but has no schema, and `Some(Some(schema))` if it has one.
    pub fn action_schema_by_name(
        &self,
        name: &str,
        generator: &mut schemars::SchemaGenerator,
    ) -> Option<Option<schemars::Schema>> {
        self.actions.action_schema_by_name(name, generator)
    }

    /// Returns a map from each deprecated action name to the name that replaces it.
    pub fn deprecated_actions_to_preferred_actions(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.deprecated_aliases()
    }

    /// Returns a map from each deprecated action name to its deprecation message.
    pub fn action_deprecation_messages(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.deprecation_messages()
    }

    /// Returns a map from each action name to its documentation string.
    pub fn action_documentation(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.documentation()
    }

    /// Calls `on_quit` when the app is about to quit, and waits for the returned future.
    ///
    /// Use it to flush data to disk or close connections. The quit can't be cancelled
    /// at this point. Native apps wait up to [`SHUTDOWN_TIMEOUT`] for the futures, so
    /// keep the work short. Keep the returned [`Subscription`] alive for the callback to
    /// run.
    pub fn on_app_quit<Fut>(
        &self,
        mut on_quit: impl FnMut(&mut App) -> Fut + 'static,
    ) -> Subscription
    where
        Fut: 'static + Future<Output = ()>,
    {
        let (subscription, activate) = self.quit_observers.insert(
            (),
            Box::new(move |cx| {
                let future = on_quit(cx);
                future.boxed_local()
            }),
        );
        activate();
        subscription
    }

    /// Calls `on_restart` when the app is about to restart, before any [`App::on_app_quit`] handlers run.
    ///
    /// Keep the returned [`Subscription`] alive for the callback to run.
    pub fn on_app_restart(&self, mut on_restart: impl 'static + FnMut(&mut App)) -> Subscription {
        let (subscription, activate) = self.restart_observers.insert(
            (),
            Box::new(move |cx| {
                on_restart(cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Calls `on_closed` with the [`WindowId`] each time a window closes.
    ///
    /// The window is no longer accessible when the callback runs. Keep the returned
    /// [`Subscription`] alive for the callback to run.
    pub fn on_window_closed(
        &self,
        mut on_closed: impl FnMut(&mut App, WindowId) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.window_closed_observers.insert((), Box::new(on_closed));
        activate();
        subscription
    }

    pub(crate) fn clear_pending_keystrokes(&mut self) {
        for window in self.windows() {
            window
                .update(self, |_, window, cx| {
                    window.clear_pending_keystrokes(cx);
                })
                .ok();
        }
    }

    /// Returns whether `action` would be handled if it were dispatched now.
    ///
    /// Checks the active window's focus path and its bindings, then the global listeners
    /// from [`App::on_action`]. Use it to enable or disable menu items.
    pub fn is_action_available(&mut self, action: &dyn Action) -> bool {
        let mut action_available = false;
        if let Some(window) = self.active_window()
            && let Ok(window_action_available) =
                window.update(self, |_, window, cx| window.is_action_available(action, cx))
        {
            action_available = window_action_available;
        }

        action_available
            || self
                .global_action_listeners
                .contains_key(&action.as_any().type_id())
    }

    /// Replaces the app's menu bar with `menus`.
    ///
    /// Menu items that name an action show the key bindings from the keymap.
    pub fn set_menus(&self, menus: impl IntoIterator<Item = Menu>) {
        let menus: Vec<Menu> = menus.into_iter().collect();
        self.platform.set_menus(menus, &self.keymap.borrow());
    }

    /// Returns the menu bar most recently set with [`App::set_menus`], or `None` if none was set.
    pub fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        self.platform.get_menus()
    }

    /// Sets the menu that appears when the user right-clicks the app's Dock icon.
    pub fn set_dock_menu(&self, menus: Vec<MenuItem>) {
        self.platform.set_dock_menu(menus, &self.keymap.borrow())
    }

    /// Runs the dock menu item at index `action`, as set with [`App::set_dock_menu`].
    ///
    /// Only Windows uses this path.
    pub fn perform_dock_menu_action(&self, action: usize) {
        self.platform.perform_dock_menu_action(action);
    }

    /// On macOS, adds `path` to the recent documents list, which the Dock menu shows.
    /// Other platforms do nothing.
    ///
    /// Call this when the user opens a document, so it appears in the recent list next time.
    pub fn add_recent_document(&self, path: &Path) {
        self.platform.add_recent_document(path);
    }

    /// Replaces the Windows jump list with `menus` and the recent `entries`.
    ///
    /// Only Windows uses this. It also sets the dock menu.
    pub fn update_jump_list(
        &self,
        menus: Vec<MenuItem>,
        entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        self.platform.update_jump_list(menus, entries)
    }

    /// Dispatches `action` to the active window, or to the global listeners if no window is active.
    ///
    /// In a window, the action follows the focus path, as a keystroke bound to it would.
    /// Without an active window, it goes to the listeners from [`App::on_action`]. See
    /// [`crate::Action`] for how actions work.
    pub fn dispatch_action(&mut self, action: &dyn Action) {
        if let Some(active_window) = self.active_window() {
            active_window
                .update(self, |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx)
                })
                .log_err();
        } else {
            self.dispatch_global_action(action);
        }
    }

    fn dispatch_global_action(&mut self, action: &dyn Action) {
        self.propagate_event = true;

        if let Some(mut global_listeners) = self
            .global_action_listeners
            .remove(&action.as_any().type_id())
        {
            for listener in &global_listeners {
                listener(action.as_any(), DispatchPhase::Capture, self);
                if !self.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                self.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            self.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }

        if self.propagate_event
            && let Some(mut global_listeners) = self
                .global_action_listeners
                .remove(&action.as_any().type_id())
        {
            for listener in global_listeners.iter().rev() {
                listener(action.as_any(), DispatchPhase::Bubble, self);
                if !self.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                self.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            self.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }
    }

    /// Returns whether a drag is in progress.
    pub fn has_active_drag(&self) -> bool {
        self.active_drag.is_some()
    }

    /// Returns the payload of the drag in progress, if any.
    pub fn active_drag(&self) -> Option<&AnyDrag> {
        self.active_drag.as_ref()
    }

    /// Returns the cursor style of the drag in progress, if it set one.
    pub fn active_drag_cursor_style(&self) -> Option<CursorStyle> {
        self.active_drag.as_ref().and_then(|drag| drag.cursor_style)
    }

    /// Starts a drag with `drag` as its payload. Refresh the windows after calling this, so the drag shows.
    ///
    /// # Panics
    ///
    /// Debug builds panic if a drag is already in progress.
    pub fn start_drag(&mut self, drag: AnyDrag) {
        debug_assert!(self.active_drag.is_none());
        self.active_drag = Some(drag);
    }

    /// Ends the drag in progress and returns its payload, if there was one.
    ///
    /// Refresh the windows after calling this.
    pub fn stop_drag(&mut self, window: &Window) -> Option<Arc<dyn Any>> {
        let drag = self.active_drag.take();
        if drag.is_some()
            && let Some(platform_drag) = self.platform_owned_drag.as_ref()
        {
            if platform_drag.source_window == window.window_handle().window_id()
                && matches!(
                    &platform_drag.state,
                    PlatformOwnedDragState::RestoredInSourceWindow
                )
            {
                self.platform_owned_drag = None;
            }
        }
        Some(drag?.value)
    }

    /// Stops active drag and clears any related effects.
    pub fn stop_active_drag(&mut self, window: &mut Window) -> bool {
        if self.stop_drag(window).is_some() {
            window.refresh();
            true
        } else {
            false
        }
    }

    pub(crate) fn hand_active_drag_to_platform(&mut self, source_window: WindowId) -> bool {
        let Some(drag) = self.active_drag.take() else {
            return false;
        };
        self.platform_owned_drag = Some(PlatformOwnedDrag {
            source_window,
            state: PlatformOwnedDragState::Suspended(drag),
        });
        true
    }

    pub(crate) fn restore_platform_drag(&mut self, source_window: WindowId) -> bool {
        let Some(platform_drag) = self
            .platform_owned_drag
            .as_mut()
            .filter(|drag| drag.source_window == source_window)
        else {
            return false;
        };
        let state = std::mem::replace(
            &mut platform_drag.state,
            PlatformOwnedDragState::RestoredInSourceWindow,
        );
        let PlatformOwnedDragState::Suspended(drag) = state else {
            return false;
        };
        self.active_drag = Some(drag);
        true
    }

    pub(crate) fn hand_restored_drag_to_platform(&mut self, source_window: WindowId) -> bool {
        let Some(platform_drag) = self.platform_owned_drag.as_mut().filter(|drag| {
            drag.source_window == source_window
                && matches!(&drag.state, PlatformOwnedDragState::RestoredInSourceWindow)
        }) else {
            return false;
        };
        let Some(drag) = self.active_drag.take() else {
            return false;
        };
        platform_drag.state = PlatformOwnedDragState::Suspended(drag);
        true
    }

    pub(crate) fn end_platform_drag(&mut self, source_window: WindowId) -> bool {
        if !self
            .platform_owned_drag
            .as_ref()
            .is_some_and(|drag| drag.source_window == source_window)
        {
            return false;
        }
        self.platform_owned_drag = None;
        self.active_drag = None;
        true
    }

    /// Sets the cursor shown during the active drag. Returns `false` if no drag is in progress.
    pub fn set_active_drag_cursor_style(
        &mut self,
        cursor_style: CursorStyle,
        window: &mut Window,
    ) -> bool {
        if let Some(ref mut drag) = self.active_drag {
            drag.cursor_style = Some(cursor_style);
            window.refresh();
            true
        } else {
            false
        }
    }

    /// Replaces the default and platform prompts with `renderer`.
    ///
    /// The renderer is called for each prompt the app shows. It receives the prompt's
    /// level, message, detail and buttons, and returns the handle that the window renders.
    /// Use [`App::reset_prompt_builder`] to go back to the default prompts.
    pub fn set_prompt_builder(
        &mut self,
        renderer: impl Fn(
            PromptLevel,
            &str,
            Option<&str>,
            &[PromptButton],
            PromptHandle,
            &mut Window,
            &mut App,
        ) -> RenderablePromptHandle
        + 'static,
    ) {
        self.prompt_builder = Some(PromptBuilder::Custom(Box::new(renderer)));
    }

    /// Reset the prompt builder to the default implementation.
    pub fn reset_prompt_builder(&mut self) {
        self.prompt_builder = Some(PromptBuilder::Default);
    }

    /// Removes the cached result for `source`, so the next [`App::fetch_asset`] loads it again.
    pub fn remove_asset<A: Asset>(&mut self, source: &A::Source) {
        let asset_id = (TypeId::of::<A>(), hash(source));
        self.loading_assets.remove(&asset_id);
    }

    /// Returns whether `source` is cached for asset type `A`, either loading or loaded.
    ///
    /// This doesn't start a load. Only available in tests and with the `test-support` feature.
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_asset<A: Asset>(&self, source: &A::Source) -> bool {
        let asset_id = (TypeId::of::<A>(), hash(source));
        self.loading_assets.contains_key(&asset_id)
    }

    /// Starts loading the asset `A` for `source` in the background, or returns the load already in progress.
    ///
    /// The result is a shared task that resolves to the asset output, and a flag that is
    /// `true` only for the call that started the load. Concurrent calls for the same
    /// `source` share one [`Asset::load`] and its cached result. Use
    /// [`App::remove_asset`] to load it again.
    pub fn fetch_asset<A: Asset>(&mut self, source: &A::Source) -> (Shared<Task<A::Output>>, bool) {
        let asset_id = (TypeId::of::<A>(), hash(source));
        let mut is_first = false;
        let task = self
            .loading_assets
            .remove(&asset_id)
            .map(|boxed_task| *boxed_task.downcast::<Shared<Task<A::Output>>>().unwrap())
            .unwrap_or_else(|| {
                is_first = true;
                let future = A::load(source.clone(), self);

                self.background_executor().spawn(future).shared()
            });

        self.loading_assets.insert(asset_id, Box::new(task.clone()));

        (task, is_first)
    }

    /// Creates a new [`FocusHandle`] for tracking keyboard focus.
    ///
    /// Attach the handle to an element with `track_focus`, and move focus to it with
    /// [`Window::focus`].
    #[track_caller]
    pub fn focus_handle(&self) -> FocusHandle {
        FocusHandle::new(&self.focus_handles)
    }

    /// Returns the debugging name of this app, if one was set.
    ///
    /// Only available in tests, with the `test-support` feature, or in debug builds.
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    pub fn get_name(&self) -> Option<&'static str> {
        self.name
    }

    /// Returns `true` if the platform file picker supports selecting a mix of files and directories.
    pub fn can_select_mixed_files_and_dirs(&self) -> bool {
        self.platform.can_select_mixed_files_and_dirs()
    }

    /// Removes an image from the sprite atlas on all windows.
    ///
    /// A window that is being updated is not in `App.windows`, so pass it as `current_window` to
    /// remove the image from it too.
    /// This is a no-op if the image is not in the sprite atlas.
    pub fn drop_image(&mut self, image: Arc<RenderImage>, current_window: Option<&mut Window>) {
        // remove the texture from all other windows
        for window in self.windows.values_mut().flatten() {
            _ = window.drop_image(image.clone());
        }

        // remove the texture from the current window
        if let Some(window) = current_window {
            _ = window.drop_image(image);
        }
    }

    /// Sets the renderer for the inspector.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn set_inspector_renderer(&mut self, f: crate::InspectorRenderer) {
        self.inspector_renderer = Some(f);
    }

    /// Registers a renderer specific to an inspector state.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn register_inspector_element<T: 'static, R: crate::IntoElement>(
        &mut self,
        f: impl 'static + Fn(crate::InspectorElementId, &T, &mut Window, &mut App) -> R,
    ) {
        self.inspector_element_registry.register(f);
    }

    /// Sets the default [`Colors`] global, which `cx.default_colors()` returns.
    pub fn init_colors(&mut self) {
        self.set_global(GlobalColors(Arc::new(Colors::default())));
    }
}

impl AppContext for App {
    /// Builds an entity that is owned by the application.
    ///
    /// The given function will be invoked with a [`Context`] and must return an object representing the entity. An
    /// [`Entity`] handle will be returned, which can be used to access the entity in a context.
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        self.update(|cx| {
            let slot = cx.entities.reserve();
            let handle = slot.clone();
            let entity = build_entity(&mut Context::new_context(cx, slot.downgrade()));

            cx.push_effect(Effect::EntityCreated {
                entity: handle.into_any(),
                tid: TypeId::of::<T>(),
                window: cx.window_update_stack.last().cloned(),
            });

            cx.entities.insert(slot, entity)
        })
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        Reservation(self.entities.reserve())
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        self.update(|cx| {
            let slot = reservation.0;
            let entity = build_entity(&mut Context::new_context(cx, slot.downgrade()));
            cx.entities.insert(slot, entity)
        })
    }

    /// Updates the entity referenced by the given handle. The function is passed a mutable reference to the
    /// entity along with a `Context` for the entity.
    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        self.update(|cx| {
            let mut entity = cx.entities.lease(handle);
            let result = update(
                &mut entity,
                &mut Context::new_context(cx, handle.downgrade()),
            );
            cx.entities.end_lease(entity);
            result
        })
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        GpuiBorrow::new(handle.clone(), self)
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let entity = self.entities.read(handle);
        read(entity, self)
    }

    fn notify(&mut self, entity_id: EntityId) {
        let window_invalidators = mem::take(
            self.window_invalidators_by_entity
                .entry(entity_id)
                .or_default(),
        );

        // `window_invalidators_by_entity` is monotonic, so an entry alone
        // doesn't mean the window is currently rendering the entity. Filter
        // through `tracked_entities` to keep invalidation tight to windows
        // that actually display this entity right now.
        let live_invalidators: SmallVec<[WindowInvalidator; 2]> = window_invalidators
            .iter()
            .filter(|(window_id, _)| {
                self.tracked_entities
                    .get(window_id)
                    .is_some_and(|set| set.contains(&entity_id))
            })
            .map(|(_, invalidator)| invalidator.clone())
            .collect();

        if live_invalidators.is_empty() {
            if self.pending_notifications.insert(entity_id) {
                self.pending_effects
                    .push_back(Effect::Notify { emitter: entity_id });
            }
        } else {
            for invalidator in &live_invalidators {
                invalidator.invalidate_view(entity_id, self);
            }
        }

        self.window_invalidators_by_entity
            .insert(entity_id, window_invalidators);
    }

    fn emit<EntityType, EventType>(&mut self, entity: &Entity<EntityType>, event: EventType)
    where
        EntityType: EventEmitter<EventType>,
        EventType: 'static,
    {
        let event = self
            .event_arena
            .alloc(|| event)
            .map(|it| it as &mut dyn Any);
        self.pending_effects.push_back(Effect::Emit {
            emitter: entity.entity_id(),
            event_type: TypeId::of::<EventType>(),
            event,
        });
    }

    fn update_window<T, F>(&mut self, handle: AnyWindowHandle, update: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.update_window_id(handle.id, update)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        App::with_window(self, entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let window = self
            .windows
            .get(window.id)
            .context("window not found")?
            .as_deref()
            .expect("attempted to read a window that is already on the stack");

        let root_view = window.root.clone().unwrap();
        let view = root_view
            .downcast::<T>()
            .map_err(|_| anyhow!("root view's type has changed"))?;

        Ok(read(view, self))
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.background_executor.spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        let mut g = self.global::<G>();
        callback(g, self)
    }

    fn insert_global_entity<T: 'static>(&mut self, entity: Entity<T>) {
        let any = entity.into_any();
        if !self.global_entities.contains(&any) {
            self.global_entities.push(any);
        }
    }

    fn remove_global_entity<T: 'static>(&mut self, entity: &Entity<T>) {
        self.global_entities
            .retain(|any| any.entity_id() != entity.entity_id());
    }

    fn global_entities<T: 'static>(&self) -> impl Iterator<Item = Entity<T>> {
        let mut iter = self.global_entities.iter();
        iter.filter_map(|any| any.clone().downcast::<T>().ok())
    }
}

/// These effects are processed at the end of each application update cycle.
pub(crate) enum Effect {
    Notify {
        emitter: EntityId,
    },
    Emit {
        emitter: EntityId,
        event_type: TypeId,
        event: ArenaBox<dyn Any>,
    },
    RefreshWindows,
    NotifyGlobalObservers {
        global_type: TypeId,
    },
    Defer {
        callback: Box<dyn FnOnce(&mut App) + 'static>,
    },
    EntityCreated {
        entity: AnyEntity,
        tid: TypeId,
        window: Option<WindowId>,
    },
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Effect::Notify { emitter } => write!(f, "Notify({})", emitter),
            Effect::Emit { emitter, .. } => write!(f, "Emit({:?})", emitter),
            Effect::RefreshWindows => write!(f, "RefreshWindows"),
            Effect::NotifyGlobalObservers { global_type } => {
                write!(f, "NotifyGlobalObservers({:?})", global_type)
            }
            Effect::Defer { .. } => write!(f, "Defer(..)"),
            Effect::EntityCreated { entity, .. } => write!(f, "EntityCreated({:?})", entity),
        }
    }
}

/// Wraps a global variable value during `update_global` while the value has been moved to the stack.
pub(crate) struct GlobalLease<G: Global> {
    global: Box<dyn Any>,
    global_type: PhantomData<G>,
}

impl<G: Global> GlobalLease<G> {
    fn new(global: Box<dyn Any>) -> Self {
        GlobalLease {
            global,
            global_type: PhantomData,
        }
    }
}

impl<G: Global> Deref for GlobalLease<G> {
    type Target = G;

    fn deref(&self) -> &Self::Target {
        self.global.downcast_ref().unwrap()
    }
}

impl<G: Global> DerefMut for GlobalLease<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.global.downcast_mut().unwrap()
    }
}

/// The state of a drag in progress, started by dragging an element in a window.
///
/// Set with [`App::start_drag`]. Read it with [`App::active_drag`]. It is cleared by
/// [`App::stop_drag`] when the drop happens.
#[derive(derive_setters::Setters)]
#[setters(generate = false)]
pub struct AnyDrag {
    /// The view that draws the dragged item under the cursor.
    pub view: AnyView,

    /// The value that the drop target receives.
    pub value: Arc<dyn Any>,

    /// The offset from the cursor to the drag view, so the view stays under the same point it was grabbed.
    #[setters(generate, rename = "offset")]
    pub cursor_offset: Point<Pixels>,

    /// The cursor shown during the drag, or `None` for the default.
    #[setters(generate)]
    pub cursor_style: Option<CursorStyle>,

    /// Resolves the payload to offer the platform if the drag leaves the window.
    /// Invoked at most once per drag gesture, at promotion time.
    pub external_payload_source: Option<ExternalDragPayloadSource>,
}

impl AnyDrag {
    /// Creates a drag that carries `value` and draws `view` under the cursor.
    pub fn new<T: 'static + Sized>(value: T, view: impl Into<AnyView>) -> Self {
        Self {
            view: view.into(),
            value: Arc::new(value),
            cursor_offset: Point::default(),
            cursor_style: None,
            external_payload_source: None,
        }
    }

    /// Sets [`AnyDrag::external_payload_source`]: the closure builds the payload if the drag leaves the window.
    pub fn external_payload(
        mut self,
        predicate: impl FnOnce(&mut Window, &mut App) -> Option<ExternalDragPayload> + 'static,
    ) -> Self {
        self.external_payload_source = Some(Box::new(predicate));
        self
    }

    /// Returns whether the dragged value has type `T`.
    pub fn is_type<T: 'static>(&self) -> bool {
        self.value.as_ref().type_id() == TypeId::of::<T>()
    }
}

/// A closure that builds the payload for a drag that leaves the window.
///
/// It runs when the drag becomes a native drag session, not when the drag starts. Set it
/// with [`AnyDrag::external_payload`].
pub type ExternalDragPayloadSource =
    Box<dyn FnOnce(&mut Window, &mut App) -> Option<ExternalDragPayload> + 'static>;

/// The state of a tooltip that is currently shown.
///
/// You need this type only when you implement tooltip behavior on a custom element.
/// Otherwise use [`Interactivity::tooltip`](crate::Interactivity::tooltip).
#[derive(Clone)]
pub struct AnyTooltip {
    /// The view that draws the tooltip.
    pub view: AnyView,

    /// The absolute position of the mouse when the tooltip was shown.
    pub mouse_position: Point<Pixels>,

    /// Given the bounds of the tooltip, checks whether the tooltip should still be visible and
    /// updates its state accordingly. This is needed atop the hovered element's mouse move handler
    /// to handle the case where the element is not painted (e.g. via use of `visible_on_hover`).
    pub check_visible_and_update: Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) -> bool>,
}

/// A keystroke that reached the app, with the action it resolved to, if any.
///
/// Passed to the callbacks from [`App::observe_keystrokes`] and [`App::intercept_keystrokes`].
#[derive(Debug)]
pub struct KeystrokeEvent {
    /// The keystroke that was pressed.
    pub keystroke: Keystroke,

    /// The action the keystroke resolved to, or `None` if no binding matched. Always
    /// `None` in an `intercept_keystrokes` callback.
    pub action: Option<Box<dyn Action>>,

    /// The key contexts that were active for the keystroke, from the outermost to the focused element.
    pub context_stack: Vec<KeyContext>,
}

/// A mutable borrow of an entity's state, taken out of GPUI while the borrow is alive.
pub struct GpuiBorrow<'a, T> {
    inner: Option<Lease<T>>,
    app: &'a mut App,
}

impl<'a, T: 'static> GpuiBorrow<'a, T> {
    fn new(inner: Entity<T>, app: &'a mut App) -> Self {
        app.start_update();
        let lease = app.entities.lease(&inner);
        Self {
            inner: Some(lease),
            app,
        }
    }
}

impl<'a, T: 'static> std::borrow::Borrow<T> for GpuiBorrow<'a, T> {
    fn borrow(&self) -> &T {
        self.inner.as_ref().unwrap().borrow()
    }
}

impl<'a, T: 'static> std::borrow::BorrowMut<T> for GpuiBorrow<'a, T> {
    fn borrow_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap().borrow_mut()
    }
}

impl<'a, T: 'static> std::ops::Deref for GpuiBorrow<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, T: 'static> std::ops::DerefMut for GpuiBorrow<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, T> Drop for GpuiBorrow<'a, T> {
    fn drop(&mut self) {
        let lease = self.inner.take().unwrap();
        self.app.notify(lease.id);
        self.app.entities.end_lease(lease);
        self.app.finish_update();
    }
}

#[cfg(test)]
mod test {
    use std::{
        cell::{Cell, RefCell},
        ffi::OsString,
        path::PathBuf,
        rc::Rc,
    };

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use crate::{AppContext, Context, Empty, IntoElement, Render, TestAppContext, Window};

    struct RenderCounter(Rc<Cell<usize>>);

    impl Render for RenderCounter {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            Empty
        }
    }

    #[gpui::test]
    fn async_app_refresh_flushes_refresh_effect(cx: &mut TestAppContext) {
        let render_count = Rc::new(Cell::new(0));

        let _window = cx.add_window({
            let render_count = render_count.clone();
            move |_, _| RenderCounter(render_count)
        });

        cx.run_until_parked();
        let render_count_before_refresh = render_count.get();

        cx.to_async().refresh();

        assert_eq!(render_count.get(), render_count_before_refresh + 1);
    }

    #[test]
    fn embedded_update_flushes_effects() {
        let cx = TestAppContext::single();
        let handle = super::ApplicationHandle { app: cx.app };
        let completed = Rc::new(Cell::new(false));

        handle.update(|cx| {
            cx.defer({
                let completed = completed.clone();
                move |_| completed.set(true)
            });
        });

        assert!(completed.get());
    }

    #[gpui::test]
    fn adding_a_tab_group_after_removal_preserves_other_groups(cx: &mut TestAppContext) {
        use super::{SystemWindowTab, SystemWindowTabController};

        let windows: Vec<_> = (0..4).map(|_| cx.add_window(|_, _| Empty)).collect();
        cx.update(|cx| {
            for (ix, window) in windows.iter().take(3).enumerate() {
                SystemWindowTabController::add_tab(
                    cx,
                    window.window_id(),
                    vec![SystemWindowTab::new(
                        format!("Window {ix}").into(),
                        (*window).into(),
                    )],
                );
            }
            SystemWindowTabController::remove_tab(cx, windows[0].window_id());
            SystemWindowTabController::add_tab(
                cx,
                windows[3].window_id(),
                vec![SystemWindowTab::new("New window".into(), windows[3].into())],
            );

            let controller = cx.global::<SystemWindowTabController>();
            assert_eq!(controller.tab_groups.len(), 3);
            for window in &windows[1..] {
                assert_eq!(controller.tabs(window.window_id()).unwrap().len(), 1);
            }
        });
    }

    #[gpui::test]
    fn drag_type_matches_the_payload(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let view = cx.new(|_| Empty);
            let drag = super::AnyDrag::new(String::from("row"), view);
            assert!(drag.is_type::<String>());
            assert!(!drag.is_type::<usize>());
            assert!(!drag.is_type::<std::sync::Arc<dyn std::any::Any>>());
        });
    }

    #[test]
    fn test_gpui_borrow() {
        let cx = TestAppContext::single();
        let observation_count = Rc::new(RefCell::new(0));

        let state = cx.update(|cx| {
            let state = cx.new(|_| false);
            cx.observe(&state, {
                let observation_count = observation_count.clone();
                move |_, _| {
                    let mut count = observation_count.borrow_mut();
                    *count += 1;
                }
            })
            .detach();

            state
        });

        cx.update(|cx| {
            // Calling this like this so that we don't clobber the borrow_mut above
            *std::borrow::BorrowMut::borrow_mut(&mut state.as_mut(cx)) = true;
        });

        cx.update(|cx| {
            state.write(cx, false);
        });

        assert_eq!(*observation_count.borrow(), 2);
    }

    #[gpui::test]
    async fn test_restart_preserves_path_and_arguments(cx: &mut TestAppContext) {
        #[cfg(unix)]
        let user_data_dir = OsString::from_vec(b"/tmp/zed data/\xff".to_vec());
        #[cfg(not(unix))]
        let user_data_dir = OsString::from("C:\\zed data");
        let arguments = vec![OsString::from("--user-data-dir"), user_data_dir];
        let restart_path = PathBuf::from("updated-zed");
        let _application =
            super::Application(cx.app.clone()).with_restart_arguments(arguments.clone());
        let restart = cx.expect_restart();

        cx.update(|cx| {
            cx.set_restart_path(restart_path.clone());
            cx.restart();
        });

        let (path, restart_arguments) = restart.await.expect("restart was not requested");
        assert_eq!(path, Some(restart_path));
        assert_eq!(restart_arguments, arguments);
    }
}
