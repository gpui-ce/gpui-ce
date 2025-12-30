//! Developer tools for inspecting GPUI applications.
//!
//! This module provides a built-in inspector UI that can be toggled to inspect
//! elements in your application. It displays element dimensions and styles.
//!
//! # Usage
//!
//! Enable the `dev-tools` feature in your `Cargo.toml`:
//!
//! ```toml
//! gpui = { version = "...", features = ["dev-tools"] }
//! ```
//!
//! Then initialize the dev tools in your application:
//!
//! ```ignore
//! use gpui::dev_tools;
//!
//! fn main() {
//!     Application::new().run(|cx: &mut App| {
//!         dev_tools::init(cx);
//!         // ... rest of your app
//!     });
//! }
//! ```
//!
//! The inspector can be toggled with the `ToggleInspector` action, which you can
//! bind to a keyboard shortcut.

use crate::default_colors::DefaultColors;
use crate::util::FluentBuilder;
use crate::{
    AnyElement, App, Context, DivInspectorState, Inspector, InspectorElementId, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, actions,
    div,
};

actions!(
    dev_tools,
    [
        /// Toggle the GPUI inspector panel for the current window.
        ToggleInspector
    ]
);

/// Helper to create a horizontal flex container
fn h_flex() -> crate::Div {
    div().flex().flex_row()
}

/// Helper to create a vertical flex container
fn v_flex() -> crate::Div {
    div().flex().flex_col()
}

/// Initialize the dev tools, registering the inspector renderer and actions.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &ToggleInspector, cx| {
        let Some(active_window) = cx.active_window() else {
            return;
        };
        cx.defer(move |cx| {
            if let Ok(()) = active_window.update(cx, |_, window, _cx| {
                window.toggle_inspector(_cx);
            }) {}
        });
    });

    cx.register_inspector_element(
        |id: InspectorElementId, state: &DivInspectorState, _window, cx| {
            DivInspectorPanel::new(id, state.clone()).render_panel(cx)
        },
    );

    cx.set_inspector_renderer(Box::new(render_inspector));
}

fn render_inspector(
    inspector: &mut Inspector,
    window: &mut Window,
    cx: &mut Context<Inspector>,
) -> AnyElement {
    let colors = cx.default_colors();
    let inspector_id = inspector.active_element_id();

    v_flex()
        .size_full()
        .bg(colors.background)
        .text_color(colors.text)
        .text_sm()
        .border_l_1()
        .border_color(colors.border)
        .child(
            h_flex()
                .justify_between()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(colors.border)
                .child(
                    div()
                        .id("pick-mode-button")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .when(inspector.is_picking(), |this| {
                            this.bg(colors.selected).text_color(colors.selected_text)
                        })
                        .when(!inspector.is_picking(), |this| {
                            this.bg(colors.container).hover(|s| s.bg(colors.separator))
                        })
                        .child("🔍 Pick")
                        .on_click(cx.listener(|inspector, _, window, _cx| {
                            inspector.start_picking();
                            window.refresh();
                        })),
                )
                .child(div().text_color(colors.text).child("GPUI Inspector")),
        )
        .child(
            v_flex()
                .id("gpui-inspector-content")
                .overflow_y_scroll()
                .px_2()
                .py_1()
                .gap_2()
                .flex_1()
                .when_some(inspector_id, |this, inspector_id| {
                    this.child(render_inspector_id(inspector_id, cx))
                })
                .children(inspector.render_inspector_states(window, cx)),
        )
        .into_any_element()
}

fn render_inspector_id(inspector_id: &InspectorElementId, cx: &App) -> AnyElement {
    let colors = cx.default_colors();
    let source_location = inspector_id.path.source_location;
    let source_location_string = source_location.to_string();

    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_base()
                        .font_weight(crate::FontWeight::SEMIBOLD)
                        .child("Element ID"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.disabled)
                        .child(format!("Instance {}", inspector_id.instance_id)),
                ),
        )
        .child(
            div()
                .text_xs()
                .bg(colors.container)
                .px_1()
                .py_0p5()
                .rounded_sm()
                .overflow_x_hidden()
                .child(source_location_string),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.disabled)
                .child(inspector_id.path.global_id.to_string()),
        )
        .into_any_element()
}

