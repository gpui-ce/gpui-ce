//! Elements lay out and paint everything in a window. They form a tree, and their layout follows
//! the web layout rules implemented by [taffy](https://github.com/DioxusLabs/taffy). Most of the
//! time you won't use this module directly. The elements behind [`div`](crate::div) and friends have their
//! own APIs, and GPUI uses the traits in this module to turn the element tree into pixels.
//!
//! # Element basics
//!
//! GPUI builds the element tree by calling [`Render::render`] on the root view of a window. It
//! then lays the tree out with Taffy and paints it, one [`Element`] at a time. When the frame is
//! done, the tree and the callbacks it registered are dropped, and the next frame builds a new one.
//! [The frame](crate::_documentation::concepts::the_frame) describes this sequence in full.
//!
//! Elements hold no state between frames, because they are rebuilt every frame. State that
//! must survive a frame lives in an entity, or in element state keyed by a [`GlobalElementId`].
//! An element can also pass small pieces of state from one of its phases to the next through its
//! associated types. [`Element`] explains the three phases.
//!
//! # Implementing your own elements
//!
//! Elements are the low-level, imperative API of GPUI. Most elements stay inside the bounds their
//! parent gives them, which keeps GPUI's layout guarantees. [`deferred`](crate::deferred) lets an
//! element paint outside those bounds, which overlays and popups need. Use that power with care.
//!
//! Most of the time you don't need a custom element. Build components from the built-in elements
//! instead, with [`RenderOnce`] and `#[derive(IntoElement)]`. Write an [`Element`] only when you
//! need your own layout or painting, such as a custom layout algorithm or a code editor. The
//! [`custom_element`](crate::_documentation::examples::custom_element) example is a complete one.

use crate::{
    A11ySubtreeBuilder, App, ArenaBox, AvailableSpace, Bounds, Context, DispatchNodeId, ElementId,
    FocusHandle, InspectorElementId, LayoutId, Pixels, Point, Size, Style, Window,
    util::FluentBuilder, window::with_element_arena,
};
use derive_more::{Deref, DerefMut};
use std::{
    any::Any,
    fmt::{self, Debug, Display},
    mem, panic,
    sync::Arc,
};

