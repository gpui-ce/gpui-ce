//! Lists of rows that all have the same height, rendered lazily.
//!
//! [`uniform_list`] renders only the rows in the viewport. It measures one row, then
//! places every row at `index * height`, so it never runs layout on rows that are
//! off screen. Use [`crate::list`] when rows have different heights.
//!
//! The [lists and scrolling guide](crate::_documentation::guides::lists_and_scrolling) compares
//! both list elements and shows how to keep their state on a view.

use crate::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, Entity, GlobalElementId, Hitbox,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, IsZero, LayoutId,
    ListSizingBehavior, Overflow, Pixels, Point, ScrollHandle, Size, StyleRefinement, Styled,
    Window, point, px, size,
};
use smallvec::SmallVec;
use std::{cell::RefCell, cmp, ops::Range, rc::Rc};

use super::ListHorizontalSizingBehavior;

/// Creates a list of rows that all have the same height, and renders only the rows in view.
///
/// Use this for long lists, such as tables, file trees or contact lists, when every row has
/// the same height. Use [`list`](fn@crate::list) when rows vary in height.
///
/// `f` receives the range of visible row indices and returns one element per row, in order.
/// Each frame it runs for the visible range. It also runs for a single row to measure it, in
/// both layout and prepaint.
/// So `f` must accept any range within `0..item_count`, and every row it returns must be the
/// height of the measured row.
///
/// The list sets its own overflow to scroll. Give it a bounded height, from its parent or from
/// [`Styled::h`] or [`Styled::size_full`]. By default the height comes from the style. With
/// [`UniformList::with_sizing_behavior`] set to [`ListSizingBehavior::Infer`], a list with no
/// bounded height is as tall as all its rows, so every row is in view and gets rendered.
///
/// Rows outside the viewport are not created. Keep your data on your view and index into it
/// from `f`, because row elements are dropped as they scroll away.
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// # use std::{cell::Cell, rc::Rc};
/// struct Contacts {
///     names: Rc<Vec<String>>,
///     widest_batch: Rc<Cell<usize>>,
/// }
///
/// impl Render for Contacts {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let names = self.names.clone();
///         let widest_batch = self.widest_batch.clone();
///         uniform_list("contacts", names.len(), move |range, _window, _cx| {
///             widest_batch.set(widest_batch.get().max(range.len()));
///             range
///                 .map(|ix| div().id(ix).h(px(32.)).child(names[ix].clone()))
///                 .collect::<Vec<_>>()
///         })
///         .size_full()
///     }
/// }
/// # let mut app = TestApp::new();
/// # let widest_batch = Rc::new(Cell::new(0));
/// # let names: Rc<Vec<String>> = Rc::new((0..10_000).map(|ix| format!("Contact {ix}")).collect());
/// # let mut window = app.open_window(|_, _| Contacts { names, widest_batch: widest_batch.clone() });
/// # window.draw();
/// // Only the rows on screen are passed to the closure, never all 10,000.
/// # assert!(widest_batch.get() > 0 && widest_batch.get() < 1_000);
/// ```
#[track_caller]
pub fn uniform_list<R>(
    id: impl Into<ElementId>,
    item_count: usize,
    f: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>,
) -> UniformList
where
    R: IntoElement,
{
    let id = id.into();
    let mut base_style = StyleRefinement::default();
    base_style.overflow.y = Some(Overflow::Scroll);

    let render_range = move |range: Range<usize>, window: &mut Window, cx: &mut App| {
        f(range, window, cx)
            .into_iter()
            .map(|component| component.into_any_element())
            .collect()
    };

    UniformList {
        item_count,
        item_to_measure_index: 0,
        render_items: Box::new(render_range),
        decorations: Vec::new(),
        interactivity: Interactivity {
            element_id: Some(id),
            base_style: Box::new(base_style),
            ..Interactivity::new()
        },
        scroll_handle: None,
        sizing_behavior: ListSizingBehavior::default(),
        horizontal_sizing_behavior: ListHorizontalSizingBehavior::default(),
    }
}

