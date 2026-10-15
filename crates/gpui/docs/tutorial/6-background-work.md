# 6. Background work

Make deletion reversible for four seconds. A stored task removes the undo offer
when its timer expires, while the event loop remains available for drawing and input.

## Remember what was deleted

Undo needs both the deleted todo and its former position. Store those together,
then remove by position so the item can return to the same place:

```
# use gpui::{Context, SharedString};
# struct Todo { id: usize, title: SharedString, done: bool }
struct Todos {
    todos: Vec<Todo>,
    /// The most recently deleted todo and its position, while it can be undone.
    deleted: Option<(usize, Todo)>,
    // ...the other fields, as before
}

impl Todos {
    fn delete(&mut self, id: usize, cx: &mut Context<Self>) {
        let Some(index) = self.todos.iter().position(|todo| todo.id == id) else {
            return;
        };
        self.deleted = Some((index, self.todos.remove(index)));
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some((index, todo)) = self.deleted.take() {
            self.todos.insert(index.min(self.todos.len()), todo);
            cx.notify();
        }
    }
}
```

`index.min(self.todos.len())` keeps the insertion index valid if other changes
have shortened the list before the undo is clicked.

Show the offer below the list while there is something to undo:

```
# use gpui::{Context, IntoElement, SharedString, colors::Colors, div, prelude::*};
# struct Todo { title: SharedString }
# struct Todos { deleted: Option<(usize, Todo)> }
# impl Todos { fn undo(&mut self, _: &mut Context<Self>) {} }
# fn column(todos: &Todos, colors: &Colors, cx: &mut Context<Todos>) -> impl IntoElement {
div()
    // ...the title, field, list and footer, as before
    .when_some(todos.deleted.as_ref(), |column, (_, todo)| {
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
    })
# }
```

[`when_some`](prelude::FluentBuilder::when_some) is like `when`, for an
`Option`: it adds the message only if there's a deleted todo, and gives you the
value inside.

## Expire the offer with a task

Use a task for the timeout. A handler runs on the application's event loop; a
blocking sleep there would prevent drawing and input. An async task can await a
timer and yield the loop until the timer is ready.

```
# use std::time::Duration;
# use gpui::{Context, Task};
# struct Todo;
# struct Todos { deleted: Option<(usize, Todo)>, undo_timeout: Task<()> }
# impl Todos {
#     fn delete(&mut self, cx: &mut Context<Self>) {
self.undo_timeout = cx.spawn(async move |todos, cx| {
    cx.background_executor().timer(Duration::from_secs(4)).await;
    todos
        .update(cx, |todos, cx| {
            todos.deleted = None;
            cx.notify();
        })
        .ok();
});
#     }
# }
```

Put that in `delete`, after the todo is removed, and add an
`undo_timeout: Task<()>` field to `Todos`, starting as `Task::ready(())`, a task
that has already finished.

Three details determine the lifetime and cancellation behavior:

* [`Context::spawn`] schedules the async block with the app. When the timer
  returns `Pending`, that poll ends and other app work can run; the task resumes
  when the timer is ready. An await whose input is already ready can continue
  in the same poll. [Executors and tasks](crate::_documentation::concepts::executors)
  describes this polling contract.
* [`timer`](BackgroundExecutor::timer) completes after four seconds.
* The block can't hold `&mut Todos` across an `.await`, because the rest of the
  app needs to use the view in the meantime. Instead, `spawn` gives it `todos`,
  a [`WeakEntity`] handle to the view, and `cx`, an [`AsyncApp`]. `update`
  borrows the view just for the duration of the closure.
* The handle is weak, so the task doesn't keep the view alive. If the window
  closes before the four seconds are up, `update` returns an error, and `.ok()`
  ignores it: there's nothing left to update.

## Owning the task

`spawn` returns a [`Task`], and **dropping a task cancels it**. That's why the
task is stored in a field rather than thrown away, and it makes cancelling easy.
Delete a second todo before the first offer expires, and the new task replaces
the old one in `undo_timeout`; dropping the old task cancels its timer. The
offer for the newest delete then gets its full four seconds.

If you forget to keep a task, it is dropped at the end of the statement and may
be dropped before its future is polled. When no caller needs the result, call
[`detach`](Task::detach) to remove that task handle's cancellation path. This
does not guarantee completion during app or process shutdown.

This task only waits, so the background executor is enough; no worker thread is
needed for the four-second timer. For heavy work such as parsing a large file, use
[`background_spawn`](AppContext::background_spawn) to run it on another thread
and await its result in a task like this one. [Executors and
tasks](crate::_documentation::concepts::executors) explains both, and the [async
work guide](crate::_documentation::guides::async_work) shows the patterns.

## The program so far

These are the focused additions to `Todos`: retain the deleted item and its
position, then replace the undo task whenever a new item is deleted. The first
block is a method-level reference; the complete source follows it.

```standalone_crate,no_run
use std::time::Duration;

use gpui::{Context, Entity, Focusable, SharedString, Task, Window, actions, prelude::*};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage};

# actions!(todos, [AddTodo]);
struct Todo {
    id: usize,
    title: SharedString,
    done: bool,
}

# #[derive(Clone, Copy, PartialEq)]
# enum Filter { All, Active, Done }
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
}
# fn main() {}
```