/// A part of a window's element tree that lays itself out and paints itself.
///
/// GPUI draws each element in three phases. Each phase runs once per frame, in this order:
///
/// 1. [`request_layout`](Element::request_layout) asks Taffy for a [`LayoutId`] that describes the
///    element's size and position rules. It returns the element's
///    [`RequestLayoutState`](Element::RequestLayoutState). The element has no bounds yet.
/// 2. [`prepaint`](Element::prepaint) receives the element's final [`Bounds`] and returns its
///    [`PrepaintState`](Element::PrepaintState). Insert hitboxes and compute anything that `paint`
///    needs here.
/// 3. [`paint`](Element::paint) receives the bounds and both states, and draws. Add primitives to
///    the scene and register event listeners here.
///
/// The two associated state types carry data from one phase to the next:
///
/// * `RequestLayoutState` is produced by `request_layout`, and `prepaint` and `paint` each get a
///   `&mut` to it. Use it for what layout produced, such as the ids of child layouts or a measured
///   text shape.
/// * `PrepaintState` is produced by `prepaint`, and `paint` gets a `&mut` to it. Use it for what
///   needs the final bounds, such as a [`Hitbox`](crate::Hitbox) or the position of a cursor.
///
/// Use `()` for a state type when the element has nothing to pass on.
///
/// A parent calls its children's phases from inside its own. A child's `request_layout` therefore
/// runs during its parent's, and its `prepaint` and `paint` run while the parent's do. Call
/// [`Window::insert_hitbox`] from `prepaint`, and [`Window::on_mouse_event`] or
/// [`Window::paint_quad`] from `paint`. Window methods check the phase they run in with debug
/// assertions, so calling one in the wrong phase fails in debug builds.
///
/// [`Element::id`] returns the element's identity, if it has one. An element with an id receives a
/// [`GlobalElementId`] in all three phases, so it can read and write state that survives from frame
/// to frame. An element without one receives `None`. See
/// [element identity](crate::_documentation::concepts::elements#identity-and-element-state).
///
/// # Example
///
/// This element draws nothing. It records the phases it runs in, and the example checks their
/// order:
///
/// ```
/// use std::{cell::RefCell, rc::Rc};
///
/// use gpui::{
///     App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
///     IntoElement, LayoutId, Pixels, Render, Style, TestApp, Window, div, prelude::*,
/// };
///
/// #[derive(Clone, Default)]
/// struct Log(Rc<RefCell<Vec<&'static str>>>);
///
/// struct Probe {
///     log: Log,
/// }
///
/// impl IntoElement for Probe {
///     type Element = Self;
///
///     fn into_element(self) -> Self::Element {
///         self
///     }
/// }
///
/// impl Element for Probe {
///     type RequestLayoutState = ();
///     type PrepaintState = Bounds<Pixels>;
///
///     fn id(&self) -> Option<ElementId> {
///         None
///     }
///
///     fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
///         None
///     }
///
///     fn request_layout(
///         &mut self,
///         _id: Option<&GlobalElementId>,
///         _inspector_id: Option<&InspectorElementId>,
///         window: &mut Window,
///         cx: &mut App,
///     ) -> (LayoutId, Self::RequestLayoutState) {
///         self.log.0.borrow_mut().push("request_layout");
///         (window.request_layout(Style::default(), [], cx), ())
///     }
///
///     fn prepaint(
///         &mut self,
///         _id: Option<&GlobalElementId>,
///         _inspector_id: Option<&InspectorElementId>,
///         bounds: Bounds<Pixels>,
///         _request_layout: &mut Self::RequestLayoutState,
///         _window: &mut Window,
///         _cx: &mut App,
///     ) -> Self::PrepaintState {
///         self.log.0.borrow_mut().push("prepaint");
///         bounds
///     }
///
///     fn paint(
///         &mut self,
///         _id: Option<&GlobalElementId>,
///         _inspector_id: Option<&InspectorElementId>,
///         _bounds: Bounds<Pixels>,
///         _request_layout: &mut Self::RequestLayoutState,
///         _prepaint: &mut Self::PrepaintState,
///         _window: &mut Window,
///         _cx: &mut App,
///     ) {
///         self.log.0.borrow_mut().push("paint");
///     }
/// }
///
/// struct ProbeView {
///     log: Log,
/// }
///
/// impl Render for ProbeView {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div().size_full().child(Probe { log: self.log.clone() })
///     }
/// }
///
/// let log = Log::default();
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let log = log.clone();
///     move |_window, _cx| ProbeView { log }
/// });
///
/// log.0.borrow_mut().clear();
/// window.draw();
/// assert_eq!(*log.0.borrow(), ["request_layout", "prepaint", "paint"]);
/// ```
///
/// For a complete element, see
/// [`custom_element`](crate::_documentation::examples::custom_element). It is a slider that inserts
/// a hitbox in `prepaint` and handles dragging in `paint`.
pub trait Element: 'static + IntoElement {
    /// The state that [`Element::request_layout`] returns for the later phases.
    ///
    /// `prepaint` and `paint` each receive a `&mut` reference to it. Use `()` if the element needs
    /// no such state.
    type RequestLayoutState: 'static;

    /// The state that [`Element::prepaint`] returns for [`Element::paint`].
    ///
    /// `paint` receives a `&mut` reference to it. Use `()` if the element needs no such state.
    type PrepaintState: 'static;

    /// Returns the element's identity, or `None` if it has none.
    ///
    /// Return an id from any element whose state must survive from one frame to the next, such as
    /// a hover, a drag or a scroll offset. GPUI finds that state again by the id's path from the
    /// window root. The id must be stable on every frame; two elements with the same id path share
    /// state and accessibility identity. The path includes every ancestor that has an id.
    fn id(&self) -> Option<ElementId>;

    /// Returns where this element was created, for the inspector and debug output.
    ///
    /// GPUI uses this only in debug builds and with the `inspector` feature. Return `None` when the
    /// location doesn't matter. To capture it, mark the constructor `#[track_caller]` and store
    /// [`std::panic::Location::caller`].
    fn source_location(&self) -> Option<&'static panic::Location<'static>>;

    /// Requests a layout node from Taffy, and returns it with the state the later phases need.
    ///
    /// Describe the element's size and position rules here: build a [`Style`] and pass it to
    /// [`Window::request_layout`], along with the layout ids of any children. Lay out the children
    /// you draw here too, by calling their `request_layout` before this element's own. The element
    /// has no bounds in this phase.
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState);

    /// Runs once the element's bounds are known, and returns the state that `paint` needs.
    ///
    /// `bounds` is the element's final rectangle in window coordinates, in logical pixels. Insert
    /// hitboxes with [`Window::insert_hitbox`] here, so that the next mouse event can be tested
    /// against them. Compute anything `paint` needs too. Calls that draw, such as
    /// [`Window::paint_quad`], belong in `paint`.
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState;

    /// Draws the element. Runs after `prepaint`, once per frame.
    ///
    /// `bounds` is the rectangle that `prepaint` received. Add primitives with the painting methods
    /// on [`Window`], such as [`Window::paint_quad`], and register event listeners with
    /// [`Window::on_mouse_event`] and related methods. Listeners stay registered until the next
    /// frame replaces them, so an event is always delivered against the frame the user saw.
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    );

    /// Returns the accessible role for this element, if any.
    /// Elements that return `None` have no accessibility node unless
    /// [`Element::is_a11y_hidden`] returns `true`.
    ///
    /// Note: inclusion in accessibility tree requires non-`None` [`id`][Element::id].
    ///
    /// See the [accessibility guide](crate::_documentation::guides::accessibility) for an overview.
    fn a11y_role(&self) -> Option<accesskit::Role> {
        None
    }

    /// Whether assistive technology should ignore this element and its
    /// descendants.
    ///
    /// GPUI gives a hidden element without a role an internal container node.
    /// AccessKit uses that node to omit the subtree.
    fn is_a11y_hidden(&self) -> bool {
        false
    }

    /// Write accessibility properties to the given node.
    /// GPUI calls this only when [`Element::a11y_role`] returns `Some` or
    /// [`Element::is_a11y_hidden`] returns `true`.
    ///
    /// See the [accessibility guide](crate::_documentation::guides::accessibility) for an overview.
    fn write_a11y_info(&self, _node: &mut accesskit::Node) {}

    /// Add synthetic child nodes to an [`Element`]. The element must have an
    /// [`.id()`][Element::id]. It must also have a
    /// [`.role()`][Element::a11y_role] or return `true` from
    /// [`Element::is_a11y_hidden`].
    ///
    /// Some elements may want to inject accessibility nodes that do not
    /// correspond to any GPUI element. For example, a custom text field element
    /// may want to inject synthetic child nodes for the text content.
    ///
    /// See [Expose elements that GPUI doesn't draw](crate::_documentation::guides::accessibility#expose-elements-that-gpui-doesnt-draw-as-nodes) in
    /// the accessibility guide for more detail.
    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Self::PrepaintState,
        _builder: &mut A11ySubtreeBuilder,
    ) {
    }

    /// Convert this element into a dynamically-typed [`AnyElement`].
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