/// The element returned by [`uniform_list`]. Configure it with the builder methods, then
/// return it from `render`. To read or set the scroll position, pass a
/// [`UniformListScrollHandle`] to [`UniformList::track_scroll`].
pub struct UniformList {
    item_count: usize,
    item_to_measure_index: usize,
    render_items: Box<
        dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> SmallVec<[AnyElement; 64]>,
    >,
    decorations: Vec<Box<dyn UniformListDecoration>>,
    interactivity: Interactivity,
    scroll_handle: Option<UniformListScrollHandle>,
    sizing_behavior: ListSizingBehavior,
    horizontal_sizing_behavior: ListHorizontalSizingBehavior,
}

/// The rows and decorations that a [`UniformList`] lays out for one frame. You don't construct it.
pub struct UniformListFrameState {
    items: SmallVec<[AnyElement; 32]>,
    decorations: SmallVec<[AnyElement; 2]>,
}

/// A handle that reads and sets the scroll position of a [`UniformList`].
///
/// Store it on your view, and pass it to [`UniformList::track_scroll`] when you build the list
/// each frame. Clones share the same state.
///
/// Scroll requests (`scroll_to_*`) take effect during the next prepaint, not immediately. A
/// request does nothing until a list tracks the handle.
#[derive(Clone, Debug, Default)]
pub struct UniformListScrollHandle(pub Rc<RefCell<UniformListScrollState>>);

/// Where a scroll request places the target row in the viewport.
///
/// The list stops at its first and last row, so a row near either end can't always reach the
/// position you ask for. It lands at the closest position the list allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollStrategy {
    /// Places the row at the top of the viewport.
    Top,
    /// Places the row in the middle of the viewport, as close as the start and end of the list allow.
    Center,
    /// Places the row at the bottom of the viewport, as close as the end of the list allows.
    Bottom,
    /// Moves the least amount that shows the row. A row above the viewport is placed at the top,
    /// and a row below it at the bottom. A row that is already fully visible doesn't move the list.
    Nearest,
}

#[derive(Clone, Copy, Debug)]
#[allow(missing_docs)]
/// A scroll request that waits for the next prepaint.
pub struct DeferredScrollToItem {
    /// The row to scroll to.
    pub item_index: usize,
    /// Where the row should land.
    pub strategy: ScrollStrategy,
    /// The number of rows to keep clear, as described for
    /// [`UniformListScrollHandle::scroll_to_item_with_offset`].
    pub offset: usize,
    /// Whether to apply `strategy` even when the row is already visible.
    pub scroll_strict: bool,
}

#[derive(Clone, Debug, Default)]
#[allow(missing_docs)]
/// The state that a [`UniformListScrollHandle`] shares with its [`UniformList`].
pub struct UniformListScrollState {
    /// The scroll position of the list. [`UniformList::track_scroll`] shares it with the list.
    pub base_handle: ScrollHandle,
    /// A scroll request from a `scroll_to_*` method, applied and cleared on the next prepaint.
    pub deferred_scroll_to_item: Option<DeferredScrollToItem>,
    /// The viewport and content sizes from the last prepaint, or `None` before the first one.
    pub last_item_size: Option<ItemSize>,
    /// Whether item 0 is at the bottom of the list, as set by [`UniformList::y_flipped`].
    pub y_flipped: bool,
}

#[derive(Copy, Clone, Debug, Default)]
/// The sizes of a [`UniformList`]'s viewport and of all its rows, from the last prepaint.
pub struct ItemSize {
    /// The size of the viewport, after the list's border and padding.
    pub item: Size<Pixels>,
    /// The size of all the rows together: the row height times the row count, and the
    /// width of the content, which is wider than the viewport when rows overflow horizontally.
    pub contents: Size<Pixels>,
}

