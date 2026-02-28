use glow::HasContext as _;
use std::{
    ffi,
    os::raw,
    ptr,
    sync::{Mutex, MutexGuard},
};

const EGL_CONTEXT_FLAGS_KHR: i32 = 0x30FC;
const EGL_CONTEXT_OPENGL_DEBUG_BIT_KHR: i32 = 0x0001;
const _EGL_PLATFORM_WAYLAND_KHR: u32 = 0x31D8;
const _EGL_PLATFORM_X11_KHR: u32 = 0x31D5;
const _EGL_PLATFORM_XCB_EXT: u32 = 0x31DC;
const EGL_PLATFORM_ANGLE_ANGLE: u32 = 0x3202;
const EGL_PLATFORM_ANGLE_TYPE_ANGLE: u32 = 0x3203;
const EGL_PLATFORM_ANGLE_TYPE_DEFAULT_ANGLE: u32 = 0x3206;
const EGL_PLATFORM_ANGLE_TYPE_METAL_ANGLE: u32 = 0x3489;
const EGL_PLATFORM_ANGLE_NATIVE_PLATFORM_TYPE_ANGLE: u32 = 0x348F;
const EGL_PLATFORM_ANGLE_DEBUG_LAYERS_ENABLED: u32 = 0x3451;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

const EGL_DEBUG_MSG_CRITICAL_KHR: u32 = 0x33B9;
const EGL_DEBUG_MSG_ERROR_KHR: u32 = 0x33BA;
const EGL_DEBUG_MSG_WARN_KHR: u32 = 0x33BB;
const EGL_DEBUG_MSG_INFO_KHR: u32 = 0x33BC;

type _XOpenDisplayFun =
    unsafe extern "system" fn(display_name: *const raw::c_char) -> *mut raw::c_void;
type _WlDisplayConnectFun =
    unsafe extern "system" fn(display_name: *const raw::c_char) -> *mut raw::c_void;
type _WlDisplayDisconnectFun = unsafe extern "system" fn(display: *const raw::c_void);

type EglInstance = egl::DynamicInstance<egl::EGL1_4>;

type WlEglWindowCreateFun = unsafe extern "system" fn(
    surface: *const raw::c_void,
    width: raw::c_int,
    height: raw::c_int,
) -> *mut raw::c_void;

type _WlEglWindowResizeFun = unsafe extern "system" fn(
    window: *const raw::c_void,
    width: raw::c_int,
    height: raw::c_int,
    dx: raw::c_int,
    dy: raw::c_int,
);

type WlEglWindowDestroyFun = unsafe extern "system" fn(window: *const raw::c_void);

type EglLabel = *const raw::c_void;

type DebugProcKHR = Option<
    unsafe extern "system" fn(
        error: egl::Enum,
        command: *const raw::c_char,
        message_type: u32,
        thread_label: EglLabel,
        object_label: EglLabel,
        message: *const raw::c_char,
    ),
>;

type EglDebugMessageControlFun =
    unsafe extern "system" fn(proc: DebugProcKHR, attrib_list: *const egl::Attrib) -> raw::c_int;

#[derive(Debug)]
pub enum PlatformError {
    Loading(egl::LoadError<libloading::Error>),
    Init(egl::Error),
}

#[derive(Clone, Copy, Debug)]
enum SrgbFrameBufferKind {
    /// No support for SRGB surface
    None,
    /// Using EGL 1.5's support for colorspaces
    Core,
    /// Using EGL_KHR_gl_colorspace
    Khr,
}

#[derive(Debug)]
struct EglContext {
    instance: EglInstance,
    display: egl::Display,
    raw: egl::Context,
    config: egl::Config,
    pbuffer: Option<egl::Surface>,
    srgb_kind: SrgbFrameBufferKind,
}

impl EglContext {
    fn make_current(&self) {
        self.instance
            .make_current(self.display, self.pbuffer, self.pbuffer, Some(self.raw))
            .unwrap();
    }
    fn unmake_current(&self) {
        self.instance
            .make_current(self.display, None, None, None)
            .unwrap();
    }
}

#[derive(Clone, Debug)]
struct Swapchain {
    surface: egl::Surface,
    extent: crate::Extent,
    info: crate::SurfaceInfo,
    swap_interval: i32,
}

unsafe impl Send for Swapchain {}
unsafe impl Sync for Swapchain {}

