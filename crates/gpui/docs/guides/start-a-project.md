# Start a project

Create a Rust crate that opens a GPUI window, then run it on your desktop. Rust 1.95 or
newer is required; each platform also needs the native tools listed at the end of this
page.

## Create the crate

Use Cargo to make a binary crate:

```sh
cargo new hello-gpui
cd hello-gpui
```

## Add the dependencies

GPUI is published as `gpui-ce` and imported as `gpui`. The
`gpui_platform` crate picks the windowing and rendering backend for the host
operating system, so your app doesn't need `#[cfg]` code for each platform.

```toml
[package]
name = "hello-gpui"
version = "0.1.0"
edition = "2024"

[dependencies]
gpui = { package = "gpui-ce", version = "0.2" }
gpui_platform = { package = "gpui_ce_platform", version = "0.1" }
```

To try changes that have not been released yet, depend on the repository instead. Pin a
commit with `rev`; a branch name moves and does not make a build reproducible:

```toml
[dependencies]
gpui = { package = "gpui-ce", git = "https://github.com/gpui-ce/gpui-ce", rev = "b5aceefaf96af3946ff26a603e5f4110d4bdf4ca" }
gpui_platform = { package = "gpui_ce_platform", git = "https://github.com/gpui-ce/gpui-ce", rev = "b5aceefaf96af3946ff26a603e5f4110d4bdf4ca" }
```

The revision shown is an example; choose the commit you intend to ship and keep both
packages on it. Pin the release version instead when one contains the changes you need.

Cargo finds both packages in the repository by name, so you don't need a
checkout.

## Write the app

Replace `src/main.rs` with a window that shows a greeting:

```rust,no_run
use gpui::{App, Context, QuitMode, Window, WindowOptions, div, prelude::*};

struct HelloWorld;

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child("Hello, GPUI!")
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_window, cx| {
                cx.new(|_cx| HelloWorld)
            })
            .unwrap();
            cx.activate(true);
        });
}
```

`gpui_platform::application()` creates the [`Application`](gpui::Application) for the
current platform. [`Application::run`] invokes your closure after the platform is ready;
the closure receives [`App`] and opens a window. `cx.new` creates the [`Render`] entity
that becomes that window's root view.

[`QuitMode`] decides what happens when windows close. `LastWindowClosed` quits
when the last window closes, on every platform. The default is
`QuitMode::Default`, which uses `Explicit` on macOS: the app keeps running after
its last window closes until [`App::quit`] is called. Choose the mode your app
needs.

## Run it

```sh
cargo run
```

A window opens with the greeting. The first build takes a while because Cargo
compiles GPUI and its dependencies.

## System requirements

GPUI renders on the GPU, and uses the windowing system of each platform.

### macOS

Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835?mt=12) from the
App Store, or from the [Apple Developer](https://developer.apple.com/download/all/)
downloads page. Launch it once and install the macOS components. Then install
the command line tools and select your Xcode:

```sh
xcode-select --install
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
```

GPUI renders with Metal. The details are in the
[setup guide](https://github.com/gpui-ce/gpui-ce/blob/main/SETUP.md).

### Linux

On Ubuntu or Debian, install the libraries that the CI job uses:

```sh
sudo apt-get update
sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libxcb-shape0-dev libxcb-xfixes0-dev libfontconfig-dev
```

GPUI builds both the Wayland and X11 backends by default. To name a backend
explicitly, enable the `wayland` or `x11` feature on `gpui_platform`:

```toml
gpui_platform = { package = "gpui_ce_platform", version = "0.1", features = ["wayland", "x11"] }
```

Use the equivalent packages on other distributions, and run desktop apps in a
graphical session. [`SETUP.md`](https://github.com/gpui-ce/gpui-ce/blob/main/SETUP.md)
lists the complete set of packages.

### Windows

Install the MSVC toolchain with rustup, along with the Visual Studio C++ build
tools and a Windows SDK. Microsoft's
[Rust development setup guide](https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup)
walks through each step. GPUI uses DirectX and Win32, and needs no extra
features on `gpui_platform`.

## Optional features

GPUI has optional features for testing, the element inspector, profiling and
hot reload. Turn one on through the `features` key of the `gpui` dependency:

```toml
gpui = { package = "gpui-ce", version = "0.2", features = ["inspector"] }
```

The [crate features table](crate#crate-features) lists the features and what
each one enables.

## See also

- [Run on the web](crate::_documentation::guides::web) builds this kind of app
  for the browser.
- [Reload code while the app runs](crate::_documentation::guides::hot_reload)
  patches render code in a running app.
- [Use platform services](crate::_documentation::guides::platform_services)
  covers the clipboard, dialogs, menus and more.
- [The tutorial](crate::_documentation::tutorial) builds a complete app step by
  step.