impl UniformListScrollHandle {
    /// Creates a handle that isn't attached to a list yet.
    ///
    /// Pass it to [`UniformList::track_scroll`] when you build the list.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(UniformListScrollState {
            base_handle: ScrollHandle::new(),
            deferred_scroll_to_item: None,
            last_item_size: None,
            y_flipped: false,
        })))
    }

    /// Scrolls the least amount that brings row `ix` fully into view, placed by `strategy`.
    ///
    /// This is non-strict: a row that is already fully visible doesn't move the list, so
    /// `ScrollStrategy::Top` does nothing for a visible row. Use [`Self::scroll_to_item_strict`]
    /// to place the row every time. The scroll takes effect on the next prepaint.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// struct Rows {
    ///     scroll: UniformListScrollHandle,
    /// }
    ///
    /// impl Render for Rows {
    ///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    ///         div().h(px(320.)).child(
    ///             uniform_list("rows", 100, |range, _window, _cx| {
    ///                 range
    ///                     .map(|ix| div().id(ix).h(px(32.)).child(format!("Row {ix}")))
    ///                     .collect::<Vec<_>>()
    ///             })
    ///             .track_scroll(&self.scroll)
    ///             .size_full(),
    ///         )
    ///     }
    /// }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_, _| Rows { scroll: UniformListScrollHandle::new() });
    /// window.update(|rows, _, _| rows.scroll.scroll_to_item(3, ScrollStrategy::Top));
    /// window.draw();
    /// // Row 3 is already in the 320px viewport, so the list didn't move.
    /// window.read(|rows, _| assert_eq!(rows.scroll.0.borrow().base_handle.offset().y, px(0.)));
    ///
    /// window.update(|rows, _, _| rows.scroll.scroll_to_item_strict(3, ScrollStrategy::Top));
    /// window.draw();
    /// // The strict form puts row 3 at the top: 3 rows of 32px scrolled past.
    /// window.read(|rows, _| assert_eq!(rows.scroll.0.borrow().base_handle.offset().y, px(-96.)));
    /// ```
    pub fn scroll_to_item(&self, ix: usize, strategy: ScrollStrategy) {
        self.0.borrow_mut().deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index: ix,
            strategy,
            offset: 0,
            scroll_strict: false,
        });
    }

    /// Places row `ix` where `strategy` says, even when the row is already visible.
    ///
    /// Unlike [`Self::scroll_to_item`], this forces the requested placement when the strategy
    /// supports one. With [`ScrollStrategy::Nearest`], a visible row still doesn't move. Use it
    /// when a row must land at a fixed position, such as the top after a keyboard jump. The scroll
    /// takes effect on the next prepaint; see [`Self::scroll_to_item`] for an example.
    pub fn scroll_to_item_strict(&self, ix: usize, strategy: ScrollStrategy) {
        self.0.borrow_mut().deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index: ix,
            strategy,
            offset: 0,
            scroll_strict: true,
        });
    }

    /// Scrolls like [`Self::scroll_to_item`], and keeps `offset` rows clear at the top of the viewport.
    ///
    /// `offset` shrinks the viewport by that many rows from the top, and `strategy` places the
    /// row in what remains:
    /// - [`ScrollStrategy::Top`] puts the row just below the cleared rows.
    /// - [`ScrollStrategy::Center`] centers the row in the rows below the cleared rows.
    /// - [`ScrollStrategy::Bottom`] places the row at the bottom of the viewport. `offset` affects
    ///   only whether the row counts as already visible, not where it lands.
    ///
    /// This is non-strict, so a row that is visible below the cleared rows doesn't move the list.
    pub fn scroll_to_item_with_offset(&self, ix: usize, strategy: ScrollStrategy, offset: usize) {
        self.0.borrow_mut().deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index: ix,
            strategy,
            offset,
            scroll_strict: false,
        });
    }

    /// Places row `ix` like [`Self::scroll_to_item_with_offset`], even when the row is already visible.
    ///
    /// The `offset` rules are the same as for [`Self::scroll_to_item_with_offset`].
    pub fn scroll_to_item_strict_with_offset(
        &self,
        ix: usize,
        strategy: ScrollStrategy,
        offset: usize,
    ) {
        self.0.borrow_mut().deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index: ix,
            strategy,
            offset,
            scroll_strict: true,
        });
    }

    /// Returns whether item 0 is at the bottom of the list, as set by [`UniformList::y_flipped`]
    /// on the last frame.
    pub fn y_flipped(&self) -> bool {
        self.0.borrow().y_flipped
    }

    /// Returns the index of the first visible row.
    ///
    /// If a scroll request is pending, this returns the row that request targets instead, so the
    /// value is right after a request and one frame later. It is zero before the first prepaint.
    ///
    /// Only available with the `test-support` feature, or in GPUI's own tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn logical_scroll_top_index(&self) -> usize {
        let this = self.0.borrow();
        this.deferred_scroll_to_item
            .as_ref()
            .map(|deferred| deferred.item_index)
            .unwrap_or_else(|| this.base_handle.logical_scroll_top().0)
    }

    /// Returns whether the rows are taller than the viewport, as of the last prepaint.
    ///
    /// Returns `false` before the first prepaint.
    pub fn is_scrollable(&self) -> bool {
        if let Some(size) = self.0.borrow().last_item_size {
            size.contents.height > size.item.height
        } else {
            false
        }
    }

    /// Returns whether the list shows its last row, or `None` if the list isn't scrollable.
    ///
    /// `None` is also returned before the first prepaint.
    pub fn is_scrolled_to_end(&self) -> Option<bool> {
        let state = self.0.borrow();
        let max_offset = state.base_handle.max_offset();
        if max_offset.y <= px(0.) {
            return None;
        }
        let offset = state.base_handle.offset();
        Some(-offset.y >= max_offset.y)
    }

    /// Scrolls to the last row, as a request for the next prepaint.
    pub fn scroll_to_bottom(&self) {
        self.scroll_to_item(usize::MAX, ScrollStrategy::Bottom);
    }
}

