use gpui::{App, KeyBinding, Menu, MenuItem, SharedString, actions};

actions!(example_template, [Quit, CloseWindow]);

/// Sets up common example boilerplate:
/// - Activates the application
/// - Sets up an app menu with the example name and a Quit action (cmd-q)
/// - Configures the app to quit when all windows are closed
pub fn example_template(cx: &mut App, name: impl Into<SharedString>) {
    cx.activate(true);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);

    cx.set_menus(vec![Menu {
        name: name.into(),
        items: vec![MenuItem::action("Quit", Quit)],
    }]);

    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
