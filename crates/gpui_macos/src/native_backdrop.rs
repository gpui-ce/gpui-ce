//! Native backdrop blur beneath the scene overlay.
//!
//! A backdrop filter painted in the overlay plane is blurred by Metal from that plane's own
//! contents, which are transparent wherever nothing else was painted in the overlay. The base
//! plane and the native child views it should frost are separate layers that Metal cannot
//! sample. So each overlay backdrop filter is mirrored by a `GPUIBackdropView` placed directly
//! beneath the overlay view, whose layer blurs the window's live composited content behind it.
//! The Metal blur still runs, so overlay content painted beneath a filter (one popover over
//! another) is blurred too.
//!
//! Sampling composited content requires `CABackdropLayer` and `CAFilter`, private Core
//! Animation classes that `NSVisualEffectView` is built on. Public within-window effect views
//! are unsuitable: they may blur a cached image of the view hierarchy, which omits layer-hosted
//! content such as a web view, and they apply a material tint the application cannot control.
//! This module is the only user of that private API. If the classes are unavailable, overlay
//! backdrop filters blur overlay content only.

use std::ffi::CString;
use std::ptr::{self, NonNull};

use gpui::BackdropFilter;
use objc2::{
    class, msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
    sel,
};
use objc2_app_kit::NSWindowOrderingMode;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

type ObjcId = *mut AnyObject;
const NIL: ObjcId = ptr::null_mut();

/// Downsampling applied before blurring; a heavy blur hides the lost resolution.
const BACKDROP_SAMPLE_SCALE: f64 = 0.25;

static mut BACKDROP_VIEW_CLASS: *const AnyClass = ptr::null();

/// Registers `GPUIBackdropView`. Leaves native backdrops disabled when Core Animation does not
/// provide the classes they need.
///
/// # Safety
///
/// Must be called once, before any window is created, like the crate's other class
/// registrations.
pub(crate) unsafe fn register_view_class() {
    if AnyClass::get(c"CABackdropLayer").is_none() || AnyClass::get(c"CAFilter").is_none() {
        log::warn!(
            "CABackdropLayer is unavailable; overlay backdrop filters will not blur native content"
        );
        return;
    }
    let mut decl = ClassBuilder::new(
        CString::new("GPUIBackdropView").unwrap().as_c_str(),
        class!(NSView),
    )
    .unwrap();
    unsafe {
        decl.add_method(
            sel!(makeBackingLayer),
            make_backing_layer as unsafe extern "C" fn(_, _) -> _,
        );
        decl.add_method(
            sel!(hitTest:),
            hit_test as unsafe extern "C" fn(_, _, _) -> _,
        );
        BACKDROP_VIEW_CLASS = decl.register() as *const AnyClass;
    }
}

unsafe extern "C" fn make_backing_layer(_: &AnyObject, _: Sel) -> ObjcId {
    unsafe {
        let layer: ObjcId = msg_send![class!(CABackdropLayer), layer];
        let _: () = msg_send![layer, setAllowsGroupBlending: Bool::YES];
        let _: () = msg_send![layer, setAllowsGroupOpacity: Bool::YES];
        let _: () = msg_send![layer, setIgnoresOffscreenGroups: Bool::YES];
        let _: () = msg_send![layer, setAllowsInPlaceFiltering: Bool::NO];
        let _: () = msg_send![layer, setScale: BACKDROP_SAMPLE_SCALE];
        layer
    }
}

/// Backdrops are purely visual; pointer events pass through to whatever is beneath.
unsafe extern "C" fn hit_test(_: &AnyObject, _: Sel, _: NSPoint) -> ObjcId {
    NIL
}

struct BackdropView {
    /// Owned by its superview.
    view: NonNull<AnyObject>,
    /// The gaussian radius currently installed on the layer, in points.
    blur_radius: f64,
}

/// The backdrop views mirroring the current overlay's backdrop filters, in paint order.
#[derive(Default)]
pub(crate) struct NativeBackdrops {
    views: Vec<BackdropView>,
}