impl Styled for UniformList {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl Element for UniformList {
    type RequestLayoutState = UniformListFrameState;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let max_items = self.item_count;
        let item_size = self.measure_item(None, window, cx);
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| match self.sizing_behavior {
                ListSizingBehavior::Infer => {
                    window.with_text_style(style.text_style().cloned(), |window| {
                        window.request_measured_layout(
                            style,
                            move |known_dimensions, available_space, _window, _cx| {
                                let desired_height = item_size.height * max_items;
                                let width = known_dimensions.width.unwrap_or(match available_space
                                    .width
                                {
                                    AvailableSpace::Definite(x) => x,
                                    AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                        item_size.width
                                    }
                                });
                                let height = match available_space.height {
                                    AvailableSpace::Definite(height) => desired_height.min(height),
                                    AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                        desired_height
                                    }
                                };
                                size(width, height)
                            },
                        )
                    })
                }
                ListSizingBehavior::Auto => window
                    .with_text_style(style.text_style().cloned(), |window| {
                        window.request_layout(style, None, cx)
                    }),
            },
        );

        (
            layout_id,
            UniformListFrameState {
                items: SmallVec::new(),
                decorations: SmallVec::new(),
            },
        )
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame_state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Hitbox> {
        let style = self
            .interactivity
            .compute_style(global_id, None, window, cx);
        let border = style.border_widths.to_pixels(window.rem_size());
        let padding = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());

        let padded_bounds = Bounds::from_corners(
            bounds.origin + point(border.left + padding.left, border.top + padding.top),
            bounds.bottom_right()
                - point(border.right + padding.right, border.bottom + padding.bottom),
        );

        let can_scroll_horizontally = matches!(
            self.horizontal_sizing_behavior,
            ListHorizontalSizingBehavior::Unconstrained
        );

        let longest_item_size = self.measure_item(None, window, cx);
        let content_width = if can_scroll_horizontally {
            padded_bounds.size.width.max(longest_item_size.width)
        } else {
            padded_bounds.size.width
        };
        let content_size = Size {
            width: content_width,
            height: longest_item_size.height * self.item_count,
        };

        let shared_scroll_offset = self.interactivity.scroll_offset.clone().unwrap();
        let item_height = longest_item_size.height;
        let shared_scroll_to_item = self.scroll_handle.as_mut().and_then(|handle| {
            let mut handle = handle.0.borrow_mut();
            handle.last_item_size = Some(ItemSize {
                item: padded_bounds.size,
                contents: content_size,
            });
            handle.deferred_scroll_to_item.take()
        });

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |_style, mut scroll_offset, hitbox, window, cx| {
                let y_flipped = if let Some(scroll_handle) = &self.scroll_handle {
                    let scroll_state = scroll_handle.0.borrow();
                    scroll_state.y_flipped
                } else {
                    false
                };

                if self.item_count > 0 {
                    let content_height = item_height * self.item_count;

                    let is_scrolled_vertically = !scroll_offset.y.is_zero();
                    let max_scroll_offset = padded_bounds.size.height - content_height;

                    if is_scrolled_vertically && scroll_offset.y < max_scroll_offset {
                        shared_scroll_offset.borrow_mut().y = max_scroll_offset;
                        scroll_offset.y = max_scroll_offset;
                    }

                    let content_width = content_size.width + padding.left + padding.right;
                    let is_scrolled_horizontally =
                        can_scroll_horizontally && !scroll_offset.x.is_zero();
                    if is_scrolled_horizontally && content_width <= padded_bounds.size.width {
                        shared_scroll_offset.borrow_mut().x = Pixels::ZERO;
                        scroll_offset.x = Pixels::ZERO;
                    }

                    if let Some(DeferredScrollToItem {
                        mut item_index,
                        mut strategy,
                        offset,
                        scroll_strict,
                    }) = shared_scroll_to_item
                    {
                        if y_flipped {
                            item_index =
                                self.item_count.saturating_sub(item_index.saturating_add(1));
                        }
                        let list_height = padded_bounds.size.height;
                        let mut updated_scroll_offset = shared_scroll_offset.borrow_mut();
                        let item_top = item_height * item_index;
                        let item_bottom = item_top + item_height;
                        let scroll_top = -updated_scroll_offset.y;
                        let offset_pixels = item_height * offset;

                        // is the selected item above/below currently visible items
                        let is_above = item_top < scroll_top + offset_pixels;
                        let is_below = item_bottom > scroll_top + list_height;

                        if scroll_strict || is_above || is_below {
                            if strategy == ScrollStrategy::Nearest {
                                if is_above {
                                    strategy = ScrollStrategy::Top;
                                } else if is_below {
                                    strategy = ScrollStrategy::Bottom;
                                }
                            }

                            let max_scroll_offset =
                                (content_height - list_height).max(Pixels::ZERO);
                            match strategy {
                                ScrollStrategy::Top => {
                                    updated_scroll_offset.y = -(item_top - offset_pixels)
                                        .clamp(Pixels::ZERO, max_scroll_offset);
                                }
                                ScrollStrategy::Center => {
                                    let item_center = item_top + item_height / 2.0;

                                    let viewport_height = list_height - offset_pixels;
                                    let viewport_center = offset_pixels + viewport_height / 2.0;
                                    let target_scroll_top = item_center - viewport_center;
                                    updated_scroll_offset.y =
                                        -target_scroll_top.clamp(Pixels::ZERO, max_scroll_offset);
                                }
                                ScrollStrategy::Bottom => {
                                    updated_scroll_offset.y = -(item_bottom - list_height)
                                        .clamp(Pixels::ZERO, max_scroll_offset);
                                }
                                ScrollStrategy::Nearest => {
                                    // Nearest, but the item is visible -> no scroll is required
                                }
                            }
                        }
                        scroll_offset = *updated_scroll_offset
                    }

                    let first_visible_element_ix =
                        (-(scroll_offset.y + padding.top) / item_height).floor() as usize;
                    let last_visible_element_ix = ((-scroll_offset.y + padded_bounds.size.height)
                        / item_height)
                        .ceil() as usize;

                    let visible_range = first_visible_element_ix
                        ..cmp::min(last_visible_element_ix, self.item_count);

                    let items = if y_flipped {
                        let flipped_range = self.item_count.saturating_sub(visible_range.end)
                            ..self.item_count.saturating_sub(visible_range.start);
                        let mut items = (self.render_items)(flipped_range, window, cx);
                        items.reverse();
                        items
                    } else {
                        (self.render_items)(visible_range.clone(), window, cx)
                    };

                    window.with_content_mask(
                        Some(style.scroll_mask(bounds, window.rem_size())),
                        |window| {
                            for (mut item, ix) in items.into_iter().zip(visible_range.clone()) {
                                let item_origin = padded_bounds.origin
                                    + scroll_offset
                                    + point(Pixels::ZERO, item_height * ix);

                                let available_width = if can_scroll_horizontally {
                                    padded_bounds.size.width + scroll_offset.x.abs()
                                } else {
                                    padded_bounds.size.width
                                };
                                let available_space = size(
                                    AvailableSpace::Definite(available_width),
                                    AvailableSpace::Definite(item_height),
                                );
                                item.layout_as_root(available_space, window, cx);
                                item.prepaint_at(item_origin, window, cx);
                                frame_state.items.push(item);
                            }

                            let bounds = Bounds::new(
                                padded_bounds.origin + scroll_offset,
                                padded_bounds.size,
                            );
                            for decoration in &self.decorations {
                                let mut decoration = decoration.as_ref().compute(
                                    visible_range.clone(),
                                    bounds,
                                    scroll_offset,
                                    item_height,
                                    self.item_count,
                                    window,
                                    cx,
                                );
                                let available_space = size(
                                    AvailableSpace::Definite(bounds.size.width),
                                    AvailableSpace::Definite(bounds.size.height),
                                );
                                decoration.layout_as_root(available_space, window, cx);
                                decoration.prepaint_at(bounds.origin, window, cx);
                                frame_state.decorations.push(decoration);
                            }
                        },
                    );
                }

                hitbox
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_, window, cx| {
                for item in &mut request_layout.items {
                    item.paint(window, cx);
                }
                for decoration in &mut request_layout.decorations {
                    decoration.paint(window, cx);
                }
            },
        )
    }
}

