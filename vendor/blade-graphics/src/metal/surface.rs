use objc2::rc::Retained;
use objc2::ClassType;
use objc2_core_foundation::CGSize;
use objc2_quartz_core::CAMetalLayer;

const SURFACE_INFO: crate::SurfaceInfo = crate::SurfaceInfo {
    format: crate::TextureFormat::Rgba8Unorm,
    alpha: crate::AlphaMode::Ignored,
};

impl super::Surface {
    /// Get the CALayerMetal for this surface, if any.
    /// This is platform specific API.
    pub fn metal_layer(&self) -> Retained<CAMetalLayer> {
        self.render_layer.clone()
    }

    pub fn info(&self) -> crate::SurfaceInfo {
        self.info
    }

    pub fn acquire_frame(&self) -> super::Frame {
        use objc2_quartz_core::CAMetalDrawable as _;
        let (drawable, texture) = objc2::rc::autoreleasepool(|_| unsafe {
            let drawable = self.render_layer.nextDrawable().unwrap();
            let texture = drawable.texture();
            (Retained::cast_unchecked(drawable), texture)
        });
        super::Frame { drawable, texture }
    }
}

impl super::Context {
    pub fn create_surface<I: raw_window_handle::HasWindowHandle>(
        &self,
        window: &I,
    ) -> Result<super::Surface, crate::NotSupportedError> {
        use objc2_foundation::NSObjectProtocol as _;

        Ok(match window.window_handle().unwrap().as_raw() {
            #[cfg(target_os = "ios")]
            raw_window_handle::RawWindowHandle::UiKit(handle) => unsafe {
                let view =
                    Retained::retain(handle.ui_view.as_ptr() as *mut objc2_ui_kit::UIView).unwrap();
                let main_layer = view.layer();
                let render_layer = if main_layer.isKindOfClass(&CAMetalLayer::class()) {
                    Retained::cast_unchecked(main_layer)
                } else {
                    use objc2_ui_kit::UIViewAutoresizing as Var;
                    let new_layer = CAMetalLayer::new();
                    new_layer.setFrame(main_layer.frame());
                    // Unlike NSView, UIView does not allow to replace main layer.
                    main_layer.addSublayer(&new_layer);
                    view.setAutoresizingMask(
                        Var::FlexibleLeftMargin
                            | Var::FlexibleWidth
                            | Var::FlexibleRightMargin
                            | Var::FlexibleTopMargin
                            | Var::FlexibleHeight
                            | Var::FlexibleBottomMargin,
                    );
                    if let Some(scene) = view.window().and_then(|w| w.windowScene()) {
                        new_layer.setContentsScale(scene.screen().nativeScale());
                    }
                    new_layer
                };
                super::Surface {
                    view: Some(Retained::cast_unchecked(view)),
                    render_layer,
                    info: SURFACE_INFO,
                }
            },
            #[cfg(target_os = "macos")]
            raw_window_handle::RawWindowHandle::AppKit(handle) => unsafe {
                let view = Retained::retain(handle.ns_view.as_ptr() as *mut objc2_app_kit::NSView)
                    .unwrap();
                let main_layer = view.layer();
                let render_layer = if main_layer
                    .as_ref()
                    .map_or(false, |layer| layer.isKindOfClass(&CAMetalLayer::class()))
                {
                    Retained::cast_unchecked(main_layer.unwrap())
                } else {
                    let new_layer = CAMetalLayer::new();
                    if let Some(layer) = main_layer {
                        new_layer.setFrame(layer.frame());
                    }
                    view.setLayer(Some(&new_layer));
                    view.setWantsLayer(true);
                    new_layer.setContentsGravity(objc2_quartz_core::kCAGravityTopLeft);
                    if let Some(window) = view.window() {
                        new_layer.setContentsScale(window.backingScaleFactor());
                    }
                    new_layer
                };
                super::Surface {
                    view: Some(view.downcast().unwrap()),
                    render_layer,
                    info: SURFACE_INFO,
                }
            },
            _ => return Err(crate::NotSupportedError::PlatformNotSupported),
        })
    }

    pub fn destroy_surface(&self, surface: &mut super::Surface) {
        surface.view = None;
    }

    pub fn reconfigure_surface(&self, surface: &mut super::Surface, config: crate::SurfaceConfig) {
        let device = self.device.lock().unwrap();
        surface.info = crate::SurfaceInfo {
            format: match config.color_space {
                crate::ColorSpace::Linear => crate::TextureFormat::Bgra8UnormSrgb,
                crate::ColorSpace::Srgb => crate::TextureFormat::Bgra8Unorm,
            },
            alpha: if config.transparent {
                crate::AlphaMode::PostMultiplied
            } else {
                //Warning: it's not really ignored! Instead, it's assumed to be 1:
                // https://developer.apple.com/documentation/quartzcore/calayer/1410763-isopaque
                crate::AlphaMode::Ignored
            },
        };
        let vsync = match config.display_sync {
            crate::DisplaySync::Block => true,
            crate::DisplaySync::Recent | crate::DisplaySync::Tear => false,
        };

        unsafe {
            surface.render_layer.setOpaque(!config.transparent);
            surface.render_layer.setDevice(Some(device.as_ref()));
            surface
                .render_layer
                .setPixelFormat(super::map_texture_format(surface.info.format));
            surface
                .render_layer
                .setFramebufferOnly(config.usage == crate::TextureUsage::TARGET);
            surface.render_layer.setMaximumDrawableCount(3);
            surface.render_layer.setDrawableSize(CGSize {
                width: config.size.width as f64,
                height: config.size.height as f64,
            });
            surface.render_layer.setDisplaySyncEnabled(vsync);
        }
    }
}