The complete `src/main.rs` at this point includes the input, stable todo ids,
filtering, the reusable checkbox component from Chapter 5, deletion, and the
cancellable undo timer. It stops before Chapter 7 adds animation and
accessibility semantics:

```standalone_crate,no_run
use std::time::Duration;

use gpui::{
    App, ClickEvent, Context, ElementId, Entity, Focusable, FontWeight, KeyBinding, QuitMode,
    SharedString, Task, Window, WindowOptions, actions, colors::Colors, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};

actions!(todos, [AddTodo]);

struct Todo { id: usize, title: SharedString, done: bool }

#[derive(IntoElement)]
struct Checkbox {
    id: ElementId,
    checked: bool,
    on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        div().id(self.id).size_5().flex_shrink_0().rounded_full().border_1()
            .border_color(colors.border).cursor_pointer()
            .when(self.checked, |checkbox| checkbox.bg(colors.selected))
            .on_click(self.on_toggle)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Filter { All, Active, Done }

struct Todos {
    todos: Vec<Todo>,
    next_id: usize,
    filter: Filter,
    input: Entity<EditableTextState>,
    deleted: Option<(usize, Todo)>,
    undo_timeout: Task<()>,
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self {
            todos: Vec::new(), next_id: 0, filter: Filter::All, input,
            deleted: None, undo_timeout: Task::ready(()),
        }
    }

    fn add(&mut self, _: &AddTodo, _: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() { return; }
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
        let Some(index) = self.todos.iter().position(|todo| todo.id == id) else { return; };
        self.deleted = Some((index, self.todos.remove(index)));
        self.undo_timeout = cx.spawn(async move |todos, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            todos.update(cx, |todos, cx| { todos.deleted = None; cx.notify(); }).ok();
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
            Filter::All => true, Filter::Active => !todo.done, Filter::Done => todo.done,
        })
    }

    fn row(&self, todo: &Todo, colors: &Colors, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let id = todo.id;
        div().id(("todo", id)).group("row").flex().items_center().gap_3().px_2().py_2()
            .rounded_md().cursor_pointer().hover(|style| style.bg(colors.container))
            .child(Checkbox {
                id: ("toggle", id).into(),
                checked: todo.done,
                on_toggle: Box::new(cx.listener(move |todos, _, _, cx| todos.toggle(id, cx))),
            })
            .child(div().flex_1().when(todo.done, |title| title.line_through()
                .text_color(colors.disabled)).child(todo.title.clone()))
            .child(div().id(("delete", id)).invisible().group_hover("row", |style| style.visible())
                .text_color(colors.disabled).cursor_pointer().child("×")
                .on_click(cx.listener(move |todos, _, _, cx| todos.delete(id, cx))))
    }

    fn filter_button(&self, filter: Filter, label: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        div().id(label).px_2().cursor_pointer()
            .when(self.filter == filter, |button| button.font_weight(FontWeight::SEMIBOLD))
            .child(label)
            .on_click(cx.listener(move |todos, _, _, cx| { todos.filter = filter; cx.notify(); }))
    }
}

impl Render for Todos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        let left = self.todos.iter().filter(|todo| !todo.done).count();
        div().key_context("Todos").on_action(cx.listener(Self::add))
            .size_full().flex().justify_center().p_8().bg(colors.background).text_color(colors.text)
            .child(div().w_full().max_w(px(480.)).flex().flex_col().gap_4()
                .child(div().text_3xl().font_weight(FontWeight::SEMIBOLD).child("Todos"))
                .child(text_input("new-todo").state(self.input.downgrade())
                    .placeholder("What needs doing?").px_3().py_2().rounded_lg()
                    .border_1().border_color(colors.border))
                .child(div().flex().flex_col().children(
                    self.visible().map(|todo| self.row(todo, &colors, cx)),
                ))
                .child(div().flex().justify_between().text_sm().text_color(colors.disabled)
                    .child(format!("{left} left"))
                    .child(div().flex().child(self.filter_button(Filter::All, "All", cx))
                        .child(self.filter_button(Filter::Active, "Active", cx))
                        .child(self.filter_button(Filter::Done, "Done", cx))))
                .when_some(self.deleted.as_ref(), |column, (_, todo)| column.child(
                    div().id("undo").px_3().py_2().rounded_lg().bg(colors.container).text_sm()
                        .cursor_pointer().child(format!("Deleted “{}”. Click to undo.", todo.title))
                        .on_click(cx.listener(|todos, _, _, cx| todos.undo(cx))),
                )))
    }
}

fn main() {
    gpui_platform::application().with_quit_mode(QuitMode::LastWindowClosed).run(|cx: &mut App| {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        cx.bind_keys([KeyBinding::new("enter", AddTodo, Some("Todos"))]);
        cx.open_window(WindowOptions::default(), |window, cx| cx.new(|cx| Todos::new(window, cx))).unwrap();
        cx.activate(true);
    });
}
```

Deletion now has an explicit undo lifetime and does not block the event loop. The
[next chapter](crate::_documentation::tutorial::ch7_polish) gives each row a
transition and gives the checkbox focus and accessibility semantics.
