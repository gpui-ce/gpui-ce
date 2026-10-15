//! Lists of rows with any height, rendered lazily.
//!
//! [`list`] lays out and paints only rows in the viewport, plus the overdraw set with
//! [`ListState::new`] (unless you call [`ListState::measure_all`]). It measures rows as it renders
//! them, so rows can differ in height. If every row is the same height, use
//! [`crate::uniform_list`], which skips per-row measurement.
//!
//! The list's state lives on your view, in a [`ListState`]. Call [`ListState::splice`] or
//! [`ListState::reset`] when rows are added, removed or moved, and [`ListState::remeasure_items`]
//! when a row changes height. A row outside the scrolled area must not change height without that
//! notice.
//!
//! For scrolling behavior and update patterns, see the
//! [lists and scrolling guide](crate::_documentation::guides::lists_and_scrolling).

use crate::{
    AnyElement, App, AppContext, AvailableSpace, Bounds, ContentMask, DispatchPhase, Edges,
    Element, EntityId, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId,
    IntoElement, Overflow, Pixels, Point, ScrollDelta, ScrollWheelEvent, Size, Style,
    StyleRefinement, Styled, Window, point, px, size,
};
use collections::VecDeque;
use refineable::Refineable as _;
use std::{cell::RefCell, ops::Range, rc::Rc};
use sum_tree::{Bias, Dimensions, SumTree};

type RenderItemFn = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static;

/// Creates a list element that renders rows of any height, using the state in `state`.
///
/// `render_item` receives the index of a row and returns its element. It runs only for rows in the
/// viewport or the overdraw, unless the state uses [`ListState::measure_all`], which renders every
/// row. Keep your data on your view and index into it from `render_item`, because row elements are
/// dropped as they scroll away.
///
/// The element sizes itself from its style by default. Use [`List::with_sizing_behavior`] to size it
/// from its rows instead. For rows that are all the same height, use [`crate::uniform_list`].
///
/// # Examples
///
/// ```
/// # use gpui::*;
/// # use std::{cell::Cell, rc::Rc};
/// struct Feed {
///     state: ListState,
///     rendered: Rc<Cell<usize>>,
/// }
///
/// impl Render for Feed {
///     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
///         let rendered = self.rendered.clone();
///         div().h(px(200.)).child(
///             list(self.state.clone(), move |ix, _window, _cx| {
///                 rendered.set(rendered.get() + 1);
///                 div().h(px(50.)).child(format!("Row {ix}")).into_any_element()
///             })
///             .size_full(),
///         )
///     }
/// }
/// # let mut app = TestApp::new();
/// # let rendered = Rc::new(Cell::new(0));
/// # let mut window = app.open_window(|_, _| Feed {
/// #     state: ListState::new(1_000, ListAlignment::Top, px(0.)),
/// #     rendered: rendered.clone(),
/// # });
/// window.draw();
/// // A 200px viewport holds four 50px rows, so only about four rows were rendered.
/// assert!(rendered.get() > 0 && rendered.get() < 20);
///
/// window.update(|feed, _, _| feed.state.splice(1_000..1_000, 1));
/// window.draw();
/// window.read(|feed, _| assert_eq!(feed.state.item_count(), 1_001));
/// ```
pub fn list(
    state: ListState,
    render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> List {
    List {
        state,
        render_item: Box::new(render_item),
        style: StyleRefinement::default(),
        sizing_behavior: ListSizingBehavior::default(),
    }
}

/// The element returned by [`list`]. Style it, then return it from `render`.
pub struct List {
    state: ListState,
    render_item: Box<RenderItemFn>,
    style: StyleRefinement,
    sizing_behavior: ListSizingBehavior,
}

impl List {
    /// Sets whether the list sizes itself from its style or from its rows.
    ///
    /// [`ListSizingBehavior::Auto`], the default, takes the size from the style. With
    /// [`ListSizingBehavior::Infer`], the list is as tall as its rows, up to the space its parent gives it.
    pub fn with_sizing_behavior(mut self, behavior: ListSizingBehavior) -> Self {
        self.sizing_behavior = behavior;
        self
    }
}

/// The state of a [`list`]. Store it on your view and pass a clone to [`list`] each frame.
///
/// The state records each row's height, the scroll position and the follow mode. Call its
/// methods when the rows change, such as [`Self::splice`] or [`Self::remeasure_items`].
/// Clones share the same state.
#[derive(Clone)]
pub struct ListState(Rc<RefCell<StateInner>>);

impl std::fmt::Debug for ListState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ListState")
    }
}

struct StateInner {
    last_layout_bounds: Option<Bounds<Pixels>>,
    last_padding: Option<Edges<Pixels>>,
    items: SumTree<ListItem>,
    logical_scroll_top: Option<ListOffset>,
    alignment: ListAlignment,
    overdraw: Pixels,
    reset: bool,
    #[allow(clippy::type_complexity)]
    scroll_handler: Option<Box<dyn FnMut(&ListScrollEvent, &mut Window, &mut App)>>,
    scrollbar_drag_start_height: Option<Pixels>,
    measuring_behavior: ListMeasuringBehavior,
    pending_scroll: Option<PendingScroll>,
    follow_state: FollowState,
}

/// Deferred scroll adjustment applied after the scroll-top item has been remeasured.
///
/// An absolute pending scroll preserves the same pixel offset into the item, which keeps
/// visible text stable while content is appended to or removed from that item. A
/// proportional pending scroll preserves the same fractional position within the item,
/// which is useful when the whole list is being resized and each item scales similarly.
#[derive(Clone)]
enum PendingScroll {
    /// Preserve the same pixel offset into the item after it is remeasured.
    Absolute { item_ix: usize, offset: Pixels },
    /// Preserve the same fractional offset into the item after it is remeasured.
    Proportional(PendingScrollFraction),
}

/// Keeps track of a fractional scroll position within an item for restoration
/// after remeasurement.
#[derive(Clone)]
struct PendingScrollFraction {
    /// The index of the item to scroll within.
    item_ix: usize,
    /// Fractional offset (0.0 to 1.0) within the item's height.
    fraction: f32,
}

/// Determines how remeasurement preserves the scroll position when the scroll-top item
/// changes height.
enum ScrollAnchor {
    /// Preserve the same pixel offset into the scroll-top item.
    Absolute,
    /// Preserve the same fractional position within the scroll-top item.
    Proportional,
}

/// Whether a [`ListState`] keeps the end of the list in view as rows are added.
///
/// Set it with [`ListState::set_follow_mode`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FollowMode {
    /// Doesn't follow. The position stays where the user or your code leaves it.
    #[default]
    Normal,
    /// Keeps the end in view, and resumes doing so when the user scrolls back to the end.
    Tail,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FollowState {
    #[default]
    Normal,
    Tail {
        is_following: bool,
    },
}

impl FollowState {
    fn is_following(&self) -> bool {
        matches!(self, FollowState::Tail { is_following: true })
    }

    fn has_stopped_following(&self) -> bool {
        matches!(
            self,
            FollowState::Tail {
                is_following: false
            }
        )
    }

    fn start_following(&mut self) {
        if let FollowState::Tail {
            is_following: false,
        } = self
        {
            *self = FollowState::Tail { is_following: true };
        }
    }

    fn stop_following(&mut self) {
        if let FollowState::Tail { is_following: true } = self {
            *self = FollowState::Tail {
                is_following: false,
            };
        }
    }
}

/// Where a [`ListState`] starts: at its first row, or at its last row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListAlignment {
    /// Starts at the first row, as most lists do.
    Top,
    /// Starts at the last row, as a chat log does. The list stays at the end until the user
    /// scrolls up.
    Bottom,
}

/// The state of a [`ListState`] after a wheel or trackpad scroll.
///
/// The callback set by [`ListState::set_scroll_handler`] receives this.
pub struct ListScrollEvent {
    /// The rows in view after this scroll, never past `count`.
    pub visible_range: Range<usize>,

    /// The total number of rows in the list.
    pub count: usize,

    /// Whether the list has an explicit scroll position. A bottom-aligned list that is pinned to
    /// the end reports `false`.
    pub is_scrolled: bool,

    /// Whether the list is following the tail after the scroll.
    pub is_following_tail: bool,
}

/// How a [`list`] or [`crate::UniformList`] sizes itself during layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListSizingBehavior {
    /// Makes the list as tall as its rows, up to the space its parent offers.
    Infer,
    /// Takes the size from the style, like any other element.
    #[default]
    Auto,
}

/// Whether a [`ListState`] measures every row or only the rows it renders.
///
/// Set it with [`ListState::measure_all`]. The default is [`Self::Visible`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListMeasuringBehavior {
    /// Measures every row on the next layout. The `bool` records whether that pass has run for
    /// the current width. Measuring every row is expensive for a large list.
    Measure(bool),
    /// Measures only the rows that are rendered.
    #[default]
    Visible,
}

impl ListMeasuringBehavior {
    fn reset(&mut self) {
        match self {
            ListMeasuringBehavior::Measure(has_measured) => *has_measured = false,
            ListMeasuringBehavior::Visible => {}
        }
    }
}

/// Whether the rows of a [`crate::UniformList`] can be wider than the list.
///
/// Set it with [`crate::UniformList::with_horizontal_sizing_behavior`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListHorizontalSizingBehavior {
    /// Rows are no wider than the list. This is the default.
    #[default]
    FitList,
    /// The content is as wide as the widest row, and the list scrolls horizontally when that is
    /// wider than the list.
    Unconstrained,
}

struct LayoutItemsResponse {
    max_item_width: Pixels,
    scroll_top: ListOffset,
    item_layouts: VecDeque<ItemLayout>,
}

struct ItemLayout {
    index: usize,
    element: AnyElement,
    size: Size<Pixels>,
}

/// The hitbox and laid-out rows that a [`List`] carries from prepaint to paint. You don't construct it.
pub struct ListPrepaintState {
    hitbox: Hitbox,
    layout: LayoutItemsResponse,
}

