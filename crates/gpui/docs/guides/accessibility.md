# Make your app accessible

Assistive technology reads an [AccessKit] tree describing the controls in a
GPUI window. Supply each control's name, role, state and supported actions,
then test the rendered window for keyboard focus, contrast and motion too.

## Give each control a role and an accessible name

An interactive element needs an [`id`][`InteractiveElement::id`] and a
[`role`][`StatefulInteractiveElement::role`] other than `GenericContainer` to
appear as a node. A label is optional for tree membership, but a control
needs a name that identifies its purpose. Set that name with
[`aria_label`][StatefulInteractiveElement::aria_label]: `Save`, for example,
rather than `save_button`.

The button below has the name Save even as its visible counter changes.
`TestApp` forces accessibility on to inspect the node without connecting a
screen reader, then invokes its Click action:

```rust
use gpui::{AnyWindowHandle, Context, Role, TestApp, Window, accesskit, div, prelude::*};

struct SaveButton {
    saves: usize,
}

impl Render for SaveButton {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("save")
            .role(Role::Button)
            .aria_label("Save")
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.saves += 1;
                cx.notify();
            }))
            .child(format!("Saved {} times", self.saves))
    }
}

let mut app = TestApp::new();
let mut window = app.open_window(|_, _| SaveButton { saves: 0 });

// The tree is built only while assistive technology is connected, or when
// it is forced. Forcing it lets a test read the tree.
window.update(|_, window, _| window.set_a11y_forced(true));
window.draw();

let (button, role) = window.update(|_, window, _| {
    let tree = window.a11y_tree().expect("the tree is built while forced");
    let (id, node) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Save"))
        .expect("the save button is in the tree");
    (*id, node.role())
});
assert_eq!(role, Role::Button);

// Let an assistive technology click the button. This runs the same handler
// as a mouse click. The handler updates the view, so go through the window
// handle, which doesn't hold the view while the closure runs.
let handle: AnyWindowHandle = window.handle().into();
app.update(|cx| {
    handle.update(cx, |_, window, cx| {
        window.perform_a11y_action(button, accesskit::Action::Click, None, cx)
    })
})
.unwrap();
window.read(|button, _| assert_eq!(button.saves, 1));
```

[`on_click`][StatefulInteractiveElement::on_click] supplies both the mouse
listener and the accessibility `Click` action. The string child needs no node
of its own because the button supplies its accessible name.

### Stable ids and text

Keep IDs stable across frames. [`GlobalElementId`] includes every ancestor ID,
so changing a parent ID replaces the child's node too. Assistive technology
then sees a removal and an addition rather than an update to the same control.

[`text!`] uses its call site as a local ID. When a loop repeats that call,
distinguish its rows through their parent IDs. This fixed list uses indices;
if items can move, use IDs that follow the items:

```rust
use gpui::{Role, div, prelude::*, text};

let todos = vec!["eat lunch", "drink water", "go to gym"];
let items = todos.into_iter().enumerate().map(|(index, todo)| {
    div().id(index).child(text!(todo))
});

let list = div().id("todo-list").role(Role::List).children(items);
```

Use [`Text::new_inaccessible`] to omit the text's ID when its parent already
represents it and a second node would duplicate the announcement.

## Describe state

Set `aria_*` properties from the values used to draw the control. They publish
state to assistive technology:

| Method | Use it for |
| --- | --- |
| [`aria_toggled`][StatefulInteractiveElement::aria_toggled] | Checkboxes and toggle buttons, with [`Toggled`] |
| [`aria_selected`][StatefulInteractiveElement::aria_selected] | Selected rows, tabs and options |
| [`aria_expanded`][StatefulInteractiveElement::aria_expanded] | Disclosure widgets, menus and accordions |
| [`aria_disabled`][StatefulInteractiveElement::aria_disabled] | Controls unavailable to assistive technology |
| [`aria_value`][StatefulInteractiveElement::aria_value] | The string value of a text field or label |
| [`aria_numeric_value`][StatefulInteractiveElement::aria_numeric_value] | Sliders, progress bars and spin buttons |

Publish a numeric control's range and step with
[`aria_min_numeric_value`][StatefulInteractiveElement::aria_min_numeric_value],
[`aria_max_numeric_value`][StatefulInteractiveElement::aria_max_numeric_value]
and [`aria_numeric_value_step`][StatefulInteractiveElement::aria_numeric_value_step]
as well as its current value. These properties describe the allowed range;
your action handler still has to enforce it.

