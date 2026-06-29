use crate::{
    BackgroundExecutor, Capslock, DevicePixels, DummyKeyboardMapper, ExternalPaths, FileDropEvent,
    ForegroundExecutor, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, Platform,
    PlatformInput, PlatformWindow as _, PriorityQueueReceiver, RunnableVariant, ScrollWheelEvent,
    Size,
    platform::cross::{
        dispatcher::{CrossEvent, Dispatcher},
        keyboard::CrossKeyboardLayout,
        render_context::{WgpuContext, WgpuOptions},
        text_system::CosmicTextSystem,
        window::CrossWindow,
    },
    point,
};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

fn device_button_to_gpui(button: u32) -> Option<MouseButton> {
    match button {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Right),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::Navigate(crate::NavigationDirection::Back)),
        4 => Some(MouseButton::Navigate(crate::NavigationDirection::Forward)),
        _ => None,
    }
}
use anyhow::Result;
use arboard::Clipboard;
use collections::FxHashMap;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use winit::event_loop::ActiveEventLoop;

thread_local! {
    static ACTIVE_CONTEXT: Cell<Option<(*const ActiveEventLoop, *mut AppState)>> = Cell::new(None);
}

// Helper to access the context
fn with_active_context<R>(f: impl FnOnce(&ActiveEventLoop, &mut AppState) -> R) -> Option<R> {
    ACTIVE_CONTEXT.with(|storage| {
        let (loop_ptr, app_ptr) = storage.get()?;
        // SAFETY: We strictly manage these pointers during winit callbacks
        unsafe { Some(f(&*loop_ptr, &mut *app_ptr)) }
    })
}

pub(crate) struct CrossPlatform {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<CosmicTextSystem>,
    wgpu_context: Arc<WgpuContext>,
    main_rx: PriorityQueueReceiver<RunnableVariant>,
    event_loop: Cell<Option<winit::event_loop::EventLoop<CrossEvent>>>,
    event_loop_proxy: winit::event_loop::EventLoopProxy<CrossEvent>,
    callbacks: Rc<PlatformCallbacks>,
    menus: RefCell<Option<Vec<crate::OwnedMenu>>>,
    dock_menu: RefCell<Vec<crate::OwnedMenuItem>>,
    single_instance: RefCell<Option<SingleInstanceRuntime>>,
}

#[derive(Default)]
struct PlatformCallbacks {
    on_open_urls: Cell<Option<Box<dyn FnMut(Vec<String>)>>>,
    on_quit: Cell<Option<Box<dyn FnMut()>>>,
    on_reopen: Cell<Option<Box<dyn FnMut()>>>,
    on_app_menu_action: Cell<Option<Box<dyn FnMut(&dyn crate::Action)>>>,
    on_will_open_app_menu: Cell<Option<Box<dyn FnMut()>>>,
    on_validate_app_menu_command: Cell<Option<Box<dyn FnMut(&dyn crate::Action) -> bool>>>,
}

struct AppState {
    windows: FxHashMap<winit::window::WindowId, CrossWindow>,
    window_handles: FxHashMap<winit::window::WindowId, crate::AnyWindowHandle>,
    on_finish_launching: Cell<Option<Box<dyn 'static + FnOnce()>>>,
    callbacks: Rc<PlatformCallbacks>,
    main_rx: PriorityQueueReceiver<RunnableVariant>,
    current_modifiers: Modifiers,
    pressed_button: Option<MouseButton>,
    click_state: ClickState,
    active_window_id: Cell<Option<winit::window::WindowId>>,
    hovered_window_id: Cell<Option<winit::window::WindowId>>,
    hovered_external_paths: Vec<PathBuf>,
}

struct SingleInstanceRuntime {
    lock_path: PathBuf,
    stop: Arc<AtomicBool>,
    listener_thread: Option<thread::JoinHandle<()>>,
}

struct ClickState {
    last_button: MouseButton,
    last_position: crate::Point<Pixels>,
    last_time: Option<Instant>,
    current_count: usize,
}