#[derive(Clone)]
enum ListItem {
    Unmeasured {
        size_hint: Option<Size<Pixels>>,
        focus_handle: Option<FocusHandle>,
    },
    Measured {
        size: Size<Pixels>,
        focus_handle: Option<FocusHandle>,
    },
}

impl ListItem {
    fn size(&self) -> Option<Size<Pixels>> {
        if let ListItem::Measured { size, .. } = self {
            Some(*size)
        } else {
            None
        }
    }

    fn size_hint(&self) -> Option<Size<Pixels>> {
        match self {
            ListItem::Measured { size, .. } => Some(*size),
            ListItem::Unmeasured { size_hint, .. } => *size_hint,
        }
    }

    fn focus_handle(&self) -> Option<FocusHandle> {
        match self {
            ListItem::Unmeasured { focus_handle, .. } | ListItem::Measured { focus_handle, .. } => {
                focus_handle.clone()
            }
        }
    }

    fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            ListItem::Unmeasured { focus_handle, .. } | ListItem::Measured { focus_handle, .. } => {
                focus_handle
                    .as_ref()
                    .is_some_and(|handle| handle.contains_focused(window, cx))
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ListItemSummary {
    count: usize,
    rendered_count: usize,
    unrendered_count: usize,
    height: Pixels,
    has_focus_handles: bool,
    has_unknown_height: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Count(usize);

#[derive(Clone, Debug, Default)]
struct Height(Pixels);

impl ListState {
    /// Creates the state for a list of `item_count` rows, to store on your view.
    ///
    /// `alignment` sets where the list starts: [`ListAlignment::Top`] at the first row, or
    /// [`ListAlignment::Bottom`] at the last row, as a chat log does.
    ///
    /// `overdraw` is extra space above and below the viewport. Rows in that space are rendered and
    /// measured, but not painted, so rows that scroll into view are already measured.
    ///
    /// Rows start unmeasured. An unmeasured row counts as zero pixels tall until it is rendered,
    /// unless it has a height hint from [`Self::with_uniform_item_height`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(3, ListAlignment::Top, px(100.));
    /// assert_eq!(state.item_count(), 3);
    /// ```
    pub fn new(item_count: usize, alignment: ListAlignment, overdraw: Pixels) -> Self {
        let this = Self(Rc::new(RefCell::new(StateInner {
            last_layout_bounds: None,
            last_padding: None,
            items: SumTree::default(),
            logical_scroll_top: None,
            alignment,
            overdraw,
            scroll_handler: None,
            reset: false,
            scrollbar_drag_start_height: None,
            measuring_behavior: ListMeasuringBehavior::default(),
            pending_scroll: None,
            follow_state: FollowState::default(),
        })));
        this.splice(0..0, item_count);
        this
    }

    /// Measures every row on the next layout, instead of only the rows that are rendered.
    ///
    /// The scrollbar then reflects the full content height from the first frame. The cost is one
    /// render and measure per row, which is slow for a large list. For rows of similar height,
    /// [`Self::with_uniform_item_height`] gives a close estimate without measuring. Measuring runs
    /// again after the width changes, and after [`Self::reset`], [`Self::remeasure`] or
    /// [`Self::remeasure_items`].
    pub fn measure_all(self) -> Self {
        self.0.borrow_mut().measuring_behavior = ListMeasuringBehavior::Measure(false);
        self
    }

    /// Gives every unmeasured row a height estimate of `height`, so the scrollbar is the right
    /// size before the rows are rendered.
    ///
    /// Each row's real height replaces the estimate when the row is rendered. This costs less than
    /// [`Self::measure_all`], and suits lists whose rows are close to one height, such as table rows.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(1_000, ListAlignment::Top, px(0.)).with_uniform_item_height(px(50.));
    /// assert_eq!(state.max_offset_for_scrollbar().y, px(50_000.));
    /// ```
    pub fn with_uniform_item_height(self, height: Pixels) -> Self {
        self.apply_uniform_item_height(height);
        self
    }

    /// Replaces every row with `element_count` new, unmeasured rows, and returns the scroll
    /// position to the start.
    ///
    /// The position goes to the first row for [`ListAlignment::Top`] and to the last row for
    /// [`ListAlignment::Bottom`]. Scroll events are dropped until the next paint. To keep the scroll
    /// position, use [`Self::splice`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(3, ListAlignment::Top, px(0.));
    /// state.reset(10);
    /// assert_eq!(state.item_count(), 10);
    /// assert_eq!(state.logical_scroll_top().item_ix, 0);
    /// ```
    pub fn reset(&self, element_count: usize) {
        let old_count = {
            let state = &mut *self.0.borrow_mut();
            state.reset = true;
            state.measuring_behavior.reset();
            state.logical_scroll_top = None;
            state.pending_scroll = None;
            state.scrollbar_drag_start_height = None;
            state.items.summary().count
        };

        self.splice(0..old_count, element_count);
    }

    /// Like [`Self::reset`], and gives every new row the height estimate `height`.
    ///
    /// See [`Self::with_uniform_item_height`].
    pub fn reset_with_uniform_height(&self, element_count: usize, height: Pixels) {
        self.reset(element_count);
        self.apply_uniform_item_height(height);
    }

    fn apply_uniform_item_height(&self, height: Pixels) {
        let size_hint = Size {
            width: px(0.),
            height,
        };
        let mut state = self.0.borrow_mut();
        let new_items = state
            .items
            .iter()
            .map(|item| ListItem::Unmeasured {
                size_hint: Some(item.size_hint().unwrap_or(size_hint)),
                focus_handle: item.focus_handle(),
            })
            .collect::<Vec<_>>();
        let mut tree = SumTree::default();
        tree.extend(new_items, ());
        state.items = tree;
    }

    /// Marks every row unmeasured, so each is measured again when next rendered.
    ///
    /// Use this when all rows change height together, such as when the font size changes. The
    /// scroll position keeps the same fraction of the row at the top of the viewport. For one row
    /// whose content changed, use [`Self::remeasure_items`].
    pub fn remeasure(&self) {
        let count = self.item_count();
        self.remeasure_items_with_scroll_anchor(0..count, ScrollAnchor::Proportional);
    }

    /// Marks the rows in `range` unmeasured, so they are measured again when next rendered.
    ///
    /// Use this when a row's content changed height but the row is still at the same index, such as
    /// streaming text. The scroll position is kept. If the row at the top of the viewport is in
    /// `range`, the same pixel offset into that row is kept. Unlike [`Self::splice`], the row count
    /// doesn't change.
    pub fn remeasure_items(&self, range: Range<usize>) {
        self.remeasure_items_with_scroll_anchor(range, ScrollAnchor::Absolute);
    }

    fn remeasure_items_with_scroll_anchor(&self, range: Range<usize>, scroll_anchor: ScrollAnchor) {
        let state = &mut *self.0.borrow_mut();

        if let Some(scroll_top) = state.logical_scroll_top {
            if range.contains(&scroll_top.item_ix) {
                state.pending_scroll = match scroll_anchor {
                    ScrollAnchor::Absolute => Some(PendingScroll::Absolute {
                        item_ix: scroll_top.item_ix,
                        offset: scroll_top.offset_in_item,
                    }),
                    ScrollAnchor::Proportional => {
                        // If the scroll-top item falls within the remeasured range,
                        // store a fractional offset so the layout can restore the
                        // proportional scroll position after the item is re-rendered
                        // at its new height.
                        let mut cursor = state.items.cursor::<Count>(());
                        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

                        cursor
                            .item()
                            .and_then(|item| {
                                item.size().map(|size| {
                                    let fraction = if size.height.0 > 0.0 {
                                        (scroll_top.offset_in_item.0 / size.height.0)
                                            .clamp(0.0, 1.0)
                                    } else {
                                        0.0
                                    };

                                    PendingScroll::Proportional(PendingScrollFraction {
                                        item_ix: scroll_top.item_ix,
                                        fraction,
                                    })
                                })
                            })
                            .or_else(|| state.pending_scroll.clone())
                    }
                };
            }
        }

        // Rebuild the tree, replacing items in the range with
        // Unmeasured copies that keep their focus handles.
        let new_items = {
            let mut cursor = state.items.cursor::<Count>(());
            let mut new_items = cursor.slice(&Count(range.start), Bias::Right);
            let invalidated = cursor.slice(&Count(range.end), Bias::Right);
            new_items.extend(
                invalidated.iter().map(|item| ListItem::Unmeasured {
                    size_hint: item.size_hint(),
                    focus_handle: item.focus_handle(),
                }),
                (),
            );
            new_items.append(cursor.suffix(), ());
            new_items
        };
        state.items = new_items;
        state.measuring_behavior.reset();
    }

    /// Returns the number of rows in this list.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(3, ListAlignment::Top, px(0.));
    /// assert_eq!(state.item_count(), 3);
    /// ```
    pub fn item_count(&self) -> usize {
        self.0.borrow().items.summary().count
    }

    /// Returns whether the list is scrolled to its end.
    ///
    /// Returns `None` when the list isn't scrollable, when it hasn't been painted yet, or when a row
    /// has no height yet, because the end isn't known until then. A row has a height once it is
    /// rendered, or once it has a height hint from [`Self::with_uniform_item_height`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(3, ListAlignment::Top, px(0.));
    /// assert_eq!(state.is_scrolled_to_end(), None);
    /// ```
    pub fn is_scrolled_to_end(&self) -> Option<bool> {
        let state = self.0.borrow();
        let bounds = state.last_layout_bounds?;
        let summary = state.items.summary();
        if summary.has_unknown_height {
            return None;
        }
        let padding = state.last_padding.unwrap_or_default();
        let content_height = summary.height + padding.top + padding.bottom;
        let scroll_max = (content_height - bounds.size.height).max(px(0.));
        if scroll_max <= px(0.) {
            return None;
        }
        let scroll_top = state.scroll_top(&state.logical_scroll_top());
        Some(scroll_top >= scroll_max)
    }

    /// Replaces the rows in `old_range` with `count` new, unmeasured rows.
    ///
    /// Call this after you add, remove or move rows. Rows after `old_range` keep their measurements
    /// and shift by the change in count. New rows count as zero pixels tall until they are rendered.
    /// If the row at the top of the viewport is in `old_range`, the list scrolls to the start of the
    /// new rows. Use [`Self::splice_focusable`] to keep a focused row rendered.
    ///
    /// `splice(old_count..old_count, 1)` appends one row, and `splice(0..0, 1)` inserts one at the top.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(3, ListAlignment::Top, px(0.));
    /// state.splice(1..2, 4); // replace the middle row with four new rows
    /// assert_eq!(state.item_count(), 6);
    /// ```
    pub fn splice(&self, old_range: Range<usize>, count: usize) {
        self.splice_focusable(old_range, (0..count).map(|_| None))
    }

    /// Replaces the rows in `old_range` like [`Self::splice`], and gives each new row an optional focus handle.
    ///
    /// Pass `None` for rows that can't take focus. If a row with a focus handle holds focus when it
    /// scrolls out of view, the list keeps rendering it after the visible rows, so keyboard input
    /// still reaches it, until focus moves elsewhere.
    pub fn splice_focusable(
        &self,
        old_range: Range<usize>,
        focus_handles: impl IntoIterator<Item = Option<FocusHandle>>,
    ) {
        let state = &mut *self.0.borrow_mut();

        let mut old_items = state.items.cursor::<Count>(());
        let mut new_items = old_items.slice(&Count(old_range.start), Bias::Right);
        old_items.seek_forward(&Count(old_range.end), Bias::Right);

        let mut spliced_count = 0;
        new_items.extend(
            focus_handles.into_iter().map(|focus_handle| {
                spliced_count += 1;
                ListItem::Unmeasured {
                    size_hint: None,
                    focus_handle,
                }
            }),
            (),
        );
        new_items.append(old_items.suffix(), ());
        drop(old_items);
        state.items = new_items;

        if let Some(ListOffset {
            item_ix,
            offset_in_item,
        }) = state.logical_scroll_top.as_mut()
        {
            if old_range.contains(item_ix) {
                *item_ix = old_range.start;
                *offset_in_item = px(0.);
            } else if old_range.end <= *item_ix {
                *item_ix = *item_ix - (old_range.end - old_range.start) + spliced_count;
            }
        }
    }

    /// Sets a callback that runs after each wheel or trackpad scroll of the list.
    ///
    /// The callback runs on every wheel event the list handles, including events that don't move it.
    /// It does not run for [`Self::scroll_to`], [`Self::scroll_by`], [`Self::scroll_to_reveal_item`]
    /// or scrollbar drags.
    ///
    /// The callback runs while the list's state is borrowed. Use the [`ListScrollEvent`] it receives,
    /// and don't call methods on the [`ListState`] from inside it.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// # use std::{cell::Cell, rc::Rc};
    /// # struct Feed { state: ListState }
    /// # impl Render for Feed {
    /// #     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    /// #         div().h(px(200.)).child(
    /// #             list(self.state.clone(), |ix, _window, _cx| {
    /// #                 div().h(px(50.)).child(format!("Row {ix}")).into_any_element()
    /// #             })
    /// #             .size_full(),
    /// #         )
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_, _| Feed {
    /// #     state: ListState::new(10, ListAlignment::Top, px(0.)).with_uniform_item_height(px(50.)),
    /// # });
    /// let scrolls = Rc::new(Cell::new(0));
    /// let counted = scrolls.clone();
    /// window.update(|feed, _, _| {
    ///     feed.state.set_scroll_handler(move |event, _window, _cx| {
    ///         assert_eq!(event.count, 10);
    ///         counted.set(counted.get() + 1);
    ///     });
    /// });
    /// window.draw();
    /// // Scroll down 100px, which is row 2 of 50px rows.
    /// window.simulate_scroll(point(px(20.), px(20.)), point(px(0.), px(-100.)));
    /// assert_eq!(scrolls.get(), 1);
    /// window.read(|feed, _| assert_eq!(feed.state.logical_scroll_top().item_ix, 2));
    /// ```
    pub fn set_scroll_handler(
        &self,
        handler: impl FnMut(&ListScrollEvent, &mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().scroll_handler = Some(Box::new(handler))
    }

    /// Returns the first visible row, and the pixel offset into that row.
    ///
    /// Before the list is scrolled, this is row 0 for [`ListAlignment::Top`]. For
    /// [`ListAlignment::Bottom`] it is the end of the list, the item count, with an offset of zero.
    /// Pass a value you saved to [`Self::scroll_to`] to restore a position.
    pub fn logical_scroll_top(&self) -> ListOffset {
        self.0.borrow().logical_scroll_top()
    }

    /// Scrolls the list by `distance` pixels: toward the end when positive, and toward the start when negative.
    ///
    /// The start is clamped at zero. Past the end, the next paint clamps the position. A scroll
    /// toward the start stops following the tail. This doesn't call the scroll handler, and it
    /// doesn't redraw, so call `cx.notify()` to show the new position.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// # use std::rc::Rc;
    /// # struct Feed { state: ListState, rows: Rc<Vec<usize>> }
    /// # impl Render for Feed {
    /// #     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    /// #         let rows = self.rows.clone();
    /// #         div().h(px(200.)).child(
    /// #             list(self.state.clone(), move |ix, _window, _cx| {
    /// #                 div().h(px(50.)).child(format!("Row {}", rows[ix])).into_any_element()
    /// #             })
    /// #             .size_full(),
    /// #         )
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_, _| Feed {
    /// #     state: ListState::new(10, ListAlignment::Top, px(0.)),
    /// #     rows: Rc::new((0..10).collect()),
    /// # });
    /// window.draw();
    /// window.update(|feed, _, cx| {
    ///     feed.state.scroll_by(px(120.));
    ///     cx.notify();
    /// });
    /// window.draw();
    /// // 120px down is row 2, 20px into it.
    /// window.read(|feed, _| {
    ///     let top = feed.state.logical_scroll_top();
    ///     assert_eq!((top.item_ix, top.offset_in_item), (2, px(20.)));
    /// });
    /// ```
    pub fn scroll_by(&self, distance: Pixels) {
        if distance == px(0.) {
            return;
        }

        let current_offset = self.logical_scroll_top();
        let state = &mut *self.0.borrow_mut();

        if distance < px(0.) {
            state.follow_state.stop_following();
        }

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(current_offset.item_ix), Bias::Right);

        let start_pixel_offset = cursor.start().height + current_offset.offset_in_item;
        let new_pixel_offset = (start_pixel_offset + distance).max(px(0.));
        if new_pixel_offset > start_pixel_offset {
            cursor.seek_forward(&Height(new_pixel_offset), Bias::Right);
        } else {
            cursor.seek(&Height(new_pixel_offset), Bias::Right);
        }

        let scroll_top = ListOffset {
            item_ix: cursor.start().count,
            offset_in_item: new_pixel_offset - cursor.start().height,
        };
        drop(cursor);
        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    /// Scrolls past the last row, so the bottom of the list stays in view while the last row grows.
    ///
    /// Unlike [`Self::scroll_to_reveal_item`], this anchors to the end of the list, not to a row. The
    /// next paint walks back from the end and shows the bottom of the last row, even while that row
    /// is still growing, as when text streams in. The scroll takes effect on the next paint.
    pub fn scroll_to_end(&self) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        state.pending_scroll = None;
        state.logical_scroll_top = Some(ListOffset {
            item_ix: item_count,
            offset_in_item: px(0.),
        });
    }

    /// Sets whether the list keeps the end in view as rows are added.
    ///
    /// [`FollowMode::Tail`] scrolls to the end now, and keeps it there on each paint while
    /// [`Self::is_following_tail`] is true. A user scroll toward the start stops following, and the
    /// list starts following again when the user scrolls back to the end. [`FollowMode::Normal`]
    /// stops following and leaves the position where it is.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(5, ListAlignment::Top, px(0.));
    /// state.set_follow_mode(FollowMode::Tail);
    /// assert!(state.is_following_tail());
    /// state.set_follow_mode(FollowMode::Normal);
    /// assert!(!state.is_following_tail());
    /// ```
    pub fn set_follow_mode(&self, mode: FollowMode) {
        let state = &mut *self.0.borrow_mut();

        match mode {
            FollowMode::Normal => {
                state.follow_state = FollowState::Normal;
            }
            FollowMode::Tail => {
                state.follow_state = FollowState::Tail { is_following: true };
                if matches!(mode, FollowMode::Tail) {
                    let item_count = state.items.summary().count;
                    state.logical_scroll_top = Some(ListOffset {
                        item_ix: item_count,
                        offset_in_item: px(0.),
                    });
                }
            }
        }
    }

    /// Stops following the tail and keeps the current position, until the user scrolls back to the end.
    ///
    /// Unlike [`Self::set_follow_mode`] with [`FollowMode::Normal`], the list stays in
    /// [`FollowMode::Tail`], so it resumes following when the view returns to the end. Does nothing
    /// if the list isn't following.
    ///
    /// Use this when something other than the user changes a row's height, such as zooming a
    /// diagram, and the position should not jump to the end.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(5, ListAlignment::Top, px(0.));
    /// state.set_follow_mode(FollowMode::Tail);
    /// state.pause_following_tail();
    /// assert!(!state.is_following_tail());
    /// ```
    pub fn pause_following_tail(&self) {
        self.0.borrow_mut().follow_state.stop_following();
    }

    /// Returns whether the list is following the tail right now.
    ///
    /// This is true after [`Self::set_follow_mode`] with [`FollowMode::Tail`], until the user scrolls
    /// toward the start or [`Self::pause_following_tail`] is called. It is false in
    /// [`FollowMode::Normal`].
    pub fn is_following_tail(&self) -> bool {
        matches!(
            self.0.borrow().follow_state,
            FollowState::Tail { is_following: true }
        )
    }

    /// Scrolls the list so that `scroll_top` is at the top of the viewport.
    ///
    /// If `scroll_top` is past the last row, the list scrolls to the end. A position before the end
    /// stops following the tail.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// let state = ListState::new(5, ListAlignment::Top, px(0.));
    /// state.scroll_to(ListOffset { item_ix: 2, offset_in_item: px(10.) });
    /// assert_eq!(state.logical_scroll_top().item_ix, 2);
    /// assert_eq!(state.logical_scroll_top().offset_in_item, px(10.));
    /// ```
    pub fn scroll_to(&self, mut scroll_top: ListOffset) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        if scroll_top.item_ix >= item_count {
            scroll_top.item_ix = item_count;
            scroll_top.offset_in_item = px(0.);
        }

        if scroll_top.item_ix < item_count {
            state.follow_state.stop_following();
        }

        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    /// Scrolls the least amount that shows row `ix` fully.
    ///
    /// A row above the current top is moved to the top of the viewport. A row below it is moved so
    /// its bottom edge sits at the bottom of the viewport. The position takes effect on the next paint.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::*;
    /// # use std::rc::Rc;
    /// # struct Feed { state: ListState, rows: Rc<Vec<usize>> }
    /// # impl Render for Feed {
    /// #     fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    /// #         let rows = self.rows.clone();
    /// #         div().h(px(200.)).child(
    /// #             list(self.state.clone(), move |ix, _window, _cx| {
    /// #                 div().h(px(50.)).child(format!("Row {}", rows[ix])).into_any_element()
    /// #             })
    /// #             .size_full(),
    /// #         )
    /// #     }
    /// # }
    /// # let mut app = TestApp::new();
    /// # let mut window = app.open_window(|_, _| Feed {
    /// #     state: ListState::new(10, ListAlignment::Top, px(0.)).with_uniform_item_height(px(50.)),
    /// #     rows: Rc::new((0..10).collect()),
    /// # });
    /// window.draw();
    /// window.update(|feed, _, _| feed.state.scroll_to_reveal_item(9));
    /// window.draw();
    /// window.read(|feed, _| {
    ///     assert_eq!(feed.state.item_is_above_viewport(9), Some(false));
    ///     assert_eq!(feed.state.item_is_below_viewport(9), Some(false));
    /// });
    /// ```
    pub fn scroll_to_reveal_item(&self, ix: usize) {
        let state = &mut *self.0.borrow_mut();

        let mut scroll_top = state.logical_scroll_top();
        let height = state
            .last_layout_bounds
            .map_or(px(0.), |bounds| bounds.size.height);
        let padding = state.last_padding.unwrap_or_default();

        if ix <= scroll_top.item_ix {
            scroll_top.item_ix = ix;
            scroll_top.offset_in_item = px(0.);
        } else {
            let mut cursor = state.items.cursor::<ListItemSummary>(());
            cursor.seek(&Count(ix + 1), Bias::Right);
            let bottom = cursor.start().height + padding.top;
            let goal_top = px(0.).max(bottom - height + padding.bottom);

            cursor.seek(&Height(goal_top), Bias::Left);
            let start_ix = cursor.start().count;
            let start_item_top = cursor.start().height;

            if start_ix >= scroll_top.item_ix {
                scroll_top.item_ix = start_ix;
                scroll_top.offset_in_item = goal_top - start_item_top;
            }
        }

        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    /// Returns the bounds of row `ix` in window coordinates, or `None` if the row is above the scroll
    /// position or has not been measured.
    ///
    /// A measured row below the viewport still has bounds, so compare them with
    /// [`Self::viewport_bounds`]. [`Self::item_is_above_viewport`] and
    /// [`Self::item_is_below_viewport`] answer that directly.
    pub fn bounds_for_item(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let state = &*self.0.borrow();

        let bounds = state.last_layout_bounds.unwrap_or_default();
        let scroll_top = state.logical_scroll_top();
        if ix < scroll_top.item_ix {
            return None;
        }

        let mut cursor = state.items.cursor::<Dimensions<Count, Height>>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

        let scroll_top = cursor.start().1.0 + scroll_top.offset_in_item;

        cursor.seek_forward(&Count(ix), Bias::Right);
        if let Some(&ListItem::Measured { size, .. }) = cursor.item() {
            let &Dimensions(Count(count), Height(top), _) = cursor.start();
            if count == ix {
                let top = bounds.top() + top - scroll_top;
                return Some(Bounds::from_corners(
                    point(bounds.left(), top),
                    point(bounds.right(), top + size.height),
                ));
            }
        }
        None
    }

    /// Tells the list that the user started dragging its scrollbar.
    ///
    /// Until [`Self::scrollbar_drag_ended`], the scrollbar uses the content height from this moment,
    /// so the thumb doesn't move while rows in the overdraw are measured.
    pub fn scrollbar_drag_started(&self) {
        let mut state = self.0.borrow_mut();
        state.scrollbar_drag_start_height = Some(state.items.summary().height);
    }

    /// Tells the list that the scrollbar drag has ended, so the scrollbar uses the live content
    /// height again. See [`Self::scrollbar_drag_started`].
    pub fn scrollbar_drag_ended(&self) {
        self.0.borrow_mut().scrollbar_drag_start_height.take();
    }

    /// Returns whether the scrollbar is being dragged.
    ///
    /// This is true between [`Self::scrollbar_drag_started`] and [`Self::scrollbar_drag_ended`].
    /// Use it to tell a scrollbar drag from a wheel or trackpad scroll, for example to stop
    /// auto-scrolling during a manual drag.
    pub fn is_scrollbar_dragging(&self) -> bool {
        self.0.borrow().scrollbar_drag_start_height.is_some()
    }

    /// Scrolls the list to the position that a scrollbar reports.
    ///
    /// `point.y` is a negative scroll offset, as [`Self::scroll_px_offset_for_scrollbar`] returns it.
    /// It is clamped to the content. A drag to the end while the list is in [`FollowMode::Tail`]
    /// resumes following the tail, and any other position stops it. Does nothing before the first paint.
    pub fn set_offset_from_scrollbar(&self, point: Point<Pixels>) {
        self.0.borrow_mut().set_offset_from_scrollbar(point);
    }

    /// Returns the largest scroll distance, as a positive `y`.
    ///
    /// This is the content height minus the viewport height, using the heights measured so far and
    /// any height hints. During a scrollbar drag it keeps the value from when the drag started.
    pub fn max_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = self.0.borrow();
        point(Pixels::ZERO, state.max_scroll_offset())
    }

    /// Returns the current scroll position for the scrollbar, as a negative `y`.
    ///
    /// The value is the negation of how far the content has moved, so it is the negative of
    /// [`Self::max_offset_for_scrollbar`] for a bottom-aligned list that is pinned to the end.
    pub fn scroll_px_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = &self.0.borrow();

        if state.logical_scroll_top.is_none() && state.alignment == ListAlignment::Bottom {
            return Point::new(px(0.), -state.max_scroll_offset());
        }

        let logical_scroll_top = state.logical_scroll_top();

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        let summary: ListItemSummary =
            cursor.summary(&Count(logical_scroll_top.item_ix), Bias::Right);
        let offset = summary.height + logical_scroll_top.offset_in_item;

        Point::new(px(0.), -offset)
    }

    /// Returns the viewport's bounds in window coordinates, from the last prepaint.
    ///
    /// Before the first prepaint this returns empty bounds at the origin.
    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().last_layout_bounds.unwrap_or_default()
    }

    /// Returns whether row `ix` is entirely above the viewport, or `None` before the first layout.
    ///
    /// Rows before the scroll position are above the viewport (`Some(true)`) even when they were
    /// never measured. Any other row needs bounds, so it returns `None` if it hasn't been measured.
    /// A zero-height viewport still gives an answer, so the result doesn't flicker while sibling UI
    /// resizes the list.
    pub fn item_is_above_viewport(&self, ix: usize) -> Option<bool> {
        let viewport_bounds = self.0.borrow().last_layout_bounds?;

        let scroll_top = self.logical_scroll_top();
        if ix < scroll_top.item_ix {
            // Rows before the logical scroll top have no item bounds, but
            // their position relative to the viewport is known from scroll state.
            return Some(true);
        }

        let item_bounds = self.bounds_for_item(ix)?;
        Some(item_bounds.bottom() <= viewport_bounds.top())
    }

    /// Returns whether row `ix` is entirely below the viewport, or `None` when the answer isn't known yet.
    ///
    /// Rows before the scroll position return `Some(false)`. Other rows follow the rules of
    /// [`Self::item_is_above_viewport`].
    pub fn item_is_below_viewport(&self, ix: usize) -> Option<bool> {
        let viewport_bounds = self.0.borrow().last_layout_bounds?;

        let scroll_top = self.logical_scroll_top();
        if ix < scroll_top.item_ix {
            // Rows before the logical scroll top have no item bounds, but
            // their position relative to the viewport is known from scroll state.
            return Some(false);
        }

        let item_bounds = self.bounds_for_item(ix)?;
        Some(item_bounds.top() >= viewport_bounds.bottom())
    }
}

impl StateInner {
    /// Re-anchor a pending scroll adjustment from a remeasure onto a newly set
    /// scroll position, so it clamps to the remeasured item's new height on
    /// the next layout instead of reverting the scroll.
    fn rebase_pending_scroll(&mut self, scroll_top: ListOffset) {
        let Some(pending) = self.pending_scroll.take() else {
            return;
        };
        if scroll_top.item_ix >= self.items.summary().count {
            return;
        }

        self.pending_scroll = match pending {
            PendingScroll::Absolute { .. } => Some(PendingScroll::Absolute {
                item_ix: scroll_top.item_ix,
                offset: scroll_top.offset_in_item,
            }),
            PendingScroll::Proportional(_) => {
                let mut cursor = self.items.cursor::<Count>(());
                cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
                cursor
                    .item()
                    .and_then(|item| item.size_hint())
                    .filter(|size| size.height.0 > 0.0)
                    .map(|size| {
                        PendingScroll::Proportional(PendingScrollFraction {
                            item_ix: scroll_top.item_ix,
                            fraction: (scroll_top.offset_in_item.0 / size.height.0).clamp(0.0, 1.0),
                        })
                    })
            }
        };
    }

    fn max_scroll_offset(&self) -> Pixels {
        let bounds = self.last_layout_bounds.unwrap_or_default();
        let height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        (height - bounds.size.height).max(px(0.))
    }

    fn visible_range(
        items: &SumTree<ListItem>,
        height: Pixels,
        scroll_top: &ListOffset,
    ) -> Range<usize> {
        let mut cursor = items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
        let start_y = cursor.start().height + scroll_top.offset_in_item;
        cursor.seek_forward(&Height(start_y + height), Bias::Left);
        scroll_top.item_ix..(cursor.start().count + 1).min(items.summary().count)
    }

    fn scroll(
        &mut self,
        scroll_top: &ListOffset,
        height: Pixels,
        delta: Point<Pixels>,
        current_view: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Drop scroll events after a reset, since we can't calculate
        // the new logical scroll top without the item heights
        if self.reset {
            return;
        }

        let padding = self.last_padding.unwrap_or_default();
        let scroll_max =
            (self.items.summary().height + padding.top + padding.bottom - height).max(px(0.));
        let new_scroll_top = (self.scroll_top(scroll_top) - delta.y)
            .max(px(0.))
            .min(scroll_max);

        let (start, ..) =
            self.items
                .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);
        let resulting_scroll_top = ListOffset {
            item_ix: start.count,
            offset_in_item: new_scroll_top - start.height,
        };

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.pending_scroll = None;
            self.logical_scroll_top = None;
        } else {
            // The user's scroll supersedes the position stashed by a
            // remeasure; re-anchor the pending adjustment so it doesn't revert
            // this scroll on the next layout.
            self.rebase_pending_scroll(resulting_scroll_top);
            self.logical_scroll_top = Some(resulting_scroll_top);
        }

        if delta.y > px(0.) {
            self.follow_state.stop_following();
        }

        if let Some(handler) = self.scroll_handler.as_mut() {
            let visible_range = Self::visible_range(&self.items, height, &resulting_scroll_top);
            handler(
                &ListScrollEvent {
                    visible_range,
                    count: self.items.summary().count,
                    is_scrolled: self.logical_scroll_top.is_some(),
                    is_following_tail: matches!(
                        self.follow_state,
                        FollowState::Tail { is_following: true }
                    ),
                },
                window,
                cx,
            );
        }

        cx.notify(current_view);
    }

    fn logical_scroll_top(&self) -> ListOffset {
        self.logical_scroll_top
            .unwrap_or_else(|| match self.alignment {
                ListAlignment::Top => ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                },
                ListAlignment::Bottom => ListOffset {
                    item_ix: self.items.summary().count,
                    offset_in_item: px(0.),
                },
            })
    }

