# Comparison with other toolkits

You want to compare?

Here you go! No talk of performance (check your use-case, and you can likely infer. Happy to accept any benchmark PRs, however.)
## At a glance

| Toolkit | UI description | State and updates | Rendering boundary |
| --- | --- | --- | --- |
| **GPUI** | Rust builds an element tree during rendering, with style methods for layout and painting | App-owned entities change explicitly and call [`Context::notify`](crate::Context::notify) to request rendering | GPUI's renderer and platform backends |
| [egui](https://docs.rs/egui/latest/egui/) | Immediate-mode Rust code lays out and paints widgets on each pass | Application code owns the persistent values passed to widgets | An egui integration and renderer, commonly `eframe` with `egui-wgpu` or `egui_glow` |
| [iced](https://github.com/iced-rs/iced) | A `view` function returns widgets; messages go to `update` | The Elm-inspired state, message, update and view loop | Renderer-agnostic core with `wgpu` and `tiny-skia` renderers |
| [Slint](https://docs.slint.dev/latest/docs/slint/) | `.slint` markup describes components, properties, bindings and callbacks | Property bindings and callbacks connect component state | Configurable renderers, including software and GPU backends |
| [Dioxus](https://dioxuslabs.com/learn/0.7/guides/platforms/desktop/) | Rust components use `rsx!`, with HTML and CSS concepts | Components use hooks and signals around a Virtual DOM | Web targets use the DOM; the desktop renderer uses the system WebView |
| [Xilem](https://github.com/linebender/xilem), [Floem](https://docs.rs/floem/latest/floem/) | Xilem builds lightweight views; Floem composes views and closures | Xilem uses reactive state and view updates; Floem uses signals and effects | Xilem has native and web backends; Floem provides its own native rendering stack |
| [Tauri](https://tauri.app/concept/architecture/) | Any web UI framework | The web frontend owns UI state; Rust exposes native capabilities through commands and events | The operating system's WebView, connected to Rust through message passing |

## egui

[egui describes itself as an immediate-mode GUI library](https://docs.rs/egui/latest/egui/#understanding-immediate-mode). Each pass runs the widget code, reads the values to display, emits shapes and responses, and leaves the application to retain values needed by the next pass. A button handler can therefore sit beside the code that draws the button.

GPUI rebuilds its element tree during rendering, then uses separate layout,
painting, and dispatch paths for focus, keys, and actions. Persistent state
lives in entities. After a mutation, [`Context::notify`](crate::Context::notify)
invalidates ordinary views that read the entity; a cached subtree needs its own
entity notified, as [the frame model](crate::_documentation::concepts::the_frame#rendering-less)
explains. The separation gives focus, keymaps, custom layout, and virtualized
large views distinct places in the system. egui keeps drawing code beside its
button response, a compact arrangement for a tool or debug panel.

## iced

[iced's official overview](https://github.com/iced-rs/iced#overview) separates application state, messages, `view`, and `update`, following The Elm Architecture. A view produces widgets, user input produces messages, and `update` changes state or returns asynchronous work. Runtime and renderer layers remain modular, with `wgpu` and `tiny-skia` implementations.

GPUI does not require one application-wide message enum or update function. An
entity can update and observe another entity, while actions and subscriptions
connect input to state. [`App`](crate::App) and its contexts still provide an
explicit update boundary, and [`Context::notify`](crate::Context::notify) controls
when changed state renders again. iced makes the message route explicit; GPUI
keeps relationships between independently owned pieces of state direct. A
feature that crosses many models must decide whether a centralized message route
or ownership near the changed state makes the dependency easier to follow.

## Slint

[Slint defines components in its `.slint` language](https://docs.slint.dev/latest/docs/slint/), with properties, bindings, callbacks, and declarative layout. The compiler can generate Rust bindings from the markup, and the project documents configurable renderers for desktop and embedded targets, including a software renderer.

GPUI keeps the interface in Rust: style methods expose flexbox and grid
concepts, while the element tree, event dispatch, and painting remain Rust
values and traits. Application code reaches entities, actions, and platform
services through those same values. Slint puts properties and bindings in
compiled components, making the component language the seam between UI design
and application code.

## Dioxus and Tauri

Dioxus uses Rust components and a Virtual DOM. The [Dioxus 0.7 desktop
guide](https://dioxuslabs.com/learn/0.7/guides/platforms/desktop/) describes a
system WebView renderer built on `wry`, with HTML and CSS supplying the UI layer
while Rust code runs natively. Its web target uses the browser DOM. The desktop
comparison here refers to that WebView backend.

[Tauri's architecture](https://tauri.app/concept/architecture/) pairs a web frontend with a Rust backend. The frontend reaches native capabilities through message passing, while the window uses the operating system's WebView. Existing web frameworks and browser tooling therefore remain part of the UI layer.

GPUI has no HTML or CSS runtime and does not put the interface in a WebView. It
uses Rust elements, its own layout and paint pipeline, and platform integrations
behind [`Platform`](crate::Platform). CSS names, flexbox, and grid transfer as
concepts, but web components and browser APIs do not.

## Xilem and Floem

[Xilem describes itself as an experimental reactive UI architecture](https://github.com/linebender/xilem). Its view values are lightweight descriptions diffed against a retained Masonry widget tree; the repository also contains a web backend. [Floem's API documentation](https://docs.rs/floem/latest/floem/) describes a view tree built once and a reactive system based on signals and effects, where reading a signal connects a view to later updates.

GPUI rebuilds elements during rendering; entities hold persistent state, and
`notify` explicitly invalidates views. Xilem moves more of that relationship
into view values diffed against a retained widget tree, while Floem uses signal
tracking and effects. All three compose views, but identity, retained widget
state, and update propagation live at different layers. When a list item
changes, the relevant diagnostic is consequently different: an entity
notification in GPUI, a view diff in Xilem, or a signal effect in Floem.

## GPUI and GPUI Community Edition

GPUI-CE is a fork of the GPUI that Zed uses. It tracks upstream and adds features for applications beyond Zed, including a web backend, a new text system and motion APIs. [Upstream](crate::_documentation::upstream) lists the differences and the porting considerations.
