# 7. Polish

The app is complete enough to use. Add a short fade for newly inserted rows and
give each checkbox the role, label, and state that assistive technology needs.

## Fade in new todos

Any element can animate. [`with_animation`](AnimationExt::with_animation) wraps
an element with an [`Animation`] and a closure that receives the animation's
progress, from `0.0` to `1.0`, and returns the element as it should look at that
moment. Add it at the end of `row`:

```
# use gpui::{Animation, AnimationExt, IntoElement, div, millis, prelude::*};
# fn row(id: usize) -> impl IntoElement {
div()
    .id(("todo", id))
    // ...the row, as before
    .with_animation(("appear", id), Animation::new(millis(200)), |row, delta| {
        row.opacity(delta)
    })
# }
```

Like a click, an animation has state that outlives a frame: when it started.
GPUI keeps that state under the id you give it, so the id decides when the
animation plays. Including the todo's id means each todo fades in once, when its
row first appears, and stays put on every frame after that.

While an animation runs, GPUI requests frames for it; when it ends, those
requests stop. If your app turns on
[`App::set_reduce_motion`], for example because the user prefers less motion,
GPUI skips the animation and shows the finished state.

## Make the checkbox accessible

Screen readers receive an interface description: each element's role, name, and
state. A round outline that fills in looks like a checkbox, but without those
properties it is only an unnamed box to assistive technology. Keyboard users
also need a tab stop and an action for Space; the same toggle handler can serve
mouse clicks, key presses, and AccessKit's click action.

Here is a complete checkbox exercised in a headless test. Enable the
`test-support` feature on `gpui` when running this example outside the
documentation. The element's click listener is also the
AccessKit `Click` action, while its `ToggleCheckbox` action handles Space after
the checkbox receives focus:

```standalone_crate
use gpui::{
    AnyWindowHandle, App, Context, FocusHandle, KeyBinding, Role, TestApp, Toggled, Window,
    actions, accesskit, div, prelude::*,
};

actions!(todos, [NextCheckbox, ToggleCheckbox]);

struct CheckboxDemo { checked: bool, focus_handle: FocusHandle }

impl CheckboxDemo {
    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.checked = !self.checked;
        cx.notify();
    }
}

impl Render for CheckboxDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("CheckboxDemo")
            .id("done")
            .role(Role::CheckBox)
            .aria_label("Done")
            .aria_toggled(Toggled::from(self.checked))
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &NextCheckbox, window, cx| window.focus_next(cx)))
            .focus_visible(|style| style.border_color(gpui::blue()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleCheckbox, _, cx| {
                this.toggle(cx);
            }))
            .child(if self.checked { "Done" } else { "Todo" })
    }
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextCheckbox, Some("CheckboxDemo")),
        KeyBinding::new("space", ToggleCheckbox, Some("CheckboxDemo")),
    ]);
}

fn main() {
    let mut app = TestApp::new();
    app.update(bind_keys);
    let mut window = app.open_window(|_, cx| CheckboxDemo {
        checked: false,
        focus_handle: cx.focus_handle().tab_stop(true),
    });
    window.update(|checkbox, window, cx| window.focus(&checkbox.focus_handle, cx));

    window.simulate_keystrokes("tab space");
    window.read(|checkbox, _| assert!(checkbox.checked));

    window.update(|_, window, _| window.set_a11y_forced(true));
    window.draw();
    let node = window.update(|_, window, _| {
        window.a11y_tree().unwrap().nodes.iter()
            .find(|(_, node)| node.label() == Some("Done"))
            .map(|(id, node)| (*id, node.role(), node.toggled()))
            .unwrap()
    });
    assert_eq!(node.1, Role::CheckBox);
    assert_eq!(node.2, Some(accesskit::Toggled::True));

    let handle: AnyWindowHandle = window.handle().into();
    app.update(|cx| {
        handle.update(cx, |_, window, cx| {
            window.perform_a11y_action(node.0, accesskit::Action::Click, None, cx)
        })
    }).unwrap();
    window.read(|checkbox, _| assert!(!checkbox.checked));
}
```

The view retains its focus handle and marks it as a tab stop. GPUI does not bind
Tab automatically: `NextCheckbox` calls `Window::focus_next`, which uses the last
rendered tab-stop map. With one stop, Tab wraps to that same checkbox; Space then
runs `ToggleCheckbox`. The stable element id preserves its accessibility node
across renders. The test forces
an accessibility tree, checks the role and toggled state, then asks the window to
perform AccessKit's `Click` action. That action runs the same click listener as a
mouse click, so the final assertion proves that keyboard and assistive-technology
activation reach the same state change. See the [accessibility guide](crate::_documentation::guides::accessibility)
for custom actions and disabled controls.

The isolated test proves the interaction. The task-list integration below keeps
that contract when rows are rebuilt from `Todos::visible`.

GPUI builds the accessibility tree from elements that have an id and a role, and
passes it to the operating system through
[AccessKit](https://accesskit.dev). The [accessibility
guide](crate::_documentation::guides::accessibility) covers roles, states, and
keyboard access.

### Carry the behavior into `Todos`

The isolated test above proves the input and accessibility paths. The complete
program below applies the same contract to the task list from Chapter 6. A
`FocusHandle` lives in `Todos` for each stable todo ID, so rebuilding a row does
not discard keyboard focus or its accessibility node. Deleting a todo removes
that handle only when the undo offer is replaced or expires; undo keeps it and
restores the same control.