    fn scroll_top(&self, logical_scroll_top: &ListOffset) -> Pixels {
        let (start, ..) = self.items.find::<ListItemSummary, _>(
            (),
            &Count(logical_scroll_top.item_ix),
            Bias::Right,
        );
        start.height + logical_scroll_top.offset_in_item
    }

    fn layout_all_items(
        &mut self,
        available_width: Pixels,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) {
        match &mut self.measuring_behavior {
            ListMeasuringBehavior::Visible => {
                return;
            }
            ListMeasuringBehavior::Measure(has_measured) => {
                if *has_measured {
                    return;
                }
                *has_measured = true;
            }
        }

        let mut cursor = self.items.cursor::<Count>(());
        let available_item_space = size(
            AvailableSpace::Definite(available_width),
            AvailableSpace::MinContent,
        );

        let mut measured_items = Vec::default();

        for (ix, item) in cursor.enumerate() {
            let size = item.size().unwrap_or_else(|| {
                let mut element = render_item(ix, window, cx);
                element.layout_as_root(available_item_space, window, cx)
            });

            measured_items.push(ListItem::Measured {
                size,
                focus_handle: item.focus_handle(),
            });
        }

        self.items = SumTree::from_iter(measured_items, ());
    }

    fn layout_items(
        &mut self,
        available_width: Option<Pixels>,
        available_height: Pixels,
        padding: &Edges<Pixels>,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutItemsResponse {
        let old_items = self.items.clone();
        let mut measured_items = VecDeque::new();
        let mut item_layouts = VecDeque::new();
        let mut rendered_height = padding.top;
        let mut max_item_width = px(0.);
        let mut scroll_top = self.logical_scroll_top();

        if self.follow_state.is_following() {
            scroll_top = ListOffset {
                item_ix: self.items.summary().count,
                offset_in_item: px(0.),
            };
            self.logical_scroll_top = Some(scroll_top);
        }

        let mut rendered_focused_item = false;

        let available_item_space = size(
            available_width.map_or(AvailableSpace::MaxContent, |width| {
                AvailableSpace::Definite(width)
            }),
            AvailableSpace::MinContent,
        );

        let mut cursor = old_items.cursor::<Count>(());

        // Render items after the scroll top, including those in the trailing overdraw
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
        for (ix, item) in cursor.by_ref().enumerate() {
            let visible_height = rendered_height - scroll_top.offset_in_item;
            if visible_height >= available_height + self.overdraw {
                break;
            }

            // Use the previously cached height and focus handle if available
            let mut size = item.size();

            // If we're within the visible area or the height wasn't cached, render and measure the item's element
            if visible_height < available_height || size.is_none() {
                let item_index = scroll_top.item_ix + ix;
                let mut element = render_item(item_index, window, cx);
                let element_size = element.layout_as_root(available_item_space, window, cx);
                size = Some(element_size);

                // If there's a pending scroll adjustment for the scroll-top
                // item, apply it.
                if ix == 0 {
                    if let Some(pending_scroll) = self.pending_scroll.take() {
                        match pending_scroll {
                            PendingScroll::Absolute { item_ix, offset }
                                if item_ix == scroll_top.item_ix =>
                            {
                                scroll_top.offset_in_item = offset.min(element_size.height);
                                self.logical_scroll_top = Some(scroll_top);
                            }
                            PendingScroll::Proportional(pending_scroll)
                                if pending_scroll.item_ix == scroll_top.item_ix =>
                            {
                                // Ensuring proportional scroll position is
                                // maintained after re-measuring.
                                scroll_top.offset_in_item =
                                    Pixels(pending_scroll.fraction * element_size.height.0);
                                self.logical_scroll_top = Some(scroll_top);
                            }
                            _ => {}
                        }
                    }
                }

                if visible_height < available_height {
                    item_layouts.push_back(ItemLayout {
                        index: item_index,
                        element,
                        size: element_size,
                    });
                    if item.contains_focused(window, cx) {
                        rendered_focused_item = true;
                    }
                }
            }

            let size = size.unwrap();
            rendered_height += size.height;
            max_item_width = max_item_width.max(size.width);
            measured_items.push_back(ListItem::Measured {
                size,
                focus_handle: item.focus_handle(),
            });
        }
        rendered_height += padding.bottom;

        // Prepare to start walking upward from the item at the scroll top.
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

        // If the rendered items do not fill the visible region, then adjust
        // the scroll top upward.
        if rendered_height - scroll_top.offset_in_item < available_height {
            while rendered_height < available_height {
                cursor.prev();
                if let Some(item) = cursor.item() {
                    let item_index = cursor.start().0;
                    let mut element = render_item(item_index, window, cx);
                    let element_size = element.layout_as_root(available_item_space, window, cx);
                    let focus_handle = item.focus_handle();
                    rendered_height += element_size.height;
                    measured_items.push_front(ListItem::Measured {
                        size: element_size,
                        focus_handle,
                    });
                    item_layouts.push_front(ItemLayout {
                        index: item_index,
                        element,
                        size: element_size,
                    });
                    if item.contains_focused(window, cx) {
                        rendered_focused_item = true;
                    }
                } else {
                    break;
                }
            }

            scroll_top = ListOffset {
                item_ix: cursor.start().0,
                offset_in_item: rendered_height - available_height,
            };

            match self.alignment {
                ListAlignment::Top => {
                    scroll_top.offset_in_item = scroll_top.offset_in_item.max(px(0.));
                    self.logical_scroll_top = Some(scroll_top);
                }
                ListAlignment::Bottom => {
                    scroll_top = ListOffset {
                        item_ix: cursor.start().0,
                        offset_in_item: rendered_height - available_height,
                    };
                    self.logical_scroll_top = None;
                }
            };
        }

        // Measure items in the leading overdraw
        let mut leading_overdraw = scroll_top.offset_in_item;
        while leading_overdraw < self.overdraw {
            cursor.prev();
            if let Some(item) = cursor.item() {
                let size = if let ListItem::Measured { size, .. } = item {
                    *size
                } else {
                    let mut element = render_item(cursor.start().0, window, cx);
                    element.layout_as_root(available_item_space, window, cx)
                };

                leading_overdraw += size.height;
                measured_items.push_front(ListItem::Measured {
                    size,
                    focus_handle: item.focus_handle(),
                });
            } else {
                break;
            }
        }

        let measured_range = cursor.start().0..(cursor.start().0 + measured_items.len());
        let mut cursor = old_items.cursor::<Count>(());
        let mut new_items = cursor.slice(&Count(measured_range.start), Bias::Right);
        new_items.extend(measured_items, ());
        cursor.seek(&Count(measured_range.end), Bias::Right);
        new_items.append(cursor.suffix(), ());
        self.items = new_items;

        // If follow_tail mode is on but the user scrolled away
        // (is_following is false), check whether the current scroll
        // position has returned to the bottom.
        if self.follow_state.has_stopped_following() {
            let padding = self.last_padding.unwrap_or_default();
            let total_height = self.items.summary().height + padding.top + padding.bottom;
            let scroll_offset = self.scroll_top(&scroll_top);
            if scroll_offset + available_height >= total_height - px(1.0) {
                self.follow_state.start_following();
            }
        }

        // If none of the visible items are focused, check if an off-screen item is focused
        // and include it to be rendered after the visible items so keyboard interaction continues
        // to work for it.
        if !rendered_focused_item {
            let mut cursor = self
                .items
                .filter::<_, Count>((), |summary| summary.has_focus_handles);
            cursor.next();
            while let Some(item) = cursor.item() {
                if item.contains_focused(window, cx) {
                    let item_index = cursor.start().0;
                    let mut element = render_item(cursor.start().0, window, cx);
                    let size = element.layout_as_root(available_item_space, window, cx);
                    item_layouts.push_back(ItemLayout {
                        index: item_index,
                        element,
                        size,
                    });
                    break;
                }
                cursor.next();
            }
        }

        LayoutItemsResponse {
            max_item_width,
            scroll_top,
            item_layouts,
        }
    }

    fn prepaint_items(
        &mut self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        content_mask: ContentMask<Pixels>,
        autoscroll: bool,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<LayoutItemsResponse, ListOffset> {
        window.transact(|window| {
            match self.measuring_behavior {
                ListMeasuringBehavior::Measure(has_measured) if !has_measured => {
                    self.layout_all_items(bounds.size.width, render_item, window, cx);
                }
                _ => {}
            }

            let mut layout_response = self.layout_items(
                Some(bounds.size.width),
                bounds.size.height,
                &padding,
                render_item,
                window,
                cx,
            );

            // Avoid honoring autoscroll requests from elements other than our children.
            window.take_autoscroll();

            // Only paint the visible items, if there is actually any space for them (taking padding into account)
            if bounds.size.height > padding.top + padding.bottom {
                let mut item_origin = bounds.origin + Point::new(px(0.), padding.top);
                item_origin.y -= layout_response.scroll_top.offset_in_item;
                for item in &mut layout_response.item_layouts {
                    window.with_content_mask(Some(content_mask), |window| {
                        item.element.prepaint_at(item_origin, window, cx);
                    });

                    if let Some(autoscroll_bounds) = window.take_autoscroll()
                        && autoscroll
                    {
                        if autoscroll_bounds.top() < bounds.top() {
                            let mut item_ix = item.index;
                            let mut offset_in_item = autoscroll_bounds.top() - item_origin.y;

                            // The requested top can sit above this item's own
                            // top. Walk into earlier items so the offset stays
                            // non-negative and no blank space appears above the
                            // list.
                            if offset_in_item < Pixels::ZERO {
                                let mut cursor = self.items.cursor::<Count>(());
                                cursor.seek(&Count(item_ix), Bias::Right);
                                while offset_in_item < Pixels::ZERO {
                                    cursor.prev();
                                    let Some(prev_item) = cursor.item() else {
                                        offset_in_item = Pixels::ZERO;
                                        break;
                                    };
                                    let size = prev_item.size().unwrap_or_else(|| {
                                        let mut element = render_item(cursor.start().0, window, cx);
                                        let item_available_size = size(
                                            bounds.size.width.into(),
                                            AvailableSpace::MinContent,
                                        );
                                        element.layout_as_root(item_available_size, window, cx)
                                    });
                                    item_ix = cursor.start().0;
                                    offset_in_item += size.height;
                                }
                            }

                            return Err(ListOffset {
                                item_ix,
                                offset_in_item,
                            });
                        } else if autoscroll_bounds.bottom() > bounds.bottom() {
                            let mut cursor = self.items.cursor::<Count>(());
                            cursor.seek(&Count(item.index), Bias::Right);
                            let mut height = bounds.size.height - padding.top - padding.bottom;

                            // Account for the height of the element down until the autoscroll bottom.
                            height -= autoscroll_bounds.bottom() - item_origin.y;

                            // Keep decreasing the scroll top until we fill all the available space.
                            while height > Pixels::ZERO {
                                cursor.prev();
                                let Some(item) = cursor.item() else { break };

                                let size = item.size().unwrap_or_else(|| {
                                    let mut item = render_item(cursor.start().0, window, cx);
                                    let item_available_size =
                                        size(bounds.size.width.into(), AvailableSpace::MinContent);
                                    item.layout_as_root(item_available_size, window, cx)
                                });
                                height -= size.height;
                            }

                            return Err(ListOffset {
                                item_ix: cursor.start().0,
                                offset_in_item: if height < Pixels::ZERO {
                                    -height
                                } else {
                                    Pixels::ZERO
                                },
                            });
                        }
                    }

                    item_origin.y += item.size.height;
                }
            } else {
                layout_response.item_layouts.clear();
            }

            Ok(layout_response)
        })
    }

    // Scrollbar support

    fn set_offset_from_scrollbar(&mut self, point: Point<Pixels>) {
        let Some(bounds) = self.last_layout_bounds else {
            return;
        };
        let height = bounds.size.height;

        let padding = self.last_padding.unwrap_or_default();
        // Scrollbar drag positions are computed from the content height
        // captured at drag start, so map them back using the same height.
        let content_height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        let scroll_max = (content_height + padding.top + padding.bottom - height).max(px(0.));
        let new_scroll_top = (-point.y).max(px(0.)).min(scroll_max);

        // If content grew during the drag, the frozen bottom is below the
        // live bottom. Treat dragging to the frozen end as resuming tail follow.
        let dragged_to_end =
            scroll_max > px(0.) && new_scroll_top >= (scroll_max - px(1.0)).max(px(0.));
        if dragged_to_end && matches!(self.follow_state, FollowState::Tail { .. }) {
            self.follow_state = FollowState::Tail { is_following: true };
            let item_count = self.items.summary().count;
            self.pending_scroll = None;
            self.logical_scroll_top = Some(ListOffset {
                item_ix: item_count,
                offset_in_item: px(0.),
            });
            return;
        }

        self.follow_state.stop_following();

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.pending_scroll = None;
            self.logical_scroll_top = None;
        } else {
            let (start, _, _) =
                self.items
                    .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);

            let scroll_top = ListOffset {
                item_ix: start.count,
                offset_in_item: new_scroll_top - start.height,
            };
            self.rebase_pending_scroll(scroll_top);
            self.logical_scroll_top = Some(scroll_top);
        }
    }
}

