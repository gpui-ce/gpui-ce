# GPUI Community Edition

GPUI is a Rust UI framework for macOS, Windows, Linux, and the web. A view
builds an element tree for layout, input, and painting; entities hold the
application state that views read. After changing state that a view displays,
call `notify` so GPUI can render the windows that depend on it.

GPUI was built by Zed Industries for the [Zed](https://zed.dev) editor. GPUI
Community Edition is a community-maintained fork for standalone applications.

To try GPUI in a new application, add these dependencies to `Cargo.toml` and put the
program below in `src/main.rs`:

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.2" }
gpui_platform = { package = "gpui_ce_platform", version = "0.1" }
```

These requirements select published releases. Repository documentation may
describe APIs newer than those releases. If this example does not compile with
the versions Cargo resolves, pin both dependencies to the same git revision as
described in the [project guide](https://docs.rs/gpui-ce/latest/gpui/_documentation/guides/start_a_project/index.html).

```rust,no_run
use gpui::{App, Context, QuitMode, Window, WindowOptions, div, prelude::*};

struct Hello;

impl Render for Hello {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().items_center().justify_center().child("Hello, GPUI!")
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Hello))
                .unwrap();
            cx.activate(true);
        });
}
```

Run it with `cargo run`. `gpui_platform` selects the platform backend; the
application callback creates the entity that becomes the window's root view.

## Platform backends

GPUI uses Parley for text shaping, wrapping, hit testing, and font fallback on
each platform. `gpui_platform` selects the windowing and rendering integrations.
The choice affects setup and available services:

- On macOS, windows use the native app backend and Metal rendering.
- On Linux and FreeBSD, enable `wayland`, `x11`, or both for desktop windows.
- On Windows, windows use Win32 and rendering uses DirectX.
- The web build uses the browser backend; see the [web guide](https://docs.rs/gpui-ce/latest/gpui/_documentation/guides/web/index.html)
  for its font and build requirements.

## The big picture

Application state lives in `Entity` values. A `Render` implementation turns
an entity into a view, and the view builds an element tree for layout, input, and
painting. Use a custom `Element` when a view needs control over those phases,
such as for a virtualized list or custom drawing. The [concepts](https://docs.rs/gpui-ce/latest/gpui/_documentation/concepts/index.html)
pages explain how entities, views, elements, contexts, and frames connect.

## Learn

- [Documentation](https://docs.rs/gpui-ce): the API reference, plus a
  [tutorial](https://docs.rs/gpui-ce/latest/gpui/_documentation/tutorial/index.html),
  [guides](https://docs.rs/gpui-ce/latest/gpui/_documentation/guides/index.html),
  and [concepts](https://docs.rs/gpui-ce/latest/gpui/_documentation/concepts/index.html)
  that connect state, rendering, and input.
- [Examples](https://github.com/gpui-ce/gpui-ce/tree/main/crates/gpui/examples): complete
  programs that also run live on [the GPUI-CE website](https://gpui-ce.github.io).
- [Discord](https://discord.gg/ENGHGjrYEn) for questions and discussion.

## Requirements

Use Rust 1.95 or newer. Desktop builds also need Xcode on macOS, development
libraries for the Linux backends, or the MSVC toolchain and Windows SDK on
Windows. The [project setup guide](https://docs.rs/gpui-ce/latest/gpui/_documentation/guides/start_a_project/index.html)
lists the requirements and installation commands.

## License

Apache-2.0.
