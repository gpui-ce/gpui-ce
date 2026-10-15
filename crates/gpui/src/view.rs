use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, Bounds, ContentMask, Context, Element, ElementId,
    Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintIndex,
    Pixels, PrepaintStateIndex, Render, RenderOnce, ResolvedDirection, Style, StyleRefinement,
    TextStyle, UnicodeBidi, WeakEntity,
};
use crate::{Empty, Window};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::mem;
use std::{any::TypeId, fmt, ops::Range};

/// A type-erased handle to a view.
///
/// It pairs an [`Entity`] with its render function, so a field or collection can hold views of
/// different types. Convert an entity with `AnyView::from` or `.into()`, and recover its type with
/// [`AnyView::downcast`]. An `AnyView` can be a child wherever a view is accepted. Two `AnyView`s
/// compare equal when they refer to the same entity.
///
/// # Example
///
/// ```
/// use gpui::{AnyView, Context, Render, TestApp, Window, div, prelude::*};
///
/// struct First;
/// struct Second;
///
/// impl Render for First {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div().child("First")
///     }
/// }
///
/// impl Render for Second {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         div().child("Second")
///     }
/// }
///
/// let mut app = TestApp::new();
/// let (first, second) = app.update(|cx| (cx.new(|_| First), cx.new(|_| Second)));
/// let views: Vec<AnyView> = vec![first.into(), second.into()];
///
/// assert!(views[0].clone().downcast::<First>().is_ok());
/// assert!(views[1].clone().downcast::<First>().is_err());
/// ```
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(value: Entity<V>) -> Self {
        AnyView {
            entity: value.into_any(),
            render: any_view::render::<V>,
        }
    }
}

impl AnyView {
    /// Returns this view as an element that reuses its last frame's output, laid out at `style`.
    ///
    /// The rendered subtree is reused until [`Context::notify`] is called on the view's entity,
    /// or until its bounds, content mask, text style, text direction, or Unicode bidirectional
    /// text mode changes. [`Window::refresh`] redraws everything. If the view reads another
    /// entity, observe that entity and notify this view when its value changes. Caching needs a
    /// definite size from `style`, because the contents aren't rendered to measure them. See
    /// [`Entity::cached`].
    pub fn cached(self, style: StyleRefinement) -> ViewElement<AnyView> {
        ViewElement::new(self).cached(style)
    }

    /// Returns a weak handle to this view. The weak handle doesn't keep the view alive.
    ///
    /// Call [`AnyWeakView::upgrade`] on it to get an `AnyView` back, or `None` once the view has
    /// been dropped.
    pub fn downgrade(&self) -> AnyWeakView {
        AnyWeakView {
            entity: self.entity.downgrade(),
            render: self.render,
        }
    }

    /// Converts this handle into an [`Entity`] of type `T`.
    ///
    /// Returns `Err` with the unchanged `AnyView` when the view isn't a `T`.
    pub fn downcast<T: 'static>(self) -> Result<Entity<T>, Self> {
        match self.entity.downcast() {
            Ok(entity) => Ok(entity),
            Err(entity) => Err(Self {
                entity,
                render: self.render,
            }),
        }
    }

    /// Returns the [`TypeId`] of the view's type.
    pub fn entity_type(&self) -> TypeId {
        self.entity.entity_type
    }

    /// Returns the [`EntityId`] of the entity that backs this view.
    pub fn entity_id(&self) -> EntityId {
        self.entity.entity_id()
    }
}

impl PartialEq for AnyView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for AnyView {}

/// `AnyView` is the type-erased [`View`]. Its render is a function pointer rather than a
/// concrete type, so it takes part in the reactive graph like any other view, with the same
/// identity as its entity.
impl View for AnyView {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.entity.entity_id())
    }

    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.render)(&self, window, cx)
    }
}

impl<V: 'static + Render> IntoElement for Entity<V> {
    type Element = ViewElement<Entity<V>>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

impl IntoElement for AnyView {
    type Element = ViewElement<AnyView>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

/// A weak, type-erased handle to a view.
///
/// It does not keep the view alive. Create one with [`AnyView::downgrade`] or convert a
/// [`WeakEntity`].
pub struct AnyWeakView {
    entity: AnyWeakEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl AnyWeakView {
    /// Returns a strong [`AnyView`] if the view is still alive, or `None` if it has been dropped.
    pub fn upgrade(&self) -> Option<AnyView> {
        let entity = self.entity.upgrade()?;
        Some(AnyView {
            entity,
            render: self.render,
        })
    }
}

impl<V: 'static + Render> From<WeakEntity<V>> for AnyWeakView {
    fn from(view: WeakEntity<V>) -> Self {
        AnyWeakView {
            entity: view.into(),
            render: any_view::render::<V>,
        }
    }
}

impl PartialEq for AnyWeakView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl std::fmt::Debug for AnyWeakView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyWeakView")
            .field("entity_id", &self.entity.entity_id)
            .finish_non_exhaustive()
    }
}