impl CrossPlatform {
    pub fn new(wgpu_options: WgpuOptions) -> Result<Self> {
        let (main_tx, main_rx) = PriorityQueueReceiver::new();
        let mut event_loop =
            winit::event_loop::EventLoop::<CrossEvent>::with_user_event().build()?;
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        let event_loop_proxy = event_loop.create_proxy();

        let dispatcher = Arc::new(Dispatcher::new(main_tx, event_loop_proxy.clone()));
        let background_executor = BackgroundExecutor::new(dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(dispatcher.clone());

        Ok(Self {
            background_executor,
            foreground_executor,
            text_system: Arc::new(CosmicTextSystem::new()),
            wgpu_context: Arc::new(WgpuContext::new(&wgpu_options)?),
            main_rx,
            event_loop: Cell::new(Some(event_loop)),
            event_loop_proxy,
            callbacks: Rc::new(PlatformCallbacks::default()),
            menus: RefCell::new(None),
            dock_menu: RefCell::new(Vec::new()),
            single_instance: RefCell::new(None),
        })
    }

    fn enable_single_instance(&self, app_id: &str) -> Result<()> {
        if self.single_instance.borrow().is_some() {
            return Ok(());
        }

        let lock_path = single_instance_lock_path(app_id);
        match acquire_single_instance_lock(&lock_path, self.event_loop_proxy.clone()) {
            Ok(runtime) => {
                self.single_instance.borrow_mut().replace(runtime);
                Ok(())
            }
            Err(err) => Err(err),
        }
    }
}

fn single_instance_lock_path(app_id: &str) -> PathBuf {
    let app_hash = seahash::hash(app_id.as_bytes());
    std::env::temp_dir().join(format!("gpui-single-instance-{app_hash:016x}.lock"))
}

fn acquire_single_instance_lock(
    lock_path: &PathBuf,
    event_loop_proxy: winit::event_loop::EventLoopProxy<
        crate::platform::cross::dispatcher::CrossEvent,
    >,
) -> Result<SingleInstanceRuntime> {
    let mut retried = false;

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut lock_file) => {
                let listener = TcpListener::bind(("127.0.0.1", 0))?;
                let port = listener.local_addr()?.port();
                writeln!(lock_file, "{port}")?;

                listener.set_nonblocking(true)?;
                let stop = Arc::new(AtomicBool::new(false));
                let thread_stop = stop.clone();
                let thread_proxy = event_loop_proxy.clone();
                let thread_lock_path = lock_path.clone();
                let listener_thread = thread::spawn(move || {
                    while !thread_stop.load(Ordering::SeqCst) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                let mut buffer = [0u8; 16];
                                let _ = stream.read(&mut buffer);
                                let _ = thread_proxy.send_event(
                                    crate::platform::cross::dispatcher::CrossEvent::SingleInstanceActivated,
                                );
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(50));
                            }
                            Err(err) => {
                                log::error!(
                                    "single-instance listener stopped for {:?}: {err:?}",
                                    thread_lock_path
                                );
                                break;
                            }
                        }
                    }
                });

                return Ok(SingleInstanceRuntime {
                    lock_path: lock_path.clone(),
                    stop,
                    listener_thread: Some(listener_thread),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                match notify_existing_single_instance(lock_path) {
                    Ok(()) => return Err(anyhow::anyhow!("another instance is already running")),
                    Err(notify_err) if !retried => {
                        retried = true;
                        if let Err(remove_err) = fs::remove_file(lock_path) {
                            log::warn!(
                                "failed to remove stale single-instance lock {:?}: {remove_err:?}",
                                lock_path
                            );
                            return Err(anyhow::anyhow!("another instance is already running"));
                        }
                        log::warn!(
                            "recovering stale single-instance lock {:?}: {notify_err:?}",
                            lock_path
                        );
                        continue;
                    }
                    Err(notify_err) => {
                        return Err(anyhow::anyhow!(
                            "failed to contact running instance for {:?}: {notify_err:?}",
                            lock_path
                        ));
                    }
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn notify_existing_single_instance(lock_path: &PathBuf) -> Result<()> {
    let port_text = fs::read_to_string(lock_path)?;
    let port: u16 = port_text.trim().parse()?;
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.write_all(b"activate")?;
    stream.flush()?;
    Ok(())
}

impl Drop for SingleInstanceRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(listener_thread) = self.listener_thread.take() {
            let _ = listener_thread.join();
        }
        let _ = fs::remove_file(&self.lock_path);
    }
}

impl Platform for CrossPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn crate::PlatformTextSystem> {
        self.text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        let mut event_loop = self.event_loop.take().expect("App is already running");

        let mut app_state = AppState {
            windows: Default::default(),
            window_handles: Default::default(),
            on_finish_launching: Cell::new(Some(on_finish_launching)),
            callbacks: self.callbacks.clone(),
            main_rx: self.main_rx.clone(),
            current_modifiers: Modifiers::default(),
            pressed_button: None,
            click_state: ClickState {
                last_button: MouseButton::Left,
                last_position: point(Pixels(0.0), Pixels(0.0)),
                last_time: None,
                current_count: 0,
            },
            active_window_id: Cell::new(None),
            hovered_window_id: Cell::new(None),
            hovered_external_paths: Vec::new(),
        };

        event_loop
            .run_app(&mut app_state)
            .expect("Failed to run App");
    }

    fn quit(&self) {
        // NOTE(mdeand): The event loop will exit when all windows are closed and there are no
        // NOTE(mdeand): more events to process. For an explicit quit, we rely on winit's exit
        // NOTE(mdeand): mechanism via the ActiveEventLoop.
        with_active_context(|event_loop, _| {
            event_loop.exit();
        });
    }

    fn restart(&self, binary_path: Option<std::path::PathBuf>) {
        let binary_path = match binary_path {
            Some(path) => path,
            None => match std::env::current_exe() {
                Ok(path) => path,
                Err(err) => {
                    log::error!("failed to resolve current executable for restart: {err:?}");
                    return;
                }
            },
        };

        let mut command = std::process::Command::new(&binary_path);
        command.args(std::env::args_os().skip(1));
        if let Err(err) = command.spawn() {
            log::error!("failed to restart app with executable {binary_path:?}: {err:?}");
            return;
        }

        self.quit();
    }

    fn activate(&self, ignoring_other_apps: bool) {
        activate_native_app(ignoring_other_apps);
        with_active_context(|_, app_state| {
            for window in app_state.windows.values() {
                window.window().set_visible(true);
            }

            if let Some(active_window_id) = app_state.active_window_id.get()
                && let Some(window) = app_state.windows.get(&active_window_id)
            {
                window.window().focus_window();
                return;
            }

            if let Some(window) = app_state.windows.values().next() {
                window.window().focus_window();
            }
        });
    }

    fn hide(&self) {
        hide_native_app();
        with_active_context(|_, app_state| {
            for window in app_state.windows.values() {
                window.window().set_visible(false);
            }
        });
    }

    fn hide_other_apps(&self) {
        hide_other_native_apps();
    }

    fn unhide_other_apps(&self) {
        unhide_other_native_apps();
    }

    fn displays(&self) -> Vec<Rc<dyn crate::PlatformDisplay>> {
        with_active_context(|event_loop, _| collect_displays(event_loop).0).unwrap_or_default()
    }

    fn primary_display(&self) -> Option<Rc<dyn crate::PlatformDisplay>> {
        with_active_context(|event_loop, _| {
            let (displays, primary_id) = collect_displays(event_loop);
            primary_id.and_then(|primary_id| {
                displays
                    .into_iter()
                    .find(|display| display.id() == primary_id)
            })
        })
        .flatten()
    }

    fn active_window(&self) -> Option<crate::AnyWindowHandle> {
        with_active_context(|_, app_state| {
            app_state
                .active_window_id
                .get()
                .and_then(|window_id| app_state.window_handles.get(&window_id).copied())
        })
        .flatten()
    }

    fn open_window(
        &self,
        handle: crate::AnyWindowHandle,
        options: crate::WindowParams,
    ) -> anyhow::Result<Box<dyn crate::PlatformWindow>> {
        let window = CrossWindow::new(self.wgpu_context.clone(), self.event_loop_proxy.clone());

        let success = with_active_context(|event_loop, app_state| {
            let bounds = options.bounds;
            let use_client_decorations = matches!(
                options.window_decorations,
                Some(crate::WindowDecorations::Client)
            );
            let mut window_origin = bounds.origin;
            if let Some(display_id) = options.display_id {
                if let Some(display) = collect_displays(event_loop)
                    .0
                    .into_iter()
                    .find(|display| display.id() == display_id)
                {
                    window_origin = display.default_bounds().origin;
                }
            }
            let mut attributes = winit::window::Window::default_attributes()
                .with_title(
                    options
                        .titlebar
                        .as_ref()
                        .and_then(|t| t.title.as_ref())
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "GPUI".into()),
                )
                .with_decorations(!use_client_decorations)
                .with_resizable(options.is_resizable)
                .with_visible(options.show)
                .with_position(winit::dpi::LogicalPosition::new(
                    window_origin.x.0 as f64,
                    window_origin.y.0 as f64,
                ))
                .with_inner_size(winit::dpi::LogicalSize::new(
                    bounds.size.width.0 as f64,
                    bounds.size.height.0 as f64,
                ));

            if let Some(min_size) = options.window_min_size {
                attributes = attributes.with_min_inner_size(winit::dpi::LogicalSize::new(
                    min_size.width.0 as f64,
                    min_size.height.0 as f64,
                ));
            }

            // Set the window/application icon when one is provided.
            // On Windows this controls the titlebar + taskbar icon.
            // On macOS, winit only sets the minimised-window thumbnail;
            // we explicitly call `NSApp setApplicationIconImage:` below.
            if let Some(ref icon) = options.app_icon {
                match winit::window::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height) {
                    Ok(winit_icon) => attributes = attributes.with_window_icon(Some(winit_icon)),
                    Err(err) => log::warn!("Failed to set window icon: {err}"),
                }
            }

            // macOS Dock icon — must be set via NSApp, not winit.
            #[cfg(target_os = "macos")]
            if let Some(ref icon) = options.app_icon {
                set_macos_dock_icon(icon);
            }

            #[cfg(target_os = "macos")]
            if use_client_decorations {
                let appears_transparent = options
                    .titlebar
                    .as_ref()
                    .map(|t| t.appears_transparent)
                    .unwrap_or(true);
                attributes = attributes
                    .with_decorations(true)
                    .with_title_hidden(true)
                    .with_titlebar_transparent(appears_transparent)
                    .with_fullsize_content_view(true);
            }

            let winit_window = event_loop
                .create_window(attributes)
                .expect("Failed to create window");
            let window_id = winit_window.id();

            window.initialize(winit_window);
            app_state.windows.insert(window_id, window.clone());
            app_state.window_handles.insert(window_id, handle);
            if options.focus {
                window.window().focus_window();
            }
            window.window().request_redraw();
        })
        .is_some();

        if !success {
            anyhow::bail!("open_window called outside of main thread event loop");
        }

        Ok(Box::new(window))
    }

    fn window_appearance(&self) -> crate::WindowAppearance {
        crate::WindowAppearance::default()
    }

    fn open_url(&self, url: &str) {
        if let Err(err) = ::open::that_detached(url) {
            log::error!("failed to open_url {url:?}: {err:?}");
        }
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.callbacks.on_open_urls.set(Some(callback));
    }

    fn register_url_scheme(&self, _url: &str) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "register_url_scheme is not yet implemented on this platform"
        )))
    }

    fn prompt_for_paths(
        &self,
        options: crate::PathPromptOptions,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<Vec<std::path::PathBuf>>>> {
        let (sender, receiver) = futures::channel::oneshot::channel();

        enum PickType {
            File,
            Folder,
        }
        // rfd does not support picking either/both files and directories. The gpui api is not clear on how various platforms should handle the options.
        let pick_type = match (options.files, options.directories) {
            (true, false) => PickType::File,
            (false, true) => PickType::Folder,
            _ => {
                let _ = sender.send(Err(anyhow::anyhow!("CrossPlatform::prompt_for_paths must be configured to select either files or directories. \
                          Platform does not support neither nor both being configured (must choose exactly one of them).")));
                return receiver;
            }
        };

        let mut dialog = rfd::AsyncFileDialog::new();
        // Diverging from gpui implementation, where the prompt is the button. rfd doesnt support this and the gpui doesnt support an explicit title (unlike prompt_for_new_path).
        // So we hijack the prompt to use it as the title.
        dialog = match options.prompt {
            Some(prompt) => dialog.set_title(prompt),
            None => dialog.set_title(match pick_type {
                PickType::File => "Open File",
                PickType::Folder => "Open Folder",
            }),
        };

        let task = self.foreground_executor().spawn(async move {
            let selection = match options.multiple {
                false => {
                    let file_handle = match pick_type {
                        PickType::File => dialog.pick_file().await,
                        PickType::Folder => dialog.pick_folder().await,
                    };
                    file_handle.map(|handle| vec![handle])
                }
                true => match pick_type {
                    PickType::File => dialog.pick_files().await,
                    PickType::Folder => dialog.pick_folders().await,
                },
            };
            let _ = match selection {
                None => sender.send(Ok(None)),
                Some(handles) => {
                    let paths = handles
                        .into_iter()
                        .map(|handle| handle.path().to_owned())
                        .collect();
                    sender.send(Ok(Some(paths)))
                }
            };
        });
        task.detach();

        receiver
    }

    fn prompt_for_new_path(
        &self,
        directory: &std::path::Path,
        suggested_name: Option<&str>,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Option<std::path::PathBuf>>> {
        let (sender, receiver) = futures::channel::oneshot::channel();

        let mut dialog = rfd::AsyncFileDialog::new();
        dialog = dialog.set_title("Save File");
        dialog = dialog.set_directory(directory);
        if let Some(file_name) = suggested_name {
            dialog = dialog.set_file_name(file_name);
        }
        let task = self.foreground_executor().spawn(async move {
            let selection = dialog.save_file().await;
            let path = selection.map(|handle| handle.path().to_owned());
            let _ = sender.send(Ok(path));
        });
        task.detach();

        receiver
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        // rfd does not support the capability for a user to select both files and folders in the same AsyncFileDialog.
        false
    }

    fn reveal_path(&self, path: &std::path::Path) {
        if let Err(err) = opener::reveal(path) {
            let fallback_path = if path.is_file() {
                path.parent().unwrap_or(path)
            } else {
                path
            };
            if let Err(fallback_err) = ::open::that_detached(fallback_path) {
                log::error!(
                    "failed to reveal path {path:?}: {err:?}; fallback open failed for {fallback_path:?}: {fallback_err:?}"
                );
            } else {
                log::warn!(
                    "failed to reveal path {path:?} ({err:?}); opened {fallback_path:?} instead"
                );
            }
        }
    }

    fn open_with_system(&self, path: &std::path::Path) {
        if let Err(err) = ::open::that_detached(path) {
            log::error!("failed to open_with_system {path:?}: {err:?}");
        }
    }

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_quit.set(Some(callback));
    }

    fn on_reopen(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_reopen.set(Some(callback));
    }

    fn enable_single_instance(&self, app_id: &str) -> anyhow::Result<()> {
        CrossPlatform::enable_single_instance(self, app_id)
    }

    fn set_menus(&self, menus: Vec<crate::Menu>, _keymap: &crate::Keymap) {
        let owned_menus = menus.into_iter().map(crate::Menu::owned).collect();
        *self.menus.borrow_mut() = Some(owned_menus);
    }

    fn get_menus(&self) -> Option<Vec<crate::OwnedMenu>> {
        self.menus.borrow().clone()
    }

    fn set_dock_menu(&self, menu: Vec<crate::MenuItem>, _keymap: &crate::Keymap) {
        *self.dock_menu.borrow_mut() = menu.into_iter().map(crate::MenuItem::owned).collect();
    }

    fn perform_dock_menu_action(&self, action: usize) {
        let selected_action = {
            let dock_menu = self.dock_menu.borrow();
            let mut index = action;
            find_action_at_index(&dock_menu, &mut index).map(|action| action.boxed_clone())
        };

        let Some(selected_action) = selected_action else {
            log::warn!("dock menu action index {action} is out of range");
            return;
        };

        if let Some(mut callback) = self.callbacks.on_app_menu_action.take() {
            callback(selected_action.as_ref());
            self.callbacks.on_app_menu_action.set(Some(callback));
        }
    }

    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn crate::Action)>) {
        self.callbacks.on_app_menu_action.set(Some(callback));
    }

    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>) {
        self.callbacks.on_will_open_app_menu.set(Some(callback));
    }

    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn crate::Action) -> bool>) {
        self.callbacks
            .on_validate_app_menu_command
            .set(Some(callback));
    }

    fn app_path(&self) -> anyhow::Result<std::path::PathBuf> {
        Ok(std::env::current_exe()?)
    }

    fn path_for_auxiliary_executable(&self, _name: &str) -> anyhow::Result<std::path::PathBuf> {
        Err(anyhow::anyhow!(
            "path_for_auxiliary_executable is not yet implemented on this platform"
        ))
    }

    fn set_cursor_style(&self, style: crate::CursorStyle) {
        use winit::window::CursorIcon;
        let icon = match style {
            crate::CursorStyle::Arrow => CursorIcon::Default,
            crate::CursorStyle::IBeam => CursorIcon::Text,
            crate::CursorStyle::Crosshair => CursorIcon::Crosshair,
            crate::CursorStyle::ClosedHand => CursorIcon::Grabbing,
            crate::CursorStyle::OpenHand => CursorIcon::Grab,
            crate::CursorStyle::PointingHand => CursorIcon::Pointer,
            crate::CursorStyle::ResizeLeft => CursorIcon::WResize,
            crate::CursorStyle::ResizeRight => CursorIcon::EResize,
            crate::CursorStyle::ResizeLeftRight => CursorIcon::EwResize,
            crate::CursorStyle::ResizeUp => CursorIcon::NResize,
            crate::CursorStyle::ResizeDown => CursorIcon::SResize,
            crate::CursorStyle::ResizeUpDown => CursorIcon::NsResize,
            crate::CursorStyle::ResizeUpLeftDownRight => CursorIcon::NwseResize,
            crate::CursorStyle::ResizeUpRightDownLeft => CursorIcon::NeswResize,
            crate::CursorStyle::ResizeColumn => CursorIcon::ColResize,
            crate::CursorStyle::ResizeRow => CursorIcon::RowResize,
            crate::CursorStyle::IBeamCursorForVerticalLayout => CursorIcon::VerticalText,
            crate::CursorStyle::DragLink => CursorIcon::Alias,
            crate::CursorStyle::DragCopy => CursorIcon::Copy,
            crate::CursorStyle::ContextualMenu => CursorIcon::ContextMenu,
            crate::CursorStyle::OperationNotAllowed => CursorIcon::NotAllowed,
            crate::CursorStyle::None => {
                with_active_context(|_, app_state| {
                    if let Some(wid) = app_state.hovered_window_id.get() {
                        if let Some(window) = app_state.windows.get(&wid) {
                            window.window().set_cursor_visible(false);
                        }
                    }
                });
                return;
            }
        };
        with_active_context(|_, app_state| {
            if let Some(wid) = app_state.hovered_window_id.get() {
                if let Some(window) = app_state.windows.get(&wid) {
                    window.window().set_cursor_visible(true);
                    window.window().set_cursor(icon);
                }
            }
        });
    }

    fn should_auto_hide_scrollbars(&self) -> bool {
        // TODO(mdeand): How do we want to implement this? For now, just return false.
        false
    }

    fn write_to_clipboard(&self, item: crate::ClipboardItem) {
        let Some(text) = item.text() else {
            log::warn!("write_to_clipboard currently supports text entries only on this platform");
            return;
        };

        match Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            Ok(()) => {}
            Err(error) => log::warn!("failed to write to clipboard: {error}"),
        }
    }

    fn read_from_clipboard(&self) -> Option<crate::ClipboardItem> {
        match Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => Some(crate::ClipboardItem::new_string(text)),
            Err(error) => {
                log::warn!("failed to read from clipboard: {error}");
                None
            }
        }
    }

    fn write_credentials(
        &self,
        _url: &str,
        _username: &str,
        _password: &[u8],
    ) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "write_credentials is not yet implemented on this platform"
        )))
    }

    fn read_credentials(
        &self,
        _url: &str,
    ) -> crate::Task<anyhow::Result<Option<(String, Vec<u8>)>>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "read_credentials is not yet implemented on this platform"
        )))
    }

    fn delete_credentials(&self, _url: &str) -> crate::Task<anyhow::Result<()>> {
        crate::Task::ready(Err(anyhow::anyhow!(
            "delete_credentials is not yet implemented on this platform"
        )))
    }

    fn keyboard_layout(&self) -> Box<dyn crate::PlatformKeyboardLayout> {
        Box::new(CrossKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn crate::PlatformKeyboardMapper> {
        Rc::new(DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {
        // TODO(mdeand): Is this possible to implement in a cross-platform way?
    }
}

impl AppState {
    fn set_active_context(&mut self, event_loop: &ActiveEventLoop) {
        ACTIVE_CONTEXT.with(|s| s.set(Some((event_loop as *const _, self as *mut _))));
    }

    fn clear_active_context(&self) {
        ACTIVE_CONTEXT.with(|s| s.set(None));
    }

    fn drain_main_queue(&mut self) {
        while let Ok(Some(runnable)) = self.main_rx.try_pop() {
            match runnable {
                RunnableVariant::Compat(runnable) => {
                    runnable.run();
                }
                RunnableVariant::Meta(runnable) => {
                    runnable.run();
                }
            }
        }
    }
}

impl winit::application::ApplicationHandler<CrossEvent> for AppState {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CrossEvent) {
        self.set_active_context(event_loop);

        match event {
            CrossEvent::WakeUp => {
                self.drain_main_queue();
            }
            CrossEvent::SurfacePresent(window_id) => {
                if let Some(window) = self.windows.get(&window_id) {
                    window.window().request_redraw();
                }
            }
            CrossEvent::SingleInstanceActivated => {
                if let Some(mut callback) = self.callbacks.on_reopen.take() {
                    callback();
                    self.callbacks.on_reopen.set(Some(callback));
                }
            }
            CrossEvent::CloseWindow(window_id) => {
                // Programmatic close: remove from platform map so the winit
                // window is dropped and the OS window actually disappears.
                self.windows.remove(&window_id);
                self.window_handles.remove(&window_id);
                if self.active_window_id.get() == Some(window_id) {
                    self.active_window_id.set(None);
                }
            }
        }

        self.clear_active_context();
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::Button { button, state } = event {
            if let Some(mouse_button) = device_button_to_gpui(button) {
                if std::env::var_os("GPUI_DEBUG_MOUSE").is_some() {
                    eprintln!(
                        "[WGPUI] DeviceEvent Button {} {:?} pressed_button={:?}",
                        button, state, self.pressed_button
                    );
                }

                match state {
                    winit::event::ElementState::Pressed => {
                        self.pressed_button = Some(mouse_button);
                    }
                    winit::event::ElementState::Released => {
                        if self.pressed_button == Some(mouse_button) {
                            self.pressed_button = None;

                            // TODO: This is a fallback for macOS when WindowEvent::MouseInput
                            // release notifications are not delivered reliably. In an ideal fix,
                            // we would avoid synthesizing MouseUp from raw device events and instead
                            // make the normal winit event path complete correctly.
                            //
                            // IMPORTANT: set_active_context must be called here so that any
                            // cx.open_window() calls triggered by the click handler have a valid
                            // event loop reference (without it they silently fail).
                            self.set_active_context(event_loop);
                            if let Some(window_id) = self.hovered_window_id.get() {
                                if let Some(window) = self.windows.get(&window_id) {
                                    let position = window.0.state.mouse_position.get();
                                    let modifiers = self.current_modifiers;
                                    let platform_event = PlatformInput::MouseUp(MouseUpEvent {
                                        button: mouse_button,
                                        position,
                                        modifiers,
                                        click_count: self.click_state.current_count,
                                    });
                                    window.0.state.callbacks.invoke_mut(
                                        &window.0.state.callbacks.on_input,
                                        |cb| {
                                            cb(platform_event.clone());
                                        },
                                    );
                                }
                            }
                            self.clear_active_context();
                        }
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.set_active_context(event_loop);

        self.drain_main_queue();

        for window in self.windows.values() {
            window.window().request_redraw();
        }

        self.clear_active_context();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {}

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {}

    fn memory_warning(&mut self, _event_loop: &ActiveEventLoop) {}

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.set_active_context(event_loop);

        if let Some(on_finish_launching) = self.on_finish_launching.take() {
            on_finish_launching();
        } else if let Some(mut callback) = self.callbacks.on_reopen.take() {
            callback();
            self.callbacks.on_reopen.set(Some(callback));
        }

        self.clear_active_context();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.set_active_context(event_loop);

        let Some(window) = self.windows.get(&window_id) else {
            return;
        };

        match event {
            winit::event::WindowEvent::HoveredFile(path) => {
                if !self.hovered_external_paths.iter().any(|p| p == &path) {
                    self.hovered_external_paths.push(path);
                }

                let position = window.0.state.mouse_position.get();

                // Start external drag once with all currently known hovered paths.
                if self.hovered_external_paths.len() == 1 {
                    let platform_event = PlatformInput::FileDrop(FileDropEvent::Entered {
                        position,
                        paths: ExternalPaths(self.hovered_external_paths.clone().into()),
                    });

                    window
                        .0
                        .state
                        .callbacks
                        .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                            cb(platform_event.clone());
                        });
                }
            }

            winit::event::WindowEvent::HoveredFileCancelled => {
                self.hovered_external_paths.clear();

                let platform_event = PlatformInput::FileDrop(FileDropEvent::Exited);

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(platform_event.clone());
                    });
            }

            winit::event::WindowEvent::DroppedFile(path) => {
                // Some backends may emit drop without prior hover events.
                if self.hovered_external_paths.is_empty() {
                    self.hovered_external_paths.push(path);

                    let position = window.0.state.mouse_position.get();
                    let entered = PlatformInput::FileDrop(FileDropEvent::Entered {
                        position,
                        paths: ExternalPaths(self.hovered_external_paths.clone().into()),
                    });

                    window
                        .0
                        .state
                        .callbacks
                        .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                            cb(entered.clone());
                        });
                }

                let position = window.0.state.mouse_position.get();
                let submit = PlatformInput::FileDrop(FileDropEvent::Submit { position });

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(submit.clone());
                    });

                self.hovered_external_paths.clear();
            }

            winit::event::WindowEvent::Resized(physical_size) => {
                if physical_size.width == 0 || physical_size.height == 0 {
                    return;
                }

                window.0.state.resize_detector.on_resize_event();
                window.window().request_redraw();
                let scale_factor = window.scale_factor();

                if let Some(renderer) = window.0.renderer.get() {
                    renderer.borrow_mut().update_drawable_size(Size {
                        width: DevicePixels(physical_size.width as i32),
                        height: DevicePixels(physical_size.height as i32),
                    });
                }
                let size = crate::Size {
                    width: crate::Pixels(physical_size.width as f32 / scale_factor),
                    height: crate::Pixels(physical_size.height as f32 / scale_factor),
                };

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_resize, |cb| {
                        cb(size, scale_factor);
                    });
            }

            winit::event::WindowEvent::Moved(_) => {
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_moved, |cb| {
                        cb();
                    });
            }

            winit::event::WindowEvent::Focused(active) => {
                if active {
                    self.active_window_id.set(Some(window_id));
                } else if self.active_window_id.get() == Some(window_id) {
                    self.active_window_id.set(None);
                }
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_active_status_change, |cb| {
                        cb(active)
                    });
            }

            winit::event::WindowEvent::ThemeChanged(_) => {
                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_appearance_changed, |cb| cb());
            }

            winit::event::WindowEvent::CloseRequested => {
                let should_close = window
                    .0
                    .state
                    .callbacks
                    .on_should_close
                    .take()
                    .map(|mut cb| {
                        let result = cb();
                        window.0.state.callbacks.on_should_close.set(Some(cb));
                        result
                    })
                    .unwrap_or(true);

                if should_close {
                    if let Some(cb) = window.0.state.callbacks.on_close.take() {
                        cb();
                    }
                    self.windows.remove(&window_id);
                    self.window_handles.remove(&window_id);
                    if self.active_window_id.get() == Some(window_id) {
                        self.active_window_id.set(None);
                    }
                }
            }

            winit::event::WindowEvent::RedrawRequested => {
                let physical_size = window.window().inner_size();
                if physical_size.width == 0 || physical_size.height == 0 {
                    return;
                }

                // Try fast blit path for pending surfaces
                let mut fast_blit_succeeded = false;
                if let Some(renderer) = window.0.renderer.get() {
                    let renderer_ref = renderer.borrow();
                    if let Some(pending_surfaces) = renderer_ref.get_pending_surfaces() {
                        fast_blit_succeeded = renderer_ref.blit_surfaces_direct(&pending_surfaces);
                    }
                }

                window.0.state.callbacks.invoke_mut(
                    &window.0.state.callbacks.on_request_frame,
                    |cb| {
                        cb(crate::RequestFrameOptions {
                            // Only force compositor if fast blit failed
                            force_render: !fast_blit_succeeded,
                            require_presentation: true,
                        });
                    },
                );
            }

            winit::event::WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        logical_key,
                        state,
                        text,
                        repeat,
                        ..
                    },
                ..
            } => {
                let modifiers = self.current_modifiers;

                if let Some(keystroke) = winit_key_to_keystroke(&logical_key, modifiers, &text) {
                    let platform_event = match state {
                        winit::event::ElementState::Pressed => {
                            PlatformInput::KeyDown(KeyDownEvent {
                                keystroke,
                                is_held: repeat,
                                prefer_character_input: false,
                            })
                        }
                        winit::event::ElementState::Released => {
                            PlatformInput::KeyUp(KeyUpEvent { keystroke })
                        }
                    };

                    window
                        .0
                        .state
                        .callbacks
                        .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                            cb(platform_event.clone());
                        });
                }
            }

            winit::event::WindowEvent::ModifiersChanged(new_modifiers) => {
                let modifiers = winit_modifiers_to_gpui(new_modifiers.state());
                self.current_modifiers = modifiers;

                window.0.state.modifiers.set(modifiers);

                let platform_event = PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers,
                    capslock: Capslock::default(),
                });

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(platform_event.clone());
                    });
            }

            winit::event::WindowEvent::CursorEntered { .. } => {
                self.hovered_window_id.set(Some(window_id));
                let was_hovered = window.0.state.is_hovered.get();
                window.0.state.is_hovered.set(true);
                if !was_hovered {
                    window.0.state.callbacks.invoke_mut(
                        &window.0.state.callbacks.on_hover_status_change,
                        |cb| {
                            cb(true);
                        },
                    );
                }
            }
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                self.hovered_window_id.set(Some(window_id));
                let was_hovered = window.0.state.is_hovered.get();
                window.0.state.is_hovered.set(true);
                if !was_hovered {
                    window.0.state.callbacks.invoke_mut(
                        &window.0.state.callbacks.on_hover_status_change,
                        |cb| {
                            cb(true);
                        },
                    );
                }
                let scale_factor = window.scale_factor();
                let position = point(
                    Pixels(position.x as f32 / scale_factor),
                    Pixels(position.y as f32 / scale_factor),
                );

                window.0.state.mouse_position.set(position);

                let platform_event = PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: self.pressed_button,
                    modifiers: self.current_modifiers,
                });

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(platform_event.clone());
                    });

                if !self.hovered_external_paths.is_empty() {
                    let file_drop_event =
                        PlatformInput::FileDrop(FileDropEvent::Pending { position });

                    window
                        .0
                        .state
                        .callbacks
                        .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                            cb(file_drop_event.clone());
                        });
                }
            }

            winit::event::WindowEvent::CursorLeft { .. } => {
                self.hovered_window_id.set(None);
                window.0.state.is_hovered.set(false);
                window.0.state.callbacks.invoke_mut(
                    &window.0.state.callbacks.on_hover_status_change,
                    |cb| {
                        cb(false);
                    },
                );
                let position = window.0.state.mouse_position.get();
                let platform_event = PlatformInput::MouseExited(MouseExitEvent {
                    position,
                    pressed_button: self.pressed_button,
                    modifiers: self.current_modifiers,
                });

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(platform_event.clone());
                    });
            }

            winit::event::WindowEvent::MouseInput { state, button, .. } => {
                let position = window.0.state.mouse_position.get();
                let mouse_button = winit_mouse_button_to_gpui(button);
                let modifiers = self.current_modifiers;

                if std::env::var_os("GPUI_DEBUG_MOUSE").is_some() {
                    eprintln!(
                        "[WGPUI] macOS MouseInput {:?} {:?} @ {:?} hovered={} pressed_button={:?}",
                        state,
                        button,
                        position,
                        window.0.state.is_hovered.get(),
                        self.pressed_button,
                    );
                }

                match state {
                    winit::event::ElementState::Pressed => {
                        self.pressed_button = Some(mouse_button);

                        let click_count =
                            self.click_state
                                .update(mouse_button, position, Instant::now());

                        let platform_event = PlatformInput::MouseDown(MouseDownEvent {
                            button: mouse_button,
                            position,
                            modifiers,
                            click_count,
                            first_mouse: false,
                        });

                        window.0.state.callbacks.invoke_mut(
                            &window.0.state.callbacks.on_input,
                            |cb| {
                                cb(platform_event.clone());
                            },
                        );
                    }
                    winit::event::ElementState::Released => {
                        self.pressed_button = None;
                        if mouse_button == MouseButton::Left {
                            window.window().request_redraw();
                        }

                        let platform_event = PlatformInput::MouseUp(MouseUpEvent {
                            button: mouse_button,
                            position,
                            modifiers,
                            click_count: self.click_state.current_count,
                        });

                        window.0.state.callbacks.invoke_mut(
                            &window.0.state.callbacks.on_input,
                            |cb| {
                                cb(platform_event.clone());
                            },
                        );
                    }
                }
            }

            winit::event::WindowEvent::MouseWheel { delta, phase, .. } => {
                let position = window.0.state.mouse_position.get();
                let modifiers = self.current_modifiers;

                let scroll_delta = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        crate::ScrollDelta::Lines(point(x, y))
                    }
                    winit::event::MouseScrollDelta::PixelDelta(delta) => {
                        let scale_factor = window.scale_factor();
                        crate::ScrollDelta::Pixels(point(
                            Pixels(delta.x as f32 / scale_factor),
                            Pixels(delta.y as f32 / scale_factor),
                        ))
                    }
                };

                let touch_phase = match phase {
                    winit::event::TouchPhase::Started => crate::TouchPhase::Started,
                    winit::event::TouchPhase::Moved => crate::TouchPhase::Moved,
                    winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                        crate::TouchPhase::Ended
                    }
                };

                let platform_event = PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position,
                    delta: scroll_delta,
                    modifiers,
                    touch_phase,
                });

                window
                    .0
                    .state
                    .callbacks
                    .invoke_mut(&window.0.state.callbacks.on_input, |cb| {
                        cb(platform_event.clone());
                    });
            }

            _ => (),
        }

        self.clear_active_context();
    }
}