```rust
use gpui::{Role, Toggled, div, prelude::*};

fn bold_toggle(bold: bool) -> impl IntoElement {
    div()
        .id("bold")
        .role(Role::CheckBox)
        .aria_label("Bold")
        .aria_toggled(Toggled::from(bold))
        .child("B")
}

fn volume_slider(volume: f64) -> impl IntoElement {
    div()
        .id("volume")
        .role(Role::Slider)
        .aria_label("Volume")
        .aria_numeric_value(volume)
        .aria_min_numeric_value(0.0)
        .aria_max_numeric_value(100.0)
        .aria_numeric_value_step(1.0)
}
```

Use [`aria_description`][StatefulInteractiveElement::aria_description] for
instructions announced after the name, role and value. The name can then
remain brief on repeated visits.
[`aria_placeholder`][StatefulInteractiveElement::aria_placeholder] describes
the hint in an empty text field.

Other properties cover structure: [`aria_level`][StatefulInteractiveElement::aria_level]
for headings, [`aria_position_in_set`][StatefulInteractiveElement::aria_position_in_set]
and [`aria_size_of_set`][StatefulInteractiveElement::aria_size_of_set] for items in
a list, the `aria_row_*` and `aria_column_*` methods for grids, and
[`aria_orientation`][StatefulInteractiveElement::aria_orientation]. Use
[`aria_modal`][StatefulInteractiveElement::aria_modal] on a dialog that blocks the
rest of the window.

Likewise, `aria_disabled(true)` describes unavailability without removing any
mouse or action listener. Guard activation in the handler, using the same
disabled value for that guard, the appearance and the accessibility flag.

## Announce changes with live regions

Give a live region a stable ID and set
[`aria_live`][StatefulInteractiveElement::aria_live]:

```rust
use gpui::{Role, accesskit, div, prelude::*};

fn status(message: &str) -> impl IntoElement {
    div()
        .id("status")
        .role(Role::Status)
        .aria_live(accesskit::Live::Polite)
        .child(message.to_string())
}
```

`accesskit::Live::Polite` requests an announcement when the user pauses;
`Assertive` requests an interruption, so reserve it for errors needing
immediate attention; `Off` silences the region.
[`aria_live_atomic`][StatefulInteractiveElement::aria_live_atomic] requests the
whole region instead of just its changed part. These are requests to assistive
technology, which controls the announcement.

Update that same node. Replacing its ID creates a new node, which may not be
announced as a status change.

## Respond to actions from assistive technology

[`accesskit::Action`] asks a node to perform an operation: `Click` requests
activation, `Focus` requests focus, and `Increment` or `Decrement` requests a
numeric change. GPUI exposes this protocol through [`AccessibleAction`]. It is
separate from the GPUI [`Action`] trait used by key bindings and menus.

Elements with click and focus support already handle those accessibility
actions. Add others with
[`on_a11y_action`][StatefulInteractiveElement::on_a11y_action]. The callback
receives optional [`accesskit::ActionData`] plus mutable window and app access.
This slider changes its entity for either numeric action:

```rust
use gpui::{Context, Role, Window, accesskit, div, prelude::*};

struct Slider {
    value: i32,
}

impl Render for Slider {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let up = cx.entity();
        let down = up.clone();
        div()
            .id("slider")
            .role(Role::Slider)
            .aria_label("Level")
            .aria_numeric_value(self.value as f64)
            .on_a11y_action(accesskit::Action::Increment, move |_data, _window, cx| {
                up.update(cx, |slider, cx| {
                    slider.value += 1;
                    cx.notify();
                });
            })
            .on_a11y_action(accesskit::Action::Decrement, move |_data, _window, cx| {
                down.update(cx, |slider, cx| {
                    slider.value -= 1;
                    cx.notify();
                });
            })
    }
}
```

Registration takes effect when the element paints with accessibility enabled.
Before that frame the window has no listener to call. After an action updates
the entity, [`Context::notify`] requests rendering, and the next frame publishes
the new value in the tree.

Every registered action is advertised on the node. Register only operations
the control implements, so assistive technology does not offer an inert action.

An accessibility action targets a node directly; it does not follow the
keyboard focus path. The Save button test above activates the node without
first focusing it.

## Expose elements that GPUI doesn't draw as nodes

A custom element can paint several interactive parts without creating child
elements. An editor's visible lines are one example. Expose them with
[`StatefulInteractiveElement::a11y_synthetic_children`]. Its closure runs after
prepaint, when visible-line information is available. The parent needs an ID
and either a role or `aria_hidden`; each synthetic key must be unique within the
call and stable across frames so its node keeps its identity.

