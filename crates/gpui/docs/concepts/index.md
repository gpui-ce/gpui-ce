# Concepts

When a counter button is clicked, the window hit-tests the pointer, a handler
changes the counter, and a later frame draws the new value. The input records,
application state and element tree have different lifetimes. These pages explain
how they fit together.

The counter lives in an entity, while views hold handles to it. Mutating the
value does not redraw a window by itself; the handler calls `notify` to announce
the change. [Ownership and data flow](crate::_documentation::concepts::ownership)
explains that contract, including events and handle lifetimes. [Contexts](crate::_documentation::concepts::contexts)
explains the `cx` passed to a handler and how that access changes across an
`await`.

A later draw turns the current counter value into elements. [Elements, components
and views](crate::_documentation::concepts::elements) distinguishes those values
from the state they retain. [The frame](crate::_documentation::concepts::the_frame)
follows the result through layout, hit testing and paint. [Layout and style](crate::_documentation::concepts::layout_and_style)
then supplies the rules for each box's size and position, including units,
constraints, flexbox, grid and overflow.

The next pointer event is tested against the interface in that frame. Keyboard
input takes another route: focus defines a path, and key contexts select the
bindings and actions that can handle it. [Input and dispatch](crate::_documentation::concepts::input_dispatch)
connects those paths and explains propagation. When a handler must wait or do
substantial work, [Executors and tasks](crate::_documentation::concepts::executors)
explains scheduling, cancellation and the threading limits of web builds.

For the reasons behind these boundaries, read [Design](crate::_documentation::design).
For a concrete operation, use the [guides](crate::_documentation::guides).