const DOUBLE_CLICK_THRESHOLD_MS: u128 = 500;
const DOUBLE_CLICK_DISTANCE: f32 = 5.0;

impl ClickState {
    fn update(
        &mut self,
        button: MouseButton,
        position: crate::Point<Pixels>,
        now: Instant,
    ) -> usize {
        let is_same_button = self.last_button == button;
        let is_within_time = self
            .last_time
            .map(|t| now.duration_since(t).as_millis() < DOUBLE_CLICK_THRESHOLD_MS)
            .unwrap_or(false);
        let distance = ((position.x - self.last_position.x).0.powi(2)
            + (position.y - self.last_position.y).0.powi(2))
        .sqrt();
        let is_within_distance = distance < DOUBLE_CLICK_DISTANCE;

        if is_same_button && is_within_time && is_within_distance {
            self.current_count += 1;
        } else {
            self.current_count = 1;
        }

        self.last_button = button;
        self.last_position = position;
        self.last_time = Some(now);

        self.current_count
    }
}

fn winit_modifiers_to_gpui(modifiers: winit::keyboard::ModifiersState) -> Modifiers {
    Modifiers {
        control: modifiers.control_key(),
        alt: modifiers.alt_key(),
        shift: modifiers.shift_key(),
        platform: modifiers.super_key(),
        function: false,
    }
}

