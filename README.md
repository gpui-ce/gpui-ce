<img src="assets/readme/banner.png" alt="GPUI Community Edition banner" width="100%">

<p align="center">
  <a href="https://gpui-ce.github.io">Website</a> ​ · ​ <a href="https://docs.rs/gpui-ce">Docs</a> ​ · ​ <a href="crates/gpui/examples">Examples</a> ​ · ​ <a href="https://discord.gg/ENGHGjrYEn">Discord</a>
</p>

# GPUI Community Edition

GPUI-CE is a community fork of Zed's [GPUI](https://gpui.rs), a GPU-accelerated
UI framework for Rust. It supports macOS, Windows, Linux, and the web.

A view builds an element tree from the state held in its entity. Input handlers
update that state through a context; `notify` requests the next redraw. For API
and feature differences from Zed's GPUI, see the
[upstream guide](crates/gpui/docs/upstream.md).

## Overview

- Layout and styling

  Build views with familiar elements, flex layouts, and Tailwind-style methods:

  ```rust
  div()
      .id("greeting")
      .flex()
      .items_center()
      .gap_2()
      .rounded_lg()
      .rounded_smoothing(0.8)
      .bg(rgba(0xffffff30))
      .backdrop_blur(px(12.0))
      .transitions(|transitions| transitions.bg(millis(200)))
      .hover(|style| style.bg(rgba(0xffffff60)))
      .child("Hello, GPUI")
  ```

  [Layout example](crates/gpui/examples/layout.rs) ​ · ​ [Styling example](crates/gpui/examples/styling.rs)

- State and events

  The app stores a view's state; an `Entity<T>` lets other code read and update
  it through a context. Mutation alone does not redraw a window. In this
  counter's `render` method, the click handler changes the count and announces
  the change with `cx.notify()`:

  ```rust
  div()
      .id("counter")
      .child(format!("Count: {}", self.count))
      .on_click(cx.listener(|this, _event, _window, cx| {
          this.count += 1;
          cx.notify();
      }))
  ```

  [Counter example](crates/gpui/examples/counter.rs) ·
  [State and ownership](crates/gpui/docs/concepts/ownership.md)

- Actions and key bindings

  Typed actions name operations independently of the keys that trigger them.
  During app setup, bind Space and Backspace to actions in the `Counter` key
  context:

  ```rust
  use gpui::{App, KeyBinding, actions};

  actions!(keybinds_example, [Increment, Reset]);

  fn register_keybinds(cx: &mut App) {
      cx.bind_keys([
          KeyBinding::new("space", Increment, Some("Counter")),
          KeyBinding::new("backspace", Reset, Some("Counter")),
      ]);
  }
  ```

  In the view, set that key context, track its focus handle, and register the
  action handlers. The bindings apply while focus is inside this element:

  ```rust
  div()
      .key_context("Counter")
      .track_focus(&self.focus_handle)
      .on_action(cx.listener(Self::increment))
      .on_action(cx.listener(Self::reset))
      .child(format!("Count: {}", self.count))
  ```

  [Keyboard example](crates/gpui/examples/keyboard.rs) ·
  [Focus and dispatch](crates/gpui/docs/concepts/input-dispatch.md)

- Virtualized lists

  Use `uniform_list` for large collections of equal-height rows. It builds the
  rows needed for the viewport and a small buffer as you scroll:

  ```rust
  uniform_list("items", 10_000, |range, _window, _cx| {
      range
          .map(|index| {
              div()
                  .h(px(24.0))
                  .child(format!("Item {index}"))
          })
          .collect()
  })
  .h(px(300.0))
  ```

  [List example](crates/gpui/examples/list.rs)

- Custom drawing

  `canvas` paints inside bounds assigned by GPUI's layout system. For a custom
  control that also needs its own layout or input handling, implement `Element`.

  ```rust
  canvas(
      |_bounds, _window, _cx| {},
      |bounds, _state, window, _cx| {
          window.paint_quad(PaintQuad::new(bounds).background(rgb(0x5078f0)));
      },
  )
  .size(px(80.0))
  ```

  [Drawing example](crates/gpui/examples/canvas.rs)

## Learn

The tutorial, guides, and concepts are included in rustdoc alongside the API
reference. Documentation snippets are compiled or run as doctests; complete
example programs are checked separately by Cargo.

- [Tutorial](https://docs.rs/gpui-ce/latest/gpui/_documentation/tutorial/index.html): build a task-list app, from the first window through tests.
- [Guides](https://docs.rs/gpui-ce/latest/gpui/_documentation/guides/index.html): get a specific job done.
- [Concepts](https://docs.rs/gpui-ce/latest/gpui/_documentation/concepts/index.html): connect ownership, rendering, layout, input, and scheduling.
- [Examples](crates/gpui/examples): complete programs. Run one with `cargo run --example counter`, or see them live on [the website](https://gpui-ce.github.io).

To read the documentation for this checkout, run `just docs` and open
`target/doc/gpui/index.html`. [`docs/writing-docs.md`](docs/writing-docs.md)
explains how to contribute to it.

## Setup

The [project guide](crates/gpui/docs/guides/start-a-project.md) creates a
standalone app. [SETUP.md](SETUP.md) covers building this repository.

## Project goals and policy

GPUI-CE aims to support Rust applications of different sizes, with reusable
components, native platform integration, and control over rendering. The
project builds on Zed's work and continues to incorporate upstream fixes.

Contributions follow the [Rust Foundation's internal AI usage policy](https://rustfoundation.org/policy/internal-ai-usage-policy/).
Contributors are responsible for understanding and reviewing their work.
[CONTRIBUTING.md](CONTRIBUTING.md) explains how to discuss and submit changes.
