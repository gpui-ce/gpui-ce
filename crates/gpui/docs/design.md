# Design

GPUI keeps application data in entities and element interaction state under
stable ids. A view reads that state and describes the current frame. When an
input handler changes an entity, `notify` schedules another render; painting
then records shapes, text and images for the GPU.

The model grew out of work on the [Zed](https://zed.dev) editor. That history
leaves three practical boundaries in application code: entity updates need a
context, scrollable elements need stable ids, and custom controls must supply
their own input and accessibility behavior. The sections below explain what
GPUI provides at each boundary and what remains yours.

## GPU rendering

Layout runs on the CPU. Painting records primitives in a [`Scene`]: a `div`
background becomes a rectangle, while text refers to rasterized glyphs in an
atlas. Compatible primitives are batched for the renderer, and an existing
glyph can be reused without rasterizing it again.

The GPU does not remove the CPU work of constructing or measuring elements. For
10,000 rows, a [virtualized list](crate::_documentation::guides::lists_and_scrolling)
limits construction to the viewport. GPU batching applies later, when the
[frame cycle](crate::_documentation::concepts::the_frame) paints those elements.

GPUI draws its own controls. Application code chooses their appearance; platform
backends provide windows, input and operating-system services. Text input methods
and accessibility still need explicit integration, so a custom control must
implement the focus, input and accessibility behavior it needs. See [text input](crate::_documentation::guides::text_input)
and [accessibility](crate::_documentation::guides::accessibility).

## Rebuilding views

A view's [`Render::render`] method builds an element tree from current state.
GPUI lays out and paints that tree; it does not reconcile a virtual DOM. The
view can therefore describe its interface in one place instead of maintaining a
separate update operation for each state transition.

Entities retain view state between frames. Elements can retain state too—scroll
position and animation progress, for example—when they have stable ids.
Recreating an element then does not reset its interaction state. [Elements](crate::_documentation::concepts::elements)
explains identity and the layout, prepaint and paint phases.

Rebuilding a complex view still costs CPU time. [`Entity::cached`] reuses a
view's rendered content until that entity is notified, the window is refreshed,
or its bounds or text environment changes. If the view reads another entity,
observe that entity and notify the cached view when its contents must change.
[Performance](crate::_documentation::guides::performance) covers when caching
and list virtualization help.

## App-owned state

Several components often need to share mutable state. GPUI stores that state in
the [`App`] and exposes typed [`Entity<T>`](Entity) handles. Reading or updating
an entity requires a context. Handles can be cloned and moved into async work
without keeping a borrow of the stored value alive.

The app coordinates entity lifetimes, observers and rendering dependencies.
Updates are explicit: use [`Entity::update`] to mutate a value, then
[`Context::notify`] to schedule dependent rendering or [`Context::emit`] to send
a typed event. GPUI does not detect field mutations automatically.
[Ownership](crate::_documentation::concepts::ownership) explains access and
lifetimes; [sharing state](crate::_documentation::guides::sharing_state) shows
how separate views communicate.

## Explicit contexts

An [`App`] reference grants access to application services. [`Context<T>`] adds
operations for the entity being updated, while [`Window`] provides layout,
painting, focus and other window state. Passing these references to callbacks
makes the required access visible in the signature and limits it to the call.
Tests can provide the same access through a headless application.

Async work uses [`AsyncApp`] or [`AsyncWindowContext`] to regain access after an
`await`. Those contexts are fallible because the app or window may have closed.
[Contexts](crate::_documentation::concepts::contexts) explains how these types
relate and which one to use.

## Style and layout

Elements carry styles set through [`Styled`] methods named after [Tailwind CSS](https://tailwindcss.com)
classes. Rust checks the method names and value types. Shared functions and
components can reuse style builders, while theme state can live in a [`Global`].
Text properties inherit through the element tree; GPUI has no stylesheet
selector cascade.

Layout uses [Taffy](https://github.com/DioxusLabs/taffy) for CSS flexbox and grid
algorithms. Those algorithms resolve parent constraints together with child
sizes. [Layout and style](crate::_documentation::concepts::layout_and_style)
connects the builder methods to those constraints and to custom element layout.

## Actions and key contexts

Keyboard commands have three parts: an [`Action`] names an operation,
[`KeyBinding`] maps a keystroke to it, and [`KeyContext`] restricts where that
binding applies. Focus determines the dispatch path. One action can serve a
shortcut, menu item and application code without duplicating its handler.

This separation lets a user rebind a command and lets the same keystroke mean
different things in an editor and a file list. Applications still define the
commands and choose which controls can receive focus. [Input dispatch](crate::_documentation::concepts::input_dispatch)
explains matching and propagation; the [keyboard guide](crate::_documentation::guides::keyboard)
shows how to connect them.

## Executors and tests

The foreground executor runs tasks on the UI thread through the platform event
loop. A task may wait for I/O without blocking that thread, then update an entity
through an async context. CPU-intensive work belongs on the background executor;
in a single-threaded browser build it still shares the main thread, so split
long work into smaller awaited steps.

Tests use a seeded scheduler and simulated time. They can reproduce task
ordering, advance a timer without waiting in real time, and drive a window from
input events. [Executors](crate::_documentation::concepts::executors) explains
polling and task lifetimes; the [testing guide](crate::_documentation::guides::testing)
shows how to use those controls in a test.

## Scope and trade-offs

GPUI provides layout, styling, input dispatch, rendering and platform
integration. The `gpui_ce_elements` crate adds editable text. Applications
compose higher-level controls such as buttons, menus and dialogs from those
pieces, and own their behavior and accessibility semantics.

Views and entities live on one UI thread. GPUI supports desktop platforms and
browsers, with no mobile backend. It is pre-1.0, so API changes may require
application updates. [Platforms](crate::_documentation::platforms) describes
backend capabilities, [comparison](crate::_documentation::comparison) contrasts
other toolkits' approaches, and [upstream](crate::_documentation::upstream)
explains the relationship to Zed's GPUI.