fn winit_mouse_button_to_gpui(button: winit::event::MouseButton) -> MouseButton {
    match button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        winit::event::MouseButton::Back => MouseButton::Navigate(crate::NavigationDirection::Back),
        winit::event::MouseButton::Forward => {
            MouseButton::Navigate(crate::NavigationDirection::Forward)
        }
        winit::event::MouseButton::Other(_) => MouseButton::Left,
    }
}

#[derive(Debug)]
struct CrossDisplay {
    id: crate::DisplayId,
    uuid: uuid::Uuid,
    bounds: crate::Bounds<Pixels>,
}

impl crate::PlatformDisplay for CrossDisplay {
    fn id(&self) -> crate::DisplayId {
        self.id
    }

    fn uuid(&self) -> anyhow::Result<uuid::Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> crate::Bounds<Pixels> {
        self.bounds
    }
}

fn collect_displays(
    event_loop: &ActiveEventLoop,
) -> (
    Vec<Rc<dyn crate::PlatformDisplay>>,
    Option<crate::DisplayId>,
) {
    let primary_fingerprint = event_loop
        .primary_monitor()
        .as_ref()
        .map(monitor_fingerprint);

    let mut primary_display_id = None;
    let mut used_display_ids = HashSet::new();
    let displays = event_loop
        .available_monitors()
        .map(|monitor| {
            let fingerprint = monitor_fingerprint(&monitor);
            let display_id = stable_display_id(&fingerprint, &mut used_display_ids);
            if Some(fingerprint.clone()) == primary_fingerprint {
                primary_display_id = Some(display_id);
            }

            let scale_factor = monitor.scale_factor() as f32;
            let position = monitor.position();
            let size = monitor.size();
            let bounds = crate::Bounds::new(
                point(
                    Pixels(position.x as f32 / scale_factor),
                    Pixels(position.y as f32 / scale_factor),
                ),
                crate::Size {
                    width: Pixels(size.width as f32 / scale_factor),
                    height: Pixels(size.height as f32 / scale_factor),
                },
            );
            let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &fingerprint);

            Rc::new(CrossDisplay {
                id: display_id,
                uuid,
                bounds,
            }) as Rc<dyn crate::PlatformDisplay>
        })
        .collect::<Vec<_>>();

    (displays, primary_display_id)
}

