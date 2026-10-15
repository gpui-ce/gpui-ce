use futures::future::BoxFuture;
use http::StatusCode;

/// An HTTP status code and a fully buffered response body.
///
/// The body is owned bytes, so a caller can decode an image or inspect an error response
/// after the request has finished. This interface does not expose response headers or a stream.
pub struct HttpResponse {
    /// The status returned by the server, including error statuses such as `404 Not Found`.
    pub status: StatusCode,
    /// The complete response body, read before the response is returned.
    pub body: Vec<u8>,
}

/// The transport GPUI uses for resources such as image URLs.
///
/// Install an implementation with [`crate::Application::with_http_client`]; code inside the
/// app retrieves it with [`crate::App::http_client`]. A newly created app uses
/// [`NullHttpClient`], so URL-backed loads fail until a client is installed. The web
/// constructors in `gpui_platform` install a fetch-based client; native applications must
/// provide one before loading URL-backed assets.
///
/// For tests, [`BlockedHttpClient`] rejects network access. With the `test-support` feature,
/// `FakeHttpClient` supplies a fixed status without making a request. The
/// [images guide](crate::_documentation::guides::images_and_assets) explains loading,
/// failure placeholders and cache eviction for a retry.
pub trait HttpClient: 'static + Send + Sync {
    /// Sends a GET request to `url` and returns its status and complete body.
    ///
    /// Follow redirects when `follow_redirects` is `true`. Return `Err` for a transport or
    /// body-read failure. Receiving an HTTP error status, such as 404, is still an `Ok`
    /// response: callers inspect [`HttpResponse::status`] and decide whether to use the body.
    fn get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>>;
}

/// An [`HttpClient`] that fails every request because no client was configured.
///
/// A new app uses it until you install another client. The error text is `No HttpClient available`.
pub struct NullHttpClient;

impl HttpClient for NullHttpClient {
    fn get(
        &self,
        _url: &str,
        _follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        Box::pin(async { anyhow::bail!("No HttpClient available") })
    }
}

/// An [`HttpClient`] that refuses every request with an I/O `PermissionDenied` error.
///
/// Install it when a run must not reach the network, such as a hermetic test.
pub struct BlockedHttpClient;

impl BlockedHttpClient {
    /// Creates a client that refuses every request. This is the same value as
    /// [`Default::default`].
    pub fn new() -> Self {
        Self
    }
}

impl Default for BlockedHttpClient {
    fn default() -> Self {
        Self
    }
}

impl HttpClient for BlockedHttpClient {
    fn get(
        &self,
        _url: &str,
        _follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        Box::pin(async {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "BlockedHttpClient disallowed request",
            )
            .into())
        })
    }
}

/// An [`HttpClient`] for tests that answers every request with one status and an empty body.
///
/// Available in unit tests and with the `test-support` feature. It never performs network I/O.
#[cfg(any(test, feature = "test-support"))]
pub struct FakeHttpClient {
    status: StatusCode,
}

#[cfg(any(test, feature = "test-support"))]
impl FakeHttpClient {
    /// Returns a client that answers every request with `404 Not Found`.
    pub fn with_404_response() -> std::sync::Arc<dyn HttpClient> {
        std::sync::Arc::new(Self {
            status: StatusCode::NOT_FOUND,
        })
    }

    /// Returns a client that answers every request with `200 OK`.
    pub fn with_200_response() -> std::sync::Arc<dyn HttpClient> {
        std::sync::Arc::new(Self {
            status: StatusCode::OK,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl HttpClient for FakeHttpClient {
    fn get(
        &self,
        _url: &str,
        _follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let status = self.status;
        Box::pin(async move {
            Ok(HttpResponse {
                status,
                body: Vec::new(),
            })
        })
    }
}