The checkbox uses one parent callback for mouse clicks, Space, and AccessKit's
`Click` action. Tab is a separate `NextCheckbox` action on the `Todos` root:
`Window::focus_next` reads the tab stops from the last rendered frame and walks
the visible rows in their rendered order. The row also keeps the stable ID in its
animation key, so a new todo fades in once without replaying when its state
changes.

```standalone_crate,no_run
use std::{collections::HashMap, time::Duration};

use gpui::{
    Animation, AnimationExt, App, ClickEvent, Context, ElementId, Entity, FocusHandle,
    Focusable, FontWeight, KeyBinding, Role, SharedString, Task, Toggled, Window, WindowOptions,
    actions, colors::Colors, div, millis, prelude::*, px,
};
use gpui_ce_elements::editable_text::{
    EditableTextState, StringStorage,
    actions::{DEFAULT_INPUT_CONTEXT, default_bindings},
    text_input,
};

actions!(todos, [AddTodo, NextCheckbox, ToggleCheckbox]);

struct Todo { id: usize, title: SharedString, done: bool }

#[derive(IntoElement)]
struct Checkbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    focus_handle: FocusHandle,
    on_toggle: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    on_key_toggle: Box<dyn Fn(&ToggleCheckbox, &mut Window, &mut App)>,
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let colors = Colors::for_appearance(window);
        div()
            .id(self.id)
            .key_context("Checkbox")
            .track_focus(&self.focus_handle)
            .role(Role::CheckBox)
            .aria_label(self.label)
            .aria_toggled(Toggled::from(self.checked))
            .focus_visible(|style| style.border_color(gpui::blue()))
            .on_click(self.on_toggle)
            .on_action(self.on_key_toggle)
            .size_5()
            .flex_shrink_0()
            .rounded_full()
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .when(self.checked, |checkbox| checkbox.bg(colors.selected))
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
    checkbox_focus: HashMap<usize, FocusHandle>,
}

impl Todos {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| EditableTextState::new(StringStorage::default(), cx));
        window.focus(&input.focus_handle(cx), cx);
        Self {
            todos: Vec::new(), next_id: 0, filter: Filter::All, input,
            deleted: None, undo_timeout: Task::ready(()), checkbox_focus: HashMap::new(),
        }
    }

    fn add(&mut self, _: &AddTodo, _: &mut Window, cx: &mut Context<Self>) {
        let title = self.input.read(cx).as_str().trim().to_string();
        if title.is_empty() { return; }
        let id = self.next_id;
        self.next_id += 1;
        self.checkbox_focus.insert(id, cx.focus_handle().tab_stop(true));
        self.todos.push(Todo { id, title: title.into(), done: false });
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
        if let Some((_, previous)) = self.deleted.take() {
            self.checkbox_focus.remove(&previous.id);
        }
        let todo = self.todos.remove(index);
        let deleted_id = todo.id;
        self.deleted = Some((index, todo));
        self.undo_timeout = cx.spawn(async move |todos, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            todos.update(cx, |todos, cx| {
                if todos.deleted.as_ref().is_some_and(|(_, todo)| todo.id == deleted_id) {
                    todos.deleted = None;
                    todos.checkbox_focus.remove(&deleted_id);
                    cx.notify();
                }
            }).ok();
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
        let focus_handle = self.checkbox_focus[&id].clone();
        div().id(("todo", id)).group("row").flex().items_center().gap_3().px_2().py_2()
            .rounded_md().cursor_pointer().hover(|style| style.bg(colors.container))
            .with_animation(("appear", id), Animation::new(millis(200)), |row, delta| row.opacity(delta))
            .child(Checkbox {
                id: ("toggle", id).into(), label: todo.title.clone(), checked: todo.done,
                focus_handle,
                on_toggle: Box::new(cx.listener(move |todos, _, _, cx| todos.toggle(id, cx))),
                on_key_toggle: Box::new(cx.listener(move |todos, _: &ToggleCheckbox, _, cx| todos.toggle(id, cx))),
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
        div().key_context("Todos")
            .on_action(cx.listener(Self::add))
            .on_action(cx.listener(|_, _: &NextCheckbox, window, cx| window.focus_next(cx)))
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
    gpui_platform::application().with_quit_mode(gpui::QuitMode::LastWindowClosed).run(|cx: &mut App| {
        cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        cx.bind_keys([
            KeyBinding::new("enter", AddTodo, Some("Todos")),
            KeyBinding::new("tab", NextCheckbox, Some("Todos")),
            KeyBinding::new("space", ToggleCheckbox, Some("Checkbox")),
        ]);
        cx.open_window(WindowOptions::default(), |window, cx| cx.new(|cx| Todos::new(window, cx))).unwrap();
        cx.activate(true);
    });
}
```

The timer and handle map follow the same ownership rule: dropping the stored
`Task` cancels the previous undo timer, while the `FocusHandle` remains until
that todo's stable ID leaves the undo state. The final source now contains the
whole app, so the preceding isolated test can be used for focused behavior
checks and this block can be copied as the tutorial's complete result.
## Next steps

Some ideas if you want to keep going:

* Edit a todo by double-clicking it. [`ClickEvent`] tells you how many times the
  user clicked.
* Move between todos with the arrow keys: actions bound in the `Todos` context,
  and a selected index in the view's state.
* Save the list to a file whenever it changes, in a task, with
  [`background_spawn`](AppContext::background_spawn) to write it off the main
  thread.

The last chapter checks the behavior built so far: [chapter
8](crate::_documentation::tutorial::ch8_testing) drives the app without a
display and verifies input and timeout behavior.