fn monitor_fingerprint(monitor: &winit::monitor::MonitorHandle) -> Vec<u8> {
    let mut fingerprint = Vec::new();
    if let Some(name) = monitor.name() {
        fingerprint.extend_from_slice(name.as_bytes());
    }

    let position = monitor.position();
    fingerprint.extend_from_slice(&position.x.to_le_bytes());
    fingerprint.extend_from_slice(&position.y.to_le_bytes());

    let size = monitor.size();
    fingerprint.extend_from_slice(&size.width.to_le_bytes());
    fingerprint.extend_from_slice(&size.height.to_le_bytes());

    fingerprint.extend_from_slice(&monitor.scale_factor().to_bits().to_le_bytes());
    fingerprint
}

fn stable_display_id(fingerprint: &[u8], used_ids: &mut HashSet<u32>) -> crate::DisplayId {
    let base = (seahash::hash(fingerprint) as u32).max(1);
    let mut candidate = base;

    while !used_ids.insert(candidate) {
        candidate = candidate.wrapping_add(1);
        if candidate == 0 {
            candidate = 1;
        }
    }

    crate::DisplayId(candidate)
}

#[cfg(target_os = "macos")]
fn activate_native_app(ignoring_other_apps: bool) {
    use objc2_app_kit::NSApp;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("activate called off main thread; skipping native activation");
        return;
    };

    let app = NSApp(mtm);
    if ignoring_other_apps {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    } else {
        unsafe { app.activate() };
    }
}

