# 8. Testing

The app is assembled; now make its two important interactions executable: type a
todo and press Enter, then delete one and advance the undo timer. `TestApp` runs
both paths without a display.

## Let tests set up the app

Tests need the same key bindings as the application. Move them out of `main` into
a function that both entry points call:

```standalone_crate
# use gpui::{App, KeyBinding, actions};
# use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};
# actions!(todos, [AddTodo]);
/// The text field's editing keys, and Enter to add a todo.
fn bind_keys(cx: &mut App) {
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);
}
```

and call `bind_keys(cx)` at the start of the closure in `main`.

GPUI's testing tools are behind its `test-support` feature, so enable it for
tests only:

```toml
[dev-dependencies]
gpui = { package = "gpui-ce", git = "https://github.com/gpui-ce/gpui-ce", features = ["test-support"] }
```

## Test typing a todo

Add a test module at the bottom of `src/main.rs`:

```standalone_crate,no_run
# use gpui::{App, Context, Entity, Focusable, KeyBinding, SharedString, Window, actions, div, prelude::*};
# use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, actions::{DEFAULT_INPUT_CONTEXT, default_bindings}, text_input};
# actions!(todos, [AddTodo]);
# struct Todo { title: SharedString }
# struct Todos { todos: Vec<Todo>, input: Entity<EditableTextState> }
# impl Todos {
#     fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
#         let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
#         window.focus(&input.focus_handle(cx), cx);
#         Self { todos: Vec::new(), input }
#     }
#     fn add(&mut self, _: &AddTodo, _: &mut Window, cx: &mut Context<Self>) {
#         let title = self.input.read(cx).as_str().trim().to_string();
#         self.todos.push(Todo { title: title.into() });
#         self.input.update(cx, |input, cx| input.emplace("", cx));
#     }
# }
# impl Render for Todos {
#     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
#         div().key_context("Todos").on_action(cx.listener(Self::add)).child(text_input("new-todo").state(self.input.downgrade()))
#     }
# }
# fn bind_keys(cx: &mut App) {
#     cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
#     cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);
# }
# fn main() {}
#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestApp;

    #[test]
    fn enter_adds_a_todo_and_clears_the_field() {
        let mut app = TestApp::new();
        app.update(bind_keys);
        let mut window = app.open_window(Todos::new);

        window.simulate_input("Milk");
        window.simulate_keystrokes("enter");

        window.read(|todos, cx| {
            assert_eq!(todos.todos.len(), 1);
            assert_eq!(todos.todos[0].title, "Milk");
            assert_eq!(todos.input.read(cx).as_str(), "");
        });
    }
}
```

Run it with `cargo test`.

[`TestApp`] is a complete GPUI app with a pretend platform: windows exist, render
and lay out, but nothing is drawn to a screen. [`open_window`](TestApp::open_window)
builds your view the same way `main` does. The window it returns can simulate
input, like typing characters and pressing keys, and can read or update the view.

This follows the production path: focus sends characters to the field, Enter
matches the `Todos` binding, and `add` mutates the list. A broken focus, binding,
or handler fails at the boundary where it matters.

## Test the undo timeout

Tasks and timers are testable too. `TestApp` uses a simulated clock that advances
only when requested, so the four-second timeout takes no wall-clock time:

```standalone_crate,no_run
# use std::time::Duration;
# use gpui::{Context, SharedString, Task, Window, div, prelude::*};
# struct Todo { id: usize, title: SharedString, done: bool }
# struct Todos { todos: Vec<Todo>, deleted: Option<(usize, Todo)>, undo_timeout: Task<()> }
# impl Todos {
#     fn new(_: &mut Window, _: &mut Context<Self>) -> Self { Self { todos: Vec::new(), deleted: None, undo_timeout: Task::ready(()) } }
#     fn delete(&mut self, id: usize, cx: &mut Context<Self>) {
#         let Some(index) = self.todos.iter().position(|todo| todo.id == id) else { return };
#         self.deleted = Some((index, self.todos.remove(index)));
#         self.undo_timeout = cx.spawn(async move |todos, cx| {
#             cx.background_executor().timer(Duration::from_secs(4)).await;
#             todos.update(cx, |todos, cx| { todos.deleted = None; cx.notify(); }).ok();
#         });
#     }
#     fn undo(&mut self, _: &mut Context<Self>) {
#         if let Some((index, todo)) = self.deleted.take() { self.todos.insert(index, todo); }
#     }
# }
# impl Render for Todos {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
# }
# fn main() {}
# #[cfg(test)]
# mod tests {
#     use super::*;
#     use gpui::TestApp;
#[test]
fn a_deleted_todo_can_be_restored_until_the_offer_expires() {
    let mut app = TestApp::new();
    let mut window = app.open_window(Todos::new);
    window.update(|todos, _, cx| {
        todos.todos.push(Todo { id: 0, title: "Bread".into(), done: false });
        todos.delete(0, cx);
        todos.undo(cx);
    });
    window.read(|todos, _| assert_eq!(todos.todos.len(), 1));

    window.update(|todos, _, cx| todos.delete(0, cx));
    app.advance_clock(Duration::from_secs(5));
    window.read(|todos, _| assert!(todos.deleted.is_none()));
}
# }
```

[`advance_clock`](TestApp::advance_clock) moves the simulated time forward and
runs everything that became ready, including the task waiting on the timer.
The test advances the clock past the deadline before reading the view. The
explicit clock makes the assertion independent of wall-clock and scheduler timing.

## Where to go next

You now have a complete GPUI app with headless checks for input and timeout
behavior. The [finished todos
example](crate::_documentation::examples::todos) contains the full source. From
here, use the documentation according to what you want to learn next:

* The [guides](crate::_documentation::guides) solve specific problems, from
  [long lists](crate::_documentation::guides::lists_and_scrolling) to [custom
  drawing](crate::_documentation::guides::custom_drawing).
* The [concepts](crate::_documentation::concepts) explain how GPUI's pieces fit
  together, starting with [ownership and data flow](crate::_documentation::concepts::ownership).
* The [examples](crate::_documentation::examples) show each part of GPUI in a
  complete program.
* The [testing guide](crate::_documentation::guides::testing) covers
  `#[gpui::test]` for async tests, randomized scheduling, and more.
