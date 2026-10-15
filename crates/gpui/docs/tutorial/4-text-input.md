# 4. Text input

The list still contains only starter data. Add a text field above it, then bind
Enter to an action that appends its text to the list.

Three GPUI mechanisms meet here: an *entity* owns editing state, *focus* routes
keystrokes to that state, and an *action* gives Enter a name that can be bound in
the surrounding view.

## Add the text field

Text editing lives in a companion crate, `gpui_ce_elements`, so add it to
`Cargo.toml` next to GPUI:

```toml
gpui_ce_elements = { git = "https://github.com/gpui-ce/gpui-ce" }
```

A text field has state of its own: the text, the cursor, the selection, the undo
history. That state lives in an [`Entity`], an
`EditableTextState`,
which your view creates and keeps. Remove the starter todos, and create the
field's state instead:

```
# use gpui::{Context, Entity, Focusable, SharedString, Window, prelude::*};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage};

# struct Todo { title: SharedString, done: bool }
struct Todos {
    todos: Vec<Todo>,
    input: Entity<EditableTextState>,
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self { todos: Vec::new(), input }
    }
}
```

`cx.new` works as it did in chapter 1: the app owns the field state and returns
an [`Entity`] handle. `Todos` keeps that handle so the action can read and clear
the field. Focusing its handle routes keyboard input there when the window opens.
At most one element in a window has focus at a time. Keystrokes go to that
element, and clicking another field moves focus there.

`new` now needs the window, so create the view with
`cx.new(|cx| Todos::new(window, cx))` in `main`.

Then show the field in `render`, right after the title. The
`text_input` element draws the
state you pass it:

```
# use gpui::{Entity, IntoElement, colors::Colors, prelude::*};
# use gpui_ce_elements::editable_text::{EditableTextState, text_input};
# fn field(input: &Entity<EditableTextState>, colors: &Colors) -> impl IntoElement {
text_input("new-todo")
    .state(input.downgrade())
    .placeholder("What needs doing?")
    .px_3()
    .py_2()
    .rounded_lg()
    .border_1()
    .border_color(colors.border)
# }
```

## Bind Enter to an action

Editing keys like Backspace and the arrows need bindings too. The text field
comes with a standard set; install them once, in `main`, before opening the
window:

```standalone_crate
# use gpui::App;
use gpui_ce_elements::editable_text::actions::{DEFAULT_INPUT_CONTEXT, default_bindings};

# fn setup(cx: &mut App) {
cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
# }
```

Enter should add the todo, but the view should not inspect raw key events. GPUI
separates the operation from the key that invokes it. You declare the operation as
an [action](Action): a type
that names a command. Then you bind a key to it, and handle it in the view:

```standalone_crate
use gpui::{App, KeyBinding, actions};

actions!(todos, [AddTodo]);

fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);
}
```

The third argument to [`KeyBinding::new`] is a *key context*. The binding applies
when the focused element is inside an element that declared that context.
Declare it on the view's root element, alongside the handler:

```standalone_crate
# use gpui::{Context, Entity, SharedString, Window, actions, div, prelude::*};
# use gpui_ce_elements::editable_text::EditableTextState;
# actions!(todos, [AddTodo]);
# struct Todo { title: SharedString, done: bool }
# struct Todos { todos: Vec<Todo>, input: Entity<EditableTextState> }
impl Todos {
    fn add(&mut self, _: &AddTodo, _window: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.todos.push(Todo { title: title.into(), done: false });
        self.input.update(cx, |input, cx| input.emplace("", cx));
        cx.notify();
    }
}

# impl Render for Todos {
#     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
div()
    .key_context("Todos")
    .on_action(cx.listener(Self::add))
    // ...the rest of the view, as before
#     }
# }
```

When Enter is pressed, GPUI searches the contexts around the focused element for a
binding whose key matches. The text field is inside `Todos`, so
`enter` → `AddTodo` matches. GPUI dispatches `AddTodo` from the focused element
up through its ancestors until a handler takes it: your `add` method.

`add` reads the field's text through its entity, adds a todo, and clears the field
with `emplace`. Reading another entity with
[`read`](Entity::read) and changing it with [`update`](Entity::update) is how
views work with each other's state.

Actions may look like a long way round compared to listening for the Enter key
directly. They pay off as an app grows: the same action can be bound to
different keys, triggered from a menu or a button, rebound by the user, and
listed with its shortcut in a command palette. [Input and
dispatch](crate::_documentation::concepts::input_dispatch) explains how GPUI
routes keys and actions.

## The program so far

<details>
<summary>src/main.rs</summary>

```standalone_crate,no_run
use gpui::{
    App, Context, Entity, Focusable, FontWeight, KeyBinding, QuitMode, SharedString, Window,
    WindowOptions, actions, colors::Colors, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};

actions!(todos, [AddTodo]);

struct Todo {
    title: SharedString,
    done: bool,
}

struct Todos {
    todos: Vec<Todo>,
    input: Entity<EditableTextState>,
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self { todos: Vec::new(), input }
    }

    fn add(&mut self, _: &AddTodo, _window: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.todos.push(Todo { title: title.into(), done: false });
        self.input.update(cx, |input, cx| input.emplace("", cx));
        cx.notify();
    }

    fn row(&self, index: usize, todo: &Todo, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id(index)
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(colors.container))
            .child(
                div()
                    .size_5()
                    .rounded_full()
                    .border_1()
                    .border_color(colors.border)
                    .when(todo.done, |checkbox| checkbox.bg(colors.selected)),
            )
            .child(
                div()
                    .flex_1()
                    .when(todo.done, |title| title.line_through().text_color(colors.disabled))
                    .child(todo.title.clone()),
            )
            .on_click(cx.listener(move |todos, _event, _window, cx| {
                let todo = &mut todos.todos[index];
                todo.done = !todo.done;
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
                    .child(div().text_3xl().font_weight(FontWeight::SEMIBOLD).child("Todos"))
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
                        div().flex().flex_col().children(
                            self.todos
                                .iter()
                                .enumerate()
                                .map(|(index, todo)| self.row(index, todo, &colors, cx)),
                        ),
                    )
                    .child(div().text_sm().text_color(colors.disabled).child(format!("{left} left"))),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
            cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);

            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| Todos::new(window, cx))
            })
            .unwrap();
            cx.activate(true);
        });
}
```

</details>

The field now supplies the text that Chapter 2 stored and Chapter 3 laid out.
Chapter 5 will reuse the same event boundary when rows gain independent controls.
In the [next chapter](crate::_documentation::tutorial::ch5_components),
deleting and filtering make stable ids necessary, and the row becomes a reusable
component.
