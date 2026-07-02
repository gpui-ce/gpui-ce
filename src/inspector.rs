/// A unique identifier for an element that can be inspected.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct InspectorElementId {
    /// Stable part of the ID.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub path: std::rc::Rc<InspectorElementPath>,
    /// Disambiguates elements that have the same path.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub instance_id: usize,
}

impl Into<InspectorElementId> for &InspectorElementId {
    fn into(self) -> InspectorElementId {
        self.clone()
    }
}

#[cfg(any(feature = "inspector", debug_assertions))]
pub use conditional::*;

#[cfg(any(feature = "inspector", debug_assertions))]
mod conditional {
    use super::*;
    use crate::px;
    use crate::{
        AnyElement, App, Bounds, Context, Empty, Entity, Focusable, GlobalElementId, IntoElement,
        Pixels, Render, SharedString, Styled, Window,
    };
    use collections::FxHashMap;
    use std::any::{Any, TypeId};

    /// `GlobalElementId` qualified by source location of element construction.
    #[derive(Debug, Eq, PartialEq, Hash)]
    pub struct InspectorElementPath {
        /// The path to the nearest ancestor element that has an `ElementId`.
        pub global_id: crate::GlobalElementId,
        /// Source location where this element was constructed.
        pub source_location: &'static std::panic::Location<'static>,
    }

    impl Clone for InspectorElementPath {
        fn clone(&self) -> Self {
            Self {
                global_id: self.global_id.clone(),
                source_location: self.source_location,
            }
        }
    }

    impl Into<InspectorElementPath> for &InspectorElementPath {
        fn into(self) -> InspectorElementPath {
            self.clone()
        }
    }

    // ── Inspector tabs ───────────────────────────────────────────────────────