mod any_view {
    use crate::{AnyElement, AnyView, App, IntoElement, Render, Window};

    pub(crate) fn render<V: 'static + Render>(
        view: &AnyView,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let view = view.clone().downcast::<V>().unwrap();
        // Record the view's Render type name so the accessibility debug dump can
        // attribute nodes to the view that produced them.
        #[cfg(debug_assertions)]
        window
            .a11y
            .view_type_names
            .insert(view.entity_id(), std::any::type_name::<V>());
        view.update(cx, |view, cx| {
            #[cfg(all(
                feature = "hot-patching",
                debug_assertions,
                not(target_family = "wasm")
            ))]
            let element = subsecond::HotFn::current(V::render).call((view, window, cx));
            #[cfg(not(all(
                feature = "hot-patching",
                debug_assertions,
                not(target_family = "wasm")
            )))]
            let element = view.render(window, cx);
            element.into_any_element()
        })
    }
}

/// A value that renders into elements and may carry an entity identity.
///
/// Every [`Entity`] of a [`Render`] type is a `View`, and so is every [`RenderOnce`] component.
/// Implement `View` directly when a component needs both props from its parent and an entity
/// identity. Otherwise, use [`Render`] for an entity-backed view or [`RenderOnce`] for a stateless
/// component.
///
/// When [`View::entity_id`] returns `Some`, that entity is the view's identity. GPUI gives the
/// view its own space for element ids, so state inside it can't collide with its siblings. A
/// [`Context::notify`] on that entity redraws only this view's subtree. When it returns `None`,
/// the view is stateless. GPUI scopes it under its type name, and it has no entity to notify.
pub trait View: 'static + Sized {
    /// Returns the entity that identifies this view, or `None` for a stateless view.
    ///
    /// A view that holds its entity as a field returns the entity's id here. The id becomes the
    /// view's [`ElementId`]. Two views backed by the same entity must not be siblings under one
    /// parent, because their element state would collide. Nesting is fine, since the parent's
    /// path keeps the ids apart.
    fn entity_id(&self) -> Option<EntityId>;

    /// Renders this view into elements. Takes `self` by value, so the view is used up.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

fn render_view<V: View>(view: V, window: &mut Window, cx: &mut App) -> AnyElement {
    #[cfg(all(
        feature = "hot-patching",
        debug_assertions,
        not(target_family = "wasm")
    ))]
    let element = subsecond::HotFn::current(V::render).call((view, window, cx));
    #[cfg(not(all(
        feature = "hot-patching",
        debug_assertions,
        not(target_family = "wasm")
    )))]
    let element = view.render(window, cx);
    element.into_any_element()
}

/// A stateless component (`RenderOnce`) is a [`View`] with no entity identity.
impl<T: RenderOnce> View for T {
    fn entity_id(&self) -> Option<EntityId> {
        None
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        #[cfg(all(
            feature = "hot-patching",
            debug_assertions,
            not(target_family = "wasm")
        ))]
        let element = subsecond::HotFn::current(<T as RenderOnce>::render).call((self, window, cx));
        #[cfg(not(all(
            feature = "hot-patching",
            debug_assertions,
            not(target_family = "wasm")
        )))]
        let element = RenderOnce::render(self, window, cx);
        element
    }
}

/// An entity that renders itself (`Render`) is a [`View`] keyed by its own id.
impl<T: Render> View for Entity<T> {
    fn entity_id(&self) -> Option<EntityId> {
        Some(Entity::entity_id(self))
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.update(cx, |this, cx| {
            #[cfg(all(
                feature = "hot-patching",
                debug_assertions,
                not(target_family = "wasm")
            ))]
            let element = subsecond::HotFn::current(T::render).call((this, window, cx));
            #[cfg(not(all(
                feature = "hot-patching",
                debug_assertions,
                not(target_family = "wasm")
            )))]
            let element = Render::render(this, window, cx);
            element.into_any_element()
        })
    }
}

impl<T: Render> Entity<T> {
    /// Returns this entity as a cached element laid out at `style`.
    ///
    /// Use it for an expensive view that changes rarely. Its rendered subtree is reused until the
    /// entity is notified with [`Context::notify`]. It is also rendered again when its bounds,
    /// content mask, text style, text direction, or Unicode bidirectional text mode changes. If
    /// the view reads another entity, observe that entity and notify this one when its value
    /// changes.
    ///
    /// Caching needs a definite size. The view is laid out from `style`, and its contents are not
    /// rendered to measure them, so `style` must give the view a size. For a view that should size
    /// itself from its contents, pass the entity as a child instead, which isn't cached.
    ///
    /// See [ownership and data flow](crate::_documentation::concepts::ownership) and
    /// [the frame](crate::_documentation::concepts::the_frame) for when frames are redrawn.
    #[track_caller]
    pub fn cached(self, style: StyleRefinement) -> ViewElement<Entity<T>> {
        ViewElement::new(self).cached(style)
    }
}

