# 5. Components

The app can add todos but cannot remove or filter them. Add both operations, then
separate the checkbox from the row so its visual contract does not own todo state.

## Give todos stable ids

Until now each row used its position as its element id. Deleting the first row
would give every later row a different id, so GPUI could attach the old row's
interaction state to the wrong todo. Give each todo a stable id instead.

Give each todo an id of its own, and find todos by id instead of by position:

```
# use gpui::{Context, SharedString};
struct Todo {
    id: usize,
    title: SharedString,
    done: bool,
}

# struct Todos { todos: Vec<Todo>, next_id: usize }
impl Todos {
    fn toggle(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(todo) = self.todos.iter_mut().find(|todo| todo.id == id) {
            todo.done = !todo.done;
            cx.notify();
        }
    }

    fn delete(&mut self, id: usize, cx: &mut Context<Self>) {
        self.todos.retain(|todo| todo.id != id);
        cx.notify();
    }
}
```

Add a `next_id: usize` field to `Todos`, and in `add`, give the new todo
`self.next_id` and increment it.

## Make a checkbox component

The checkbox needs an element ID, a checked value, and a callback; it does not need
to know what a `Todo` is. That boundary makes it a [`RenderOnce`] component: the
parent supplies state and meaning, while the component supplies the element tree.

```
# use gpui::{App, ClickEvent, ElementId, Window, colors::Colors, div, prelude::*};
#[derive(IntoElement)]
struct Checkbox {
    id: ElementId,
    checked: bool,
    on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id(self.id)
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
```

`#[derive(IntoElement)]` lets you pass `Checkbox` to `child`. Unlike a view's
`render`, `RenderOnce::render` takes the component by value: GPUI consumes and
rebuilds it on each frame. The component does not own todo state; its callback
lets the parent decide what a toggle means.

At this point the row can be split at three different boundaries:

* **A method** that returns `impl IntoElement`, like `row`. It can read the
  view's fields directly, so it's the simplest choice for a piece of one view.
* **A component**, like `Checkbox`: a struct with the data it shows. Use it for
  pieces you reuse across views.
* **A view**: another entity that implements [`Render`]. Use it for pieces with
  state of their own, like the text field's
  `EditableTextState`.

[Elements, components and views](crate::_documentation::concepts::elements)
describes all three in depth.

## Delete on hover

Use the checkbox in `row`, then add a delete button that appears while the
pointer is over the row:

```
# use gpui::{App, ClickEvent, Context, ElementId, SharedString, Window, colors::Colors, div, prelude::*};
# #[derive(IntoElement)]
# struct Checkbox { id: ElementId, checked: bool, on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)> }
# impl RenderOnce for Checkbox {
#     fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement { div() }
# }
# struct Todo { id: usize, title: SharedString, done: bool }
# struct Todos;
# impl Todos {
#     fn toggle(&mut self, _: usize, _: &mut Context<Self>) {}
#     fn delete(&mut self, _: usize, _: &mut Context<Self>) {}
fn row(&self, todo: &Todo, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            on_toggle: Box::new(cx.listener(move |todos, _, _, cx| todos.toggle(id, cx))),
        })
        .child(
            div()
                .flex_1()
                .when(todo.done, |title| title.line_through().text_color(colors.disabled))
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
}
# }
```

[`group`](InteractiveElement::group) names the row, and
[`group_hover`](InteractiveElement::group_hover) styles the delete button when
the pointer is anywhere over the group, not just over the button itself.

The ids are pairs such as `("toggle", id)`. An id only has to be unique among
siblings, but the prefix keeps the row, checkbox, and delete button distinct as
the row is rebuilt.

The `move` listeners outlive this call. Copying `todo.id` into `id` lets them
own the value they need instead of borrowing the row that is being rebuilt.

## Filter the list

Add a filter to the view's state. An enum describes exactly the three choices:

```
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Active,
    Done,
}
```

Store `filter: Filter` in `Todos`, starting at `Filter::All`. A method returns
the todos the filter lets through, and `render` shows those:

```
# #[derive(Clone, Copy, PartialEq)] enum Filter { All, Active, Done }
# struct Todo { done: bool }
# struct Todos { todos: Vec<Todo>, filter: Filter }
# impl Todos {
fn visible(&self) -> impl Iterator<Item = &Todo> {
    self.todos.iter().filter(|todo| match self.filter {
        Filter::All => true,
        Filter::Active => !todo.done,
        Filter::Done => todo.done,
    })
}
# }
```

Then put three buttons in the footer, next to the count:

```
# use gpui::{Context, FontWeight, IntoElement, div, prelude::*};
# #[derive(Clone, Copy, PartialEq)] enum Filter { All, Active, Done }
# struct Todos { filter: Filter }
# impl Todos {
fn filter_button(&self, filter: Filter, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
    div()
        .id(label)
        .px_2()
        .cursor_pointer()
        .when(self.filter == filter, |button| button.font_weight(FontWeight::SEMIBOLD))
        .child(label)
        .on_click(cx.listener(move |todos, _, _, cx| {
            todos.filter = filter;
            cx.notify();
        }))
}
# }
```

Choosing a filter changes one field and notifies. The next `render` uses that
field for the visible rows and the selected button. The unfinished count remains
derived from the full list, so it continues to answer “how many active todos are
left?” regardless of which filter is selected.

## The program so far

<details>
<summary>src/main.rs</summary>

```standalone_crate,no_run
use gpui::{
    App, ClickEvent, Context, ElementId, Entity, Focusable, FontWeight, KeyBinding, QuitMode,
    SharedString, Window, WindowOptions, actions, colors::Colors, div, prelude::*, px,
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
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self { todos: Vec::new(), next_id: 0, filter: Filter::All, input }
    }

    fn add(&mut self, _: &AddTodo, _window: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() {
            return;
        }
        self.todos.push(Todo { id: self.next_id, title: title.into(), done: false });
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
        self.todos.retain(|todo| todo.id != id);
        cx.notify();
    }

    fn visible(&self) -> impl Iterator<Item = &Todo> {
        self.todos.iter().filter(|todo| match self.filter {
            Filter::All => true,
            Filter::Active => !todo.done,
            Filter::Done => todo.done,
        })
    }

    fn row(&self, todo: &Todo, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
                on_toggle: Box::new(cx.listener(move |todos, _, _, cx| todos.toggle(id, cx))),
            })
            .child(
                div()
                    .flex_1()
                    .when(todo.done, |title| title.line_through().text_color(colors.disabled))
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
    }

    fn filter_button(&self, filter: Filter, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(label)
            .px_2()
            .cursor_pointer()
            .when(self.filter == filter, |button| button.font_weight(FontWeight::SEMIBOLD))
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
                    ),
            )
    }
}

#[derive(IntoElement)]
struct Checkbox {
    id: ElementId,
    checked: bool,
    on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .id(self.id)
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

Deleting is currently instant and final. In the [next
chapter](crate::_documentation::tutorial::ch6_background_work), a task keeps an
undo message alive for four seconds after a delete.
