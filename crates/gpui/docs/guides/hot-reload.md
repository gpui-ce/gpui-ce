# Reload code while the app runs

Patch view code into a running debug build without restarting the process or recreating
its windows. The `hot-patching` feature hooks GPUI's frame stages; Dioxus' `dx` CLI
builds and sends the patches.

Hot patching is experimental. Keep the normal restart workflow available for changes
that affect initialization or state layout.

## Enable the feature

Turn on the `hot-patching` feature of the `gpui` dependency. Until a published
release includes it, depend on a git revision that does:

```toml
[dependencies]
gpui = { package = "gpui-ce", git = "https://github.com/gpui-ce/gpui-ce", features = ["hot-patching"] }
gpui_platform = { package = "gpui_ce_platform", git = "https://github.com/gpui-ce/gpui-ce" }
```

The feature only has an effect in debug builds on native targets. In release
builds and on the web, GPUI compiles the hooks out, so the code runs as it
would without the feature.

## Install Dioxus CLI

Install the `dx` version that matches the Subsecond runtime GPUI uses, 0.7.10:

```sh
cargo install dioxus-cli --version 0.7.10 --locked
```

GPUI depends on Subsecond 0.7.10, the runtime that `dx` patches, so use that
version of `dx` too. Check the installed version with `dx --version`.

## Run with hot patching

From your application's Cargo project, which depends on GPUI as above, run:

```sh
dx serve --hot-patch
```

`dx` builds the app, starts it, and watches your sources. Edit and save a `render`
function; when the patch applies, the existing window redraws with the new code while
the process keeps running.

A view is any type that implements [`Render`] or [`RenderOnce`]. Put your UI
code in `render`, because that's the function GPUI patches first:

```rust
use gpui::{Context, Window, div, prelude::*};

struct Counter {
    count: usize,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Edit this text while `dx serve --hot-patch` runs.
        div().child(format!("Count: {}", self.count))
    }
}
```

Because the process keeps running, entities and windows keep their state across a
patch. A counter's value is still there after you change the text around it.

## What gets patched

GPUI hooks four stages of each frame, so changes to these functions can apply
without a restart:

- **Render**: the `render` method of [`Render`], [`RenderOnce`], and the view
  trait that GPUI calls to build a view's element tree.
- **Layout**: `request_layout` on an [`Element`].
- **Prepaint**: `prepaint` on an [`Element`].
- **Paint**: `paint` on an [`Element`].

Other functions may require a restart. Keep UI code in these stages, and leave
setup, such as creating entities and registering actions, in code that runs
once at startup. A patch replaces function bodies; it does not migrate the layout or
state schema of an existing entity. If a change requires a new field, a different
constructor, or incompatible serialized state, restart the process.

## Limitations

- **Debug builds only.** GPUI skips the hooks in release builds. Hot patching
  does not run on WebAssembly targets either.
- **Failed patches.** A patch can fail, or crash the application. When that
  happens, stop `dx` and restart it.

Report problems in the [GPUI-CE issue tracker](https://github.com/gpui-ce/gpui-ce/issues).

## See also

- [Start a project](crate::_documentation::guides::start_a_project) sets up the
  crate that this guide patches.
- [Crate features](crate#crate-features) lists `hot-patching` with the other
  optional features.
- [Troubleshooting](crate::_documentation::guides::troubleshooting) covers
  common build and runtime problems.
