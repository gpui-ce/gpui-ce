use glow::HasContext as _;
use wasm_bindgen::JsCast;

pub struct PlatformContext {
    #[allow(unused)]
    webgl2: web_sys::WebGl2RenderingContext,
    glow: glow::Context,
}

pub struct PlatformSurface {
    info: crate::SurfaceInfo,
    extent: crate::Extent,
}
#[derive(Debug)]
pub struct PlatformFrame {
    framebuf: glow::Framebuffer,
    extent: crate::Extent,
}

pub type PlatformError = ();

impl super::Surface {
    pub fn info(&self) -> crate::SurfaceInfo {
        self.platform.info
    }
    pub fn acquire_frame(&self) -> super::Frame {
        let size = self.platform.extent;
        super::Frame {
            platform: PlatformFrame {
                framebuf: self.framebuf,
                extent: self.platform.extent,
            },
            texture: super::Texture {
                inner: super::TextureInner::Renderbuffer {
                    raw: self.renderbuf,
                },
                target_size: [size.width as u16, size.height as u16],
                format: self.platform.info.format,
            },
        }
    }
}

impl PlatformContext {
    pub(super) fn present(&self, frame: PlatformFrame) {
        unsafe {
            super::present_blit(&self.glow, frame.framebuf, frame.extent);
        }
    }
}

impl super::Context {
    pub unsafe fn init(_desc: crate::ContextDesc) -> Result<Self, crate::NotSupportedError> {
        let canvas = web_sys::window()
            .and_then(|win| win.document())
            .expect("Cannot get document")
            .get_element_by_id("blade")
            .expect("Canvas is not found")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("Failed to downcast to canvas type");

        let context_options = js_sys::Object::new();
        js_sys::Reflect::set(
            &context_options,
            &"antialias".into(),
            &wasm_bindgen::JsValue::FALSE,
        )
        .expect("Cannot create context options");
        //Note: could also set: "alpha", "premultipliedAlpha"

        let webgl2 = canvas
            .get_context_with_context_options("webgl2", &context_options)
            .expect("Cannot create WebGL2 context")
            .and_then(|context| context.dyn_into::<web_sys::WebGl2RenderingContext>().ok())
            .expect("Cannot convert into WebGL2 context");

        let glow = glow::Context::from_webgl2_context(webgl2.clone());

        let capabilities = super::Capabilities::empty();
        let limits = super::Limits {
            uniform_buffer_alignment: unsafe {
                glow.get_parameter_i32(glow::UNIFORM_BUFFER_OFFSET_ALIGNMENT) as u32
            },
        };
        let device_information = crate::DeviceInformation {
            is_software_emulated: false,
            device_name: glow.get_parameter_string(glow::VENDOR),
            driver_name: glow.get_parameter_string(glow::RENDERER),
            driver_info: glow.get_parameter_string(glow::VERSION),
        };

        Ok(super::Context {
            platform: PlatformContext { webgl2, glow },
            capabilities,
            toggles: super::Toggles::default(),
            limits,
            device_information,
        })
    }

    pub fn create_surface<I>(
        &self,
        _window: &I,
    ) -> Result<super::Surface, crate::NotSupportedError> {
        let platform = PlatformSurface {
            info: crate::SurfaceInfo {
                format: crate::TextureFormat::Rgba8Unorm,
                alpha: crate::AlphaMode::PreMultiplied,
            },
            extent: crate::Extent::default(),
        };
        Ok(unsafe {
            super::Surface {
                platform,
                renderbuf: self.platform.glow.create_renderbuffer().unwrap(),
                framebuf: self.platform.glow.create_framebuffer().unwrap(),
            }
        })
    }

    pub fn destroy_surface(&self, _surface: &mut super::Surface) {}

    pub fn reconfigure_surface(&self, surface: &mut super::Surface, config: crate::SurfaceConfig) {
        //TODO: create WebGL context here
        let format_desc = super::describe_texture_format(surface.platform.info.format);
        let gl = &self.platform.glow;
        //Note: this code can be shared with EGL
        unsafe {
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
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        }
        surface.platform.extent = config.size;
    }

    /// Obtain a lock to the EGL context and get handle to the [`glow::Context`] that can be used to
    /// do rendering.
    pub(super) fn lock(&self) -> &glow::Context {
        &self.platform.glow
    }
}