#[cfg(not(target_os = "macos"))]
fn activate_native_app(_ignoring_other_apps: bool) {}

#[cfg(target_os = "macos")]
fn hide_native_app() {
    use objc2_app_kit::NSApp;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("hide called off main thread; skipping native hide");
        return;
    };

    let app = NSApp(mtm);
    app.hide(None);
}

#[cfg(not(target_os = "macos"))]
fn hide_native_app() {}

#[cfg(target_os = "macos")]
fn hide_other_native_apps() {
    use objc2_app_kit::NSApp;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("hide_other_apps called off main thread; skipping native hide-other-apps");
        return;
    };

    let app = NSApp(mtm);
    app.hideOtherApplications(None);
}

#[cfg(not(target_os = "macos"))]
fn hide_other_native_apps() {
    log::debug!("hide_other_apps has no native equivalent on this platform");
}

#[cfg(target_os = "macos")]
fn unhide_other_native_apps() {
    use objc2_app_kit::NSApp;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("unhide_other_apps called off main thread; skipping native unhide-all-apps");
        return;
    };

    let app = NSApp(mtm);
    unsafe { app.unhideAllApplications(None) };
}

#[cfg(not(target_os = "macos"))]
fn unhide_other_native_apps() {
    log::debug!("unhide_other_apps has no native equivalent on this platform");
}

