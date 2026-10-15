# Run on the web

The first browser build looks like a desktop build: target `wasm32-unknown-unknown`, call
`gpui_platform::application()`, and let Trunk serve the generated files. The differences
appear once the app starts. A browser gives GPUI one canvas instead of native windows,
provides neither system-font lookup nor a local filesystem, and supports workers only
when the binary and page satisfy extra requirements. This guide starts with those runtime
constraints, then builds the entry point and page around them.

## Account for browser constraints

Some browser limits change application code; others decide whether the deployed binary
can start. Keep the two kinds separate while debugging: a missing font or asset is an
application concern, while shared-memory headers determine whether a threaded module can
instantiate at all.

- **One window, one canvas.** GPUI appends a single `<canvas>` to the page and draws the
  window into it. Its size follows the browser viewport. Opening a second top-level
  window, or reopening one after it closes, fails with `WebWindowError`.
- **GPU rendering.** GPUI uses WebGPU when the browser has it, and falls back to
  WebGL2 otherwise. To force one, pass a `WebBackendPreference` to
  `gpui_platform::application_with_web_backend()`.
- **Fonts come from you.** Browsers don't expose system fonts, so add every font
  the app uses with [`TextSystem::add_fonts`].
- **No filesystem.** Embed assets in the binary with `include_bytes!`, or fetch
  them over HTTP. `gpui_platform::application()` installs a fetch-based
  [`HttpClient`](crate::http_client::HttpClient), so [`App::http_client`] works.
- **Background work.** [`BackgroundExecutor`] uses Web Workers only when the binary has
  the `multithreaded` feature and shared-memory Wasm flags, the page is cross-origin
  isolated, and the browser provides `SharedArrayBuffer`, `Atomics`, and
  `Atomics.waitAsync`. A threaded binary that loads successfully falls back to the main
  thread when those runtime APIs are missing. A binary linked for shared memory can fail
  before GPUI starts if the page is not isolated. See [executors and tasks](crate::_documentation::concepts::executors).
- **Platform services differ.** Some services work in the browser, some do
  nothing, and some return an error. The
  [platform services guide](crate::_documentation::guides::platform_services)
  lists them.

## Write the entry point

The browser runs `main` after the page loads. Call `gpui_platform::web_init` first so a
panic reaches the browser console. Register fonts inside the `application().run` callback
before opening the window; that callback is the first point where the app's text system
exists. Copy the Regular and SemiBold font files from the repository's
[IBM Plex Sans assets](https://github.com/gpui-ce/gpui-ce/tree/main/assets/fonts/ibm-plex-sans)
into `assets/fonts/` in your project. These paths are relative to `src/main.rs`:

```rust,no_run
use std::borrow::Cow;

use gpui::{App, Context, Window, WindowOptions, div, prelude::*};

struct Hello;

impl Render for Hello {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family("IBM Plex Sans")
            .flex()
            .items_center()
            .justify_center()
            .child("Hello from the browser")
    }
}

/// Registers the fonts the app draws with.
fn load_fonts(cx: &mut App) {
    # #[cfg(any())]
    let regular = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");
    # let regular = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"));
    # #[cfg(any())]
    let semibold = include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf");
    # let semibold = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf"));
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(regular.as_slice()),
            Cow::Borrowed(semibold.as_slice()),
        ])
        .expect("the bundled fonts are valid");
}

fn main() {
    #[cfg(target_family = "wasm")]
    gpui_platform::web_init();

    gpui_platform::application().run(|cx: &mut App| {
        load_fonts(cx);

        cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_cx| Hello))
            .unwrap();
        cx.activate(true);
    });
}
```

[`Application::run`] invokes your closure only after browser graphics have started. If
graphics cannot start, GPUI draws an error message in the page and never invokes the
closure, so code inside it cannot recover from that failure.

The `#[cfg(target_family = "wasm")]` attribute keeps `web_init` out of native
builds, so the same file still compiles for your desktop target.

## Add the page

Trunk builds the crate and writes the JavaScript glue that loads it. Put `index.html`
beside `Cargo.toml`. Its `data-trunk` link selects the Rust binary, and the stylesheet
gives GPUI a canvas that fills the viewport:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Hello GPUI</title>
    <link data-trunk rel="rust" />
    <style>
      html, body { margin: 0; height: 100%; overflow: hidden; }
      canvas { position: fixed; inset: 0; }
    </style>
  </head>
  <body></body>
</html>
```

GPUI appends the canvas to `<body>`. The stylesheet pins it to the viewport and disables
page scrolling; if the crate contains multiple binaries, add `data-bin="<name>"` to the
link.

When you need Web Workers, compile the WebAssembly target with atomics and serve the
page with cross-origin isolation. `gpui_ce_platform` deliberately depends on
`gpui_ce_web` with its default features disabled; add the matching `gpui_ce_web` package
as a direct dependency with `features = ["multithreaded"]` so Cargo enables that feature
for the shared package. The repository's web example uses nightly `build-std` with
`+atomics,+bulk-memory,+mutable-globals`, `--shared-memory`, and the TLS linker exports
in `crates/gpui_web/examples/hello_web/.cargo/config.toml`. Copy that config when you
need the threaded build; a normal stable `wasm32-unknown-unknown` build remains a valid
single-threaded deployment.

The dependency entry is:

```toml
gpui_web = { package = "gpui_ce_web", version = "0.1", features = ["multithreaded"] }
```

With Trunk, add the headers used by the repository example:

```toml
[serve]
headers = { "Cross-Origin-Embedder-Policy" = "require-corp", "Cross-Origin-Opener-Policy" = "same-origin" }
```

These headers affect the server response, not the Rust binary. Without cross-origin
isolation, a threaded binary can fail during WebAssembly instantiation before GPUI can
select a dispatcher. A binary built without shared-memory flags can load and run
single-threaded. If a threaded build does load but the browser lacks the required APIs,
GPUI logs a warning and runs the work on the main thread.

`single_threaded_web()` disables worker use at runtime, but it cannot remove shared-memory
linker flags from an already-built binary. If the deployment cannot instantiate shared
WebAssembly memory, use the non-threaded Cargo feature and build configuration instead.

## Build and serve

Install the WebAssembly target and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Start the development server from the crate directory. Trunk serves the generated
`.wasm` and JavaScript together; with the configuration above, it also sends the
headers needed for a threaded build:

```sh
trunk serve
```

After `trunk serve` starts, open `http://127.0.0.1:8080` to see the app. Trunk rebuilds
and reloads the page when you save a file. For production, run `trunk build --release`;
it writes the files to `dist/`. Configure the production server to send the same isolation
headers when deploying a threaded build.

The live examples on the [GPUI site](https://gpui-ce.github.io/) are built this
way. The player lives in
[`site/player`](https://github.com/gpui-ce/gpui-ce/tree/main/site/player). Its
`index.html` is the model for the page above, and it adds a loading message and
a dark-mode background.

## See also

- [Start a project](crate::_documentation::guides::start_a_project) covers the
  native setup.
- [Use platform services](crate::_documentation::guides::platform_services)
  lists what works in the browser.
- [Platforms](crate::_documentation::platforms) describes each target.
