# 1. Hello, window

Create a binary crate, open its first window, and render a title. This shell stays
intact as later chapters add state, input, layout, and tests.

## Create the project

Make a new binary crate and add GPUI to it:

```sh
cargo new todos
cd todos
```

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.2" }
gpui_platform = { package = "gpui_ce_platform", version = "0.1" }
```

`gpui` contains the application, window, element, and styling APIs.
`gpui_platform` supplies the platform runner for macOS, Windows, Linux, and the
web. Cargo uses the package name `gpui-ce`; Rust imports it as `gpui`.

## Write the app

Replace `src/main.rs` with this:

```no_run
use gpui::{App, Context, QuitMode, Window, WindowOptions, colors::Colors, div, prelude::*};

struct Todos;

impl Render for Todos {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .p_8()
            .bg(colors.background)
            .text_color(colors.text)
            .text_3xl()
            .child("Todos")
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_window, cx| {
                cx.new(|_cx| Todos)
            })
            .unwrap();
            cx.activate(true);
        });
}
```

Run it with `cargo run`. The window shows “Todos” in large type. Closing the
last window ends the process because this example selects
[`QuitMode::LastWindowClosed`].

## What the code does

**`main` starts the application.** `gpui_platform::application()` creates an
[`Application`] for the platform you built for, and [`run`](Application::run)
starts its event loop. The closure runs once the platform is ready, and receives
an [`App`]: the object that owns everything in your program. `run` doesn't
return until the program quits. [`QuitMode::LastWindowClosed`] makes it quit
when its last window closes, which is how most apps behave on Windows and
Linux; on macOS, apps normally keep running until you quit them.

**`open_window` creates a window.** It takes [`WindowOptions`], which control
the window's size, title bar and more, and a closure that builds the window's
*root view*. [`activate`](App::activate) brings the app to the front.

**A view is a struct that can render.** `Todos` implements [`Render`]. GPUI
calls its `render` method whenever the window needs to be drawn, and `render`
returns a tree of *elements* describing what to show. Here the tree is a single
[`div`] containing some text.

**`cx.new` hands the view to GPUI.** The app owns your views and the rest of your
state. `cx.new(|_cx| Todos)` gives the app a `Todos` and returns an
[`Entity<Todos>`](Entity), a handle you use to reach it. The [ownership and data
flow](crate::_documentation::concepts::ownership) concept page explains why
GPUI keeps views behind entities.

## Styling elements

Each method called on `div()` sets one style property:

* `size_full` makes the element fill its parent, here the whole window.
* `p_8` adds padding. The numbers follow the scale from Tailwind CSS, where each
  step is a quarter of a *rem*, the window's base font size of 16 pixels. So
  `p_8` is two rems, or 32 pixels.
* `bg` and `text_color` set the background and text colors. Text properties
  apply to all the text inside an element, including its children's.
* `text_3xl` sets a large font size.

[`Colors::for_appearance`](colors::Colors::for_appearance) selects a small
palette for the window's light or dark appearance. Passing those colors through
the element tree keeps this first example readable; later chapters use the same
palette for rows and controls.

These methods come from the [`Styled`] trait, imported through
`gpui::prelude::*`. The [layout and style](crate::_documentation::concepts::layout_and_style)
concept page explains how those properties become layout and paint.

In the [next chapter](crate::_documentation::tutorial::ch2_state) you give the
view some todos, and make them respond to clicks.