fn find_action_at_index<'a>(
    items: &'a [crate::OwnedMenuItem],
    index: &mut usize,
) -> Option<&'a dyn crate::Action> {
    for item in items {
        match item {
            crate::OwnedMenuItem::Action { action, .. } => {
                if *index == 0 {
                    return Some(action.as_ref());
                }
                *index -= 1;
            }
            crate::OwnedMenuItem::Submenu(submenu) => {
                if let Some(action) = find_action_at_index(&submenu.items, index) {
                    return Some(action);
                }
            }
            crate::OwnedMenuItem::Separator | crate::OwnedMenuItem::SystemMenu(_) => {}
        }
    }

    None
}

fn winit_key_to_keystroke(
    logical_key: &winit::keyboard::Key,
    modifiers: Modifiers,
    text: &Option<winit::keyboard::SmolStr>,
) -> Option<Keystroke> {
    use winit::keyboard::Key as WKey;
    use winit::keyboard::NamedKey;

    let (key, key_char) = match logical_key {
        WKey::Named(named) => {
            let key_name = match named {
                NamedKey::Backspace => "backspace",
                NamedKey::Tab => "tab",
                NamedKey::Enter => "enter",
                NamedKey::Escape => "escape",
                NamedKey::Space => "space",
                NamedKey::ArrowLeft => "left",
                NamedKey::ArrowRight => "right",
                NamedKey::ArrowUp => "up",
                NamedKey::ArrowDown => "down",
                NamedKey::Home => "home",
                NamedKey::End => "end",
                NamedKey::PageUp => "pageup",
                NamedKey::PageDown => "pagedown",
                NamedKey::Insert => "insert",
                NamedKey::Delete => "delete",
                NamedKey::F1 => "f1",
                NamedKey::F2 => "f2",
                NamedKey::F3 => "f3",
                NamedKey::F4 => "f4",
                NamedKey::F5 => "f5",
                NamedKey::F6 => "f6",
                NamedKey::F7 => "f7",
                NamedKey::F8 => "f8",
                NamedKey::F9 => "f9",
                NamedKey::F10 => "f10",
                NamedKey::F11 => "f11",
                NamedKey::F12 => "f12",
                NamedKey::BrowserBack => "back",
                NamedKey::BrowserForward => "forward",
                // Modifier-only keys don't produce keystrokes by themselves
                NamedKey::Shift
                | NamedKey::Control
                | NamedKey::Alt
                | NamedKey::Super
                | NamedKey::Meta => return None,
                _ => return None,
            };
            let key_char = match named {
                NamedKey::Space
                    if !modifiers.control
                        && !modifiers.platform
                        && !modifiers.function
                        && !modifiers.alt =>
                {
                    Some(" ".to_string())
                }

                _ => None,
            };
            (key_name.to_string(), key_char)
        }
        WKey::Character(ch) => {
            let key = ch.to_lowercase();
            let key_char = text.as_ref().map(|t| t.to_string()).or_else(|| {
                if !modifiers.control
                    && !modifiers.platform
                    && !modifiers.function
                    && !modifiers.alt
                {
                    if modifiers.shift {
                        Some(ch.to_uppercase().to_string())
                    } else {
                        Some(ch.to_string())
                    }
                } else {
                    None
                }
            });
            (key, key_char)
        }
        WKey::Unidentified(_) | WKey::Dead(_) => return None,
    };

    Some(Keystroke {
        modifiers,
        key,
        key_char,
    })
}