#[derive(Debug)]
pub struct PlatformFrame {
    swapchain: Swapchain,
    framebuf: glow::Framebuffer,
}

pub struct PlatformSurface {
    library: Option<libloading::Library>,
    window_handle: raw_window_handle::RawWindowHandle,
    swapchain: Option<Swapchain>,
}

pub(super) struct ContextInner {
    glow: glow::Context,
    egl: EglContext,
}

pub struct PlatformContext {
    inner: Mutex<ContextInner>,
}

pub struct ContextLock<'a> {
    guard: MutexGuard<'a, ContextInner>,
}
impl<'a> std::ops::Deref for ContextLock<'a> {
    type Target = glow::Context;
    fn deref(&self) -> &Self::Target {
        &self.guard.glow
    }
}
impl<'a> Drop for ContextLock<'a> {
    fn drop(&mut self) {
        self.guard.egl.unmake_current();
    }
}

impl super::Context {
    pub unsafe fn init(desc: crate::ContextDesc) -> Result<Self, crate::NotSupportedError> {
        let egl = unsafe {
            let egl_result = if cfg!(windows) {
                egl::DynamicInstance::<egl::EGL1_4>::load_required_from_filename("libEGL.dll")
            } else if cfg!(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "tvos"
            )) {
                egl::DynamicInstance::<egl::EGL1_4>::load_required_from_filename("libEGL.dylib")
            } else {
                egl::DynamicInstance::<egl::EGL1_4>::load_required()
            };
            egl_result.map_err(PlatformError::Loading)?
        };

        let client_extensions = match egl.query_string(None, egl::EXTENSIONS) {
            Ok(ext) => ext.to_string_lossy().into_owned(),
            Err(_) => String::new(),
        };
        log::debug!(
            "Client extensions: {:#?}",
            client_extensions.split_whitespace().collect::<Vec<_>>()
        );

        if desc.validation && client_extensions.contains("EGL_KHR_debug") {
            log::info!("Enabling EGL debug output");
            let function: EglDebugMessageControlFun = {
                let addr = egl.get_proc_address("eglDebugMessageControlKHR").unwrap();
                unsafe { std::mem::transmute(addr) }
            };
            let attributes = [
                EGL_DEBUG_MSG_CRITICAL_KHR as egl::Attrib,
                1,
                EGL_DEBUG_MSG_ERROR_KHR as egl::Attrib,
                1,
                EGL_DEBUG_MSG_WARN_KHR as egl::Attrib,
                1,
                EGL_DEBUG_MSG_INFO_KHR as egl::Attrib,
                1,
                egl::ATTRIB_NONE,
            ];
            unsafe { (function)(Some(egl_debug_proc), attributes.as_ptr()) };
        }

        let display = if let Some(egl1_5) = egl.upcast::<egl::EGL1_5>() {
            if client_extensions.contains("EGL_ANGLE_platform_angle") {
                log::info!("Using Angle");
                let display_attributes = [
                    EGL_PLATFORM_ANGLE_TYPE_ANGLE as egl::Attrib,
                    if cfg!(any(
                        target_os = "macos",
                        target_os = "ios",
                        target_os = "tvos",
                    )) {
                        EGL_PLATFORM_ANGLE_TYPE_METAL_ANGLE
                    } else {
                        EGL_PLATFORM_ANGLE_TYPE_DEFAULT_ANGLE
                    } as egl::Attrib,
                    EGL_PLATFORM_ANGLE_NATIVE_PLATFORM_TYPE_ANGLE as egl::Attrib,
                    EGL_PLATFORM_SURFACELESS_MESA as egl::Attrib,
                    EGL_PLATFORM_ANGLE_DEBUG_LAYERS_ENABLED as egl::Attrib,
                    if desc.validation { 1 } else { 0 },
                    egl::ATTRIB_NONE,
                ];
                egl1_5
                    .get_platform_display(
                        EGL_PLATFORM_ANGLE_ANGLE,
                        ptr::null_mut(),
                        &display_attributes,
                    )
                    .unwrap()
            } else if client_extensions.contains("EGL_MESA_platform_surfaceless") {
                log::info!("Using surfaceless platform");
                egl1_5
                    .get_platform_display(
                        EGL_PLATFORM_SURFACELESS_MESA,
                        ptr::null_mut(),
                        &[egl::ATTRIB_NONE],
                    )
                    .unwrap()
            } else {
                log::info!("EGL_MESA_platform_surfaceless not available. Using default platform");
                egl.get_display(egl::DEFAULT_DISPLAY).unwrap()
            }
        } else {
            egl.get_display(egl::DEFAULT_DISPLAY).unwrap()
        };