impl std::fmt::Debug for ListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unmeasured { .. } => write!(f, "Unrendered"),
            Self::Measured { size, .. } => f.debug_struct("Rendered").field("size", size).finish(),
        }
    }
}

/// A position in a list: a row index and a pixel offset into that row.
///
/// [`ListState::logical_scroll_top`] returns one. Pass one to [`ListState::scroll_to`] to restore a position.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListOffset {
    /// The index of the row at the top of the viewport.
    pub item_ix: usize,
    /// The distance in pixels from the top of that row to the top of the viewport.
    pub offset_in_item: Pixels,
}

impl List {
    /// The list's style with vertical scrolling enabled by default, so the list
    /// clips and masks its items even when no overflow was refined.
    fn base_style(&self) -> Style {
        let mut style = Style::default();
        style.overflow.y = Overflow::Scroll;
        style.refine(&self.style);
        style
    }
}

impl Element for List {
    type RequestLayoutState = ();
    type PrepaintState = ListPrepaintState;

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let layout_id = match self.sizing_behavior {
            ListSizingBehavior::Infer => {
                let style = self.base_style();
                window.with_text_style(style.text_style().cloned(), |window| {
                    let state = &mut *self.state.0.borrow_mut();

                    let available_height = if let Some(last_bounds) = state.last_layout_bounds {
                        last_bounds.size.height
                    } else {
                        // If we don't have the last layout bounds (first render),
                        // we might just use the overdraw value as the available height to layout enough items.
                        state.overdraw
                    };
                    let padding = style.padding.to_pixels(
                        state.last_layout_bounds.unwrap_or_default().size.into(),
                        window.rem_size(),
                    );

                    let layout_response = state.layout_items(
                        None,
                        available_height,
                        &padding,
                        &mut self.render_item,
                        window,
                        cx,
                    );
                    let max_element_width = layout_response.max_item_width;

                    let summary = state.items.summary();
                    let total_height = summary.height;

                    window.request_measured_layout(
                        style,
                        move |known_dimensions, available_space, _window, _cx| {
                            let width =
                                known_dimensions
                                    .width
                                    .unwrap_or(match available_space.width {
                                        AvailableSpace::Definite(x) => x,
                                        AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                            max_element_width
                                        }
                                    });
                            let height = match available_space.height {
                                AvailableSpace::Definite(height) => total_height.min(height),
                                AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                    total_height
                                }
                            };
                            size(width, height)
                        },
                    )
                })
            }
            ListSizingBehavior::Auto => {
                let mut style = Style::default();
                style.refine(&self.style);
                window.with_text_style(style.text_style().cloned(), |window| {
                    window.request_layout(style, None, cx)
                })
            }
        };
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> ListPrepaintState {
        let state = &mut *self.state.0.borrow_mut();
        state.reset = false;

        let style = self.base_style();
        let content_mask = style.scroll_mask(bounds, window.rem_size());

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        // If the width of the list has changed, invalidate all cached item heights. A measured
        // height depends on the old width, but a hint given up front, such as the one from
        // `with_uniform_item_height`, doesn't, so unmeasured rows keep theirs.
        if state
            .last_layout_bounds
            .is_none_or(|last_bounds| last_bounds.size.width != bounds.size.width)
        {
            let new_items = SumTree::from_iter(
                state.items.iter().map(|item| ListItem::Unmeasured {
                    size_hint: match item {
                        ListItem::Unmeasured { size_hint, .. } => *size_hint,
                        ListItem::Measured { .. } => None,
                    },
                    focus_handle: item.focus_handle(),
                }),
                (),
            );

            state.items = new_items;
            state.measuring_behavior.reset();
        }

        let padding = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());
        let layout = match state.prepaint_items(
            bounds,
            padding,
            content_mask,
            true,
            &mut self.render_item,
            window,
            cx,
        ) {
            Ok(layout) => layout,
            Err(autoscroll_request) => {
                state.logical_scroll_top = Some(autoscroll_request);
                state
                    .prepaint_items(
                        bounds,
                        padding,
                        content_mask,
                        false,
                        &mut self.render_item,
                        window,
                        cx,
                    )
                    .unwrap()
            }
        };

        state.last_layout_bounds = Some(bounds);
        state.last_padding = Some(padding);
        ListPrepaintState { hitbox, layout }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();

        // Register the scroll listener before painting children so that, in
        // the bubble phase (which runs in reverse registration order),
        // children's scroll-wheel handlers run first and can stop propagation
        // to prevent the list from scrolling. This matches the ordering of
        // div-based scroll containers.
        let list_state = self.state.clone();
        let height = bounds.size.height;
        let scroll_top = prepaint.layout.scroll_top;
        let hitbox_id = prepaint.hitbox.id;
        let mut accumulated_scroll_delta = ScrollDelta::default();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox_id.should_handle_scroll(window) {
                accumulated_scroll_delta = accumulated_scroll_delta.coalesce(event.delta);
                let pixel_delta = accumulated_scroll_delta.pixel_delta(px(20.));
                list_state.0.borrow_mut().scroll(
                    &scroll_top,
                    height,
                    pixel_delta,
                    current_view,
                    window,
                    cx,
                )
            }
        });

        let style = self.base_style();
        window.with_content_mask(
            Some(style.scroll_mask(bounds, window.rem_size())),
            |window| {
                for item in &mut prepaint.layout.item_layouts {
                    item.element.paint(window, cx);
                }
            },
        );
    }
}

