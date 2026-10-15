//! The app you build in the GPUI tutorial: a small, keyboard-friendly todo list.
//!
//! Each chapter of the tutorial adds one part of this program, from the first
//! window to the tests at the bottom. Run it with `cargo run --example todos`,
//! and its tests with `cargo test --example todos`.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, Bounds, ClickEvent, Context, ElementId, Entity, Focusable,
    FontWeight, KeyBinding, QuitMode, Role, SharedString, Task, Toggled, Window, WindowBounds,
    WindowOptions, actions, colors::Colors, div, millis, prelude::*, px, size,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};

actions!(todos, [AddTodo]);

struct Todo {
    id: usize,
    title: SharedString,
    done: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Active,
    Done,
}

struct Todos {
    todos: Vec<Todo>,
    next_id: usize,
    filter: Filter,
    input: Entity<EditableTextState>,
    /// The most recently deleted todo and its position, while it can be undone.
    deleted: Option<(usize, Todo)>,
    undo_timeout: Task<()>,
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self {
            todos: Vec::new(),
            next_id: 0,
            filter: Filter::All,
            input,
            deleted: None,
            undo_timeout: Task::ready(()),
        }
    }

    fn add(&mut self, _: &AddTodo, _window: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.todos.push(Todo {
            id: self.next_id,
            title: title.into(),
            done: false,
        });
        self.next_id += 1;
        self.input.update(cx, |input, cx| input.emplace("", cx));
        cx.notify();
    }

    fn toggle(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(todo) = self.todos.iter_mut().find(|todo| todo.id == id) {
            todo.done = !todo.done;
            cx.notify();
        }
    }

    fn delete(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(index) = self.todos.iter().position(|todo| todo.id == id) else {
            return;
        };
        self.deleted = Some((index, self.todos.remove(index)));
        // Offer to undo for a few seconds. Assigning a new task drops, and so
        // cancels, the previous one.
        self.undo_timeout = cx.spawn(async move |todos, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            todos
                .update(cx, |todos, cx| {
                    todos.deleted = None;
                    cx.notify();
                })
                .ok();
        });
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some((index, todo)) = self.deleted.take() {
            self.todos.insert(index.min(self.todos.len()), todo);
            cx.notify();
        }
    }

    fn visible(&self) -> impl Iterator<Item = &Todo> {
        self.todos.iter().filter(|todo| match self.filter {
            Filter::All => true,
            Filter::Active => !todo.done,
            Filter::Done => todo.done,
        })
    }

    fn row(
        &self,
        todo: &Todo,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = todo.id;
        div()
            .id(("todo", id))
            .group("row")
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py_2()
            .rounded_md()
            .hover(|style| style.bg(colors.container))
            .child(Checkbox {
                id: ("toggle", id).into(),
                checked: todo.done,
                label: todo.title.clone(),
                on_toggle: Box::new(cx.listener(move |todos, _, _, cx| todos.toggle(id, cx))),
            })
            .child(
                div()
                    .flex_1()
                    .when(todo.done, |title| {
                        title.line_through().text_color(colors.disabled)
                    })
                    .child(todo.title.clone()),
            )
            .child(
                div()
                    .id(("delete", id))
                    .invisible()
                    .group_hover("row", |style| style.visible())
                    .text_color(colors.disabled)
                    .cursor_pointer()
                    .child("×")
                    .on_click(cx.listener(move |todos, _, _, cx| todos.delete(id, cx))),
            )
            // New rows fade in. The animation's id includes the todo's, so it
            // plays once for each todo rather than on every frame.
            .with_animation(("appear", id), Animation::new(millis(200)), |row, delta| {
                row.opacity(delta)
            })
    }

    fn filter_button(
        &self,
        filter: Filter,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .px_2()
            .cursor_pointer()
            .when(self.filter == filter, |button| {
                button.font_weight(FontWeight::SEMIBOLD)
            })
            .child(label)
            .on_click(cx.listener(move |todos, _, _, cx| {
                todos.filter = filter;
                cx.notify();
            }))
    }
}

impl Render for Todos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let left = self.todos.iter().filter(|todo| !todo.done).count();

        div()
            .key_context("Todos")
            .on_action(cx.listener(Self::add))
            .size_full()
            .flex()
            .justify_center()
            .p_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .w_full()
                    .max_w(px(480.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_3xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Todos"),
                    )
                    .child(
                        text_input("new-todo")
                            .state(self.input.downgrade())
                            .placeholder("What needs doing?")
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .border_1()
                            .border_color(colors.border),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(self.visible().map(|todo| self.row(todo, &colors, cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .text_color(colors.disabled)
                            .child(format!("{left} left"))
                            .child(
                                div()
                                    .flex()
                                    .child(self.filter_button(Filter::All, "All", cx))
                                    .child(self.filter_button(Filter::Active, "Active", cx))
                                    .child(self.filter_button(Filter::Done, "Done", cx)),
                            ),
                    )
                    .when_some(self.deleted.as_ref(), |column, (_, todo)| {
                        column.child(
                            div()
                                .id("undo")
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .bg(colors.container)
                                .text_sm()
                                .cursor_pointer()
                                .child(format!("Deleted “{}”. Click to undo.", todo.title))
                                .on_click(cx.listener(|todos, _, _, cx| todos.undo(cx))),
                        )
                    }),
            )
    }
}

/// A round checkbox. It knows nothing about todos, so any view can use it.
#[derive(IntoElement)]
struct Checkbox {
    id: ElementId,
    checked: bool,
    /// What the checkbox is for, announced by screen readers.
    label: SharedString,
    on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id(self.id)
            .role(Role::CheckBox)
            .aria_label(self.label)
            .aria_toggled(if self.checked {
                Toggled::True
            } else {
                Toggled::False
            })
            .size_5()
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .when(self.checked, |checkbox| checkbox.bg(colors.selected))
            .on_click(self.on_toggle)
    }
}

/// The text field's editing keys, and Enter to add a todo.
fn bind_keys(cx: &mut App) {
    cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
    cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(run);
}

/// Opens the example window. The browser player and the example tests call this too.
pub fn run(cx: &mut App) {
    bind_keys(cx);
    let bounds = Bounds::centered(None, size(px(560.), px(560.)), cx);
    cx.open_window(
        WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
        |window, cx| cx.new(|cx| Todos::new(window, cx)),
    )
    .unwrap();
    cx.activate(true);
}

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

    #[test]
    fn a_deleted_todo_can_be_restored_until_the_offer_expires() {
        let mut app = TestApp::new();
        let mut window = app.open_window(Todos::new);
        window.update(|todos, _, cx| {
            todos.todos.push(Todo {
                id: 0,
                title: "Bread".into(),
                done: false,
            });
            todos.delete(0, cx);
            todos.undo(cx);
        });
        window.read(|todos, _| assert_eq!(todos.todos.len(), 1));

        window.update(|todos, _, cx| todos.delete(0, cx));
        app.advance_clock(Duration::from_secs(5));
        window.read(|todos, _| assert!(todos.deleted.is_none()));
    }
}