        let egl_context = EglContext::init(&desc, egl, display)?;
        egl_context.make_current();
        let (glow, capabilities, toggles, device_information, limits) =
            egl_context.load_functions(&desc);
        egl_context.unmake_current();

        Ok(Self {
            platform: PlatformContext {
                inner: Mutex::new(ContextInner {
                    glow,
                    egl: egl_context,
                }),
            },
            capabilities,
            toggles,
            limits,
            device_information,
        })
    }

    pub fn create_surface<I: raw_window_handle::HasWindowHandle>(
        &self,
        window: I,
    ) -> Result<super::Surface, crate::NotSupportedError> {
        use raw_window_handle::RawWindowHandle as Rwh;

        let window_handle = window.window_handle().unwrap().as_raw();
        let library = match window_handle {
            Rwh::Xlib(_) => Some(find_x_library().unwrap()),
            Rwh::Xcb(_) => Some(find_x_library().unwrap()),
            Rwh::Wayland(_) => Some(find_wayland_library().unwrap()),
            _ => None,
        };

        Ok(unsafe {
            let guard = self.lock();
            super::Surface {
                platform: PlatformSurface {
                    library,
                    window_handle,
                    swapchain: None,
                },
                renderbuf: guard.create_renderbuffer().unwrap(),
                framebuf: guard.create_framebuffer().unwrap(),
            }
        })
    }

    pub fn destroy_surface(&self, surface: &mut super::Surface) {
        use raw_window_handle::RawWindowHandle as Rwh;

        let inner = self.platform.inner.lock().unwrap();
        if let Some(s) = surface.platform.swapchain.take() {
            inner
                .egl
                .instance
                .destroy_surface(inner.egl.display, s.surface)
                .unwrap();
        }
        if let Rwh::Wayland(handle) = surface.platform.window_handle {
            unsafe {
                let wl_egl_window_destroy: libloading::Symbol<WlEglWindowDestroyFun> = surface
                    .platform
                    .library
                    .as_ref()
                    .unwrap()
                    .get(b"wl_egl_window_destroy")
                    .unwrap();
                wl_egl_window_destroy(handle.surface.as_ptr());
            }
        }
        inner.egl.make_current();
        unsafe {
            inner.glow.delete_renderbuffer(surface.renderbuf);
            inner.glow.delete_framebuffer(surface.framebuf);
        }
        inner.egl.unmake_current();
    }

    pub fn reconfigure_surface(&self, surface: &mut super::Surface, config: crate::SurfaceConfig) {
        use raw_window_handle::RawWindowHandle as Rwh;

        let (mut temp_xlib_handle, mut temp_xcb_handle);
        #[allow(trivial_casts)]
        let native_window_ptr = match surface.platform.window_handle {
            Rwh::Xlib(handle) if cfg!(windows) => handle.window as *mut ffi::c_void,
            Rwh::Xlib(handle) => {
                temp_xlib_handle = handle.window;
                &mut temp_xlib_handle as *mut _ as *mut ffi::c_void
            }
            Rwh::Xcb(handle) => {
                temp_xcb_handle = handle.window;
                &mut temp_xcb_handle as *mut _ as *mut ffi::c_void
            }
            Rwh::AndroidNdk(handle) => handle.a_native_window.as_ptr(),
            Rwh::Wayland(handle) => unsafe {
                let wl_egl_window_create: libloading::Symbol<WlEglWindowCreateFun> = surface
                    .platform
                    .library
                    .as_ref()
                    .unwrap()
                    .get(b"wl_egl_window_create")
                    .unwrap();
                wl_egl_window_create(
                    handle.surface.as_ptr(),
                    config.size.width as _,
                    config.size.height as _,
                ) as *mut _ as *mut ffi::c_void
            },
            Rwh::Win32(handle) => handle.hwnd.get() as *mut ffi::c_void,
            Rwh::AppKit(handle) => {
                #[cfg(not(target_os = "macos"))]
                let window_ptr = handle.ns_view.as_ptr();
                #[cfg(target_os = "macos")]
                let window_ptr = unsafe {
                    use objc2::{msg_send, runtime::Object};
                    // ns_view always have a layer and don't need to verify that it exists.
                    let layer: *mut Object =
                        msg_send![handle.ns_view.as_ptr() as *mut Object, layer];
                    layer as *mut ffi::c_void
                };
                window_ptr
            }
            other => {
                panic!("Unable to connect with RWH {:?}", other);
            }
        };

        if !config.allow_exclusive_full_screen {
            log::warn!("Unable to forbid exclusive full screen");
        }

        let inner = self.platform.inner.lock().unwrap();
        if let Some(s) = surface.platform.swapchain.take() {
            inner
                .egl
                .instance
                .destroy_surface(inner.egl.display, s.surface)
                .unwrap();
        }

        let mut attributes = vec![
            egl::RENDER_BUFFER,
            // We don't want any of the buffering done by the driver, because we
            // manage a swapchain on our side.
            // Some drivers just fail on surface creation seeing `EGL_SINGLE_BUFFER`.
            if cfg!(any(target_os = "android", target_os = "macos", windows)) {
                egl::BACK_BUFFER
            } else {
                egl::SINGLE_BUFFER
            },
        ];

        //TODO: detect if linear color space is supported
        match inner.egl.srgb_kind {
            SrgbFrameBufferKind::None => {}
            SrgbFrameBufferKind::Core | SrgbFrameBufferKind::Khr => {
                attributes.push(egl::GL_COLORSPACE);
                attributes.push(egl::GL_COLORSPACE_SRGB);
            }
        }
        let alpha = if config.transparent {
            crate::AlphaMode::PreMultiplied //TODO: verify
        } else {
            crate::AlphaMode::Ignored
        };
        attributes.push(egl::ATTRIB_NONE as i32);

        let format = match config.color_space {
            crate::ColorSpace::Linear => crate::TextureFormat::Rgba8UnormSrgb,
            crate::ColorSpace::Srgb => crate::TextureFormat::Rgba8Unorm,
        };

        surface.platform.swapchain = Some(Swapchain {
            // Careful, we can still be in 1.4 version even if `upcast` succeeds
            surface: match inner.egl.instance.upcast::<egl::EGL1_5>() {
                Some(egl) => {
                    let attributes_usize = attributes
                        .into_iter()
                        .map(|v| v as usize)
                        .collect::<Vec<_>>();
                    unsafe {
                        egl.create_platform_window_surface(
                            inner.egl.display,
                            inner.egl.config,
                            native_window_ptr,
                            &attributes_usize,
                        )
                        .unwrap()
                    }
                }
                _ => unsafe {
                    inner
                        .egl
                        .instance
                        .create_window_surface(
                            inner.egl.display,
                            inner.egl.config,
                            native_window_ptr,
                            Some(&attributes),
                        )
                        .unwrap()
                },
            },
            extent: config.size,
            info: crate::SurfaceInfo { format, alpha },
            swap_interval: match config.display_sync {
                crate::DisplaySync::Block => 1,
                crate::DisplaySync::Recent | crate::DisplaySync::Tear => 0,
            },
        });

        let format_desc = super::describe_texture_format(format);
        inner.egl.make_current();
        unsafe {
            let gl = &inner.glow;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(surface.renderbuf));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                format_desc.internal,
                config.size.width as _,
                config.size.height as _,
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(surface.framebuf));
            gl.framebuffer_renderbuffer(
                glow::READ_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(surface.renderbuf),
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        };
        inner.egl.unmake_current();
    }

    pub(super) fn lock(&self) -> ContextLock {
        let inner = self.platform.inner.lock().unwrap();
        inner.egl.make_current();
        ContextLock { guard: inner }
    }
}

