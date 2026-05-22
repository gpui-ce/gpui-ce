use crate::elements::smooth_scroll::SmoothScrollState;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Div, Element, ElementId, Entity,
    GlobalElementId, Hitbox, InteractiveElement, IntoElement, Pixels, Render, ScrollHandle, Size,
    Stateful, StatefulInteractiveElement, Styled, Window, div, point, px, size,
};
use std::{cell::RefCell, cmp, ops::Range, rc::Rc};

use smallvec::SmallVec;

/// A deferred request to scroll the virtual list to a specific item.
#[derive(Clone, Copy, Debug)]
pub struct DeferredScroll {
    /// The index of the item to scroll to.
    pub item_index: usize,
}

/// Shared scroll and animation state for a [`VirtualList`].
#[derive(Debug, Default)]
pub struct VirtualListScrollState {
    /// Deferred scroll request consumed during prepaint.
    pub deferred_scroll: Option<DeferredScroll>,

    /// Smooth scrolling animation state.
    pub smooth_scroll: SmoothScrollState,
}

/// Controller for programmatically scrolling a [`VirtualList`].
#[derive(Clone)]
pub struct VirtualListScrollController {
    /// Shared scroll state.
    pub state: Rc<RefCell<VirtualListScrollState>>,
}

impl VirtualListScrollController {
    /// Creates a new virtual list scroll controller.
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(VirtualListScrollState::default())),
        }
    }

    /// Scrolls the list so the specified item becomes visible.
    pub fn scroll_to_item(&self, item_index: usize) {
        self.state.borrow_mut().deferred_scroll = Some(DeferredScroll { item_index });
    }
}

/// A virtualized scrollable list supporting variable-height items.
///
/// Only visible items and a configurable overscan region are rendered,
/// making this suitable for very large datasets.
#[allow(clippy::type_complexity)]
pub struct VirtualList {
    id: ElementId,
    base: Stateful<Div>,
    scroll_handle: ScrollHandle,

    heights: Rc<Vec<Pixels>>,
    offsets: Vec<Pixels>,
    content_height: Pixels,

    scroll_state: Rc<RefCell<VirtualListScrollState>>,

    render: Box<
        dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> SmallVec<[AnyElement; 32]>,
    >,

    overscan: usize,
}

/// Creates a new virtualized list.
///
/// The provided `heights` vector defines the height of every item in the list.
/// Visible items are lazily rendered using the provided callback.
pub fn vlist<R, V>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    heights: Rc<Vec<Pixels>>,
    scroll_handle: ScrollHandle,
    controller: VirtualListScrollController,
    f: impl 'static + Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) -> Vec<R>,
) -> VirtualList
where
    R: IntoElement,
    V: Render,
{
    let id = id.into();

    let render = move |range: Range<usize>, window: &mut Window, cx: &mut App| {
        view.update(cx, |this, cx| {
            f(this, range, window, cx)
                .into_iter()
                .map(gpui::IntoElement::into_any_element)
                .collect()
        })
    };

    let mut offsets = Vec::with_capacity(heights.len());

    let mut sum = px(0.0);

    for h in heights.iter() {
        offsets.push(sum);
        sum += *h;
    }

    let base = div()
        .id(id.clone())
        .size_full()
        .overflow_scroll()
        .track_scroll(&scroll_handle);

    VirtualList {
        id,
        base,
        scroll_handle,
        heights,
        offsets,
        content_height: sum,
        scroll_state: controller.state.clone(),
        render: Box::new(render),
        overscan: 16,
    }
}

impl VirtualList {
    fn find_index(&self, pos: Pixels) -> usize {
        self.offsets
            .partition_point(|&o| o <= pos)
            .saturating_sub(1)
    }
}

/// Per-frame render state for [`VirtualList`].
pub struct FrameState {
    items: SmallVec<[AnyElement; 32]>,
}

impl IntoElement for VirtualList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for VirtualList {
    type RequestLayoutState = FrameState;

    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let layout_id = self.base.interactivity().request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );

        (
            layout_id,
            FrameState {
                items: SmallVec::new(),
            },
        )
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let viewport_height = bounds.size.height;

        let mut logical_scroll = self.scroll_handle.offset().y;

        if let Some(deferred) = self.scroll_state.borrow_mut().deferred_scroll.take() {
            let target = deferred.item_index;

            if target < self.offsets.len() {
                let item_top = self.offsets[target];

                let centered = item_top - (viewport_height / 2.0) + (self.heights[target] / 2.0);

                let new_scroll = -centered.max(px(0.0));

                self.scroll_handle.set_offset(point(px(0.0), new_scroll));

                logical_scroll = new_scroll;
            }
        }

        let max_scroll = (self.content_height - viewport_height).max(px(0.0));

        logical_scroll = logical_scroll.clamp(-max_scroll, px(0.0));

        self.scroll_handle
            .set_offset(point(px(0.0), logical_scroll));

        let visual_scroll = {
            let mut state = self.scroll_state.borrow_mut();

            state.smooth_scroll.set_target(logical_scroll);

            if state.smooth_scroll.update() {
                window.refresh();
            }

            state.smooth_scroll.current()
        };

        let mut start = self.find_index(-visual_scroll);

        let mut end = self.find_index(-visual_scroll + viewport_height) + 1;

        start = start.saturating_sub(self.overscan);

        end = cmp::min(end + self.overscan + 1, self.heights.len());

        let visible = start..end;

        let items = (self.render)(visible.clone(), window, cx);

        let content_mask = ContentMask { bounds };

        window.with_content_mask(Some(content_mask), |window| {
            for (mut item, ix) in items.into_iter().zip(visible.clone()) {
                let y = self.offsets[ix] + visual_scroll;

                let origin = bounds.origin + point(px(0.0), y);

                let available = size(
                    gpui::AvailableSpace::Definite(bounds.size.width),
                    gpui::AvailableSpace::Definite(self.heights[ix]),
                );

                item.layout_as_root(available, window, cx);

                item.prepaint_at(origin, window, cx);

                layout.items.push(item);
            }
        });

        self.base.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            Size {
                width: bounds.size.width,
                height: self.content_height,
            },
            window,
            cx,
            |_style, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.base.interactivity().paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_, window, cx| {
                for item in &mut layout.items {
                    item.paint(window, cx);
                }
            },
        )
    }
}