impl NativeBackdrops {
    /// Matches the backdrop views to `filters`, creating views directly beneath `overlay_view`
    /// in `parent` and removing views that are no longer needed (all of them, when `filters`
    /// is empty).
    ///
    /// `parent` is the unflipped `GPUIView`, `view_height` its height in points, and filter
    /// geometry is in device pixels at `scale_factor`.
    ///
    /// # Safety
    ///
    /// Must be called on the main thread with `overlay_view` a subview of `parent`.
    pub(crate) unsafe fn sync(
        &mut self,
        parent: ObjcId,
        overlay_view: ObjcId,
        filters: &[BackdropFilter],
        view_height: f64,
        scale_factor: f32,
    ) {
        let class = unsafe { BACKDROP_VIEW_CLASS };
        if class.is_null() {
            return;
        }
        while self.views.len() > filters.len() {
            let backdrop = self.views.pop().expect("length checked");
            unsafe {
                let _: () = msg_send![backdrop.view.as_ptr(), removeFromSuperview];
            }
        }

        let scale_factor = scale_factor as f64;
        for (index, filter) in filters.iter().enumerate() {
            unsafe {
                if index == self.views.len() {
                    let view: ObjcId = msg_send![&*class, alloc];
                    let zero = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
                    let view: ObjcId = msg_send![view, initWithFrame: zero];
                    let Some(view) = NonNull::new(view) else {
                        return;
                    };
                    let _: () = msg_send![view.as_ptr(), setWantsLayer: Bool::YES];
                    // Directly beneath the overlay, so above every earlier backdrop: paint order.
                    let _: () = msg_send![
                        parent,
                        addSubview: view.as_ptr(),
                        positioned: NSWindowOrderingMode::Below,
                        relativeTo: overlay_view
                    ];
                    let _: () = msg_send![view.as_ptr(), release];
                    self.views.push(BackdropView {
                        view,
                        blur_radius: -1.,
                    });
                }
                self.views[index].apply(filter, view_height, scale_factor);
            }
        }
    }
}

impl BackdropView {
    unsafe fn apply(&mut self, filter: &BackdropFilter, view_height: f64, scale_factor: f64) {
        let view = self.view.as_ptr();
        let bounds = filter.bounds.intersect(&filter.content_mask.bounds);
        let width = (bounds.size.width.0 as f64 / scale_factor).max(0.);
        let height = (bounds.size.height.0 as f64 / scale_factor).max(0.);
        let x = bounds.origin.x.0 as f64 / scale_factor;
        let y = view_height - bounds.origin.y.0 as f64 / scale_factor - height;
        let blur_radius = filter.max_blur_radius() as f64 / scale_factor;
        unsafe {
            let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
            let _: () = msg_send![view, setFrame: frame];
            let _: () = msg_send![view, setAlphaValue: filter.opacity as f64];

            let layer: ObjcId = msg_send![view, layer];
            if layer.is_null() {
                return;
            }
            // Core Animation rounds all corners alike; use the top-left radius.
            let corner_radius = filter.corner_radii.top_left.0 as f64 / scale_factor;
            let _: () = msg_send![layer, setCornerRadius: corner_radius];
            let _: () = msg_send![layer, setMasksToBounds: Bool::YES];
            let supports_corner_curve: Bool =
                msg_send![layer, respondsToSelector: sel!(setCornerCurve:)];
            if supports_corner_curve.as_bool() {
                let curve = NSString::from_str(if filter.corner_smoothing > 0. {
                    "continuous"
                } else {
                    "circular"
                });
                let _: () = msg_send![layer, setCornerCurve: &*curve];
            }

            if self.blur_radius != blur_radius {
                // Layer filters are copied into the render tree when assigned, so changing a
                // value inside the installed filter would not take effect. Install a new one.
                let blur: ObjcId = msg_send![
                    class!(CAFilter),
                    filterWithType: &*NSString::from_str("gaussianBlur")
                ];
                let radius: ObjcId = msg_send![class!(NSNumber), numberWithDouble: blur_radius];
                let normalize_edges: ObjcId =
                    msg_send![class!(NSNumber), numberWithBool: Bool::YES];
                let _: () = msg_send![
                    blur,
                    setValue: radius,
                    forKey: &*NSString::from_str("inputRadius")
                ];
                let _: () = msg_send![
                    blur,
                    setValue: normalize_edges,
                    forKey: &*NSString::from_str("inputNormalizeEdges")
                ];
                let filters: ObjcId = msg_send![class!(NSArray), arrayWithObject: blur];
                let _: () = msg_send![layer, setFilters: filters];
                self.blur_radius = blur_radius;
            }
        }
    }
}