impl IntoElement for UniformList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// An element drawn over the rows of a [`UniformList`], such as indent guides.
///
/// Add one with [`UniformList::with_decoration`]. The list computes it each frame, after the rows.
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// # use std::{cell::Cell, ops::Range, rc::Rc};
/// struct CountCalls(Rc<Cell<usize>>);
///
/// impl UniformListDecoration for CountCalls {
///     fn compute(
///         &self,
///         _visible_range: Range<usize>,
///         _bounds: Bounds<Pixels>,
///         _scroll_offset: Point<Pixels>,
///         _item_height: Pixels,
///         _item_count: usize,
///         _window: &mut Window,
///         _cx: &mut App,
///     ) -> AnyElement {
///         self.0.set(self.0.get() + 1);
///         div().into_any_element()
///     }
/// }
///
/// struct Rows {
///     calls: Rc<Cell<usize>>,
///     scroll: UniformListScrollHandle,
/// }
///
/// impl Render for Rows {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         uniform_list("rows", 100, |range, _window, _cx| {
///             range
///                 .map(|ix| div().id(ix).h(px(32.)).child(format!("Row {ix}")))
///                 .collect::<Vec<_>>()
///         })
///         .track_scroll(&self.scroll)
///         .with_decoration(CountCalls(self.calls.clone()))
///         .size_full()
///     }
/// }
/// # let mut app = TestApp::new();
/// # let calls = Rc::new(Cell::new(0));
/// # let mut window = app.open_window(|_, _| Rows {
/// #     calls: calls.clone(),
/// #     scroll: UniformListScrollHandle::new(),
/// # });
/// window.draw();
/// assert!(calls.get() > 0);
/// ```
pub trait UniformListDecoration {
    /// Returns the element to draw, given the rows in view and the list's geometry.
    ///
    /// `visible_range` holds the rows in view. `bounds` has the viewport's size, moved by
    /// `scroll_offset`, so that row `ix` is at `bounds.origin + point(0, item_height * ix)`.
    /// `scroll_offset` is zero or negative, and `item_count` is the number of rows. The element
    /// is laid out at `bounds`.
    fn compute(
        &self,
        visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        item_height: Pixels,
        item_count: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement;
}

impl<T: UniformListDecoration + 'static> UniformListDecoration for Entity<T> {
    fn compute(
        &self,
        visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        item_height: Pixels,
        item_count: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.update(cx, |inner, cx| {
            inner.compute(
                visible_range,
                bounds,
                scroll_offset,
                item_height,
                item_count,
                window,
                cx,
            )
        })
    }
}

