//! Cross-platform screen capture source discovery and stream adaptation.

use crate::{
    ForegroundExecutor, ScreenCaptureFrame, ScreenCaptureSource, ScreenCaptureStream,
    SourceMetadata,
};
use anyhow::Result;
use futures::channel::oneshot;
use std::rc::Rc;

#[path = "screen_capture/windows.rs"]
mod platform;

/// Lists the screens that can be captured. The result arrives on the returned receiver.
///
/// Enumeration runs on a background thread. The receiver resolves once it finishes, with the
/// screens as [`ScreenCaptureSource`]s. It resolves with an error if the platform can't list
/// screens. A screen that fails to initialize is left out of the list and logged. Each source
/// can then be captured with [`ScreenCaptureSource::stream`].
///
/// This module exists only in Windows builds with the `screen-capture` feature.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(all(target_os = "windows", feature = "screen-capture"))]
/// # fn main() {
/// use gpui::{App, screen_capture};
///
/// fn list_screens(cx: &App) {
///     let sources = screen_capture::screen_sources(cx.foreground_executor());
///     cx.foreground_executor()
///         .spawn(async move {
///             if let Ok(Ok(sources)) = sources.await {
///                 for source in sources {
///                     if let Ok(metadata) = source.metadata() {
///                         println!("{:?}", metadata.label);
///                     }
///                 }
///             }
///         })
///         .detach();
/// }
/// # }
/// # #[cfg(not(all(target_os = "windows", feature = "screen-capture")))]
/// # fn main() {}
/// ```
pub fn screen_sources(
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    platform::screen_sources(foreground_executor)
}

fn to_dyn_screen_capture_sources<T: ScreenCaptureSource + 'static>(
    sources_rx: oneshot::Receiver<Result<Vec<T>>>,
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    let (dyn_sources_tx, dyn_sources_rx) = oneshot::channel();
    foreground_executor
        .spawn(async move {
            match sources_rx.await {
                Ok(Ok(results)) => dyn_sources_tx
                    .send(Ok(results
                        .into_iter()
                        .map(|source| Rc::new(source) as Rc<dyn ScreenCaptureSource>)
                        .collect()))
                    .ok(),
                Ok(Err(error)) => dyn_sources_tx.send(Err(error)).ok(),
                Err(oneshot::Canceled) => None,
            }
        })
        .detach();
    dyn_sources_rx
}

fn to_dyn_screen_capture_stream<T: ScreenCaptureStream + 'static>(
    stream_rx: oneshot::Receiver<Result<T>>,
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>> {
    let (dyn_stream_tx, dyn_stream_rx) = oneshot::channel();
    foreground_executor
        .spawn(async move {
            match stream_rx.await {
                Ok(Ok(stream)) => dyn_stream_tx
                    .send(Ok(Box::new(stream) as Box<dyn ScreenCaptureStream>))
                    .ok(),
                Ok(Err(error)) => dyn_stream_tx.send(Err(error)).ok(),
                Err(oneshot::Canceled) => None,
            }
        })
        .detach();
    dyn_stream_rx
}
