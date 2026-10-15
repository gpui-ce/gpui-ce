# 2. State and events

The Chapter 1 window has a title but no mutable data. Add a todo list, render one
row per item, and let a click toggle that item.

## Give the view some state

A view's state is the data in its fields. Add a `Todo` type and give `Todos` a
list of them, with a few starter items:

```
use gpui::SharedString;

struct Todo {
    title: SharedString,
    done: bool,
}

struct Todos {
    todos: Vec<Todo>,
}

impl Todos {
    fn new() -> Self {
        Self {
            todos: vec![
                Todo { title: "Buy milk".into(), done: false },
                Todo { title: "Water the plants".into(), done: true },
                Todo { title: "Call Sam".into(), done: false },
            ],
        }
    }
}
```

[`SharedString`] is GPUI's immutable string type for displayed text. Cloning it
is cheap, which matters because `render` rebuilds the element tree whenever the
view changes.

Create the view with `Todos::new()` in `main`:

```
# use gpui::{App, AppContext, Context, Window, WindowOptions, div, prelude::*};
# struct Todos;
# impl Todos { fn new() -> Self { Todos } }
# impl Render for Todos {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
# }
# fn open(cx: &mut App) {
cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_cx| Todos::new()))
    .unwrap();
# }
```

## Render a row per todo

`render` maps the state to elements. [`children`](ParentElement::children) adds
one child for each item produced by an iterator:

```
# use gpui::{Context, SharedString, Window, colors::Colors, div, prelude::*};
# struct Todo { title: SharedString, done: bool }
# struct Todos { todos: Vec<Todo> }
impl Render for Todos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .p_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_3xl().child("Todos"))
            .children(self.todos.iter().enumerate().map(|(index, todo)| {
                div()
                    .id(index)
                    .when(todo.done, |row| row.line_through())
                    .child(todo.title.clone())
                    .on_click(cx.listener(move |todos, _event, _window, cx| {
                        let todo = &mut todos.todos[index];
                        todo.done = !todo.done;
                        cx.notify();
                    }))
            }))
    }
}
```

Run it and click a task. The handler flips `done`, notifies the view, and the next
render adds or removes the strikethrough.

## What happens when you click

The click reaches the redraw through three linked pieces:

**`id` makes the element interactive.** Clicking involves state that lasts
longer than one frame: GPUI has to remember that the mouse went down on this row
before it can report a click when the mouse comes up. Elements are rebuilt on
every frame, so GPUI keeps that state for you, keyed by the element's id. Only
elements with an [`id`](InteractiveElement::id) have methods like
[`on_click`](StatefulInteractiveElement::on_click). Ids must be unique among
their siblings and stable from one frame to the next. The row's index works for
now.

**`cx.listener` connects a handler to the view.** An element's event handler
receives the event, the window and the app, but not your view: the element
doesn't know which view created it. [`Context::listener`] wraps your closure so
that it also receives `&mut Todos`, the view that was rendering. Inside it you
can change the view's fields directly.

**`cx.notify()` asks for a redraw.** GPUI does not watch your fields. When an
event handler changes state that affects what's on screen, call
[`notify`](Context::notify). GPUI then renders the window again before the next
frame, and your `render` method draws the todo in its new state. Forgetting to
notify is the most common reason a GPUI view doesn't update.

**`when` styles conditionally.** [`when`](prelude::FluentBuilder::when) applies a
closure to the element only if a condition holds, so you can keep building the
element in one expression.

## The program so far

<details>
<summary>src/main.rs</summary>

```no_run
use gpui::{App, Context, QuitMode, SharedString, Window, WindowOptions, colors::Colors, div, prelude::*};

struct Todo {
    title: SharedString,
    done: bool,
}

struct Todos {
    todos: Vec<Todo>,
}

impl Todos {
    fn new() -> Self {
        Self {
            todos: vec![
                Todo { title: "Buy milk".into(), done: false },
                Todo { title: "Water the plants".into(), done: true },
                Todo { title: "Call Sam".into(), done: false },
            ],
        }
    }
}

impl Render for Todos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);

        div()
            .size_full()
            .p_8()
            .bg(colors.background)
            .text_color(colors.text)
            .child(div().text_3xl().child("Todos"))
            .children(self.todos.iter().enumerate().map(|(index, todo)| {
                div()
                    .id(index)
                    .when(todo.done, |row| row.line_through())
                    .child(todo.title.clone())
                    .on_click(cx.listener(move |todos, _event, _window, cx| {
                        let todo = &mut todos.todos[index];
                        todo.done = !todo.done;
                        cx.notify();
                    }))
            }))
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_window, cx| cx.new(|_cx| Todos::new()))
                .unwrap();
            cx.activate(true);
        });
}
```

</details>

The state now changes through the full event path. The rows are still an
unstructured stack, so Chapter 3 gives that same tree a usable layout and derives
the unfinished count from the list. In the [next chapter](crate::_documentation::tutorial::ch3_layout_and_style),
you use the same render function to add structure, spacing, and a derived count.