impl IntoElement for List {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for List {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl sum_tree::Item for ListItem {
    type Summary = ListItemSummary;

    fn summary(&self, _: ()) -> Self::Summary {
        match self {
            ListItem::Unmeasured {
                size_hint,
                focus_handle,
            } => ListItemSummary {
                count: 1,
                rendered_count: 0,
                unrendered_count: 1,
                height: if let Some(size) = size_hint {
                    size.height
                } else {
                    px(0.)
                },
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: size_hint.is_none(),
            },
            ListItem::Measured {
                size, focus_handle, ..
            } => ListItemSummary {
                count: 1,
                rendered_count: 1,
                unrendered_count: 0,
                height: size.height,
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: false,
            },
        }
    }
}

impl sum_tree::ContextLessSummary for ListItemSummary {
    fn zero() -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &Self) {
        self.count += summary.count;
        self.rendered_count += summary.rendered_count;
        self.unrendered_count += summary.unrendered_count;
        self.height += summary.height;
        self.has_focus_handles |= summary.has_focus_handles;
        self.has_unknown_height |= summary.has_unknown_height;
    }
}

impl<'a> sum_tree::Dimension<'a, ListItemSummary> for Count {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.count;
    }
}

impl<'a> sum_tree::Dimension<'a, ListItemSummary> for Height {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.height;
    }
}

