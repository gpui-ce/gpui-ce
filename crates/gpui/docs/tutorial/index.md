# Tutorial: build a todo list

Build a keyboard-friendly todo list from an empty Rust project. You will first
create a window, then add state, layout, text input, reusable components and
filtering. The later chapters add background work for undo, animation and
accessibility, and finally headless tests. Each chapter leaves the previous
program runnable so you can check the new behavior before continuing.

<div class="gpui-example" data-example="todos">

The finished app is the `todos` example. Run it with `cargo run --example todos`
in a checkout of the GPUI repository.

</div>

Follow the chapters in order because each one extends the program from the
previous chapter:

1. [Hello, window](tutorial::ch1_hello_window): an application, a window and
   your first view.
2. [State and events](tutorial::ch2_state): data that changes when you click,
   and telling GPUI to redraw.
3. [Layout and style](tutorial::ch3_layout_and_style): flexbox, spacing, color
   and hover states.
4. [Text input](tutorial::ch4_text_input): a text field, focus, actions and key
   bindings.
5. [Components](tutorial::ch5_components): reusable pieces and list filtering.
6. [Background work](tutorial::ch6_background_work): undoing a delete with a
   task that expires.
7. [Polish](tutorial::ch7_polish): animation and accessibility.
8. [Testing](tutorial::ch8_testing): driving the app from a headless test,
   including input and asynchronous work.

## Before you start

You should be comfortable with Rust structs, closures, traits and iterators.
No experience with another UI framework is required.

You need a recent stable Rust toolchain and, on Linux, a few system packages.
[Start a project](crate::_documentation::guides::start_a_project) lists them for
each platform.

Compare your code with the [finished todos example](crate::_documentation::examples::todos)
if you get stuck or want to skip ahead. Follow the links at each chapter's end
when you need to adapt that step beyond the todo list or want the GPUI concept
behind the mechanism it introduced.
