use gpui::{App, KeyBinding, Menu, MenuItem, SharedString, actions};

actions!(example_template, [Quit, CloseWindow]);

/// Sets up common example boilerplate
pub fn example_template(cx: &mut App, name: impl Into<SharedString>) {
    /// Bring the example window to the front
    cx.activate(true);

    /// Define the quit action...
    cx.on_action(|_: &Quit, cx| cx.quit());
    /// ...then bind it to cmd+q
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);

    /// Set up an app menu with the example name and a Quit action (cmd-q)
    cx.set_menus(vec![Menu {
        name: name.into(),
        items: vec![MenuItem::action("Quit", Quit)],
    }]);

    /// Quit the app when all windows are closed
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