impl sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Count {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.count).unwrap()
    }
}

impl sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Height {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.height).unwrap()
    }
}

#[cfg(test)]
mod test {

    use gpui::{ScrollDelta, ScrollWheelEvent};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::{
        self as gpui, AppContext, Bounds, Context, Element, FollowMode, InteractiveElement,
        IntoElement, ListState, Render, Styled, TestAppContext, Window, canvas, div, list, point,
        px, size,
    };

    #[gpui::test]
    fn test_autoscroll_above_item_top_renders_items_above(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));
        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |ix, _, _| {
                    if ix == 2 {
                        // Request an autoscroll whose top sits 30px above item 2's
                        // own top, mimicking a scroll-margin overshoot.
                        canvas(
                            |bounds, window, _| {
                                window.request_autoscroll(Bounds::from_corners(
                                    point(bounds.left(), bounds.top() - px(30.)),
                                    point(bounds.right(), bounds.top() + px(5.)),
                                ));
                            },
                            |_, _, _, _| {},
                        )
                        .h(px(20.))
                        .w_full()
                        .into_any()
                    } else {
                        div().h(px(20.)).w_full().into_any()
                    }
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(60.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // 30px above item 2's top, with 20px items, lands 10px into item 0.
        let scroll_top = state.logical_scroll_top();
        assert!(
            scroll_top.offset_in_item >= px(0.),
            "offset_in_item must never be negative (would leave blank space above), got {:?}",
            scroll_top.offset_in_item,
        );
        assert_eq!(scroll_top.item_ix, 0);
        assert_eq!(scroll_top.offset_in_item, px(10.));
    }

    #[gpui::test]
    fn test_reset_after_paint_before_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        // Ensure that the list is scrolled to the top
        state.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(10.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        // Paint
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Reset
        state.reset(5);

        // And then receive a scroll event _before_ the next paint
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(1.), px(1.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-500.))),
            ..Default::default()
        });

        // Scroll position should stay at the top of the list
        assert_eq!(state.logical_scroll_top().item_ix, 0);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_uniform_item_height_survives_the_first_layout(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state =
            ListState::new(10, crate::ListAlignment::Top, px(0.)).with_uniform_item_height(px(50.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        // Only the rows that fit in 200px are measured. The rest keep their hint.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });
        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));

        // Revealing the last row scrolls it to the bottom edge, 300px down.
        state.scroll_to_reveal_item(9);
        let top = state.logical_scroll_top();
        assert_eq!(px(50. * top.item_ix as f32) + top.offset_in_item, px(300.));
    }

    #[gpui::test]
    fn test_scroll_by_positive_and_negative_distance(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(20.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        // Paint
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Test positive distance: start at item 1, move down 30px
        state.scroll_by(px(30.));

        // Should move to item 2
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 1);
        assert_eq!(offset.offset_in_item, px(10.));

        // Test negative distance: start at item 2, move up 30px
        state.scroll_by(px(-30.));

        // Should move back to item 1
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));

        // Test zero distance
        state.scroll_by(px(0.));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_scroll_handler_reports_the_resulting_visible_range(cx: &mut TestAppContext) {
        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(20.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        for alignment in [crate::ListAlignment::Top, crate::ListAlignment::Bottom] {
            let cx = cx.add_empty_window();
            let state = ListState::new(5, alignment, px(0.)).measure_all();
            state.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            let reported = Rc::new(RefCell::new(Vec::new()));
            state.set_scroll_handler({
                let reported = reported.clone();
                move |event, _, _| {
                    reported.borrow_mut().push((
                        event.visible_range.clone(),
                        event.count,
                        event.is_scrolled,
                    ));
                }
            });
            cx.draw(point(px(0.), px(0.)), size(px(100.), px(40.)), |_, cx| {
                cx.new(|_| TestView(state.clone())).into_any_element()
            });

            // Scroll two items down, then past the end of the list.
            for distance in [-40., -1000.] {
                cx.simulate_event(ScrollWheelEvent {
                    position: point(px(50.), px(10.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(distance))),
                    ..Default::default()
                });
            }

            assert_eq!(
                &*reported.borrow(),
                &[
                    (2..4, 5, true),
                    (3..5, 5, alignment == crate::ListAlignment::Top),
                ],
                "{alignment:?}: callbacks should report the viewport after the scroll"
            );
        }
    }

    #[gpui::test]
    fn test_child_scroll_handler_can_stop_list_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));
        let child_saw_event = Rc::new(Cell::new(false));

        struct TestView {
            state: ListState,
            child_saw_event: Rc<Cell<bool>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let child_saw_event = self.child_saw_event.clone();
                list(self.state.clone(), move |_, _, _| {
                    let child_saw_event = child_saw_event.clone();
                    div()
                        .h(px(20.))
                        .w_full()
                        .on_scroll_wheel(move |_, _, cx| {
                            child_saw_event.set(true);
                            cx.stop_propagation();
                        })
                        .into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                child_saw_event: child_saw_event.clone(),
            })
            .into_any_element()
        });

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(10.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        assert!(
            child_saw_event.get(),
            "the child's scroll-wheel handler should run"
        );
        // The child stopped propagation, so the list must not have scrolled.
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    struct TestListView(ListState);
    impl Render for TestListView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            list(self.0.clone(), |_, _, _| {
                div().h(px(20.)).w_full().into_any()
            })
            .w_full()
            .h_full()
        }
    }

    #[gpui::test]
    fn test_item_viewport_queries_return_none_before_layout(_cx: &mut TestAppContext) {
        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        assert_eq!(state.item_is_above_viewport(0), None);
        assert_eq!(state.item_is_below_viewport(0), None);
    }

    #[gpui::test]
    fn test_item_viewport_queries_before_logical_scroll_top(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(1), Some(true));
        assert_eq!(state.item_is_below_viewport(1), Some(false));
    }

    #[gpui::test]
    fn test_item_viewport_queries_measured_item_inside_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(2), Some(false));
        assert_eq!(state.item_is_below_viewport(2), Some(false));
    }

    #[gpui::test]
    fn test_item_viewport_queries_measured_item_above_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(20.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(2), Some(true));
        assert_eq!(state.item_is_below_viewport(2), Some(false));
    }

    #[gpui::test]
    fn test_item_viewport_queries_measured_item_below_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));
    }

    #[gpui::test]
    fn test_item_viewport_queries_remain_stable_with_zero_height_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));

        // Squeeze the list to zero height, e.g. because a sibling element
        // (sized based on the queries above) consumed all the space. The
        // answers must remain definitive rather than becoming `None`,
        // otherwise the sibling's size can oscillate between frames.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(0.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(1), Some(true));
        assert_eq!(state.item_is_below_viewport(1), Some(false));
        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));
    }

    #[gpui::test]
    fn test_item_viewport_queries_after_scroll_to_end_before_layout(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        state.scroll_to_end();

        assert_eq!(state.logical_scroll_top().item_ix, state.item_count());
        assert_eq!(state.item_is_above_viewport(0), Some(true));
        assert_eq!(state.item_is_below_viewport(0), Some(false));
    }

    #[gpui::test]
    fn test_measure_all_after_width_change(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        // First draw at width 100: all 10 items measured (total 500px).
        // Viewport is 200px, so max scroll offset should be 300px.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));

        // Second draw at a different width: items get invalidated.
        // Without the fix, max_offset would drop because unmeasured items
        // contribute 0 height.
        cx.draw(point(px(0.), px(0.)), size(px(200.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));
    }

    #[gpui::test]
    fn test_remeasure(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // Create a list with 10 items, each 100px tall. We'll keep a reference
        // to the item height so we can later change the height and assert how
        // `ListState` handles it.
        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        // Simulate scrolling 40px inside the element with index 2. Since the
        // original item height is 100px, this equates to 40% inside the item.
        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(40.));

        // Update the `item_height` to be 50px instead of 100px so we can assert
        // that the scroll position is proportionally preserved, that is,
        // instead of 40px from the top of item 2, it should be 20px, since the
        // item's height has been halved.
        item_height.set(50);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(20.));
    }

    #[gpui::test]
    fn test_remeasure_item_preserves_scroll_offset(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 5 { height } else { 100 };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.scroll_to(gpui::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        item_height.set(200);
        state.remeasure_items(5..6);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(40.));
    }

    #[gpui::test]
    fn test_remeasure_then_scroll_does_not_revert_scroll_position(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(100.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = {
            let state = state.clone();
            cx.update(|_, cx| cx.new(|_| TestView(state)))
        };

        state.scroll_to(gpui::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.remeasure_items(5..6);

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(70.));

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(
            offset.offset_in_item,
            px(70.),
            "scrolling after a remeasure should not be reverted by the stale pending scroll"
        );
    }

    #[gpui::test]
    fn test_scroll_after_remeasure_clamps_to_shrunk_item_height(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 5 { height } else { 100 };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = {
            let state = state.clone();
            let item_height = item_height.clone();
            cx.update(|_, cx| cx.new(|_| TestView { state, item_height }))
        };

        state.scroll_to(gpui::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        // Item 5 shrinks from 100px to 50px and is remeasured...
        item_height.set(50);
        state.remeasure_items(5..6);

        // ...and then the user scrolls down by 30px before the next frame,
        // landing at offset 70.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        // The rebased pending scroll clamps the user's offset to the item's
        // new height instead of leaving it pointing past the end of the item.
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(50.));
    }

    #[gpui::test]
    fn test_follow_tail_stays_at_bottom_as_items_grow(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items, each 50px tall → 500px total content, 200px viewport.
        // With follow-tail on, the list should always show the bottom.
        let item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.set_follow_mode(FollowMode::Tail);

        // First paint — items are 50px, total 500px, viewport 200px.
        // Follow-tail should anchor to the end.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        // The scroll should be at the bottom: the last visible items fill the
        // 200px viewport from the end of 500px of content (offset 300px).
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());

        // Simulate items growing (e.g. streaming content makes each item taller).
        // 10 items × 80px = 800px total.
        item_height.set(80);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        // After growth, follow-tail should have re-anchored to the new end.
        // 800px total − 200px viewport = 600px offset → item 7 at offset 40px,
        // but follow-tail anchors to item_count (10), and layout walks back to
        // fill 200px, landing at item 7 (7 × 80 = 560, 800 − 560 = 240 > 200,
        // so item 8: 8 × 80 = 640, 800 − 640 = 160 < 200 → keeps walking →
        // item 7: offset = 800 − 200 = 600, item_ix = 600/80 = 7, remainder 40).
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 7);
        assert_eq!(offset.offset_in_item, px(40.));
        assert!(state.is_following_tail());
    }

    #[gpui::test]
    fn test_pause_following_tail_reengages_when_still_at_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));
        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Pausing while the view is still at the bottom (e.g. a no-op zoom)
        // must not strand follow-tail: the next layout has to re-engage so the
        // invariant "at the bottom + new content => visible" is preserved.
        state.pause_following_tail();
        assert!(!state.is_following_tail());

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "pausing while at the bottom must re-engage follow-tail on the next layout"
        );
    }

    #[gpui::test]
    fn test_pause_following_tail_freezes_off_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items, 200px viewport. Item height is adjustable to simulate a
        // diagram block growing/shrinking on zoom.
        let item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                item_height: item_height.clone(),
            })
        });
        state.set_follow_mode(FollowMode::Tail);

        // At the bottom: 500px content, 200px viewport → top at item 6.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert_eq!(state.logical_scroll_top().item_ix, 6);
        assert!(state.is_following_tail());

        // Pause, then grow items (a zoom-in that pushes content below the fold).
        // The frozen top must stay put rather than snapping to the new end, and
        // following stays paused since we're no longer at the bottom.
        state.pause_following_tail();
        item_height.set(80);
        state.remeasure();
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(
            !state.is_following_tail(),
            "a paused list must not re-engage while the frozen top is off the bottom"
        );

        // Shrink back (zoom-out) so the frozen top once again reaches the
        // bottom: follow-tail must re-engage on its own.
        item_height.set(50);
        state.remeasure();
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "returning to the bottom must restore follow-tail"
        );
    }

    #[gpui::test]
    fn test_follow_tail_disengages_on_user_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        state.set_follow_mode(FollowMode::Tail);

        // Paint with follow-tail — scroll anchored to the bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });
        assert!(state.is_following_tail());

        // Simulate the user scrolling up.
        // This should disengage follow-tail.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
            ..Default::default()
        });

        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the user scrolls toward the start"
        );
    }

    #[gpui::test]
    fn test_follow_tail_disengages_on_scrollbar_reposition(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        // Paint with follow-tail — scroll anchored to the bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Simulate the scrollbar moving the viewport to the middle.
        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the scrollbar manually repositions the list"
        );

        // A subsequent draw should preserve the user's manual position instead
        // of snapping back to the end.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_scrollbar_drag_with_growing_content(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let last_item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView {
            state: ListState,
            last_item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let last_item_height = self.last_item_height.clone();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 9 {
                        last_item_height.get()
                    } else {
                        50
                    };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                last_item_height: last_item_height.clone(),
            })
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.scrollbar_drag_started();

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let scrollbar_offset_before_growth = state.scroll_px_offset_for_scrollbar();

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));

        last_item_height.set(550);
        state.remeasure_items(9..10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));
        assert_eq!(
            state.scroll_px_offset_for_scrollbar(),
            scrollbar_offset_before_growth
        );

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_set_follow_tail_snaps_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        // Scroll to the middle of the list (item 3).
        state.scroll_to(gpui::ListOffset {
            item_ix: 3,
            offset_in_item: px(0.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(!state.is_following_tail());

        // Enable follow-tail — this should immediately snap the scroll anchor
        // to the end, like the user just sent a prompt.
        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        // After paint, scroll should be at the bottom.
        // 500px total − 200px viewport = 300px offset → item 6, offset 0.
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());
    }

    #[gpui::test]
    fn test_bottom_aligned_scrollbar_offset_at_end(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        const ITEMS: usize = 10;
        const ITEM_SIZE: f32 = 50.0;

        let state = ListState::new(
            ITEMS,
            crate::ListAlignment::Bottom,
            px(ITEMS as f32 * ITEM_SIZE),
        );

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(ITEM_SIZE)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Bottom-aligned lists start pinned to the end: logical_scroll_top returns
        // item_ix == item_count, meaning no explicit scroll position has been set.
        assert_eq!(state.logical_scroll_top().item_ix, ITEMS);

        let max_offset = state.max_offset_for_scrollbar();
        let scroll_offset = state.scroll_px_offset_for_scrollbar();

        assert_eq!(
            -scroll_offset.y, max_offset.y,
            "scrollbar offset ({}) should equal max offset ({}) when list is pinned to bottom",
            -scroll_offset.y, max_offset.y,
        );
    }

    /// When the user scrolls away from the bottom during follow_tail,
    /// follow_tail suspends. If they scroll back to the bottom, the
    /// next paint should re-engage follow_tail using fresh measurements.
    #[gpui::test]
    fn test_follow_tail_reengages_when_scrolled_back_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Scroll up — follow_tail should suspend (not fully disengage).
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(50.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        // Scroll back down to the bottom.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            ..Default::default()
        });

        // After a paint, follow_tail should re-engage because the
        // layout confirmed we're at the true bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom"
        );
    }

    /// When an item is spliced to unmeasured (0px) while follow_tail
    /// is suspended, the re-engagement check should still work correctly
    #[gpui::test]
    fn test_follow_tail_reengagement_not_fooled_by_unmeasured_items(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 20 items × 50px = 1000px total, 200px viewport, 1000px
        // overdraw so all items get measured during the follow_tail
        // paint (matching realistic production settings).
        let state = ListState::new(20, crate::ListAlignment::Top, px(1000.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Scroll up a meaningful amount — suspends follow_tail.
        // 20 items × 50px = 1000px. viewport 200px. scroll_max = 800px.
        // Scrolling up 200px puts us at 600px, clearly not at bottom.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(200.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        // Invalidate the last item (simulates EntryUpdated calling
        // remeasure_items). This makes items.summary().height
        // temporarily wrong (0px for the invalidated item).
        state.remeasure_items(19..20);

        // Paint — layout re-measures the invalidated item with its true
        // height. The re-engagement check uses these fresh measurements.
        // Since we scrolled 200px up from the 800px max, we're at
        // ~600px — NOT at the bottom, so follow_tail should NOT
        // re-engage.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            !state.is_following_tail(),
            "follow_tail should not falsely re-engage due to an unmeasured item \
             reducing items.summary().height"
        );
    }

    #[gpui::test]
    fn test_follow_tail_reengages_after_scrollbar_disengagement(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport, scroll_max = 300px.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Drag the scrollbar up to the middle — follow_tail should suspend.
        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        assert!(!state.is_following_tail());

        // Drag the scrollbar back to the bottom — follow_tail should re-engage
        // on the next paint.
        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom via the scrollbar"
        );
    }

    #[gpui::test]
    fn test_follow_tail_reengages_after_scrollbar_drag_to_bottom_while_growing(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.scrollbar_drag_started();

        state.splice(10..10, 10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        state.scrollbar_drag_ended();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage when the user drags the scrollbar to \
             the bottom of its track, even when content has grown during the drag \
             (so frozen_bottom < live_bottom)"
        );
    }
}