impl UniformList {
    /// Chooses the row that sets the width and height of every row. `None` measures row 0.
    ///
    /// An index past the last row measures the last row. The list measures the chosen row every
    /// frame, so the row should be cheap to render.
    pub fn with_width_from_item(mut self, item_index: Option<usize>) -> Self {
        self.item_to_measure_index = item_index.unwrap_or(0);
        self
    }

    /// Sets how the list sizes itself, like [`List::with_sizing_behavior`](crate::List::with_sizing_behavior).
    ///
    /// [`ListSizingBehavior::Auto`], the default, takes the size from the style.
    /// [`ListSizingBehavior::Infer`] makes the list as tall as all its rows, up to the space its
    /// parent gives it.
    pub fn with_sizing_behavior(mut self, behavior: ListSizingBehavior) -> Self {
        self.sizing_behavior = behavior;
        self
    }

    /// Sets whether rows can be wider than the list.
    ///
    /// With [`ListHorizontalSizingBehavior::FitList`], the default, rows are no wider than the list.
    /// With [`ListHorizontalSizingBehavior::Unconstrained`], the content is as wide as the row
    /// chosen by [`Self::with_width_from_item`], and the list scrolls horizontally when that is
    /// wider than the list.
    pub fn with_horizontal_sizing_behavior(
        mut self,
        behavior: ListHorizontalSizingBehavior,
    ) -> Self {
        self.horizontal_sizing_behavior = behavior;
        match behavior {
            ListHorizontalSizingBehavior::FitList => {
                self.interactivity.base_style.overflow.x = None;
            }
            ListHorizontalSizingBehavior::Unconstrained => {
                self.interactivity.base_style.overflow.x = Some(Overflow::Scroll);
            }
        }
        self
    }