#[cfg(test)]
mod tests {
    use super::winit_key_to_keystroke;
    use crate::Modifiers;
    use winit::keyboard::{Key, NamedKey};

    #[test]
    fn translates_space_to_text_input() {
        let keystroke =
            winit_key_to_keystroke(&Key::Named(NamedKey::Space), Modifiers::default(), &None)
                .expect("space should produce a keystroke");

        assert_eq!(keystroke.key, "space");
        assert_eq!(keystroke.key_char.as_deref(), Some(" "));
    }

    #[test]
    fn does_not_treat_command_space_as_text_input() {
        let keystroke = winit_key_to_keystroke(
            &Key::Named(NamedKey::Space),
            Modifiers {
                platform: true,
                ..Modifiers::default()
            },
            &None,
        )
        .expect("command-space should still produce a keystroke");

        assert_eq!(keystroke.key, "space");
        assert_eq!(keystroke.key_char, None);
    }
}

/// Set the macOS application Dock icon by calling `NSApp setApplicationIconImage:`.
///
/// `winit`'s `with_window_icon` only sets the per-window miniaturised thumbnail
/// (shown in the Dock when *that specific window* is minimised). To change the
/// live Dock tile for the running process — which is what users see as the "app
/// icon" — we must call `[NSApp setApplicationIconImage:]` directly.
///
/// This is a no-op on all other platforms (the cfg gate in the call site ensures
/// this function is never compiled in non-macOS builds).
#[cfg(target_os = "macos")]
fn set_macos_dock_icon(icon: &crate::WindowIcon) {
    use image::{ImageBuffer, Rgba, imageops};
    use objc2::ClassType;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    // macOS Dock icons are capped at 512×512 (1024×1024 @2×).
    // If we hand NSImage a larger image it reports a bigger "natural size"
    // and the Dock renders the tile bigger than every other app icon.
    const MAX_DOCK_ICON_PX: u32 = 512;

    let Some(buf) =
        ImageBuffer::<Rgba<u8>, _>::from_raw(icon.width, icon.height, icon.rgba.clone())
    else {
        log::warn!("set_macos_dock_icon: icon dimensions don't match pixel buffer");
        return;
    };

    // Downscale only if the image is larger than the Dock cap.
    let buf = if icon.width > MAX_DOCK_ICON_PX || icon.height > MAX_DOCK_ICON_PX {
        imageops::resize(
            &buf,
            MAX_DOCK_ICON_PX,
            MAX_DOCK_ICON_PX,
            imageops::FilterType::Lanczos3,
        )
    } else {
        buf
    };

    let mut png: Vec<u8> = Vec::new();
    if buf
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        log::warn!("set_macos_dock_icon: failed to encode icon as PNG");
        return;
    }

    // SAFETY: `open_window` (our only call site) is always invoked on the main
    // thread, which is the only thread on which AppKit objects may be used.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    unsafe {
        let data = NSData::with_bytes(&png);
        if let Some(ns_image) = NSImage::initWithData(NSImage::alloc(), &data) {
            let app = NSApplication::sharedApplication(mtm);
            app.setApplicationIconImage(Some(&ns_image));
        } else {
            log::warn!("set_macos_dock_icon: NSImage could not decode PNG data");
        }
    }
}
