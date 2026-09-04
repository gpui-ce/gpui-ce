use gpui::SharedString;
use std::rc::Rc;

use crate::platform::PlatformWebView;

/// A handle for controlling a [`WebView`] after creation.
///
/// Obtained via the `on_create_handle` callback or by querying the element tree.
/// This handle is cloneable and can be used to control the webview from anywhere
/// in your application.
#[derive(Clone)]
pub struct WebViewHandle {
    inner: Rc<dyn PlatformWebView>,
}

impl WebViewHandle {
    pub(crate) fn new(inner: Rc<dyn PlatformWebView>) -> Self {
        Self { inner }
    }

    /// Navigate to a URL.
    pub fn load_url(&self, url: &str) {
        self.inner.load_url(url);
    }

    /// Navigate to a data URL with HTML content.
    pub fn load_html(&self, html: &str) {
        self.inner.load_html(html);
    }

    /// Navigate back.
    pub fn go_back(&self) {
        self.inner.go_back();
    }

    /// Navigate forward.
    pub fn go_forward(&self) {
        self.inner.go_forward();
    }

    /// Reload the current page.
    pub fn reload(&self) {
        self.inner.reload();
    }

    /// Stop loading.
    pub fn stop_loading(&self) {
        self.inner.stop_loading();
    }

    /// Execute JavaScript in the webview.
    pub fn evaluate_javascript(
        &self,
        script: &str,
        callback: Option<Box<dyn FnOnce(Result<String, String>) + Send>>,
    ) {
        self.inner.evaluate_javascript(script, callback);
    }

    /// Get the current page title.
    pub fn title(&self) -> SharedString {
        self.inner.title()
    }

    /// Get the current page URL.
    pub fn url(&self) -> SharedString {
        self.inner.url()
    }

    /// Focus the webview.
    pub fn focus(&self) {
        self.inner.focus();
    }
}