/// A type that can be converted into an [`Element`], so it can be a child of another element.
///
/// [`ParentElement::child`] takes any `IntoElement`. Strings, [`Entity`](crate::Entity) views and every element
/// implement it. Derive it with `#[derive(IntoElement)]` on a [`RenderOnce`] component to make the
/// component usable wherever an element is.
///
/// # Example
///
/// ```
/// use gpui::{IntoElement, div, prelude::*};
///
/// let row = div().child("Name").child(String::from("Ada"));
/// let _element = row.into_element();
/// ```
pub trait IntoElement: Sized {
    /// The element type that this type converts into.
    ///
    /// Strings convert into text elements. A [`RenderOnce`] component that derives `IntoElement`
    /// converts into the element that renders it.
    type Element: Element;

    /// Converts `self` into an [`Element`].
    ///
    /// GPUI calls this when it needs the element. You rarely call it yourself, but it is the method
    /// to implement.
    fn into_element(self) -> Self::Element;

    /// Converts `self` into an [`AnyElement`], with its concrete type erased.
    ///
    /// Use this when two branches return different element types, or a list holds mixed types.
    fn into_any_element(self) -> AnyElement {
        self.into_element().into_any()
    }
}

impl<T: IntoElement> FluentBuilder for T {}

/// A view: an entity that GPUI draws in a window.
///
/// Implement `Render` for the state of a view. GPUI can then draw an [`Entity`](crate::Entity) of that type as an
/// element, and [`Context::notify`] redraws it. Use a view when a part of the interface has state
/// that must survive from frame to frame. For stateless pieces, use [`RenderOnce`].
///
/// GPUI calls `render` each time it draws the window and reaches this view. The view keeps its
/// state in `self`, not in the elements it returns. Those elements are consumed by the frame and
/// dropped, and the next call builds new ones. So `render` should only read state and build
/// elements from it. [`Entity::cached`](crate::Entity::cached) skips the call while the view is unchanged.
///
/// # Example
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{Context, IntoElement, Render, TestApp, Window, div, prelude::*};
///
/// struct Greeting {
///     name: String,
///     renders: Rc<Cell<usize>>,
/// }
///
/// impl Render for Greeting {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         self.renders.set(self.renders.get() + 1);
///         div().child(format!("Hello, {}", self.name))
///     }
/// }
///
/// let renders = Rc::new(Cell::new(0));
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let renders = renders.clone();
///     move |_window, _cx| Greeting {
///         name: "Ada".into(),
///         renders,
///     }
/// });
///
/// renders.set(0);
/// window.draw();
/// assert_eq!(renders.get(), 1);
/// ```
pub trait Render: 'static + Sized {
    /// Builds the view's element tree from its current state.
    ///
    /// `window` is the window being drawn, and `cx` gives access to this view through
    /// [`Context<Self>`](Context). Return the elements the view shows. Don't change state in this
    /// method, because GPUI may call it many times, or skip it, for one change.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}