/// The element that draws a [`View`].
///
/// It wraps the view and runs it through the three element phases. Its id is the view's entity
/// when the view has one. You rarely name this type: entities, [`AnyView`] and derived components
/// all convert into it. Create one with [`ViewElement::new`].
#[doc(hidden)]
pub struct ViewElement<V: View> {
    view: Option<V>,
    entity_id: Option<EntityId>,
    cached_style: Option<StyleRefinement>,
    #[cfg(debug_assertions)]
    source: &'static core::panic::Location<'static>,
}

impl<V: View> ViewElement<V> {
    /// Wraps `view` as an element. The element takes the view's entity id, if it has one.
    #[track_caller]
    pub fn new(view: V) -> Self {
        let entity_id = view.entity_id();
        ViewElement {
            entity_id,
            cached_style: None,
            view: Some(view),
            #[cfg(debug_assertions)]
            source: core::panic::Location::caller(),
        }
    }

    /// Enable caching of this view's rendered subtree, laid out at `style`.
    /// The composer supplies the layout style because caching skips rendering
    /// the contents to measure them.
    ///
    /// Crate-private on purpose: caching is only sound for entity-backed views,
    /// where [`Context::notify`] is the contract that busts the cache. A stateless
    /// view has no such contract, so a frozen subtree could never be invalidated.
    /// Reach this through [`Entity::cached`] or [`AnyView::cached`], which are
    /// entity-backed by construction.
    pub(crate) fn cached(mut self, style: StyleRefinement) -> Self {
        self.cached_style = Some(style);
        self
    }
}

impl<V: View> IntoElement for ViewElement<V> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct ViewElementState {
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    cache_key: ViewElementCacheKey,
    accessed_entities: FxHashSet<EntityId>,
}

/// The state that a [`ViewElement`] passes from `request_layout` to `prepaint` and `paint`.
///
/// It holds the view's rendered element until prepaint takes it. GPUI also records the entities
/// read while producing the subtree so the surrounding window can account for those reads when
/// dispatching invalidations.
#[derive(Default)]
#[doc(hidden)]
pub struct ViewElementRequestLayoutState {
    element: Option<AnyElement>,
    detached_layout_id: Option<LayoutId>,
    accessed_entities: FxHashSet<EntityId>,
}

#[derive(Default)]
struct ViewDirectionState {
    known: bool,
    contribution: Option<ResolvedDirection>,
}

#[derive(PartialEq)]
struct ViewElementCacheKey {
    bounds: Bounds<Pixels>,
    content_mask: ContentMask<Pixels>,
    text_style: TextStyle,
    direction: ResolvedDirection,
    unicode_bidi: UnicodeBidi,
}

impl<V: View> Element for ViewElement<V> {
    type RequestLayoutState = ViewElementRequestLayoutState;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        self.entity_id.map(ElementId::View)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        #[cfg(debug_assertions)]
        return Some(self.source);

        #[cfg(not(debug_assertions))]
        return None;
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let entity_id = self.entity_id;
        let request_layout = |window: &mut Window| {
            let caching_disabled = window.is_inspector_picking(cx);

            if let (Some(entity_id), Some(style)) = (
                entity_id,
                self.cached_style.as_ref().filter(|_| !caching_disabled),
            ) {
                let mut root_style = Style::default();
                root_style.refine(style);
                let layout_id = window.request_layout(root_style, None, cx);
                let (known, contribution) = window.with_element_state::<ViewDirectionState, _>(
                    global_id.unwrap(),
                    |state, _window| {
                        let state = state.unwrap_or_default();
                        ((state.known, state.contribution), state)
                    },
                );
                let should_probe =
                    !known || window.dirty_views.contains(&entity_id) || window.refreshing;

                if should_probe {
                    let ((element, detached_layout_id), accessed_entities) = cx
                        .detect_accessed_entities(|cx| {
                            let mut element = render_view(self.view.take().unwrap(), window, cx);
                            let detached_layout_id = element.request_layout(window, cx);

                            (element, detached_layout_id)
                        });
                    window.set_layout_logical_children(
                        layout_id,
                        std::slice::from_ref(&detached_layout_id),
                    );

                    return (
                        layout_id,
                        ViewElementRequestLayoutState {
                            element: Some(element),
                            detached_layout_id: Some(detached_layout_id),
                            accessed_entities,
                        },
                    );
                }

                window.set_layout_auto_direction_hint(layout_id, contribution);

                return (layout_id, ViewElementRequestLayoutState::default());
            }

            let mut element = render_view(self.view.take().unwrap(), window, cx);
            let layout_id = element.request_layout(window, cx);

            (
                layout_id,
                ViewElementRequestLayoutState {
                    element: Some(element),
                    ..Default::default()
                },
            )
        };

