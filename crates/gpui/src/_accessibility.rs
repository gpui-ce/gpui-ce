//! # Accessibility in GPUI
//!
//! Accessibility lets people use your app regardless of disability. It includes:
//!
//! - Sufficient text contrast.
//! - Options to disable animations.
//! - Adjustable text sizes.
//!
//! This guide covers programmatic accessibility, which lets screen readers,
//! Braille displays, and other assistive technology inspect and interact with
//! your app. Contributor docs are in the `a11y` module.
//!
//! GPUI uses [AccessKit] for programmatic accessibility, called "accessibility"
//! throughout this guide.
//!
//! A minimal example can be found in the `examples/a11y` directory.
//!
//! ## Background
//!
//! Accessibility support is based on two key capabilities:
//! - Exposing information about the current UI state to assistive technology.
//! - Responding to actions requested by assistive technology.
//!
//! For example, a screen reader might want to announce to the user that a new
//! button has appeared. The user may then want to use a voice control program
//! to press that button.
//!
//! ### IDs in GPUI - [`ElementId`] and [`GlobalElementId`]
//!
//! In GPUI, each [`Element`] can have an [`id`][Element::id]:
//! ```rust
//! # use gpui::*;
//! let div_with_id = div().id("my-id").child(text!("hello"));
//!
//! // IDs are optional
//! let div_without_id = div().child(text!("hello"));
//! ```
//!
//! [`Element`]s with IDs are also assigned a [`GlobalElementId`]. This global
//! ID is formed by composing all the non-`None` IDs of its ancestors. For
//! example:
//! ```rust
//! # use gpui::*;
//! let inner = div().id("inner-id");
//! let middle = div().child(inner);  // no ID
//! let outer = div().id("outer-id").child(middle);
//! ```
//! Here, `inner`'s global ID is roughly `["outer-id", "inner-id"]`.
//!
//! Since `middle` doesn't have an ID itself, it has no global ID.
//!
//! [`GlobalElementId`]s should be unique per-frame. Duplicate global IDs in the
//! same frame will likely cause bugs.
//!
//! ### IDs and accessibility
//!
//! Each frame, GPUI reports nodes with global IDs and non-`None`
//! [`role`][Element::a11y_role]s to assistive technology. The role identifies
//! the node as a button, label, table, or other type. Set it with
//! [`div().id(...).role()`][StatefulInteractiveElement::role].
//!
//! Nodes with the same global ID *across frames* are considered to be "the
//! same" node. For example:
//! ```rust
//! # use gpui::*;
//! // The UI in frame 1
//! let frame_1 = div()
//!     .id("parent")
//!     .role(Role::Button)
//!     .child(
//!       div()
//!         .id("id-1")
//!         .role(Role::Label)
//!         .child(text!("hello"))
//!     );
//!
//! // The UI on the next frame
//! let frame_2 = div()
//!     .id("parent")
//!     .role(Role::Button)
//!     .child(
//!       div()
//!         .id("id-2")  // <- different ID
//!         .role(Role::Label)
//!         .child(text!("hello"))
//!     );
//! ```
//! Although the UI looks unchanged, the new ID tells assistive technology
//! that one node was removed and another added. This can trigger unexpected
//! announcements. Keep IDs stable for nodes that persist across frames.
//! Nodes without a [`role`][Element::a11y_role] are not reported at all.
//!
//! #### IDs and text
//!
//! The [`text!`] macro wraps strings in [`Text`] and derives an ID from
//! its source location. For example:
//!
//! ```rust
//! # use gpui::*;
//! let a = text!("a");
//! let b = text!("b");
//!
//! // Different source locations, different IDs
//! assert_ne!(a.id(), b.id());
//!
//! // However:
//!
//! fn make_text(s: &str) -> Text { text!(s) }
//!
//! let a = make_text("a");
//! let b = make_text("b");
//!
//! // Both `a` and `b` are produced by the same `text!` invocation, so the IDs
//! // are the same
//! assert_eq!(a.id(), b.id());
//! ```
//! Reusing the same invocation in an iterator can create duplicate IDs:
//! ```rust
//! # use gpui::*;
//! let todos = vec!["eat lunch", "drink water", "go to gym"];
//! let todo_divs = todos.into_iter().map(|todo| {
//!     text!(todo)
//! });
//!
//! div()
//!     .id("todo-list")
//!     .role(Role::Document)
//!     .children(todo_divs);  // ERROR: multiple nodes with the same global ID
//! ```
//!
//! Each item shares the same [`text!`] invocation and ancestors, producing
//! duplicate global IDs. Release builds silently drop some of these nodes.
//!
//! To fix this, you can set an ID:
//! ```rust
//! # use gpui::*;
//! let todos = vec!["eat lunch", "drink water", "go to gym"];
//! let todo_divs = todos.into_iter().enumerate().map(|(index, todo)| {
//!     text!(todo).with_id(index)  // OR `text(id = index, todo)`
//! });
//!
//! div()
//!     .id("todo-list")
//!     .role(Role::Document)
//!     .children(todo_divs);
//! ```
//! Another possible solution is to wrap the [`text!`] in another node that
//! *does* have a unique global ID. For example:
//! ```rust
//! # use gpui::*;
//! let todos = vec!["eat lunch", "drink water", "go to gym"];
//! let todo_divs = todos.into_iter().enumerate().map(|(index, todo)| {
//!     div().id(index).child(text!(todo))
//! });
//!
//! div()
//!     .id("todo-list")
//!     .role(Role::Document)
//!     .children(todo_divs);
//! ```
//! Global IDs include ancestor IDs, so giving each parent a unique ID also
//! distinguishes its children. Each item gets a unique global ID and therefore
//! a unique AccessKit [`NodeId`][accesskit::NodeId].
//!
//! [`Text::new_inaccessible`] creates text without an ID. Use it when a parent
//! [`div`] already has a label, such as in a custom button, to avoid duplicating
//! text in the accessibility tree.
//!
//! ## Hiding content from assistive technology
//!
//! [`aria_hidden()`][StatefulInteractiveElement::aria_hidden] hides an
//! element and all of its descendants from assistive technology. It does not
//! change rendering or input:
//! ```rust
//! # use gpui::*;
//! let decorative_content = div()
//!     .id("decorative-content")
//!     .aria_hidden()
//!     .child(text!("Decorative text"));
//! ```
//!
//! GPUI requires an [`ElementId`] so it can create a stable AccessKit node for
//! the hidden subtree. The element does not need an accessible role. Omitting
//! `aria_hidden()` on a descendant cannot override a hidden ancestor.
//!
//! Do not hide elements that can receive keyboard focus or contain focusable
//! descendants. Hidden elements keep their focus and input handlers.
//!
//! ### Handling actions
//!
//! People using assistive technology may use a keyboard or a specialized input
//! system, such as voice control. Assistive technology dispatches actions to
//! specific UI nodes. Register handlers to respond to these requests.
//!
//! These actions use [`accesskit::Action`], re-exported as [`AccessibleAction`],
//! and are unrelated to GPUI's [`Action`] trait.
//!
//! To respond to an accessible action, use
//! [`div().on_a11y_action()`][InteractiveElement::on_a11y_action]:
//! ```rust,ignore
//! div()
//!     .id("my-slider")
//!     .role(Role::Slider)
//!     .on_a11y_action(AccessibleAction::Increment, |_extra, _window, _cx| {
//!         position += 1;
//!         cx.notify();
//!     })
//!     .child(my_cool_slider());
//! ```
//!
//! Some common actions are registered automatically. For example,
//! [`.on_click()`][StatefulInteractiveElement::on_click] adds an
//! [`AccessibleAction::Click`] handler that calls the click handler.
//!
//! ## Synthetic children
//!
//! [`Element::a11y_synthetic_children`] lets a custom element expose multiple
//! nodes. A text editor can have [`Role::TextInput`] with [`Role::TextRun`]
//! children. For example:
//!
//! ```rust,ignore
//! # use gpui::*;
//! impl Element for MyCustomTextField {
//!
//!     // ...
//!     
//!     fn a11y_role(&self) -> Option<Role> {
//!         Some(Role::TextInput)
//!     }
//!     
//!     fn a11y_synthetic_children(
//!         &mut self,
//!         _prepaint: &mut Self::PrepaintState,
//!         builder: &mut A11ySubtreeBuilder,
//!     ) {
//!         // Create the synthetic child node
//!         let mut run = accesskit::Node::new(Role::TextRun);
//!         run.set_value(self.text.clone());
//!         run.set_character_lengths(
//!             self.text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>(),
//!         );
//!
//!         // Insert it as a child of `MyCustomTextField`
//!         let run_id = builder.synthetic_node_id(0);
//!         builder.push_child(run_id, run);
//!
//!         // You can also mutate the parent (i.e. the `MyCustomTextField`)
//!         let caret = accesskit::TextPosition {
//!             node: run_id,
//!             character_index: self.cursor,
//!         };
//!         builder.parent_node().set_text_selection(accesskit::TextSelection {
//!             anchor: caret,
//!             focus: caret,
//!         });
//!     }
//! }
//! ```
//!
//! Synthetic children are added after [prepaint][Element::prepaint], so they
//! can use prepaint state to determine what is visible on screen.
//!
//! ## Further reading
//!
//! - [AccessKit]: The cross-platform accessibility toolkit GPUI uses
//!   internally.
//! - [MDN WAI-ARIA basics][mdn-aria]: Introduction to roles, properties, and
//!   states.
//! - [ARIA Authoring Practices Guide][apg]: W3C patterns for accessible
//!   widgets.
//!
//! GPUI mimics web APIs but may behave differently from a browser given the
//! same attributes.
//!
//! [AccessKit]: https://accesskit.dev/
//! [mdn-aria]: https://developer.mozilla.org/en-US/docs/Learn_web_development/Core/Accessibility/WAI-ARIA_basics
//! [apg]: https://www.w3.org/WAI/ARIA/apg/

#[cfg(doc)]
use crate::*; // so I don't have to qualify every type :)
