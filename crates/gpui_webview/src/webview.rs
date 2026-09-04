use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable, SharedString, Style, StyleRefinement, Styled, Window,
};
use std::rc::Rc;

use crate::platform::{self, PlatformWebView};
use crate::webview_handle::WebViewHandle;

/// Configuration for creating a platform webview.
#[allow(dead_code)]
pub(crate) struct WebViewConfig {
    pub url: Option<String>,
    pub html: Option<String>,
    pub transparent: bool,
    pub devtools: bool,
    pub zoom: f32,
    pub bounds: Bounds<Pixels>,
    pub initial_title: Option<SharedString>,
}

impl Default for WebViewConfig {
    fn default() -> Self {
        Self {
            url: None,
            html: None,
            transparent: false,
            devtools: false,
            zoom: 1.0,
            bounds: Bounds::default(),
            initial_title: None,
        }
    }
}

/// Persistent state for a `WebView` element, stored across frames.
pub(crate) struct WebViewState {
    pub(crate) platform_webview: Rc<dyn PlatformWebView>,
    pub(crate) last_bounds: Bounds<Pixels>,
    pub(crate) last_known_url: SharedString,
}

/// A cross-platform native webview element for GPUI.
///
/// `WebView` renders a native webview (WebView2 on Windows, WKWebView on macOS)
/// as a child view embedded inside GPUI's layout system.
///
/// # Example
///
/// ```ignore
/// use gpui_webview::WebView;
///
/// fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///     div().child(
///         WebView::new("my-webview")
///             .url("https://example.com")
///             .w(px(800))
///             .h(px(600))
///     )
/// }
/// ```
pub struct WebView {
    id: SharedString,
    config: WebViewConfig,
    on_navigation: Option<Box<dyn FnMut(&str) -> bool>>,
    on_page_load: Option<Box<dyn FnMut(&str)>>,
    on_url_changed: Option<Box<dyn FnMut(&str, &mut Window, &mut App)>>,
    on_create_handle: Option<Box<dyn FnMut(WebViewHandle, &mut Window, &mut App)>>,
    style: StyleRefinement,
}

impl WebView {
    /// Create a new WebView element with a unique ID.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            config: WebViewConfig::default(),
            on_navigation: None,
            on_page_load: None,
            on_url_changed: None,
            on_create_handle: None,
            style: StyleRefinement::default(),
        }
    }

    /// Set the initial URL to load.
    pub fn url(mut self, url: &str) -> Self {
        self.config.url = Some(url.to_string());
        self
    }

    /// Set initial HTML content.
    pub fn html(mut self, html: &str) -> Self {
        self.config.html = Some(html.to_string());
        self
    }

    /// Set whether the webview background is transparent.
    pub fn transparent(mut self, transparent: bool) -> Self {
        self.config.transparent = transparent;
        self
    }

    /// Set whether the webview is dev-tools-enabled.
    pub fn devtools(mut self, enabled: bool) -> Self {
        self.config.devtools = enabled;
        self
    }

    /// Set zoom factor.
    pub fn zoom(mut self, factor: f32) -> Self {
        self.config.zoom = factor;
        self
    }

    /// Callback fired when navigation is attempted. Return false to block.
    pub fn on_navigation(mut self, callback: impl FnMut(&str) -> bool + 'static) -> Self {
        self.on_navigation = Some(Box::new(callback));
        self
    }

    /// Callback fired when a page finishes loading.
    pub fn on_page_load(mut self, callback: impl FnMut(&str) + 'static) -> Self {
        self.on_page_load = Some(Box::new(callback));
        self
    }

    /// Callback fired whenever the webview's main-frame URL changes.
    ///
    /// The URL is polled each frame and the callback is invoked with the new
    /// URL. This is useful for keeping an address bar in sync with internal
    /// page navigation (link clicks, redirects, etc.).
    pub fn on_url_changed(
        mut self,
        callback: impl FnMut(&str, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_url_changed = Some(Box::new(callback));
        self
    }

    /// Callback fired when the webview is first created, providing a handle.
    pub fn on_create_handle(
        mut self,
        callback: impl FnMut(WebViewHandle, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_create_handle = Some(Box::new(callback));
        self
    }

    fn element_id(&self) -> ElementId {
        ElementId::from(self.id.clone())
    }
}

impl IntoElement for WebView {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WebView {
    type RequestLayoutState = Style;
    type PrepaintState = Option<WebViewHandle>;

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        window.with_optional_element_state::<WebViewState, Option<WebViewHandle>>(
            id,
            |state, window| {
                let is_first_creation = !matches!(state, Some(Some(_)));
                let mut webview_state = match state {
                    Some(Some(ref prev)) => {
                        // Update bounds if changed
                        if prev.last_bounds != bounds {
                            prev.platform_webview.set_bounds(bounds);
                        }
                        WebViewState {
                            platform_webview: prev.platform_webview.clone(),
                            last_bounds: bounds,
                            last_known_url: prev.last_known_url.clone(),
                        }
                    }
                    _ => {
                        // First prepaint: create the native webview
                        self.config.bounds = bounds;
                        let platform_webview =
                            platform::create_platform_webview(window, &self.config, cx);
                        let rc: Rc<dyn PlatformWebView> = Rc::from(platform_webview);
                        WebViewState {
                            platform_webview: rc,
                            last_bounds: bounds,
                            last_known_url: SharedString::default(),
                        }
                    }
                };

                let handle = WebViewHandle::new(webview_state.platform_webview.clone());

                // Detect main-frame URL changes (e.g. internal navigation) and
                // notify observers. `url()` reports the top-level document URL,
                // so sub-frame navigations don't trigger spurious updates.
                let current_url = webview_state.platform_webview.url();
                if current_url != webview_state.last_known_url {
                    let changed_url = current_url.clone();
                    webview_state.last_known_url = current_url;
                    // Ignore the transient blank state right after creation.
                    if !changed_url.is_empty()
                        && changed_url != "about:blank"
                        && let Some(ref mut callback) = self.on_url_changed
                    {
                        callback(&changed_url, window, cx);
                        // Callbacks fired from prepaint run mid-draw, where
                        // entity-change notifications can't schedule a
                        // redraw, so request one explicitly.
                        window.defer(cx, |window, _cx| window.refresh());
                    }
                }

                // Fire the on_create_handle callback on first creation
                if is_first_creation && let Some(ref mut callback) = self.on_create_handle {
                    callback(handle.clone(), window, cx);
                }

                (Some(handle), Some(webview_state))
            },
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        // Native webview renders itself via the OS compositor.
    }
}

impl Styled for WebView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Shorthand for creating a [`WebView`] element.
pub fn webview(id: impl Into<SharedString>) -> WebView {
    WebView::new(id)
}