impl PlatformContext {
    pub(super) fn present(&self, frame: PlatformFrame) {
        let sc = frame.swapchain;
        let inner = self.inner.lock().unwrap();
        inner
            .egl
            .instance
            .make_current(
                inner.egl.display,
                Some(sc.surface),
                Some(sc.surface),
                Some(inner.egl.raw),
            )
            .unwrap();
        inner
            .egl
            .instance
            .swap_interval(inner.egl.display, sc.swap_interval)
            .unwrap();

        unsafe {
            super::present_blit(&inner.glow, frame.framebuf, sc.extent);
        }

        inner
            .egl
            .instance
            .swap_buffers(inner.egl.display, sc.surface)
            .unwrap();
        inner
            .egl
            .instance
            .make_current(inner.egl.display, None, None, None)
            .unwrap();
    }
}

impl super::Surface {
    pub fn info(&self) -> crate::SurfaceInfo {
        self.platform.swapchain.as_ref().unwrap().info
    }

    pub fn acquire_frame(&mut self) -> super::Frame {
        let sc = self.platform.swapchain.as_ref().unwrap();
        super::Frame {
            platform: PlatformFrame {
                swapchain: sc.clone(),
                framebuf: self.framebuf,
            },
            texture: super::Texture {
                inner: super::TextureInner::Renderbuffer {
                    raw: self.renderbuf,
                },
                target_size: [sc.extent.width as u16, sc.extent.height as u16],
                format: sc.info.format,
            },
        }
    }
}

