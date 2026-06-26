#import "../template.typ": template
#import "../components.typ": collapsible

#show: template.with(current: "hot-patching")

= Hot-patching

GPUI-CE's hot-patching feature allows you to edit the source code of your application
and immediately see the effect of those edits without restarting the application and without
repeatedly navigating to the part of the application you are working on after every change.

== Setup

Using hot-patching requires no changes to your existing application, but you need to
install the #link("https://crates.io/crates/dioxus-cli")[Dioxus CLI]:

#collapsible(
  summary: [Method 1: Using #link("https://crates.io/crates/cargo-binstall")[`cargo binstall`] (recommended)],
  open: true,
  name: "installation-methods",
)[
  To install a pre-built binary of the Dioxus CLI, run:

  ```bash
  cargo binstall dioxus-cli
  ```

  Note: you need to have `cargo-binstall` installed to use this method.
]

#collapsible(summary: [Method 2: Using `cargo install`], name: "installation-methods")[
  To compile the Dioxus CLI from source, run:

  ```bash
  cargo install dioxus-cli --locked
  ```
]

#collapsible(summary: [Method 3: Using pacman], name: "installation-methods")[
  If you are using Arch Linux, you can install the Dioxus CLI from pacman:

  ```bash
  sudo pacman -S dioxus-cli
  ```
]

== Using hot-patching

To use hot-patching, simply run your application with

```bash
dx serve --hot-patch
```

instead of

```bash
cargo run
```

If you change any UI code in your application and save the source code file, you should see the updated UI after a few seconds.

If you do not have an existing GPUI-CE application to test hot-patching with, you can use one of the examples in the GPUI-CE repository:

Run
```bash
git clone https://github.com/gpui-ce/gpui-ce.git
cd gpui-ce/crates/gpui/examples/learn/
dx serve --hot-patch -p gpui --example interactive_elements
```
and edit `interactive_elements.rs` to see the UI update.

== Limitations

Hot-patching is currently not supported for WASM targets.

The hot-patching feature in Dioxus CLI as well as the integration in GPUI-CE is still experimental and may occasionally crash during hot-patching. We appreciate any feedback whether the feature works for your setup and whether there are any missing functions that do not get updated, if you change them.
Feel free to comment on the #link("https://github.com/gpui-ce/gpui-ce/pull/68")[GitHub issue] or in the GPUI-CE Discord.