```rust
use gpui::{Role, accesskit, div, prelude::*};

fn text_field(text: &'static str) -> impl IntoElement {
    div()
        .id("text-field")
        .role(Role::TextInput)
        .a11y_synthetic_children(move |builder| {
            let run_id = builder.synthetic_node_id(0);
            let mut run = accesskit::Node::new(Role::TextRun);
            run.set_value(text);
            builder.push_child(run_id, run);
        })
}
```

For a custom [`Element`], implement [`Element::a11y_synthetic_children`] instead
and set the parent's properties through [`A11ySubtreeBuilder::parent_node`].

## Hide decoration

Decoration can remain visible without entering the accessibility tree.
[`aria_hidden`][StatefulInteractiveElement::aria_hidden] excludes an element
and its descendants while preserving their rendering and input behavior:

```rust
use gpui::{div, prelude::*, text};

let divider = div()
    .id("divider")
    .aria_hidden()
    .child(text!("---"));
```

Keep focusable and interactive controls out of hidden subtrees. Their handlers
still work, so focus could land on a control that assistive technology cannot
report.

## Test the tree

The tree is normally absent until an assistive-technology client connects.
Tests can force it with [`Window::set_a11y_forced`] on a [`TestApp`] window, as
the button example does. For a running application, use
[`Application::with_accessibility_forced`]:

```rust,no_run
fn main() {
    gpui_platform::application()
        .with_accessibility_forced()
        .run(|_cx| {
            // Open windows here. Call `Window::debug_a11y_tree_json` to dump
            // the tree while the app runs.
        });
}
```

Inspect the last frame with [`Window::debug_a11y_tree_json`], which returns a
JSON string or `None` if that frame built no tree. Look there when a control
is missing or nested under the wrong parent. [`Window::a11y_tree`] returns
the tree itself for assertions.

Use [`Application::new_inaccessible`] to turn accessibility off entirely, for
example in a headless benchmark.

## Motion, contrast and focus

The tree describes what assistive technology can inspect, not every part of the
rendered experience. Test motion, contrast and keyboard focus in the window as
well.

**Reduced motion.** GPUI does not read the operating system's reduced-motion
setting. Read it in platform code and pass it to
[`App::set_reduce_motion`]. Animations then render in a static state, and
[`App::reduce_motion`] tells your own animations to do the same. The
[animation example](crate::_documentation::examples::animation) shows how an
animation checks it.

```rust
use gpui::App;

fn apply_motion_preference(cx: &mut App, prefers_reduced_motion: bool) {
    cx.set_reduce_motion(prefers_reduced_motion);
}
```

**Contrast.** GPUI has no contrast checker. Choose text and background colors
that meet the contrast ratio your product requires, and check them with a tool
or a color picker. Check disabled and placeholder text too.

**Focus.** Focus should remain visible. Give a focusable element a clear style
for keyboard focus with [`InteractiveElement::focus_visible`]. The element must
track a [`FocusHandle`]:

```rust
use gpui::{FocusHandle, div, prelude::*, rgb};

fn text_button(focus_handle: &FocusHandle) -> impl IntoElement {
    div()
        .id("ok")
        .track_focus(focus_handle)
        .focus_visible(|style| style.border_color(rgb(0x3b82f6)))
        .child("OK")
}
```

Make every control reachable and operable from the keyboard, in a sensible
order. Keyboard actions follow the focused element's path: capture runs toward
the target, then bubble returns toward the root unless propagation stops.
Keyboard dispatch and Tab traversal use the last rendered focus and tab-stop
maps, so draw newly added controls before relying on their registration. The
[focus guide](crate::_documentation::guides::focus) and
[keyboard guide](crate::_documentation::guides::keyboard) cover these routes.

## See also

- [AccessKit] documents the platform model behind the tree.
- The [MDN WAI-ARIA basics][mdn-aria] and the [ARIA Authoring Practices
  Guide][apg] describe the roles, states and widget patterns this page uses.
  GPUI follows their names, but it doesn't always behave like a browser with
  the same attributes.
- The [focus guide](crate::_documentation::guides::focus) and
  [keyboard guide](crate::_documentation::guides::keyboard) cover the keyboard
  path that assistive technology also relies on.
- The [testing guide](crate::_documentation::guides::testing) shows how to run
  views headlessly.

[AccessKit]: https://accesskit.dev/
[mdn-aria]: https://developer.mozilla.org/en-US/docs/Learn_web_development/Core/Accessibility/WAI-ARIA_basics
[apg]: https://www.w3.org/WAI/ARIA/apg/