impl Render for Empty {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A component: a value that describes a pattern of elements and renders it once.
///
/// Use `RenderOnce` for reusable pieces of interface that keep no state of their own. Build the
/// component from plain data, and derive [`IntoElement`] on it so it can be a child of an element.
/// The component's `render` takes `self` by value, and the component is consumed when it is drawn.
/// A component can't call [`Context::notify`], because it has no entity. To report events, store a
/// callback in the component and let the parent view decide what the callback does. Use [`Render`]
/// for a piece of interface that holds state.
///
/// Elements that need state across frames need ids. Pass the id in from the parent, and use it
/// with [`InteractiveElement::id`](crate::InteractiveElement::id). GPUI scopes a component's elements under the component's type
/// name, so two instances of the same component side by side would otherwise share their state.
///
/// # Example
///
/// ```
/// use std::{cell::Cell, rc::Rc};
///
/// use gpui::{App, Context, IntoElement, Render, RenderOnce, TestApp, Window, div, prelude::*};
///
/// #[derive(IntoElement)]
/// struct Badge {
///     renders: Rc<Cell<usize>>,
/// }
///
/// impl RenderOnce for Badge {
///     fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
///         self.renders.set(self.renders.get() + 1);
///         div().child("3")
///     }
/// }
///
/// struct Inbox {
///     renders: Rc<Cell<usize>>,
/// }
///
/// impl Render for Inbox {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div().child(Badge {
///             renders: self.renders.clone(),
///         })
///     }
/// }
///
/// let renders = Rc::new(Cell::new(0));
/// let mut app = TestApp::new();
/// let mut window = app.open_window({
///     let renders = renders.clone();
///     move |_window, _cx| Inbox { renders }
/// });
///
/// renders.set(0);
/// window.draw();
/// assert_eq!(renders.get(), 1);
/// ```
pub trait RenderOnce: 'static {
    /// Builds the component's element tree. Takes `self` by value, so the component is used up.
    ///
    /// GPUI calls this once for each time the component appears in a frame. Return the elements
    /// the component shows.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

/// A parent element whose children all have one concrete type.
///
/// Use it when a container's children carry a shared type, such as rows with their own data.
/// Most containers use [`ParentElement`], which accepts any [`IntoElement`].
pub trait ParentElementTyped {
    /// The type of child that this element accepts.
    type Child;

    /// Adds the given children to the end of this element's children.
    fn extend(&mut self, elements: impl IntoIterator<Item = Self::Child>);