        if let Some(entity_id) = entity_id {
            window.with_rendered_view(entity_id, request_layout)
        } else {
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                request_layout,
            )
        }
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            window.set_view_id(entity_id);
            window.with_rendered_view(entity_id, |window| {
                if request_layout.detached_layout_id.is_none()
                    && let Some(mut element) = request_layout.element.take()
                {
                    element.prepaint(window, cx);
                    return Some(element);
                }

                window.with_element_state::<ViewElementState, _>(
                    global_id.unwrap(),
                    |element_state, window| {
                        let cache_key = ViewElementCacheKey {
                            bounds,
                            content_mask: window.content_mask(),
                            text_style: window.text_style(),
                            direction: window.resolved_direction(),
                            unicode_bidi: window.resolved_unicode_bidi(),
                        };

                        if request_layout.element.is_none()
                            && let Some(mut element_state) = element_state
                            && element_state.cache_key == cache_key
                            && !window.dirty_views.contains(&entity_id)
                            && !window.refreshing
                        {
                            let prepaint_start = window.prepaint_index();
                            window.reuse_prepaint(element_state.prepaint_range.clone());
                            cx.entities
                                .extend_accessed(&element_state.accessed_entities);
                            let prepaint_end = window.prepaint_index();
                            element_state.prepaint_range = prepaint_start..prepaint_end;

                            return (None, element_state);
                        }

                        let refreshing = mem::replace(&mut window.refreshing, true);
                        let prepaint_start = window.prepaint_index();
                        let mut accessed_entities =
                            mem::take(&mut request_layout.accessed_entities);
                        let (element, additional_entities) = cx.detect_accessed_entities(|cx| {
                            let mut element = request_layout.element.take().unwrap_or_else(|| {
                                render_view(self.view.take().unwrap(), window, cx)
                            });
                            element.layout_as_root(bounds.size.into(), window, cx);
                            element.prepaint_at(bounds.origin, window, cx);

                            element
                        });
                        accessed_entities.extend(additional_entities);

                        if let Some(detached_layout_id) = request_layout.detached_layout_id.take() {
                            let contribution =
                                window.layout_auto_direction_contribution(detached_layout_id);
                            window.with_element_state::<ViewDirectionState, _>(
                                global_id.unwrap(),
                                |_state, _window| {
                                    (
                                        (),
                                        ViewDirectionState {
                                            known: true,
                                            contribution,
                                        },
                                    )
                                },
                            );
                        }

                        let prepaint_end = window.prepaint_index();
                        window.refreshing = refreshing;

                        (
                            Some(element),
                            ViewElementState {
                                accessed_entities,
                                prepaint_range: prepaint_start..prepaint_end,
                                paint_range: PaintIndex::default()..PaintIndex::default(),
                                cache_key,
                            },
                        )
                    },
                )
            })
        } else {
            // Stateless path: just prepaint the element.
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                |window| {
                    request_layout
                        .element
                        .as_mut()
                        .unwrap()
                        .prepaint(window, cx);
                },
            );
            Some(request_layout.element.take().unwrap())
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        element: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            window.with_rendered_view(entity_id, |window| {
                let caching_disabled = window.is_inspector_picking(cx);
                if self.cached_style.is_some() && !caching_disabled {
                    window.with_element_state::<ViewElementState, _>(
                        global_id.unwrap(),
                        |element_state, window| {
                            let mut element_state = element_state.unwrap();

                            let paint_start = window.paint_index();

                            if let Some(element) = element {
                                let refreshing = mem::replace(&mut window.refreshing, true);
                                element.paint(window, cx);
                                window.refreshing = refreshing;
                            } else {
                                window.reuse_paint(element_state.paint_range.clone());
                            }

                            let paint_end = window.paint_index();
                            element_state.paint_range = paint_start..paint_end;

                            ((), element_state)
                        },
                    )
                } else {
                    element.as_mut().unwrap().paint(window, cx);
                }
            });
        } else {
            // Stateless path: just paint the element.
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                |window| {
                    element.as_mut().unwrap().paint(window, cx);
                },
            );
        }
    }
}

/// A view that renders nothing.
///
/// Its `render` returns [`Empty`]. Use it as a placeholder view, for example as the root of a
/// window that has no content yet.
pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}
