use gpui::{Bounds, Pixels, SharedString};

#[allow(dead_code)]
pub(crate) trait PlatformWebView {
    fn set_bounds(&self, bounds: Bounds<Pixels>);
    fn set_visible(&self, visible: bool);
    fn load_url(&self, url: &str);
    fn load_html(&self, html: &str);
    fn go_back(&self);
    fn go_forward(&self);
    fn reload(&self);
    fn stop_loading(&self);
    fn evaluate_javascript(
        &self,
        script: &str,
        callback: Option<Box<dyn FnOnce(Result<String, String>) + Send>>,
    );
    fn title(&self) -> SharedString;
    fn url(&self) -> SharedString;
    fn set_devtools_enabled(&self, enabled: bool);
    fn focus(&self);
}

#[cfg(feature = "webview")]
mod wry_backend;

#[cfg(feature = "webview")]
pub(crate) use wry_backend::*;

#[cfg(not(feature = "webview"))]
mod stub;

#[cfg(not(feature = "webview"))]
pub(crate) use stub::*;