    /// Adds one child to this element, and returns the element for chaining.
    fn child(mut self, child: Self::Child) -> Self
    where
        Self: Sized,
    {
        self.extend(std::iter::once(child));
        self
    }

    /// Adds several children to this element, in iteration order, and returns the element.
    fn children(mut self, children: impl IntoIterator<Item = Self::Child>) -> Self
    where
        Self: Sized,
    {
        self.extend(children);
        self
    }
}

/// An element that can hold any number of children of any type.
///
/// This is the trait behind `.child()` and `.children()` on containers such as [`div`](crate::div). Each
/// child is anything that implements [`IntoElement`], so strings, views and other elements can be
/// mixed in one container.
pub trait ParentElement {
    /// Adds the given children to the end of this element's children.
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    /// Adds one child to this element, and returns the element for chaining.
    fn child(mut self, child: impl IntoElement) -> Self
    where
        Self: Sized,
    {
        self.extend(std::iter::once(child.into_element().into_any()));
        self
    }

    /// Adds several children to this element, in iteration order, and returns the element.
    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self
    where
        Self: Sized,
    {
        self.extend(children.into_iter().map(|child| child.into_any_element()));
        self
    }
}

/// Identifies one element across frames: the path of [`ElementId`]s from the window's root to it.
///
/// GPUI passes this to the phase methods of an element that has an [`Element::id`]. Pass it to
/// [`Window::with_element_state`] to read and write state that belongs to the element. Two
/// elements share a `GlobalElementId` only when their id paths are equal.
#[derive(Deref, DerefMut, Clone, Default, Debug, Eq, PartialEq, Hash)]
pub struct GlobalElementId(pub(crate) Arc<[ElementId]>);

impl Display for GlobalElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, element_id) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", element_id)?;
        }
        Ok(())
    }
}

impl GlobalElementId {
    pub(crate) fn accesskit_node_id(&self) -> accesskit::NodeId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::default();
        self.hash(&mut hasher);
        accesskit::NodeId(hasher.finish())
    }
}

trait ElementObject {
    fn inner_element(&mut self) -> &mut dyn Any;

    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId;

    fn prepaint(&mut self, window: &mut Window, cx: &mut App);

    fn paint(&mut self, window: &mut Window, cx: &mut App);

    fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels>;
}

/// An [`Element`] together with the state GPUI keeps while drawing it in one frame.
///
/// GPUI creates a `Drawable` for each element in the tree, and an [`AnyElement`] holds one.
/// Its phase state moves through the frame's phases in order. You don't construct it yourself.
/// Use `element` to reach the wrapped element.
pub struct Drawable<E: Element> {
    /// The wrapped element.
    pub element: E,
    phase: ElementDrawPhase<E::RequestLayoutState, E::PrepaintState>,
}

#[derive(Default)]
enum ElementDrawPhase<RequestLayoutState, PrepaintState> {
    #[default]
    Start,
    RequestLayout {
        layout_id: LayoutId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        request_layout: RequestLayoutState,
    },
    LayoutComputed {
        layout_id: LayoutId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        available_space: Size<AvailableSpace>,
        request_layout: RequestLayoutState,
    },
    Prepaint {
        node_id: DispatchNodeId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: RequestLayoutState,
        prepaint: PrepaintState,
        inline_fragments: Option<Arc<[Bounds<Pixels>]>>,
    },
    Painted,
}