unsafe fn find_library(paths: &[&str]) -> Option<libloading::Library> {
    paths
        .iter()
        .find_map(|&path| libloading::Library::new(path).ok())
}
fn find_x_library() -> Option<libloading::Library> {
    unsafe { libloading::Library::new("libX11.so").ok() }
}
fn find_wayland_library() -> Option<libloading::Library> {
    unsafe { find_library(&["libwayland-egl.so.1", "libwayland-egl.so"]) }
}

unsafe extern "system" fn egl_debug_proc(
    error: egl::Enum,
    command_raw: *const raw::c_char,
    message_type: u32,
    _thread_label: EglLabel,
    _object_label: EglLabel,
    message_raw: *const raw::c_char,
) {
    let log_severity = match message_type {
        EGL_DEBUG_MSG_CRITICAL_KHR | EGL_DEBUG_MSG_ERROR_KHR => log::Level::Error,
        EGL_DEBUG_MSG_WARN_KHR => log::Level::Warn,
        EGL_DEBUG_MSG_INFO_KHR => log::Level::Info,
        _ => log::Level::Debug,
    };
    let command = unsafe { ffi::CStr::from_ptr(command_raw) }.to_string_lossy();
    let message = if message_raw.is_null() {
        "".into()
    } else {
        unsafe { ffi::CStr::from_ptr(message_raw) }.to_string_lossy()
    };

    log::log!(
        log_severity,
        "EGL '{}' code 0x{:x}: {}",
        command,
        error,
        message,
    );
}

const LOG_LEVEL_SEVERITY: &[(log::Level, u32)] = &[
    (log::Level::Error, glow::DEBUG_SEVERITY_HIGH),
    (log::Level::Warn, glow::DEBUG_SEVERITY_MEDIUM),
    (log::Level::Info, glow::DEBUG_SEVERITY_LOW),
    (log::Level::Trace, glow::DEBUG_SEVERITY_NOTIFICATION),
];

fn gl_debug_message_callback(source: u32, gltype: u32, id: u32, severity: u32, message: &str) {
    let source_str = match source {
        glow::DEBUG_SOURCE_API => "API",
        glow::DEBUG_SOURCE_WINDOW_SYSTEM => "Window System",
        glow::DEBUG_SOURCE_SHADER_COMPILER => "ShaderCompiler",
        glow::DEBUG_SOURCE_THIRD_PARTY => "Third Party",
        glow::DEBUG_SOURCE_APPLICATION => "Application",
        glow::DEBUG_SOURCE_OTHER => "Other",
        _ => unreachable!(),
    };

    let &(log_severity, _) = LOG_LEVEL_SEVERITY
        .iter()
        .find(|&&(_level, sev)| sev == severity)
        .unwrap();

    let type_str = match gltype {
        glow::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "Deprecated Behavior",
        glow::DEBUG_TYPE_ERROR => "Error",
        glow::DEBUG_TYPE_MARKER => "Marker",
        glow::DEBUG_TYPE_OTHER => "Other",
        glow::DEBUG_TYPE_PERFORMANCE => "Performance",
        glow::DEBUG_TYPE_POP_GROUP => "Pop Group",
        glow::DEBUG_TYPE_PORTABILITY => "Portability",
        glow::DEBUG_TYPE_PUSH_GROUP => "Push Group",
        glow::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "Undefined Behavior",
        _ => unreachable!(),
    };

    log::log!(
        log_severity,
        "GLES: [{}/{}] ID {} : {}",
        source_str,
        type_str,
        id,
        message
    );
}