    /// Tabs available in the inspector panel, similar to browser devtools.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum InspectorTab {
        /// Hierarchical element tree (like the DOM inspector).
        Elements,
        /// Style properties for the selected element.
        Styles,
        /// Box-model layout visualization.
        Layout,
        /// Registered event listeners on the selected element.
        EventListeners,
    }

    impl InspectorTab {
        /// All tabs in display order.
        pub fn all() -> [InspectorTab; 4] {
            [
                InspectorTab::Elements,
                InspectorTab::Styles,
                InspectorTab::Layout,
                InspectorTab::EventListeners,
            ]
        }

        /// Human-readable label for the tab.
        pub fn label(&self) -> &'static str {
            match self {
                InspectorTab::Elements => "Elements",
                InspectorTab::Styles => "Styles",
                InspectorTab::Layout => "Layout",
                InspectorTab::EventListeners => "Listeners",
            }
        }
    }

    // ── Element tree / info ──────────────────────────────────────────────────

    /// Per-element metadata collected during frame rendering.
    #[derive(Debug, Clone)]
    pub struct InspectorElementInfo {
        /// Unique inspector identifier for this element.
        pub inspector_id: InspectorElementId,
        /// The element's global ID path (used to reconstruct hierarchy).
        pub global_id: GlobalElementId,
        /// Display-friendly element type name (e.g. "div", "text", "img").
        pub element_type: SharedString,
        /// Layout bounds of the element.
        pub bounds: Bounds<Pixels>,
        /// Depth in the element tree (0 = root).
        pub depth: usize,
        /// Source file where the element was constructed.
        pub source_file: SharedString,
        /// Line number in the source file.
        pub source_line: u32,
        /// Registered event listener types.
        pub event_listeners: Vec<InspectorEventListener>,
        /// Active element states (hovered, focused, active, etc.).
        pub element_states: Vec<SharedString>,
        /// Display string for the element (e.g., "div#myid.myclass").
        pub display_label: SharedString,
    }

    /// A node in the inspector element tree.
    #[derive(Debug, Clone)]
    pub struct InspectorTreeNode {
        /// Inspector ID, if this node has one (leaf text nodes may not).
        pub inspector_id: Option<InspectorElementId>,
        /// Display-friendly type name.
        pub element_type: SharedString,
        /// Display label for the element (e.g., "div#header").
        pub display_label: SharedString,
        /// Layout bounds.
        pub bounds: Bounds<Pixels>,
        /// Depth in the tree.
        pub depth: usize,
        /// Source file path.
        pub source_file: SharedString,
        /// Source line number.
        pub source_line: u32,
        /// Child tree nodes.
        pub children: Vec<InspectorTreeNode>,
        /// Whether this node is currently selected.
        pub is_selected: bool,
        /// Whether this node is currently hovered (in picking mode).
        pub is_hovered: bool,
        /// Event listener types.
        pub event_listeners: Vec<InspectorEventListener>,
    }

    /// Describes a registered event listener on an element.
    #[derive(Debug, Clone)]
    pub struct InspectorEventListener {
        /// Event type (e.g., "click", "mousedown", "hover").
        pub event_type: SharedString,
        /// Source location where the listener was attached.
        pub location: SharedString,
    }

    /// Box-model layout info for the selected element.
    #[derive(Debug, Clone)]
    pub struct InspectorLayoutInfo {
        /// Total element bounds.
        pub bounds: Bounds<Pixels>,
        /// Margin edge widths.
        pub margin: EdgeWidths,
        /// Border edge widths.
        pub border: EdgeWidths,
        /// Padding edge widths.
        pub padding: EdgeWidths,
        /// Content area size.
        pub content_size: crate::Size<Pixels>,
    }

    /// Edge widths for margin/border/padding.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct EdgeWidths {
        /// Top edge width.
        pub top: Pixels,
        /// Right edge width.
        pub right: Pixels,
        /// Bottom edge width.
        pub bottom: Pixels,
        /// Left edge width.
        pub left: Pixels,
    }

    // ── Inspector ────────────────────────────────────────────────────────────

    /// Function set on `App` to render the inspector UI.
    pub type InspectorRenderer =
        Box<dyn Fn(&mut Inspector, &mut Window, &mut Context<Inspector>) -> AnyElement>;

    /// Manages inspector state - which element is currently selected, picking mode, tab state,
    /// panel size, and element tree data.
    pub struct Inspector {
        active_element: Option<InspectedElement>,
        pub(crate) pick_depth: Option<f32>,
        active_tab: InspectorTab,
        panel_width: Pixels,
        min_panel_width: Pixels,
        max_panel_width_ratio: f32,
        resizing: bool,
        search_query: SharedString,
        element_tree: Vec<InspectorTreeNode>,
        element_infos: Vec<InspectorElementInfo>,
        collapsed_nodes: FxHashMap<InspectorElementId, bool>,
        active_layout: Option<InspectorLayoutInfo>,
    }

    struct InspectedElement {
        id: InspectorElementId,
        states: FxHashMap<TypeId, Box<dyn Any>>,
    }

    impl InspectedElement {
        fn new(id: InspectorElementId) -> Self {
            InspectedElement {
                id,
                states: FxHashMap::default(),
            }
        }
    }

    impl Inspector {
        /// Minimum panel width in pixels.
        pub const MIN_PANEL_WIDTH: f32 = 200.0;
        /// Default panel width in rems.
        pub const DEFAULT_PANEL_WIDTH_REMS: f32 = 30.0;
        /// Maximum panel width as a fraction of viewport width.
        pub const MAX_PANEL_WIDTH_RATIO: f32 = 0.8;

        pub(crate) fn new() -> Self {
            Self {
                active_element: None,
                pick_depth: Some(0.0),
                active_tab: InspectorTab::Elements,
                panel_width: Pixels::default(),
                min_panel_width: px(Self::MIN_PANEL_WIDTH),
                max_panel_width_ratio: Self::MAX_PANEL_WIDTH_RATIO,
                resizing: false,
                search_query: SharedString::default(),
                element_tree: Vec::new(),
                element_infos: Vec::new(),
                collapsed_nodes: FxHashMap::default(),
                active_layout: None,
            }
        }

        /// Initialize panel width based on rem size (called after construction when rem size is known).
        pub(crate) fn init_panel_width(&mut self, rem_size: Pixels) {
            if self.panel_width.0 == 0.0 {
                self.panel_width = rems(Self::DEFAULT_PANEL_WIDTH_REMS).to_pixels(rem_size);
                self.min_panel_width = px(Self::MIN_PANEL_WIDTH);
            }
        }

        pub(crate) fn select(&mut self, id: InspectorElementId, window: &mut Window) {
            self.set_active_element_id(id, window);
            self.pick_depth = None;
        }

        pub(crate) fn hover(&mut self, id: InspectorElementId, window: &mut Window) {
            if self.is_picking() {
                let changed = self.set_active_element_id(id, window);
                if changed {
                    self.pick_depth = Some(0.0);
                }
            }
        }

        pub(crate) fn set_active_element_id(
            &mut self,
            id: InspectorElementId,
            window: &mut Window,
        ) -> bool {
            let changed = Some(&id) != self.active_element_id();
            if changed {
                self.active_element = Some(InspectedElement::new(id));
                window.refresh();
            }
            changed
        }

        /// ID of the currently hovered or selected element.
        pub fn active_element_id(&self) -> Option<&InspectorElementId> {
            self.active_element.as_ref().map(|e| &e.id)
        }

        pub(crate) fn with_active_element_state<T: 'static, R>(
            &mut self,
            window: &mut Window,
            f: impl FnOnce(&mut Option<T>, &mut Window) -> R,
        ) -> R {
            let Some(active_element) = &mut self.active_element else {
                return f(&mut None, window);
            };

            let type_id = TypeId::of::<T>();
            let mut inspector_state = active_element
                .states
                .remove(&type_id)
                .map(|state| *state.downcast().unwrap());

            let result = f(&mut inspector_state, window);

            if let Some(inspector_state) = inspector_state {
                active_element
                    .states
                    .insert(type_id, Box::new(inspector_state));
            }

            result
        }

        /// Starts element picking mode, allowing the user to select elements by clicking.
        pub fn start_picking(&mut self) {
            self.pick_depth = Some(0.0);
        }

        /// Returns whether the inspector is currently in picking mode.
        pub fn is_picking(&self) -> bool {
            self.pick_depth.is_some()
        }

        // ── Tab management ───────────────────────────────────────────────

        /// Returns the currently active tab.
        pub fn active_tab(&self) -> InspectorTab {
            self.active_tab
        }

        /// Sets the active tab.
        pub fn set_tab(&mut self, tab: InspectorTab) {
            self.active_tab = tab;
        }

        /// Cycle to the next tab.
        pub fn next_tab(&mut self) {
            let tabs = InspectorTab::all();
            let idx = tabs.iter().position(|t| *t == self.active_tab).unwrap_or(0);
            self.active_tab = tabs[(idx + 1) % tabs.len()];
        }

        /// Cycle to the previous tab.
        pub fn previous_tab(&mut self) {
            let tabs = InspectorTab::all();
            let idx = tabs.iter().position(|t| *t == self.active_tab).unwrap_or(0);
            self.active_tab = tabs[(idx + tabs.len() - 1) % tabs.len()];
        }

        // ── Panel resizing ───────────────────────────────────────────────

        /// Current panel width in pixels.
        pub fn panel_width(&self) -> Pixels {
            self.panel_width
        }

        /// Set the panel width, clamped to min/max bounds relative to the viewport.
        pub fn set_panel_width(&mut self, width: Pixels, viewport_width: Pixels) {
            let max_width = viewport_width * self.max_panel_width_ratio;
            self.panel_width = width.clamp(self.min_panel_width, max_width);
        }

        /// Start resize-dragging the panel.
        pub fn start_resizing(&mut self) {
            self.resizing = true;
        }

        /// Stop resize-dragging the panel.
        pub fn stop_resizing(&mut self) {
            self.resizing = false;
        }

        /// Whether the panel is currently being resized.
        pub fn is_resizing(&self) -> bool {
            self.resizing
        }

        // ── Search ───────────────────────────────────────────────────────

        /// Current search/filter query for the element tree.
        pub fn search_query(&self) -> &str {
            &self.search_query
        }

        /// Set the search/filter query.
        pub fn set_search_query(&mut self, query: SharedString) {
            self.search_query = query;
        }

        // ── Element tree ─────────────────────────────────────────────────

        /// The element tree for the current frame.
        pub fn element_tree(&self) -> &[InspectorTreeNode] {
            &self.element_tree
        }

        /// Store per-element info collected during frame rendering and rebuild the tree.
        pub(crate) fn set_element_infos(&mut self, infos: Vec<InspectorElementInfo>) {
            self.element_infos = infos;
            self.build_tree();
        }

        /// Get per-element infos for the current frame.
        pub fn element_infos(&self) -> &[InspectorElementInfo] {
            &self.element_infos
        }

        /// Get the active element's layout info.
        pub fn active_layout(&self) -> Option<&InspectorLayoutInfo> {
            self.active_layout.as_ref()
        }

        /// Set layout info for the active element.
        pub(crate) fn set_active_layout(&mut self, layout: InspectorLayoutInfo) {
            self.active_layout = Some(layout);
        }

        /// Whether a tree node is collapsed.
        pub fn is_collapsed(&self, id: &InspectorElementId) -> bool {
            self.collapsed_nodes.get(id).copied().unwrap_or(false)
        }

        /// Toggle collapse state of a tree node.
        pub fn toggle_collapsed(&mut self, id: InspectorElementId) {
            let entry = self.collapsed_nodes.entry(id).or_insert(false);
            *entry = !*entry;
        }

        /// Collapse all nodes.
        pub fn collapse_all(&mut self) {
            self.collapsed_nodes.clear();
        }

        /// Expand all nodes.
        pub fn expand_all(&mut self) {
            self.collapsed_nodes.clear();
            for info in &self.element_infos {
                self.collapsed_nodes
                    .insert(info.inspector_id.clone(), false);
            }
        }

        fn build_tree(&mut self) {
            self.element_tree.clear();

            if self.element_infos.is_empty() {
                return;
            }

            let mut sorted = self.element_infos.clone();
            sorted.sort_by_key(|info| info.depth);

            let mut root_nodes: Vec<InspectorTreeNode> = Vec::new();
            let selector_id = self.active_element_id().cloned();

            for info in &sorted {
                let node = InspectorTreeNode {
                    inspector_id: Some(info.inspector_id.clone()),
                    element_type: info.element_type.clone(),
                    display_label: info.display_label.clone(),
                    bounds: info.bounds,
                    depth: info.depth,
                    source_file: info.source_file.clone(),
                    source_line: info.source_line,
                    children: Vec::new(),
                    is_selected: selector_id.as_ref() == Some(&info.inspector_id),
                    is_hovered: false,
                    event_listeners: info.event_listeners.clone(),
                };
                root_nodes.push(node);
            }

            self.element_tree = Self::build_tree_from_flat(root_nodes);
        }

        fn build_tree_from_flat(nodes: Vec<InspectorTreeNode>) -> Vec<InspectorTreeNode> {
            let mut roots: Vec<InspectorTreeNode> = Vec::new();
            let mut stack: Vec<(usize, *mut Vec<InspectorTreeNode>)> = Vec::new();

            for node in nodes {
                let depth = node.depth;

                while stack.last().map_or(false, |(d, _)| *d >= depth) {
                    stack.pop();
                }

                if stack.is_empty() {
                    roots.push(node);
                    let roots_ptr = &mut roots as *mut Vec<InspectorTreeNode>;
                    stack.push((depth, roots_ptr));
                } else {
                    let (_, children_ptr) = stack.last().unwrap();
                    unsafe {
                        (*children_ptr).last_mut().unwrap().children.push(node);
                        let last_child = (*children_ptr).last_mut().unwrap();
                        let children_ptr = &mut last_child.children as *mut Vec<InspectorTreeNode>;
                        stack.push((depth, children_ptr));
                    }
                }
            }

            roots
        }

        /// Renders elements for all registered inspector states of the active inspector element.
        pub fn render_inspector_states(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Vec<AnyElement> {
            let mut elements = Vec::new();
            if let Some(active_element) = self.active_element.take() {
                for (type_id, state) in &active_element.states {
                    if let Some(render_inspector) = cx
                        .inspector_element_registry
                        .renderers_by_type_id
                        .remove(type_id)
                    {
                        let mut element = (render_inspector)(
                            active_element.id.clone(),
                            state.as_ref(),
                            window,
                            cx,
                        );
                        elements.push(element);
                        cx.inspector_element_registry
                            .renderers_by_type_id
                            .insert(*type_id, render_inspector);
                    }
                }

                self.active_element = Some(active_element);
            }

            elements
        }
    }

    impl Render for Inspector {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.init_panel_width(window.rem_size());

            if let Some(inspector_renderer) = cx.inspector_renderer.take() {
                let result = inspector_renderer(self, window, cx);
                cx.inspector_renderer = Some(inspector_renderer);
                result
            } else {
                default_inspector_renderer(self, window, cx)
            }
        }
    }

    // ── Built-in default inspector renderer ──────────────────────────────────

    fn default_inspector_renderer(
        inspector: &mut Inspector,
        window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let tab = inspector.active_tab();
        let is_picking = inspector.is_picking();

        crate::div()
            .flex()
            .flex_col()
            .size_full()
            .bg(DEFAULT_BG)
            .text_color(DEFAULT_TEXT)
            .font_family("ui-monospace, monospace".into())
            .child(
                // Resize handle (left edge)
                crate::div()
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .h_full()
                    .w(px(4.))
                    .bg(crate::rgba(0xffffff10))
                    .hover(|style| style.bg(crate::rgba(0x61afefff)))
                    .on_mouse_down(
                        crate::MouseButton::Left,
                        cx.listener(
                            |inspector: &mut Inspector,
                             _event: &crate::MouseDownEvent,
                             _window,
                             _cx| {
                                inspector.start_resizing();
                            },
                        ),
                    ),
            )
            .child(
                // Tab bar
                render_tab_bar(inspector, window, cx),
            )
            .child(
                // Search bar (only for Elements tab)
                crate::div().when(tab == InspectorTab::Elements, |this| {
                    this.child(render_search_bar(inspector, window, cx))
                }),
            )
            .child(
                // Toolbar with action buttons
                render_toolbar(inspector, window, cx),
            )
            .child(
                // Tab content
                crate::div().flex_1().overflow_y_scroll().child(match tab {
                    InspectorTab::Elements => render_elements_tab(inspector, window, cx),
                    InspectorTab::Styles => render_styles_tab(inspector, window, cx),
                    InspectorTab::Layout => render_layout_tab(inspector, window, cx),
                    InspectorTab::EventListeners => {
                        render_event_listeners_tab(inspector, window, cx)
                    }
                }),
            )
            .child(if is_picking {
                render_picking_indicator(inspector, window, cx)
            } else {
                crate::div().into_any_element()
            })
            .into_any_element()
    }

    // ── Default colors ───────────────────────────────────────────────────────

    const DEFAULT_BG: crate::Hsla = crate::rgba(0x1e1e2ecc);
    const DEFAULT_TEXT: crate::Hsla = crate::rgba(0xcdd6f4ff);
    const ACCENT: crate::Hsla = crate::rgba(0x89b4faff);
    const ACCENT_DIM: crate::Hsla = crate::rgba(0x89b4fa33);
    const BORDER: crate::Hsla = crate::rgba(0x313244ff);
    const TAB_ACTIVE_BG: crate::Hsla = crate::rgba(0x313244ff);
    const TAB_INACTIVE_BG: crate::Hsla = crate::rgba(0x1e1e2ecc);
    const DIM_TEXT: crate::Hsla = crate::rgba(0x6c7086ff);
    const HOVER_BG: crate::Hsla = crate::rgba(0x45475a66);
    const SELECTED_BG: crate::Hsla = crate::rgba(0x89b4fa33);
    const MARGIN_COLOR: crate::Hsla = crate::rgba(0xf9e2afcc);
    const BORDER_COLOR: crate::Hsla = crate::rgba(0xfab387cc);
    const PADDING_COLOR: crate::Hsla = crate::rgba(0xa6e3a1cc);
    const CONTENT_COLOR: crate::Hsla = crate::rgba(0x89b4facc);

    // ── Tab bar ──────────────────────────────────────────────────────────────

    fn render_tab_bar(
        inspector: &mut Inspector,
        _window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let active = inspector.active_tab();

        crate::div()
            .flex()
            .flex_row()
            .border_b_1()
            .border_color(BORDER)
            .children(InspectorTab::all().iter().map(|tab| {
                let is_active = *tab == active;
                crate::div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .bg(if is_active {
                        TAB_ACTIVE_BG
                    } else {
                        TAB_INACTIVE_BG
                    })
                    .text_color(if is_active { ACCENT } else { DIM_TEXT })
                    .when(is_active, |this| this.border_b_2().border_color(ACCENT))
                    .on_click({
                        let tab = *tab;
                        cx.listener(
                            move |inspector: &mut Inspector,
                                  _event: &crate::ClickEvent,
                                  _window,
                                  _cx| {
                                inspector.set_tab(tab);
                            },
                        )
                    })
                    .child(tab.label().into())
                    .into_any_element()
            }))
            .into_any_element()
    }

    // ── Search bar ───────────────────────────────────────────────────────────

    fn render_search_bar(
        inspector: &mut Inspector,
        _window: &mut Window,
        _cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let query = inspector.search_query().to_string();

        crate::div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(6.))
            .py(px(4.))
            .border_b_1()
            .border_color(BORDER)
            .bg(TAB_INACTIVE_BG)
            .child(
                crate::div()
                    .text_sm()
                    .text_color(DIM_TEXT)
                    .mr(px(4.))
                    .child(SharedString::from("🔍")),
            )
            .child(
                crate::div()
                    .flex_1()
                    .text_sm()
                    .text_color(if query.is_empty() {
                        DIM_TEXT
                    } else {
                        DEFAULT_TEXT
                    })
                    .child(if query.is_empty() {
                        SharedString::from("Filter elements...")
                    } else {
                        SharedString::from(query.as_str())
                    }),
            )
            .into_any_element()
    }

    // ── Toolbar ──────────────────────────────────────────────────────────────

    fn render_toolbar(
        inspector: &mut Inspector,
        _window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let is_picking = inspector.is_picking();
        let has_selection = inspector.active_element_id().is_some();

        crate::div()
            .flex()
            .flex_row()
            .px(px(6.))
            .py(px(4.))
            .gap(px(4.))
            .border_b_1()
            .border_color(BORDER)
            .child(
                // Pick element button
                crate::div()
                    .px(px(6.))
                    .py(px(2.))
                    .text_xs()
                    .cursor_pointer()
                    .bg(if is_picking {
                        ACCENT_DIM
                    } else {
                        TAB_INACTIVE_BG
                    })
                    .text_color(if is_picking { ACCENT } else { DEFAULT_TEXT })
                    .border_1()
                    .border_color(if is_picking { ACCENT } else { BORDER })
                    .rounded_sm()
                    .on_click(cx.listener(
                        |inspector: &mut Inspector, _: &crate::ClickEvent, _window, _cx| {
                            inspector.start_picking();
                        },
                    ))
                    .child(SharedString::from("Pick")),
            )
            .child(
                // Collapse all button
                crate::div()
                    .px(px(6.))
                    .py(px(2.))
                    .text_xs()
                    .cursor_pointer()
                    .bg(TAB_INACTIVE_BG)
                    .text_color(DEFAULT_TEXT)
                    .border_1()
                    .border_color(BORDER)
                    .rounded_sm()
                    .when(!has_selection, |this| this.opacity(0.5))
                    .on_click(cx.listener(
                        |inspector: &mut Inspector, _: &crate::ClickEvent, _window, _cx| {
                            inspector.collapse_all();
                        },
                    ))
                    .child(SharedString::from("Collapse")),
            )
            .child(
                // Expand all button
                crate::div()
                    .px(px(6.))
                    .py(px(2.))
                    .text_xs()
                    .cursor_pointer()
                    .bg(TAB_INACTIVE_BG)
                    .text_color(DEFAULT_TEXT)
                    .border_1()
                    .border_color(BORDER)
                    .rounded_sm()
                    .when(!has_selection, |this| this.opacity(0.5))
                    .on_click(cx.listener(
                        |inspector: &mut Inspector, _: &crate::ClickEvent, _window, _cx| {
                            inspector.expand_all();
                        },
                    ))
                    .child(SharedString::from("Expand")),
            )
            .into_any_element()
    }

    // ── Elements tab ─────────────────────────────────────────────────────────

    fn render_elements_tab(
        inspector: &mut Inspector,
        _window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let tree = inspector.element_tree().to_vec();
        let selected_id = inspector.active_element_id().cloned();

        crate::div()
            .flex()
            .flex_col()
            .py(px(2.))
            .children(render_tree_nodes_recursive(
                &tree,
                0,
                &selected_id,
                inspector,
                cx,
            ))
            .into_any_element()
    }

    fn render_tree_nodes_recursive(
        nodes: &[InspectorTreeNode],
        depth: usize,
        selected_id: &Option<InspectorElementId>,
        inspector: &mut Inspector,
        cx: &mut Context<Inspector>,
    ) -> Vec<AnyElement> {
        let mut elements = Vec::new();

        for node in nodes {
            let indent = depth * 16;
            let is_selected = node.is_selected;
            let is_collapsed = node
                .inspector_id
                .as_ref()
                .map_or(false, |id| inspector.is_collapsed(id));
            let has_children = !node.children.is_empty();

            let node_id = node.inspector_id.clone();

            // Row for this node
            let row = crate::div()
                .flex()
                .flex_row()
                .items_center()
                .px(px(4.))
                .py(px(1.))
                .pl(px(indent as f32))
                .cursor_pointer()
                .hover(|style| style.bg(HOVER_BG))
                .when(is_selected, |this| {
                    this.bg(SELECTED_BG).border_l_2().border_color(ACCENT)
                })
                .when(!is_selected, |this| {
                    this.border_l_2().border_color(crate::transparent_black())
                })
                .child(
                    // Expand/collapse toggle
                    crate::div()
                        .w(px(14.))
                        .text_xs()
                        .text_color(if has_children {
                            DEFAULT_TEXT
                        } else {
                            crate::transparent_black()
                        })
                        .child(if has_children {
                            if is_collapsed {
                                SharedString::from("▶")
                            } else {
                                SharedString::from("▼")
                            }
                        } else {
                            SharedString::from(" ")
                        }),
                )
                .child(
                    // Element type
                    crate::div()
                        .text_xs()
                        .text_color(ACCENT)
                        .child(node.element_type.clone()),
                )
                .child(
                    // Display label (id, classes)
                    crate::div()
                        .text_xs()
                        .text_color(DIM_TEXT)
                        .ml(px(4.))
                        .child(node.display_label.clone()),
                )
                .child(
                    // Source location
                    crate::div()
                        .text_xs()
                        .text_color(DIM_TEXT)
                        .ml(px(8.))
                        .child(SharedString::from(format!(
                            "{}:{}",
                            node.source_file, node.source_line
                        ))),
                );

            let row = if let Some(id) = node_id {
                row.on_click({
                    let id = id.clone();
                    cx.listener(
                        move |inspector: &mut Inspector,
                              _event: &crate::ClickEvent,
                              _window,
                              _cx| {
                            inspector.set_active_element_id(id.clone(), _window);
                            inspector.stop_resizing();
                        },
                    )
                })
            } else {
                row
            };

            elements.push(row.into_any_element());

            // Render children if not collapsed
            if has_children && !is_collapsed {
                elements.extend(render_tree_nodes_recursive(
                    &node.children,
                    depth + 1,
                    selected_id,
                    inspector,
                    cx,
                ));
            }
        }

        elements
    }

    // ── Styles tab ───────────────────────────────────────────────────────────

    fn render_styles_tab(
        inspector: &mut Inspector,
        window: &mut Window,
        cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let has_selection = inspector.active_element_id().is_some();
        let mut elements = Vec::new();

        if has_selection {
            // Render registered inspector states (style editors, etc.)
            elements.extend(inspector.render_inspector_states(window, cx));

            if elements.is_empty() {
                elements.push(
                    crate::div()
                        .p(px(8.))
                        .text_sm()
                        .text_color(DIM_TEXT)
                        .child(SharedString::from(
                            "No style editors registered for this element type.",
                        ))
                        .into_any_element(),
                );
            }

            // Also show computed bounds info
            if let Some(layout) = inspector.active_layout() {
                elements.push(
                    crate::div()
                        .p(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            crate::div()
                                .text_xs()
                                .text_color(ACCENT)
                                .font_weight(crate::FontWeight::BOLD)
                                .child(SharedString::from("Computed")),
                        )
                        .child(style_property_row(
                            "width",
                            &format!("{:.1}px", layout.bounds.size.width.0),
                        ))
                        .child(style_property_row(
                            "height",
                            &format!("{:.1}px", layout.bounds.size.height.0),
                        ))
                        .child(style_property_row(
                            "x",
                            &format!("{:.1}px", layout.bounds.origin.x.0),
                        ))
                        .child(style_property_row(
                            "y",
                            &format!("{:.1}px", layout.bounds.origin.y.0),
                        ))
                        .child(style_property_row(
                            "margin",
                            &format!(
                                "{:.1} {:.1} {:.1} {:.1}",
                                layout.margin.top.0,
                                layout.margin.right.0,
                                layout.margin.bottom.0,
                                layout.margin.left.0,
                            ),
                        ))
                        .child(style_property_row(
                            "border",
                            &format!(
                                "{:.1} {:.1} {:.1} {:.1}",
                                layout.border.top.0,
                                layout.border.right.0,
                                layout.border.bottom.0,
                                layout.border.left.0,
                            ),
                        ))
                        .child(style_property_row(
                            "padding",
                            &format!(
                                "{:.1} {:.1} {:.1} {:.1}",
                                layout.padding.top.0,
                                layout.padding.right.0,
                                layout.padding.bottom.0,
                                layout.padding.left.0,
                            ),
                        ))
                        .child(style_property_row(
                            "content",
                            &format!(
                                "{:.1} x {:.1}",
                                layout.content_size.width.0, layout.content_size.height.0,
                            ),
                        ))
                        .into_any_element(),
                );
            }
        } else {
            elements.push(
                crate::div()
                    .p(px(8.))
                    .text_sm()
                    .text_color(DIM_TEXT)
                    .child(SharedString::from("Select an element to view its styles."))
                    .into_any_element(),
            );
        }

        crate::div()
            .flex()
            .flex_col()
            .children(elements)
            .into_any_element()
    }

    fn style_property_row(name: &str, value: &str) -> AnyElement {
        crate::div()
            .flex()
            .flex_row()
            .px(px(8.))
            .py(px(1.))
            .gap(px(8.))
            .child(
                crate::div()
                    .w(px(80.))
                    .text_xs()
                    .text_color(ACCENT)
                    .child(SharedString::from(name)),
            )
            .child(
                crate::div()
                    .flex_1()
                    .text_xs()
                    .text_color(DEFAULT_TEXT)
                    .child(SharedString::from(value)),
            )
            .into_any_element()
    }

    // ── Layout tab ───────────────────────────────────────────────────────────

    fn render_layout_tab(
        inspector: &mut Inspector,
        _window: &mut Window,
        _cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let has_selection = inspector.active_element_id().is_some();

        if !has_selection {
            return crate::div()
                .p(px(8.))
                .text_sm()
                .text_color(DIM_TEXT)
                .child(SharedString::from("Select an element to view its layout."))
                .into_any_element();
        }

        let layout = inspector.active_layout().cloned();

        crate::div()
            .flex()
            .flex_col()
            .p(px(8.))
            .gap(px(8.))
            .child(
                crate::div()
                    .text_xs()
                    .text_color(ACCENT)
                    .font_weight(crate::FontWeight::BOLD)
                    .child(SharedString::from("Box Model")),
            )
            .child(if let Some(ref layout) = layout {
                render_box_model(layout)
            } else {
                crate::div()
                    .text_sm()
                    .text_color(DIM_TEXT)
                    .child(SharedString::from("Layout data not yet available."))
                    .into_any_element()
            })
            .child(if let Some(ref layout) = layout {
                render_layout_details(layout)
            } else {
                crate::div().into_any_element()
            })
            .into_any_element()
    }

    fn render_box_model(layout: &InspectorLayoutInfo) -> AnyElement {
        let scale = 0.3;
        let margin_w = layout.margin;
        let border_w = layout.border;
        let padding_w = layout.padding;
        let content = layout.content_size;

        let total_width = margin_w.left
            + border_w.left
            + padding_w.left
            + content.width
            + padding_w.right
            + border_w.right
            + margin_w.right;
        let total_height = margin_w.top
            + border_w.top
            + padding_w.top
            + content.height
            + padding_w.bottom
            + border_w.bottom
            + margin_w.bottom;

        let scaled_w = total_width * scale;
        let scaled_h = total_height * scale;

        crate::div()
            .w(scaled_w.max(px(100.)))
            .h(scaled_h.max(px(60.)))
            .relative()
            // Margin layer (orange)
            .child(
                crate::div()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .w(scaled_w)
                    .h(scaled_h)
                    .bg(MARGIN_COLOR)
                    .opacity(0.3),
            )
            // Border layer (peach)
            .child(
                crate::div()
                    .absolute()
                    .top(margin_w.top * scale)
                    .left(margin_w.left * scale)
                    .w((border_w.left
                        + padding_w.left
                        + content.width
                        + padding_w.right
                        + border_w.right)
                        * scale)
                    .h((border_w.top
                        + padding_w.top
                        + content.height
                        + padding_w.bottom
                        + border_w.bottom)
                        * scale)
                    .bg(BORDER_COLOR)
                    .opacity(0.3),
            )
            // Padding layer (green)
            .child(
                crate::div()
                    .absolute()
                    .top((margin_w.top + border_w.top) * scale)
                    .left((margin_w.left + border_w.left) * scale)
                    .w((padding_w.left + content.width + padding_w.right) * scale)
                    .h((padding_w.top + content.height + padding_w.bottom) * scale)
                    .bg(PADDING_COLOR)
                    .opacity(0.3),
            )
            // Content layer (blue)
            .child(
                crate::div()
                    .absolute()
                    .top((margin_w.top + border_w.top + padding_w.top) * scale)
                    .left((margin_w.left + border_w.left + padding_w.left) * scale)
                    .w(content.width * scale)
                    .h(content.height * scale)
                    .bg(CONTENT_COLOR)
                    .opacity(0.5),
            )
            // Labels
            .child(
                crate::div()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .text_xs()
                    .text_color(MARGIN_COLOR)
                    .child(SharedString::from("margin")),
            )
            .child(
                crate::div()
                    .absolute()
                    .top((margin_w.top) * scale)
                    .left((margin_w.left) * scale)
                    .text_xs()
                    .text_color(BORDER_COLOR)
                    .child(SharedString::from("border")),
            )
            .child(
                crate::div()
                    .absolute()
                    .top((margin_w.top + border_w.top) * scale)
                    .left((margin_w.left + border_w.left) * scale)
                    .text_xs()
                    .text_color(PADDING_COLOR)
                    .child(SharedString::from("padding")),
            )
            .child(
                crate::div()
                    .absolute()
                    .top((margin_w.top + border_w.top + padding_w.top) * scale)
                    .left((margin_w.left + border_w.left + padding_w.left) * scale)
                    .text_xs()
                    .text_color(CONTENT_COLOR)
                    .child(SharedString::from(format!(
                        "{:.0} x {:.0}",
                        content.width.0, content.height.0
                    ))),
            )
            .into_any_element()
    }

    fn render_layout_details(layout: &InspectorLayoutInfo) -> AnyElement {
        crate::div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(layout_detail_section(
                "Position",
                &[
                    ("x", layout.bounds.origin.x.0),
                    ("y", layout.bounds.origin.y.0),
                    ("width", layout.bounds.size.width.0),
                    ("height", layout.bounds.size.height.0),
                ],
            ))
            .child(layout_detail_section(
                "Margin",
                &[
                    ("top", layout.margin.top.0),
                    ("right", layout.margin.right.0),
                    ("bottom", layout.margin.bottom.0),
                    ("left", layout.margin.left.0),
                ],
            ))
            .child(layout_detail_section(
                "Border",
                &[
                    ("top", layout.border.top.0),
                    ("right", layout.border.right.0),
                    ("bottom", layout.border.bottom.0),
                    ("left", layout.border.left.0),
                ],
            ))
            .child(layout_detail_section(
                "Padding",
                &[
                    ("top", layout.padding.top.0),
                    ("right", layout.padding.right.0),
                    ("bottom", layout.padding.bottom.0),
                    ("left", layout.padding.left.0),
                ],
            ))
            .child(layout_detail_section(
                "Content",
                &[
                    ("width", layout.content_size.width.0),
                    ("height", layout.content_size.height.0),
                ],
            ))
            .into_any_element()
    }

    fn layout_detail_section(title: &str, values: &[(&str, f32)]) -> AnyElement {
        crate::div()
            .flex()
            .flex_col()
            .child(
                crate::div()
                    .text_xs()
                    .text_color(ACCENT)
                    .font_weight(crate::FontWeight::BOLD)
                    .mb(px(2.))
                    .child(SharedString::from(title)),
            )
            .children(values.iter().map(|(name, value)| {
                crate::div()
                    .flex()
                    .flex_row()
                    .px(px(4.))
                    .child(
                        crate::div()
                            .w(px(60.))
                            .text_xs()
                            .text_color(DIM_TEXT)
                            .child(SharedString::from(*name)),
                    )
                    .child(
                        crate::div()
                            .text_xs()
                            .text_color(DEFAULT_TEXT)
                            .child(SharedString::from(format!("{:.1}px", value))),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    // ── Event Listeners tab ──────────────────────────────────────────────────

    fn render_event_listeners_tab(
        inspector: &mut Inspector,
        _window: &mut Window,
        _cx: &mut Context<Inspector>,
    ) -> AnyElement {
        let has_selection = inspector.active_element_id().is_some();

        if !has_selection {
            return crate::div()
                .p(px(8.))
                .text_sm()
                .text_color(DIM_TEXT)
                .child(SharedString::from(
                    "Select an element to view its event listeners.",
                ))
                .into_any_element();
        }

        let selected_id = inspector.active_element_id().cloned();
        let mut listeners: Vec<&InspectorEventListener> = Vec::new();

        for info in inspector.element_infos() {
            if Some(&info.inspector_id) == selected_id.as_ref() {
                listeners.extend(&info.event_listeners);
                break;
            }
        }

        if listeners.is_empty() {
            return crate::div()
                .p(px(8.))
                .text_sm()
                .text_color(DIM_TEXT)
                .child(SharedString::from(
                    "No event listeners registered on this element.",
                ))
                .into_any_element();
        }

        crate::div()
            .flex()
            .flex_col()
            .children(listeners.iter().map(|listener| {
                crate::div()
                    .flex()
                    .flex_row()
                    .px(px(8.))
                    .py(px(3.))
                    .border_b_1()
                    .border_color(BORDER)
                    .hover(|style| style.bg(HOVER_BG))
                    .child(
                        crate::div()
                            .px(px(4.))
                            .py(px(1.))
                            .rounded_sm()
                            .bg(ACCENT_DIM)
                            .text_xs()
                            .text_color(ACCENT)
                            .child(listener.event_type.clone()),
                    )
                    .child(
                        crate::div()
                            .text_xs()
                            .text_color(DIM_TEXT)
                            .ml(px(8.))
                            .child(listener.location.clone()),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    // ── Picking indicator ────────────────────────────────────────────────────

    fn render_picking_indicator(
        _inspector: &mut Inspector,
        _window: &mut Window,
        _cx: &mut Context<Inspector>,
    ) -> AnyElement {
        crate::div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.))
            .py(px(4.))
            .bg(ACCENT_DIM)
            .border_t_1()
            .border_color(ACCENT)
            .child(
                crate::div()
                    .text_xs()
                    .text_color(ACCENT)
                    .child(SharedString::from(
                        "Picking mode active - click an element to select it",
                    )),
            )
            .into_any_element()
    }

    // ── Type name helper ─────────────────────────────────────────────────────

    /// Extract a display-friendly element type name from a Rust type path.
    pub fn inspector_type_name<T: ?Sized>() -> SharedString {
        let full = std::any::type_name::<T>();
        let name = full.rsplit("::").next().unwrap_or(full);
        SharedString::from(name.to_lowercase())
    }

    #[derive(Default)]
    pub(crate) struct InspectorElementRegistry {
        renderers_by_type_id: FxHashMap<
            TypeId,
            Box<dyn Fn(InspectorElementId, &dyn Any, &mut Window, &mut App) -> AnyElement>,
        >,
    }

    impl InspectorElementRegistry {
        pub fn register<T: 'static, R: IntoElement>(
            &mut self,
            f: impl 'static + Fn(InspectorElementId, &T, &mut Window, &mut App) -> R,
        ) {
            self.renderers_by_type_id.insert(
                TypeId::of::<T>(),
                Box::new(move |id, value, window, cx| {
                    let value = value.downcast_ref().unwrap();
                    f(id, value, window, cx).into_any_element()
                }),
            );
        }
    }

    // ── Resize overlay ───────────────────────────────────────────────────────
    // When the inspector is being resized, this handles mouse events for the drag.

    pub(crate) fn handle_inspector_resize(
        inspector: &mut Inspector,
        event: &dyn Any,
        viewport_width: Pixels,
    ) -> bool {
        if inspector.is_resizing() {
            if let Some(move_event) = event.downcast_ref::<crate::MouseMoveEvent>() {
                let new_width = viewport_width - move_event.position.x;
                inspector.set_panel_width(new_width, viewport_width);
                return true;
            }
            if event.downcast_ref::<crate::MouseUpEvent>().is_some() {
                inspector.stop_resizing();
                return true;
            }
        }
        false
    }
}

/// Provides definitions used by `#[derive_inspector_reflection]`.
#[cfg(any(feature = "inspector", debug_assertions))]
pub mod inspector_reflection {
    use std::any::Any;

    /// Reification of a function that has the signature `fn some_fn(T) -> T`. Provides the name,
    /// documentation, and ability to invoke the function.
    #[derive(Clone, Copy)]
    pub struct FunctionReflection<T> {
        /// The name of the function
        pub name: &'static str,
        /// The method
        pub function: fn(Box<dyn Any>) -> Box<dyn Any>,
        /// Documentation for the function
        pub documentation: Option<&'static str>,
        /// `PhantomData` for the type of the argument and result
        pub _type: std::marker::PhantomData<T>,
    }

    impl<T: 'static> FunctionReflection<T> {
        /// Invoke this method on a value and return the result.
        pub fn invoke(&self, value: T) -> T {
            let boxed = Box::new(value) as Box<dyn Any>;
            let result = (self.function)(boxed);
            *result
                .downcast::<T>()
                .expect("Type mismatch in reflection invoke")
        }
    }
}