impl<E: Element> Drawable<E> {
    pub(crate) fn new(element: E) -> Self {
        Drawable {
            element,
            phase: ElementDrawPhase::Start,
        }
    }

    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Start => {
                let global_id = self.element.id().map(|element_id| {
                    window.element_id_stack.push(element_id);
                    GlobalElementId(Arc::from(&*window.element_id_stack))
                });

                let inspector_id;
                #[cfg(any(feature = "inspector", debug_assertions))]
                {
                    inspector_id = self.element.source_location().map(|source| {
                        let path = crate::InspectorElementPath {
                            global_id: GlobalElementId(Arc::from(&*window.element_id_stack)),
                            source_location: source,
                        };
                        window.build_inspector_element_id(path)
                    });
                }
                #[cfg(not(any(feature = "inspector", debug_assertions)))]
                {
                    inspector_id = None;
                }

                #[cfg(all(
                    feature = "hot-patching",
                    debug_assertions,
                    not(target_family = "wasm")
                ))]
                let (layout_id, request_layout) = subsecond::HotFn::current(E::request_layout)
                    .call((
                        &mut self.element,
                        global_id.as_ref(),
                        inspector_id.as_ref(),
                        window,
                        cx,
                    ));
                #[cfg(not(all(
                    feature = "hot-patching",
                    debug_assertions,
                    not(target_family = "wasm")
                )))]
                let (layout_id, request_layout) = self.element.request_layout(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                self.phase = ElementDrawPhase::RequestLayout {
                    layout_id,
                    global_id,
                    inspector_id,
                    request_layout,
                };
                layout_id
            }
            _ => panic!("must call request_layout only once"),
        }
    }

    pub(crate) fn prepaint(&mut self, window: &mut Window, cx: &mut App) {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                global_id,
                inspector_id,
                mut request_layout,
            }
            | ElementDrawPhase::LayoutComputed {
                layout_id,
                global_id,
                inspector_id,
                mut request_layout,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    window.element_id_stack.push(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*window.element_id_stack);
                }

                let bounds = window.layout_bounds(layout_id);
                let mut pushed_a11y_node = false;
                if window.a11y.is_active() {
                    if let Some(global_id) = global_id.as_ref() {
                        let hidden = self.element.is_a11y_hidden();
                        let role = self.element.a11y_role();
                        if role.is_some() || hidden {
                            let role = role.unwrap_or(accesskit::Role::GenericContainer);
                            let node_id = global_id.accesskit_node_id();
                            let mut node = accesskit::Node::new(role);
                            let scale = window.scale_factor();
                            node.set_bounds(accesskit::Rect {
                                x0: (bounds.origin.x.0 * scale) as f64,
                                y0: (bounds.origin.y.0 * scale) as f64,
                                x1: ((bounds.origin.x.0 + bounds.size.width.0) * scale) as f64,
                                y1: ((bounds.origin.y.0 + bounds.size.height.0) * scale) as f64,
                            });
                            self.element.write_a11y_info(&mut node);
                            if hidden {
                                node.set_hidden();
                            }
                            window.a11y.node_bounds.insert(node_id, bounds);
                            pushed_a11y_node = window.a11y.nodes.push(node_id, node);
                            #[cfg(debug_assertions)]
                            if pushed_a11y_node {
                                let view = window
                                    .a11y
                                    .view_type_names
                                    .get(&window.current_view())
                                    .copied();
                                let source_location = self.element.source_location();
                                window.a11y.nodes.record_node_info(
                                    node_id,
                                    crate::window::a11y::debug::NodeDebugInfo {
                                        synthetic: false,
                                        view,
                                        element_id: global_id.0.last().map(|id| format!("{id:?}")),
                                        source_location,
                                    },
                                );
                            }
                        }
                    }
                }

                let node_id = window.next_frame.dispatch_tree.push_node();

                let inline_fragments = window.inline_fragments(layout_id);
                let previous_fragments = std::mem::replace(
                    &mut window.current_inline_fragments,
                    inline_fragments.clone(),
                );

                let mut prepaint = window.with_layout_direction_context(layout_id, |window| {
                    #[cfg(all(
                        feature = "hot-patching",
                        debug_assertions,
                        not(target_family = "wasm")
                    ))]
                    {
                        subsecond::HotFn::current(E::prepaint).call((
                            &mut self.element,
                            global_id.as_ref(),
                            inspector_id.as_ref(),
                            bounds,
                            &mut request_layout,
                            window,
                            cx,
                        ))
                    }

                    #[cfg(not(all(
                        feature = "hot-patching",
                        debug_assertions,
                        not(target_family = "wasm")
                    )))]
                    {
                        self.element.prepaint(
                            global_id.as_ref(),
                            inspector_id.as_ref(),
                            bounds,
                            &mut request_layout,
                            window,
                            cx,
                        )
                    }
                });

                window.current_inline_fragments = previous_fragments;
                window.next_frame.dispatch_tree.pop_node();

                if pushed_a11y_node {
                    if let Some(global_id) = global_id.as_ref() {
                        #[cfg(debug_assertions)]
                        let creator = crate::window::a11y::debug::NodeCreator {
                            view: window
                                .a11y
                                .view_type_names
                                .get(&window.current_view())
                                .copied(),
                            element_id: global_id.0.last().map(|id| format!("{id:?}")),
                            source_location: self.element.source_location(),
                        };
                        let mut builder = A11ySubtreeBuilder::new(
                            global_id.accesskit_node_id(),
                            &mut window.a11y.nodes,
                        );
                        #[cfg(debug_assertions)]
                        {
                            builder = builder.with_creator(creator);
                        }
                        self.element
                            .a11y_synthetic_children(&mut prepaint, &mut builder);
                    }
                    window.a11y.nodes.pop();
                }

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                self.phase = ElementDrawPhase::Prepaint {
                    node_id,
                    global_id,
                    inspector_id,
                    bounds,
                    request_layout,
                    prepaint,
                    inline_fragments,
                };
            }
            _ => panic!("must call request_layout before prepaint"),
        }
    }

    pub(crate) fn paint(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> (E::RequestLayoutState, E::PrepaintState) {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Prepaint {
                node_id,
                global_id,
                inspector_id,
                bounds,
                mut request_layout,
                mut prepaint,
                inline_fragments,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    window.element_id_stack.push(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*window.element_id_stack);
                }

                window.next_frame.dispatch_tree.set_active_node(node_id);

                let previous_fragments =
                    std::mem::replace(&mut window.current_inline_fragments, inline_fragments);

                #[cfg(all(
                    feature = "hot-patching",
                    debug_assertions,
                    not(target_family = "wasm")
                ))]
                subsecond::HotFn::current(E::paint).call((
                    &mut self.element,
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    &mut prepaint,
                    window,
                    cx,
                ));
                #[cfg(not(all(
                    feature = "hot-patching",
                    debug_assertions,
                    not(target_family = "wasm")
                )))]
                self.element.paint(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    &mut prepaint,
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                window.current_inline_fragments = previous_fragments;

                self.phase = ElementDrawPhase::Painted;
                (request_layout, prepaint)
            }
            _ => panic!("must call prepaint before paint"),
        }
    }

    fn compute_layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Size<Pixels>) {
        if matches!(&self.phase, ElementDrawPhase::Start) {
            self.request_layout(window, cx);
        }

        let layout_id = match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                global_id,
                inspector_id,
                request_layout,
            } => {
                window.compute_layout(layout_id, available_space, cx);
                self.phase = ElementDrawPhase::LayoutComputed {
                    layout_id,
                    global_id,
                    inspector_id,
                    available_space,
                    request_layout,
                };
                layout_id
            }
            ElementDrawPhase::LayoutComputed {
                layout_id,
                global_id,
                inspector_id,
                available_space: prev_available_space,
                request_layout,
            } => {
                if available_space != prev_available_space {
                    window.compute_layout(layout_id, available_space, cx);
                }
                self.phase = ElementDrawPhase::LayoutComputed {
                    layout_id,
                    global_id,
                    inspector_id,
                    available_space,
                    request_layout,
                };
                layout_id
            }
            _ => panic!("cannot measure after painting"),
        };

        (layout_id, window.layout_bounds(layout_id).size)
    }

    pub(crate) fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        self.compute_layout_as_root(available_space, window, cx).1
    }
}