/// Panel that displays the inspector state for a div element.
struct DivInspectorPanel {
    _id: InspectorElementId,
    state: DivInspectorState,
}

impl DivInspectorPanel {
    fn new(id: InspectorElementId, state: DivInspectorState) -> Self {
        Self { _id: id, state }
    }

    fn render_panel(self, cx: &App) -> AnyElement {
        let colors = cx.default_colors();

        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(crate::FontWeight::SEMIBOLD)
                            .child("Layout"),
                    )
                    .child(self.render_layout_info(cx)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(crate::FontWeight::SEMIBOLD)
                            .child("Styles"),
                    )
                    .child(
                        div()
                            .bg(colors.container)
                            .rounded_md()
                            .p_2()
                            .text_xs()
                            .overflow_x_hidden()
                            .child(self.render_style_info(cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_layout_info(&self, cx: &App) -> AnyElement {
        let colors = cx.default_colors();
        let bounds = &self.state.bounds;
        let content_size = &self.state.content_size;

        v_flex()
            .bg(colors.container)
            .rounded_md()
            .p_2()
            .gap_1()
            .text_xs()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.render_label_value("x", format!("{:.1}", f32::from(bounds.origin.x))),
                    )
                    .child(
                        self.render_label_value("y", format!("{:.1}", f32::from(bounds.origin.y))),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(self.render_label_value(
                        "width",
                        format!("{:.1}", f32::from(bounds.size.width)),
                    ))
                    .child(self.render_label_value(
                        "height",
                        format!("{:.1}", f32::from(bounds.size.height)),
                    )),
            )
            .when(content_size != &bounds.size, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .child(self.render_label_value(
                            "content_w",
                            format!("{:.1}", f32::from(content_size.width)),
                        ))
                        .child(self.render_label_value(
                            "content_h",
                            format!("{:.1}", f32::from(content_size.height)),
                        )),
                )
            })
            .into_any_element()
    }

