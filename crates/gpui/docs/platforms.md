# Platforms

`gpui_platform::application()` selects a backend for the target you compile.
The views still use [`App`] and [`Window`], but the backend supplies the display,
GPU integration and operating-system services. A program that compiles for a
second target may still need different fonts, window options or service fallbacks.

All targets require Rust 1.95 or newer. [Start a project](crate::_documentation::guides::start_a_project)
lists the toolchains and system packages to install.

## Desktop rendering and input

The default integrations are:

| Target | Windowing | GPU | Glyph rasterization |
| --- | --- | --- | --- |
| macOS | AppKit | Metal | Core Text |
| Windows | Win32 | Direct3D 11 | DirectWrite |
| Linux | Wayland or X11 | wgpu over Vulkan or OpenGL | Swash |

All three desktop backends support multiple windows, system fonts, fonts you add,
input methods and AccessKit accessibility. On Linux, accessibility uses AT-SPI;
IME uses Wayland's `text-input-v3` or X11's XIM. macOS supplies trackpad gestures,
Windows supplies touchpad pan and zoom, and both Linux windowing backends handle
touch input.

Parley shapes and lays out text on every target. Given the same font files and
shaping settings, it usually produces the same advances and line breaks. That
does not make glyph pixels identical: macOS and Windows use native rasterizers,
while Linux and the web use Swash. If an application needs particular families,
bundle them rather than relying on the host's font set. The [text guide](crate::_documentation::guides::text)
explains registration and fallback; [images and assets](crate::_documentation::guides::images_and_assets)
covers bundling resources.

On macOS, install Xcode and its command line tools for the Metal shader compiler.
Windows builds need the MSVC toolchain and a Windows SDK; the renderer requires
Direct3D 11 feature level 11.0. The `windows-manifest` feature on `gpui`, enabled
by default, embeds an application manifest in Windows executables.

### Linux and FreeBSD

At startup, GPUI selects Wayland when `WAYLAND_DISPLAY` is set, otherwise X11
when `DISPLAY` is set. A backend must be compiled in as well as available in the
session. The published `gpui_ce_platform` package normally brings both through
its dependencies' default features. Its `wayland` and `x11` features forward to
`gpui_ce_linux` for applications maintaining a feature-minimal dependency graph.

The build uses development packages for Wayland, X11, xkbcommon and fontconfig;
the setup guide lists the packages. Running the binary also needs the appropriate
display server and a Vulkan or OpenGL driver. A successful build does not check
those runtime conditions. FreeBSD uses the Linux backends and is not tested in CI.

## Desktop services

The desktop backends provide text and image clipboards, file dialogs, system
notifications, credential storage, and requests to open URLs or files in the
system handler. Their implementations differ:

* File dialogs use the XDG desktop portal on Linux. A running desktop still
  needs a working portal for a picker to open.
* Notifications use the platform notification center, including Freedesktop
  notifications on Linux. Credential storage uses Keychain on macOS and Secret
  Service on Linux; Windows also provides a native credential store.
* macOS draws an application menu bar and a Dock menu. Windows stores application
  menus without drawing them and supplies a Jump List for the taskbar. Linux
  provides neither kind of menu through these APIs.
* Haptic feedback is available on macOS Force Touch trackpads. Other backends
  report no haptic support.
* Screen capture is available on macOS through ScreenCaptureKit and on Windows
  through Windows Graphics Capture. Enable `screen-capture` on `gpui_platform`;
  the core `gpui` feature supplies Windows capture. Linux has no capture backend.

Desktop windows can be transparent or blurred, with backend-specific effects:
Windows includes Mica, and Linux blur depends on KDE. Wayland also supports
`WindowKind::LayerShell` surfaces; macOS, Windows and the web do not.

Availability does not promise success on a particular machine. A permission can
be denied or a system service absent. Depending on the API, an unsupported call
returns an error, returns `None`, or does nothing. The [platform services guide](crate::_documentation::guides::platform_services)
shows how to handle those outcomes and retain asynchronous operations.

## Browser builds

The browser backend draws into one canvas filling the page. It uses wgpu with
WebGPU, or WebGL2 as a fallback. `Auto` tries them in that order; forcing either
backend makes startup fail if that backend is unavailable. A hidden text area
connects IME to the canvas, and the backend receives pointer and touch events.
It does not yet expose an accessibility tree.

There is no system-font lookup. Call [`TextSystem::add_fonts`] with every font
the app needs before drawing text. There is also no desktop filesystem access:
load resources from bytes or URLs instead of file paths.

The canvas follows the viewport, so window size, position and title-bar options
are ignored. Popup, floating and dialog kinds are unsupported. Opening a second
top-level window, or reopening the first after it closes, returns a
`WebWindowError`.

For services, plan around the browser's boundary:

* Text and image clipboard writes are available, but reads are asynchronous.
  [`App::read_from_clipboard`] always returns `None`; use
  [`App::read_from_clipboard_async`] for a permission-gated read.
* Opening a URL is supported; opening a local file through the system is not.
* Application and taskbar menus, native file dialogs, system notifications,
  persistent native credentials, haptics and screen capture are unavailable
  through GPUI's browser backend.
* Native transparency, blur and layer-shell window effects are unavailable.

### Background work on the web

A normal build runs background futures on the browser's main thread. Web Workers
require the `gpui_web` `multithreaded` feature, a binary linked for shared-memory
WebAssembly, and an isolated page in a browser providing `SharedArrayBuffer`,
`Atomics` and `Atomics.waitAsync`. The server must send
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`.

If the threaded binary loads but the runtime APIs are missing, the dispatcher
can fall back to the main thread. Missing cross-origin isolation can instead
prevent shared-memory WebAssembly from instantiating, before GPUI starts. For a
deployment without isolation, build a non-threaded binary. [Run on the web](crate::_documentation::guides::web)
shows the Cargo, linker, font and server configuration together.

## Selecting features

Cargo features belong to the package declaring them. A feature on `gpui` does
not enable a same-named feature on `gpui_platform` unless a dependency explicitly
forwards it. In addition to the backend flags above:

| Feature | Package | Effect |
| --- | --- | --- |
| `macos-wgpu` | `gpui_platform` | Selects wgpu for macOS windows instead of the native renderer. |
| `custom-gpu` | `gpui`, `gpui_wgpu` | Exposes custom GPU drawing and device sharing; on macOS, also select the wgpu renderer. |

The [crate features](crate#crate-features) table lists the remaining features
and their defaults.