impl<E> ElementObject for Drawable<E>
where
    E: Element,
    E::RequestLayoutState: 'static,
{
    fn inner_element(&mut self) -> &mut dyn Any {
        &mut self.element
    }

    #[inline]
    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        Drawable::request_layout(self, window, cx)
    }

    #[inline]
    fn prepaint(&mut self, window: &mut Window, cx: &mut App) {
        Drawable::prepaint(self, window, cx);
    }

    #[inline]
    fn paint(&mut self, window: &mut Window, cx: &mut App) {
        Drawable::paint(self, window, cx);
    }

    #[inline]
    fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        Drawable::layout_as_root(self, available_space, window, cx)
    }
}

/// An element with its concrete type erased, so that elements of different types can share one
/// type.
///
/// Use it when a function must return one type but its branches build different elements, or when
/// a list holds elements of mixed types. Create one with [`IntoElement::into_any_element`] or
/// [`Element::into_any`]. Its methods run the same phases as the element it wraps.
///
/// # Example
///
/// ```
/// use gpui::{AnyElement, Div, IntoElement, div, prelude::*};
///
/// fn label(wide: bool) -> AnyElement {
///     if wide {
///         div().w_full().child("Wide").into_any_element()
///     } else {
///         "Narrow".into_any_element()
///     }
/// }
///
/// let mut wide = label(true);
/// assert!(wide.downcast_mut::<Div>().is_some());
///
/// let mut narrow = label(false);
/// assert!(narrow.downcast_mut::<Div>().is_none());
/// ```
pub struct AnyElement(ArenaBox<dyn ElementObject>);