    /// Adds a decoration that is drawn over the rows. Decorations are drawn in the order added.
    pub fn with_decoration(mut self, decoration: impl UniformListDecoration + 'static) -> Self {
        self.decorations.push(Box::new(decoration));
        self
    }

    fn measure_item(
        &self,
        list_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        if self.item_count == 0 {
            return Size::default();
        }

        let item_ix = cmp::min(self.item_to_measure_index, self.item_count - 1);
        let mut items = (self.render_items)(item_ix..item_ix + 1, window, cx);
        let Some(mut item_to_measure) = items.pop() else {
            return Size::default();
        };
        let available_space = size(
            list_width.map_or(AvailableSpace::MaxContent, |width| {
                AvailableSpace::Definite(width)
            }),
            AvailableSpace::MinContent,
        );
        item_to_measure.layout_as_root(available_space, window, cx)
    }

    /// Connects the list to `handle`, so the list reads scroll requests from it and writes its
    /// scroll position to it.
    ///
    /// Without this, the list still scrolls with the wheel, but nothing outside it can read or
    /// set the position.
    pub fn track_scroll(mut self, handle: &UniformListScrollHandle) -> Self {
        self.interactivity.tracked_scroll_handle = Some(handle.0.borrow().base_handle.clone());
        self.scroll_handle = Some(handle.clone());
        self
    }

    /// Puts item 0 at the bottom of the list, so the first row sits at the bottom and the last at
    /// the top.
    ///
    /// The list keeps its scroll position on the same rows when the value changes. This only takes
    /// effect when a handle is set with [`Self::track_scroll`]. Call it after `track_scroll`,
    /// because it reads the handle that `track_scroll` sets.
    pub fn y_flipped(mut self, y_flipped: bool) -> Self {
        if let Some(ref scroll_handle) = self.scroll_handle {
            let mut scroll_state = scroll_handle.0.borrow_mut();
            let mut base_handle = &scroll_state.base_handle;
            let offset = base_handle.offset();
            match scroll_state.last_item_size {
                Some(last_size) if scroll_state.y_flipped != y_flipped => {
                    let new_y_offset =
                        -(offset.y + last_size.contents.height - last_size.item.height);
                    base_handle.set_offset(point(offset.x, new_y_offset));
                    scroll_state.y_flipped = y_flipped;
                }
                // Handle case where list is initially flipped.
                None if y_flipped => {
                    base_handle.set_offset(point(offset.x, Pixels::MIN));
                    scroll_state.y_flipped = y_flipped;
                }
                _ => {}
            }
        }
        self
    }
}

impl InteractiveElement for UniformList {
    fn interactivity(&mut self) -> &mut crate::Interactivity {
        &mut self.interactivity
    }
}

#[cfg(test)]
mod test {
    use crate::TestAppContext;

    #[gpui::test]
    fn scroll_to_bottom_in_flipped_list_does_not_overflow(cx: &mut TestAppContext) {
        use crate::{
            AppContext as _, Context, IntoElement, Render, Styled, UniformListScrollHandle, Window,
            div, px, size, uniform_list,
        };

        struct FlippedRows(UniformListScrollHandle);
        impl Render for FlippedRows {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                uniform_list("rows", 47, |range, _, _| {
                    range.map(|_| div().h(px(20.))).collect()
                })
                .track_scroll(&self.0)
                .y_flipped(true)
                .size_full()
            }
        }