impl EglContext {
    fn init(
        desc: &crate::ContextDesc,
        egl: EglInstance,
        display: egl::Display,
    ) -> Result<Self, crate::NotSupportedError> {
        let version = egl.initialize(display).map_err(PlatformError::Init)?;
        let vendor = egl.query_string(Some(display), egl::VENDOR).unwrap();
        let display_extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .unwrap()
            .to_string_lossy();
        log::info!("Display vendor {:?}, version {:?}", vendor, version,);
        log::debug!(
            "Display extensions: {:#?}",
            display_extensions.split_whitespace().collect::<Vec<_>>()
        );

        let srgb_kind = if version >= (1, 5) {
            log::info!("\tEGL surface: +srgb");
            SrgbFrameBufferKind::Core
        } else if display_extensions.contains("EGL_KHR_gl_colorspace") {
            log::info!("\tEGL surface: +srgb khr");
            SrgbFrameBufferKind::Khr
        } else {
            log::warn!("\tEGL surface: -srgb");
            SrgbFrameBufferKind::None
        };

        if log::max_level() >= log::LevelFilter::Trace {
            log::trace!("Configurations:");
            let config_count = egl.get_config_count(display).unwrap();
            let mut configurations = Vec::with_capacity(config_count);
            egl.get_configs(display, &mut configurations).unwrap();
            for &config in configurations.iter() {
                log::trace!("\tCONFORMANT=0x{:X}, RENDERABLE=0x{:X}, NATIVE_RENDERABLE=0x{:X}, SURFACE_TYPE=0x{:X}, ALPHA_SIZE={}",
                    egl.get_config_attrib(display, config, egl::CONFORMANT).unwrap(),
                    egl.get_config_attrib(display, config, egl::RENDERABLE_TYPE).unwrap(),
                    egl.get_config_attrib(display, config, egl::NATIVE_RENDERABLE).unwrap(),
                    egl.get_config_attrib(display, config, egl::SURFACE_TYPE).unwrap(),
                    egl.get_config_attrib(display, config, egl::ALPHA_SIZE).unwrap(),
                );
            }
        }

        let (config, _supports_presentation) = choose_config(&egl, display, srgb_kind)?;
        egl.bind_api(egl::OPENGL_ES_API).unwrap();

        let mut khr_context_flags = 0;
        let supports_khr_context = display_extensions.contains("EGL_KHR_create_context");

        //TODO: make it so `Device` == EGL Context
        let mut context_attributes = vec![
            egl::CONTEXT_CLIENT_VERSION,
            3, // Request GLES 3.0 or higher
        ];
        if desc.validation {
            if version >= (1, 5) {
                log::info!("\tEGL context: +debug");
                context_attributes.push(egl::CONTEXT_OPENGL_DEBUG);
                context_attributes.push(egl::TRUE as _);
            } else if supports_khr_context {
                log::info!("\tEGL context: +debug KHR");
                khr_context_flags |= EGL_CONTEXT_OPENGL_DEBUG_BIT_KHR;
            } else {
                log::info!("\tEGL context: -debug");
            }
        }
        if khr_context_flags != 0 {
            context_attributes.push(EGL_CONTEXT_FLAGS_KHR);
            context_attributes.push(khr_context_flags);
        }
        context_attributes.push(egl::NONE);
        let context = match egl.create_context(display, config, None, &context_attributes) {
            Ok(context) => context,
            Err(e) => {
                log::warn!("unable to create GLES 3.x context: {:?}", e);
                return Err(PlatformError::Init(e).into());
            }
        };

        // Testing if context can be binded without surface
        // and creating dummy pbuffer surface if not.
        let pbuffer =
            if version >= (1, 5) || display_extensions.contains("EGL_KHR_surfaceless_context") {
                log::info!("\tEGL context: +surfaceless");
                None
            } else {
                let attributes = [egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE];
                egl.create_pbuffer_surface(display, config, &attributes)
                    .map(Some)
                    .map_err(|e| {
                        log::warn!("Error in create_pbuffer_surface: {:?}", e);
                        PlatformError::Init(e)
                    })?
            };

        Ok(Self {
            instance: egl,
            display,
            raw: context,
            config,
            pbuffer,
            srgb_kind,
        })
    }