impl AnyElement {
    pub(crate) fn new<E>(element: E) -> Self
    where
        E: 'static + Element,
        E::RequestLayoutState: Any,
    {
        let element = with_element_arena(|arena| arena.alloc(|| Drawable::new(element)))
            .map(|element| element as &mut dyn ElementObject);
        AnyElement(element)
    }

    /// Returns a mutable reference to the wrapped element if it has type `T`, or `None` otherwise.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0.inner_element().downcast_mut::<T>()
    }

    /// Runs the request-layout phase on the wrapped element, and returns its layout id.
    ///
    /// Call this from a parent's `request_layout`, before the parent's own layout is requested.
    pub fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        self.0.request_layout(window, cx)
    }

    /// Runs the prepaint phase on the wrapped element, with the bounds that layout gave it.
    ///
    /// Returns the [`FocusHandle`] of a focused element in this subtree, if one became focused
    /// during this call. Otherwise returns `None`.
    pub fn prepaint(&mut self, window: &mut Window, cx: &mut App) -> Option<FocusHandle> {
        let focus_assigned = window.next_frame.focus.is_some();

        self.0.prepaint(window, cx);

        if !focus_assigned && let Some(focus_id) = window.next_frame.focus {
            return FocusHandle::for_id(focus_id, &cx.focus_handles);
        }

        None
    }

    /// Runs the paint phase on the wrapped element. Call it after [`AnyElement::prepaint`].
    pub fn paint(&mut self, window: &mut Window, cx: &mut App) {
        self.0.paint(window, cx);
    }

    /// Lays out this element as the root of its own layout, within `available_space`, and returns
    /// its size.
    ///
    /// The element is not placed in the window by this call. Use it to measure an element before
    /// placing it with [`AnyElement::prepaint_at`]. If the layout hasn't been requested yet, this
    /// method requests it first.
    pub fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        self.0.layout_as_root(available_space, window, cx)
    }

    /// Runs the prepaint phase with `origin` as the element's origin, in window coordinates.
    ///
    /// Call [`AnyElement::layout_as_root`] first, or use [`AnyElement::prepaint_as_root`], which
    /// does both. Returns the focus handle of a focused element in the subtree that became focused
    /// during this call, if any.
    pub fn prepaint_at(
        &mut self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FocusHandle> {
        window.with_absolute_element_offset(origin, |window| self.prepaint(window, cx))
    }

    /// Lays out this element as a root within `available_space`, then prepaints it at `origin`.
    ///
    /// Use it for an element that sits outside its parent's layout, such as a popover placed at a
    /// window position. Returns the focus handle of a focused element in the subtree that became
    /// focused during this call, if any.
    pub fn prepaint_as_root(
        &mut self,
        origin: Point<Pixels>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FocusHandle> {
        self.layout_as_root(available_space, window, cx);
        window.with_absolute_element_offset(origin, |window| self.prepaint(window, cx))
    }
}

impl Element for AnyElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint(window, cx);
    }
}

impl IntoElement for AnyElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }

    fn into_any_element(self) -> AnyElement {
        self
    }
}

/// An element that draws nothing and takes up no space.
///
/// Return it where a view or component must produce an element but has nothing to show. Its
/// layout has `display: none`, so it doesn't affect its siblings.
pub struct Empty;

impl IntoElement for Empty {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Empty {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    display: crate::Display::None,
                    ..Default::default()
                },
                None,
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}