    fn render_label_value(&self, label: &str, value: String) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                div()
                    .text_color(crate::rgb(0x888888))
                    .child(SharedString::from(label.to_string())),
            )
            .child(div().child(SharedString::from(value)))
            .into_any_element()
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn render_style_info(&self, _cx: &App) -> AnyElement {
        let style = &self.state.base_style;
        let mut lines: Vec<SharedString> = Vec::new();

        if let Some(display) = &style.display {
            lines.push(format!("display: {:?}", display).into());
        }

        if let Some(visibility) = &style.visibility {
            lines.push(format!("visibility: {:?}", visibility).into());
        }

        if let Some(overflow_x) = &style.overflow.x {
            lines.push(format!("overflow-x: {:?}", overflow_x).into());
        }

        if let Some(overflow_y) = &style.overflow.y {
            lines.push(format!("overflow-y: {:?}", overflow_y).into());
        }

        if let Some(position) = &style.position {
            lines.push(format!("position: {:?}", position).into());
        }

        if let Some(flex_direction) = &style.flex_direction {
            lines.push(format!("flex-direction: {:?}", flex_direction).into());
        }

        if let Some(flex_wrap) = &style.flex_wrap {
            lines.push(format!("flex-wrap: {:?}", flex_wrap).into());
        }

        if let Some(align_items) = &style.align_items {
            lines.push(format!("align-items: {:?}", align_items).into());
        }

        if let Some(align_content) = &style.align_content {
            lines.push(format!("align-content: {:?}", align_content).into());
        }

        if let Some(justify_content) = &style.justify_content {
            lines.push(format!("justify-content: {:?}", justify_content).into());
        }

        if let Some(flex_grow) = &style.flex_grow {
            lines.push(format!("flex-grow: {}", flex_grow).into());
        }

        if let Some(flex_shrink) = &style.flex_shrink {
            lines.push(format!("flex-shrink: {}", flex_shrink).into());
        }

        if let Some(width) = &style.size.width {
            lines.push(format!("width: {:?}", width).into());
        }

        if let Some(height) = &style.size.height {
            lines.push(format!("height: {:?}", height).into());
        }

        if let Some(min_width) = &style.min_size.width {
            lines.push(format!("min-width: {:?}", min_width).into());
        }

        if let Some(min_height) = &style.min_size.height {
            lines.push(format!("min-height: {:?}", min_height).into());
        }

        if let Some(max_width) = &style.max_size.width {
            lines.push(format!("max-width: {:?}", max_width).into());
        }

        if let Some(max_height) = &style.max_size.height {
            lines.push(format!("max-height: {:?}", max_height).into());
        }

        if let Some(top) = &style.inset.top {
            lines.push(format!("top: {:?}", top).into());
        }

        if let Some(right) = &style.inset.right {
            lines.push(format!("right: {:?}", right).into());
        }

        if let Some(bottom) = &style.inset.bottom {
            lines.push(format!("bottom: {:?}", bottom).into());
        }

        if let Some(left) = &style.inset.left {
            lines.push(format!("left: {:?}", left).into());
        }

        if let Some(margin_top) = &style.margin.top {
            lines.push(format!("margin-top: {:?}", margin_top).into());
        }

        if let Some(margin_right) = &style.margin.right {
            lines.push(format!("margin-right: {:?}", margin_right).into());
        }

        if let Some(margin_bottom) = &style.margin.bottom {
            lines.push(format!("margin-bottom: {:?}", margin_bottom).into());
        }

        if let Some(margin_left) = &style.margin.left {
            lines.push(format!("margin-left: {:?}", margin_left).into());
        }

        if let Some(padding_top) = &style.padding.top {
            lines.push(format!("padding-top: {:?}", padding_top).into());
        }

        if let Some(padding_right) = &style.padding.right {
            lines.push(format!("padding-right: {:?}", padding_right).into());
        }

        if let Some(padding_bottom) = &style.padding.bottom {
            lines.push(format!("padding-bottom: {:?}", padding_bottom).into());
        }

        if let Some(padding_left) = &style.padding.left {
            lines.push(format!("padding-left: {:?}", padding_left).into());
        }

        if let Some(gap_width) = &style.gap.width {
            lines.push(format!("column-gap: {:?}", gap_width).into());
        }

        if let Some(gap_height) = &style.gap.height {
            lines.push(format!("row-gap: {:?}", gap_height).into());
        }

        if let Some(background) = &style.background {
            lines.push(format!("background: {:?}", background).into());
        }

        if let Some(border_color) = &style.border_color {
            lines.push(format!("border-color: {:?}", border_color).into());
        }

        // corner_radii is a CornersRefinement, not an Option
        let corner_radii = &style.corner_radii;
        if corner_radii.top_left.is_some()
            || corner_radii.top_right.is_some()
            || corner_radii.bottom_right.is_some()
            || corner_radii.bottom_left.is_some()
        {
            lines.push(format!("border-radius: {:?}", corner_radii).into());
        }

        if let Some(opacity) = &style.opacity {
            lines.push(format!("opacity: {}", opacity).into());
        }

        if lines.is_empty() {
            lines.push("(no styles set)".into());
        }

        v_flex()
            .gap_0p5()
            .children(lines.into_iter().map(|line| div().child(line)))
            .into_any_element()
    }

    #[cfg(not(any(feature = "inspector", debug_assertions)))]
    fn render_style_info(&self, _cx: &App) -> AnyElement {
        div().child("(styles not available)").into_any_element()
    }
}