    unsafe fn load_functions(
        &self,
        desc: &crate::ContextDesc,
    ) -> (
        glow::Context,
        super::Capabilities,
        super::Toggles,
        crate::DeviceInformation,
        super::Limits,
    ) {
        let mut gl = glow::Context::from_loader_function(|name| {
            self.instance
                .get_proc_address(name)
                .map_or(ptr::null(), |p| p as *const _)
        });
        if desc.validation {
            if gl.supports_debug() {
                log::info!("Enabling GLES debug output");
                gl.enable(glow::DEBUG_OUTPUT);
                gl.debug_message_callback(gl_debug_message_callback);
                for &(level, severity) in LOG_LEVEL_SEVERITY.iter() {
                    gl.debug_message_control(
                        glow::DONT_CARE,
                        glow::DONT_CARE,
                        severity,
                        &[],
                        level <= log::max_level(),
                    );
                }
            } else {
                log::warn!("Can't enable validation");
            }
        }

        let extensions = gl.supported_extensions();
        log::debug!("Extensions: {:#?}", extensions);
        let vendor = gl.get_parameter_string(glow::VENDOR);
        let renderer = gl.get_parameter_string(glow::RENDERER);
        let version = gl.get_parameter_string(glow::VERSION);
        log::info!("Vendor: {}", vendor);
        log::info!("Renderer: {}", renderer);
        log::info!("Version: {}", version);
        let device_information = crate::DeviceInformation {
            is_software_emulated: false,
            device_name: vendor,
            driver_name: renderer,
            driver_info: version,
        };

        let mut capabilities = super::Capabilities::empty();
        capabilities.set(
            super::Capabilities::BUFFER_STORAGE,
            extensions.contains("GL_EXT_buffer_storage"),
        );
        capabilities.set(
            super::Capabilities::DRAW_BUFFERS_INDEXED,
            if gl.version().is_embedded {
                (gl.version().major, gl.version().minor) >= (3, 2)
            } else {
                (gl.version().major, gl.version().minor) >= (3, 0)
            },
            // glow uses unsuffixed functions like glEnablei instead of glEnableiEXT.
            // Therefore, GL_EXT_draw_buffers_indexed is not sufficient.
        );

        let toggles = super::Toggles {
            scoping: desc.capture
                && (gl.supports_debug() || {
                    log::warn!("Scoping is not supported");
                    false
                }),
            timing: desc.timing
                && (extensions.contains("GL_EXT_disjoint_timer_query") || {
                    log::warn!("Timing is not supported");
                    false
                }),
        };

        let limits = super::Limits {
            uniform_buffer_alignment: gl.get_parameter_i32(glow::UNIFORM_BUFFER_OFFSET_ALIGNMENT)
                as u32,
        };
        (gl, capabilities, toggles, device_information, limits)
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        if let Err(e) = self.instance.destroy_context(self.display, self.raw) {
            log::warn!("Error in destroy_context: {:?}", e);
        }
        if let Err(e) = self.instance.terminate(self.display) {
            log::warn!("Error in terminate: {:?}", e);
        }
    }
}

/// Choose GLES framebuffer configuration.
fn choose_config(
    egl: &EglInstance,
    display: egl::Display,
    srgb_kind: SrgbFrameBufferKind,
) -> Result<(egl::Config, bool), crate::NotSupportedError> {
    //TODO: EGL_SLOW_CONFIG
    let tiers = [
        (
            "off-screen",
            &[
                egl::SURFACE_TYPE,
                egl::PBUFFER_BIT,
                egl::RENDERABLE_TYPE,
                egl::OPENGL_ES2_BIT,
            ][..],
        ),
        ("presentation", &[egl::SURFACE_TYPE, egl::WINDOW_BIT][..]),
        #[cfg(not(target_os = "android"))]
        (
            "native-render",
            &[egl::NATIVE_RENDERABLE, egl::TRUE as _][..],
        ),
    ];

    let mut attributes = Vec::with_capacity(9);
    for tier_max in (0..tiers.len()).rev() {
        let name = tiers[tier_max].0;
        log::info!("\tTrying {}", name);

        attributes.clear();
        for &(_, tier_attr) in tiers[..=tier_max].iter() {
            attributes.extend_from_slice(tier_attr);
        }
        // make sure the Alpha is enough to support sRGB
        match srgb_kind {
            SrgbFrameBufferKind::None => {}
            _ => {
                attributes.push(egl::ALPHA_SIZE);
                attributes.push(8);
            }
        }
        attributes.push(egl::NONE);

        match egl.choose_first_config(display, &attributes) {
            Ok(Some(config)) => {
                return Ok((config, tier_max >= 1));
            }
            Ok(None) => {
                log::warn!("No config found!");
            }
            Err(e) => {
                log::error!("error in choose_first_config: {:?}", e);
            }
        }
    }

    Err(crate::NotSupportedError::NoSupportedDeviceFound)
}
