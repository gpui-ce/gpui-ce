# Open and manage windows

A window owns its root view. Other parts of the app can reach that view through
a window handle, which remains usable only while the window is open and its
root has the expected type.

## Open a window with options

[`App::open_window`] calls your builder with the new [`Window`] and [`App`].
The builder creates an `Entity<V>` for the root; `open_window` returns its
[`WindowHandle<V>`]. This Notes window starts from [`WindowOptions`] defaults,
then sets a centered rectangle, a title and a minimum size:

```rust
use gpui::{
    AppContext, Bounds, Context, IntoElement, Render, TestApp, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};

struct Notes {
    text: String,
    dirty: bool,
}

impl Render for Notes {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(self.text.clone())
    }
}

let mut app = TestApp::new();
let notes = app.update(|cx| {
    let bounds = Bounds::centered(None, size(px(480.), px(320.)), cx);
    cx.open_window(
        WindowOptions::default()
            .window_bounds(WindowBounds::Windowed(bounds))
            .titlebar(Some(TitlebarOptions {
                title: Some("Notes".into()),
                ..Default::default()
            }))
            .window_min_size(size(px(320.), px(200.))),
        |_window, cx| {
            cx.new(|_| Notes {
                text: "Hello".into(),
                dirty: false,
            })
        },
    )
    .unwrap()
});
```

The commonly used options are:

* **`window_bounds`** takes a [`WindowBounds`]. `Windowed` opens at a rectangle in screen
  coordinates. [`Bounds::centered`] builds one centered on a display, or on the primary
  display when you pass `None`. `Maximized` and `Fullscreen` take the rectangle to restore
  when leaving that state. Omitting `window_bounds` lets GPUI choose initial bounds.
* **`titlebar`** takes an optional [`TitlebarOptions`]. Its `title` sets the initial title,
  and `appears_transparent` hides the system titlebar so you can draw your own, on macOS
  and Windows. Pass `None` to remove the titlebar altogether.
* **`window_min_size`** stops the user from resizing below a size. Use `is_resizable` and
  `is_minimizable` to turn those capabilities off.
* **`kind`** takes a [`WindowKind`]. The default is `WindowKind::Normal`. `WindowKind::PopUp`
  stays above other windows, so use it sparingly. `WindowKind::AnchoredPopup` places a
  native menu or tooltip relative to a parent window.
* **`focus`** and **`show`** control whether the window takes focus and appears when it
  opens. Set `show(false)` for a window you will show later, and `focus(false)` if opening
  it should preserve the active window.

## Keep a handle and update the window later

[`WindowHandle::update`] borrows the current root, its [`Window`] and the
root's context for a callback. [`WindowHandle::read`] only borrows the root
for inspection. Both can fail: keeping a [`WindowHandle`] does not keep the
window open, and replacing its root with another type invalidates typed access.
The example updates Notes, then reads the saved text:

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: false })
#     })
#     .unwrap()
# });
app.update(|cx| {
    notes
        .update(cx, |notes, _window, cx| {
            notes.text = "Saved".into();
            notes.dirty = false;
            cx.notify();
        })
        .unwrap();
});

app.update(|cx| {
    let notes = notes.read(cx).unwrap();
    assert_eq!(notes.text, "Saved");
});
```

`WindowHandle` is `Copy`. A controller, view and close observer can each retain
a copy; the copies identify the same window and compare equal. After that
window closes, reads and updates fail through every copy.

## Work with several windows

Call `open_window` for each native window. [`App::windows`] lists them as
[`AnyWindowHandle`]s, suitable for a window switcher with several root types.
[`AnyWindowHandle::downcast`] checks a handle's recorded root type; it does
not query a root that has since been replaced (see [root replacement](#replace-the-root-view)).
[`App::active_window`] selects the window currently active in the operating
system, for commands that should follow the user's current window.

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let _notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: false })
#     })
#     .unwrap()
# });
app.update(|cx| {
    cx.open_window(WindowOptions::default(), |_window, cx| {
        cx.new(|_| Notes {
            text: "Second".into(),
            dirty: false,
        })
    })
    .unwrap();
});

app.update(|cx| assert_eq!(cx.windows().len(), 2));
```

## Close a window and react to it

[`Window::remove_window`] marks a window for removal after its current update.
To close a different window, update its handle and call `remove_window` in that
callback. [`App::on_window_closed`] runs after removal for every closed window,
including ones opened elsewhere in the app. Here the observer counts the
second window's closure:

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let _notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: false })
#     })
#     .unwrap()
# });
use std::{cell::Cell, rc::Rc};

let second = app.update(|cx| {
    cx.open_window(WindowOptions::default(), |_window, cx| {
        cx.new(|_| Notes { text: "Second".into(), dirty: false })
    })
    .unwrap()
});

let closed = Rc::new(Cell::new(0));
app.update(|cx| {
    let closed = closed.clone();
    cx.on_window_closed(move |_cx, _window_id| closed.set(closed.get() + 1))
        .detach();
});

app.update(|cx| {
    second
        .update(cx, |_notes, window, _cx| window.remove_window())
        .unwrap();
});