        let handle = UniformListScrollHandle::new();
        let window = cx.open_window(size(px(200.), px(200.)), |_, _| FlippedRows(handle.clone()));
        let mut draw = || {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw();
        handle.scroll_to_bottom();
        draw();
        assert_eq!(handle.0.borrow().base_handle.offset().y, px(0.));
        assert!(handle.0.borrow().deferred_scroll_to_item.is_none());
    }

    #[gpui::test]
    fn test_scroll_strategy_nearest(cx: &mut TestAppContext) {
        use crate::{
            Context, FocusHandle, ScrollStrategy, UniformListScrollHandle, Window, div, prelude::*,
            px, uniform_list,
        };
        use std::ops::Range;

        actions!(example, [SelectNext, SelectPrev]);

        struct TestView {
            index: usize,
            length: usize,
            scroll_handle: UniformListScrollHandle,
            focus_handle: FocusHandle,
            visible_range: Range<usize>,
        }

        impl TestView {
            pub fn select_next(
                &mut self,
                _: &SelectNext,
                window: &mut Window,
                _: &mut Context<Self>,
            ) {
                if self.index + 1 == self.length {
                    self.index = 0
                } else {
                    self.index += 1;
                }
                self.scroll_handle
                    .scroll_to_item(self.index, ScrollStrategy::Nearest);
                window.refresh();
            }

            pub fn select_previous(
                &mut self,
                _: &SelectPrev,
                window: &mut Window,
                _: &mut Context<Self>,
            ) {
                if self.index == 0 {
                    self.index = self.length - 1
                } else {
                    self.index -= 1;
                }
                self.scroll_handle
                    .scroll_to_item(self.index, ScrollStrategy::Nearest);
                window.refresh();
            }
        }

        impl Render for TestView {
            fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .id("list-example")
                    .track_focus(&self.focus_handle)
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::select_previous))
                    .size_full()
                    .child(
                        uniform_list(
                            "entries",
                            self.length,
                            cx.processor(|this, range: Range<usize>, _window, _cx| {
                                this.visible_range = range.clone();
                                range
                                    .map(|ix| div().id(ix).h(px(20.0)).child(format!("Item {ix}")))
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll_handle)
                        .h(px(200.0)),
                    )
            }
        }

        let (view, cx) = cx.add_window_view(|window, cx| {
            let focus_handle = cx.focus_handle();
            window.focus(&focus_handle, cx);
            TestView {
                scroll_handle: UniformListScrollHandle::new(),
                index: 0,
                focus_handle,
                length: 47,
                visible_range: 0..0,
            }
        });

        // 10 out of 47 items are visible

        // First 9 times selecting next item does not scroll
        for ix in 1..10 {
            cx.dispatch_action(SelectNext);
            view.read_with(cx, |view, _| {
                assert_eq!(view.index, ix);
                assert_eq!(view.visible_range, 0..10);
            })
        }

        // Now each time the list scrolls down by 1
        for ix in 10..47 {
            cx.dispatch_action(SelectNext);
            view.read_with(cx, |view, _| {
                assert_eq!(view.index, ix);
                assert_eq!(view.visible_range, ix - 9..ix + 1);
            })
        }

        // After the last item we move back to the start
        cx.dispatch_action(SelectNext);
        view.read_with(cx, |view, _| {
            assert_eq!(view.index, 0);
            assert_eq!(view.visible_range, 0..10);
        });

        // Return to the last element
        cx.dispatch_action(SelectPrev);
        view.read_with(cx, |view, _| {
            assert_eq!(view.index, 46);
            assert_eq!(view.visible_range, 37..47);
        });

        // First 9 times selecting previous does not scroll
        for ix in (37..46).rev() {
            cx.dispatch_action(SelectPrev);
            view.read_with(cx, |view, _| {
                assert_eq!(view.index, ix);
                assert_eq!(view.visible_range, 37..47);
            })
        }

        // Now each time the list scrolls up by 1
        for ix in (0..37).rev() {
            cx.dispatch_action(SelectPrev);
            view.read_with(cx, |view, _| {
                assert_eq!(view.index, ix);
                assert_eq!(view.visible_range, ix..ix + 10);
            })
        }
    }
}
