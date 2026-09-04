use super::PlatformWebView;
use crate::webview::WebViewConfig;
use gpui::{App, Bounds, Pixels, SharedString, Window};

pub(crate) struct StubWebView;

impl StubWebView {
    pub fn new(_window: &Window, _config: &WebViewConfig) -> Self {
        panic!("gpui_webview: WebView requires the 'webview' feature to be enabled")
    }
}

impl PlatformWebView for StubWebView {
    fn set_bounds(&self, _: Bounds<Pixels>) {}
    fn set_visible(&self, _: bool) {}
    fn load_url(&self, _: &str) {}
    fn load_html(&self, _: &str) {}
    fn go_back(&self) {}
    fn go_forward(&self) {}
    fn reload(&self) {}
    fn stop_loading(&self) {}
    fn evaluate_javascript(
        &self,
        _: &str,
        _: Option<Box<dyn FnOnce(Result<String, String>) + Send>>,
    ) {
    }
    fn title(&self) -> SharedString {
        SharedString::default()
    }
    fn url(&self) -> SharedString {
        SharedString::default()
    }
    fn set_devtools_enabled(&self, _: bool) {}
    fn focus(&self) {}
}

pub(crate) fn create_platform_webview(
    window: &Window,
    config: &WebViewConfig,
    _cx: &mut App,
) -> Box<dyn PlatformWebView> {
    Box::new(StubWebView::new(window, config))
}