app.update(|cx| {
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(closed.get(), 1);
});
```

The title-bar close button first asks whether the window should close.
[`Window::on_window_should_close`] can veto that request by returning `false`;
`remove_window` bypasses the question and schedules removal directly. The
callback below rejects a title-bar close while Notes has unsaved changes.
Its weak entity handle lets the view be released independently of the callback:

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: true })
#     })
#     .unwrap()
# });
app.update(|cx| {
    notes
        .update(cx, |_notes, window, cx| {
            let view = cx.entity().downgrade();
            window.on_window_should_close(cx, move |_window, cx| {
                view.upgrade().map_or(true, |notes| !notes.read(cx).dirty)
            });
        })
        .unwrap();
});
```

If the close workflow opens a custom prompt, answering it restores the earlier
element focus, or clears focus if that element has been removed. The
[platform-services guide](crate::_documentation::guides::platform_services)
explains prompt responses and native versus custom dialogs.

### Quit when the last window closes

[`QuitMode`] controls what happens after the last window closes.
`QuitMode::Default` keeps the app running on macOS and quits elsewhere;
`QuitMode::LastWindowClosed` quits on all platforms. With `QuitMode::Explicit`,
the app waits for [`App::quit`]. Set the policy on the application builder,
as below, or change it with [`App::set_quit_mode`].

```rust,no_run
use gpui::{App, Context, IntoElement, QuitMode, Render, Window, WindowOptions, div, prelude::*};

struct Notes;

impl Render for Notes {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Notes")
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

fn run(cx: &mut App) {
    cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_| Notes))
        .unwrap();
    cx.activate(true);
}
```

## Query the window's state

Read measurements during render to size the content, or query platform state
in a window update before issuing a window command:

* [`Window::bounds`] is the window's rectangle in screen coordinates. It can span displays.
* [`Window::viewport_size`] is the drawable area, which excludes the titlebar. Lay out your
  content from this size.
* [`Window::scale_factor`] is the number of device pixels per logical pixel. It's `2.0` on
  many high-density displays.
* [`Window::is_window_active`] tells you whether the operating system considers the window
  active. Key input reaches the focused element only while the window is active.
* [`Window::window_bounds`], [`Window::is_maximized`] and [`Window::is_fullscreen`] report
  its state. Use [`Window::toggle_fullscreen`], [`Window::zoom_window`] and
  [`Window::minimize_window`] to change it.

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: false })
#     })
#     .unwrap()
# });
app.update(|cx| {
    notes
        .update(cx, |_notes, window, _cx| {
            assert!(window.scale_factor() > 0.0);
            let _content_size = window.viewport_size();
            let _focused = window.is_window_active();
        })
        .unwrap();
});
```

## Replace the root view

[`Window::replace_root`] installs a new entity and requests a redraw. The old
view is released when its last strong handle is dropped. A `WindowHandle<V>`
can still read or update the replacement if it also has type `V`; replacing
it with a different type makes those calls fail, as the assertion below shows.

An existing [`AnyWindowHandle`] keeps its original type tag too. Its
`downcast` method cannot discover a replacement's type. For navigation across
root types, use [`AnyWindowHandle::update`], then downcast the live [`AnyView`]
passed to its callback. That checks the entity actually installed in the window.

```rust
# use gpui::{AppContext, Context, IntoElement, Render, TestApp, Window, WindowOptions, div, prelude::*};
# struct Notes {
#     text: String,
#     dirty: bool,
# }
# impl Render for Notes {
#     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
#         div().child(self.text.clone())
#     }
# }
# let mut app = TestApp::new();
# let notes = app.update(|cx| {
#     cx.open_window(WindowOptions::default(), |_window, cx| {
#         cx.new(|_| Notes { text: "Hello".into(), dirty: false })
#     })
#     .unwrap()
# });
struct Settings;

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("Settings")
    }
}

app.update(|cx| {
    notes
        .update(cx, |_notes, window, cx| {
            window.replace_root(cx, |_window, _cx| Settings);
        })
        .unwrap();
});

app.read(|cx| assert!(notes.read(cx).is_err()));
```

## Web differences

On the web, the page's canvas is the sole window:

* `App::open_window` supports one top-level window. A second call returns an error, and
  reopening a window after it closes isn't supported.
* Only `WindowKind::Normal` works. `PopUp`, `AnchoredPopup`, `Floating` and dialog windows
  return an error.
* The window fills the page. Of the `WindowOptions` fields, only `kind` and `show` are
  read, so `window_bounds`, `titlebar`, `window_min_size` and the resize options have no
  effect.
* `Window::minimize_window` and `Window::zoom_window` log a warning and do nothing.
  `App::quit` logs a warning and does nothing.

Use one long-lived window for a web app and handle `open_window` errors when
sharing startup code with desktop builds.

## See also

- [Windows example](crate::_documentation::examples::windows), which opens windows with
  different options and closes them from a list.
- [Share state between views](crate::_documentation::guides::sharing_state), for observing
  state that several windows show.
- [Run work in the background](crate::_documentation::guides::async_work), for working
  with a window from async code.
- [Contexts](crate::_documentation::concepts::contexts), for `Window`, `App` and
  `Context<T>`.
