//! Common helpers for GPUI examples. Import with:
//!
//! ```ignore
//! #[path = "../shared/prelude.rs"]
//! mod example_prelude;
//! use example_prelude::init_example;
//! ```

use gpui::{App, KeyBinding, Menu, MenuItem, SharedString, actions};

actions!(example, [Quit, CloseWindow]);

/// Activates the app, adds a named menu with Cmd-Q, and quits when all windows close.
pub fn init_example(cx: &mut App, name: impl Into<SharedString>) {
    cx.activate(true);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);

    cx.set_menus(vec![Menu {
        name: name.into(),
        items: vec![MenuItem::action("Quit", Quit)],
        disabled: false,
    }]);

    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
