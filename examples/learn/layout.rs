//! Layout Patterns Example
//!
//! This example demonstrates different layout approaches in GPUI:
//!
//! 1. Flexbox - Row and column layouts with alignment
//! 2. Grid - Two-dimensional layouts with spans
//! 3. Common patterns - Sidebar, header/footer, centering

use gpui::{
    App, Application, Bounds, Context, Div, Hsla, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

// ============================================================================
// Helper: Colored block for visualization
// ============================================================================

fn block(label: &'static str, color: Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .bg(color)
        .border_1()
        .border_color(gpui::white().opacity(0.3))
        .rounded_md()
        .text_xs()
        .text_color(gpui::white())
        .child(label)
}

// ============================================================================
// Flexbox Examples
// ============================================================================

fn flexbox_row_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("flex().flex_row().gap_2()"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(block("A", gpui::red()).size_8())
                .child(block("B", gpui::green()).size_8())
                .child(block("C", gpui::blue()).size_8()),
        )
}

fn flexbox_column_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("flex().flex_col().gap_2()"),
        )
        .child(
            div()
                .h_24()
                .flex()
                .flex_col()
                .gap_2()
                .child(block("A", gpui::red()).h_6())
                .child(block("B", gpui::green()).h_6())
                .child(block("C", gpui::blue()).h_6()),
        )
}

fn flexbox_justify_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("justify_between / justify_center / justify_end"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .p_1()
                        .bg(rgb(0x1e293b))
                        .rounded_sm()
                        .child(block("Start", gpui::red()).px_2().py_1())
                        .child(block("End", gpui::blue()).px_2().py_1()),
                )
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .p_1()
                        .bg(rgb(0x1e293b))
                        .rounded_sm()
                        .child(block("Center", gpui::green()).px_2().py_1()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .p_1()
                        .bg(rgb(0x1e293b))
                        .rounded_sm()
                        .child(block("End", gpui::yellow()).px_2().py_1()),
                ),
        )
}

fn flexbox_grow_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("flex_1 (grow) vs flex_none (fixed)"),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(block("fixed", gpui::red()).flex_none().w_16().h_8())
                .child(block("flex_1 (grows)", gpui::green()).flex_1().h_8())
                .child(block("fixed", gpui::blue()).flex_none().w_16().h_8()),
        )
}

// ============================================================================
// Grid Examples
// ============================================================================

fn grid_basic_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("grid().grid_cols(3).gap_1()"),
        )
        .child(
            div()
                .grid()
                .grid_cols(3)
                .gap_1()
                .child(block("1", gpui::red()).h_8())
                .child(block("2", gpui::green()).h_8())
                .child(block("3", gpui::blue()).h_8())
                .child(block("4", gpui::yellow()).h_8())
                .child(block("5", gpui::red()).h_8())
                .child(block("6", gpui::green()).h_8()),
        )
}

fn grid_span_example() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("col_span / row_span"),
        )
        .child(
            div()
                .grid()
                .grid_cols(4)
                .grid_rows(3)
                .gap_1()
                .child(
                    block("Header (col_span_full)", gpui::red())
                        .col_span_full()
                        .h_6(),
                )
                .child(
                    block("Side", gpui::green())
                        .col_span(1)
                        .row_span(2)
                        .h_full(),
                )
                .child(
                    block("Content (col_span 3)", gpui::blue())
                        .col_span(3)
                        .row_span(2)
                        .h_full(),
                ),
        )
}

// ============================================================================
// Common Layout Patterns
// ============================================================================

fn app_shell_pattern() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("App Shell: Header + Sidebar + Content"),
        )
        .child(
            div()
                .h_32()
                .flex()
                .flex_col()
                .border_1()
                .border_color(rgb(0x334155))
                .rounded_md()
                .overflow_hidden()
                .child(
                    div()
                        .h_6()
                        .flex()
                        .items_center()
                        .px_2()
                        .bg(rgb(0x334155))
                        .text_xs()
                        .text_color(gpui::white())
                        .child("Header"),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .child(
                            div()
                                .w_16()
                                .bg(rgb(0x1e293b))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .child("Side"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .bg(rgb(0x0f172a))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(rgb(0x94a3b8))
                                .child("Content"),
                        ),
                ),
        )
}

fn centered_pattern() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("Centering: items_center + justify_center"),
        )
        .child(
            div()
                .h_20()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x1e293b))
                .rounded_md()
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(gpui::blue())
                        .rounded_md()
                        .text_xs()
                        .text_color(gpui::white())
                        .child("Perfectly Centered"),
                ),
        )
}

fn stack_pattern() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("Stack: Overlapping with absolute positioning"),
        )
        .child(
            div()
                .h_20()
                .relative()
                .bg(rgb(0x1e293b))
                .rounded_md()
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .left_2()
                        .size_10()
                        .bg(gpui::red().opacity(0.7))
                        .rounded_md(),
                )
                .child(
                    div()
                        .absolute()
                        .top_4()
                        .left_4()
                        .size_10()
                        .bg(gpui::green().opacity(0.7))
                        .rounded_md(),
                )
                .child(
                    div()
                        .absolute()
                        .top_6()
                        .left_6()
                        .size_10()
                        .bg(gpui::blue().opacity(0.7))
                        .rounded_md(),
                ),
        )
}

// ============================================================================
// Main Application View
// ============================================================================

struct LayoutExample;

impl Render for LayoutExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("main")
            .size_full()
            .p_4()
            .bg(rgb(0x0f172a))
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .max_w(px(600.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(gpui::white())
                                    .child("Layout Patterns"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x94a3b8))
                                    .child("Flexbox, Grid, and common layout patterns in GPUI"),
                            ),
                    )
                    .child(section("Flexbox: Row", flexbox_row_example()))
                    .child(section("Flexbox: Column", flexbox_column_example()))
                    .child(section(
                        "Flexbox: Justify Content",
                        flexbox_justify_example(),
                    ))
                    .child(section("Flexbox: Grow/Shrink", flexbox_grow_example()))
                    .child(section("Grid: Basic", grid_basic_example()))
                    .child(section("Grid: Spans", grid_span_example()))
                    .child(section("Pattern: App Shell", app_shell_pattern()))
                    .child(section("Pattern: Centering", centered_pattern()))
                    .child(section("Pattern: Stack", stack_pattern())),
            )
    }
}

fn section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    let surface: Hsla = rgb(0x1e293b).into();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(surface.opacity(0.5))
        .rounded_lg()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(gpui::white())
                .child(title),
        )
        .child(content)
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(650.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| LayoutExample),
        )
        .expect("Failed to open window");

        cx.activate(true);
    });
}
