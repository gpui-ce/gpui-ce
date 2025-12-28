//! Styling Patterns Example
//!
//! This example demonstrates different styling approaches in GPUI:
//!
//! 1. Interactive states - hover, active, focus, focus_visible
//! 2. Conditional styling - when, when_some, map
//! 3. Theming patterns - using globals for consistent colors

use gpui::{
    App, Application, Bounds, Context, FocusHandle, Global, Hsla, KeyBinding, Render, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

actions!(styling_example, [Tab, TabPrev]);

// ============================================================================
// Theme System
// ============================================================================
//
// A simple theme system using GPUI's Global trait.
// This allows consistent colors throughout the application.

#[derive(Clone)]
struct Theme {
    background: Hsla,
    surface: Hsla,
    surface_hover: Hsla,
    text_primary: Hsla,
    text_secondary: Hsla,
    accent: Hsla,
    accent_hover: Hsla,
    accent_active: Hsla,
    success: Hsla,
    warning: Hsla,
    error: Hsla,
    border: Hsla,
    focus_ring: Hsla,
}

impl Global for Theme {}

impl Theme {
    fn dark() -> Self {
        Self {
            background: rgb(0x0f172a).into(),
            surface: rgb(0x1e293b).into(),
            surface_hover: rgb(0x334155).into(),
            text_primary: rgb(0xf8fafc).into(),
            text_secondary: rgb(0x94a3b8).into(),
            accent: rgb(0x3b82f6).into(),
            accent_hover: rgb(0x2563eb).into(),
            accent_active: rgb(0x1d4ed8).into(),
            success: rgb(0x22c55e).into(),
            warning: rgb(0xeab308).into(),
            error: rgb(0xef4444).into(),
            border: rgb(0x334155).into(),
            focus_ring: rgb(0x60a5fa).into(),
        }
    }
}

// ============================================================================
// Interactive States Example
// ============================================================================

fn interactive_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .bg(theme.accent)
        .text_color(gpui::white())
        .text_sm()
        .hover(|style| style.bg(theme.accent_hover))
        .active(|style| style.bg(theme.accent_active))
        .child(label)
}

fn focus_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    focus_handle: &FocusHandle,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .track_focus(focus_handle)
        .px_4()
        .py_2()
        .rounded_md()
        .cursor_pointer()
        .bg(theme.surface)
        .text_color(theme.text_primary)
        .text_sm()
        .border_2()
        .border_color(gpui::transparent_black())
        .hover(|style| style.bg(theme.surface_hover))
        .focus(|style| style.border_color(theme.accent))
        .focus_visible(|style| style.border_color(theme.focus_ring).shadow_sm())
        .child(label)
}

fn interactive_states_section(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(theme.text_secondary)
                .child("hover() / active() - Mouse interaction states"),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(interactive_button("btn-1", "Hover me", theme))
                .child(interactive_button("btn-2", "Click me", theme)),
        )
}

// ============================================================================
// Conditional Styling Example
// ============================================================================

fn status_badge(status: &'static str, variant: StatusVariant, theme: &Theme) -> impl IntoElement {
    let (bg, text) = match variant {
        StatusVariant::Success => (theme.success, gpui::white()),
        StatusVariant::Warning => (theme.warning, rgb(0x000000).into()),
        StatusVariant::Error => (theme.error, gpui::white()),
        StatusVariant::Neutral => (theme.surface, theme.text_primary),
    };

    div()
        .px_2()
        .py_0p5()
        .rounded_full()
        .text_xs()
        .bg(bg)
        .text_color(text)
        .child(status)
}

#[derive(Clone, Copy)]
enum StatusVariant {
    Success,
    Warning,
    Error,
    Neutral,
}

fn list_item(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    is_selected: bool,
    is_disabled: bool,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .border_1()
        .border_color(gpui::transparent_black())
        .when(is_disabled, |el| {
            el.opacity(0.5)
                .cursor_not_allowed()
                .bg(theme.surface.opacity(0.5))
                .text_color(theme.text_secondary)
        })
        .when(!is_disabled && is_selected, |el| {
            el.bg(theme.accent.opacity(0.2))
                .border_color(theme.accent)
                .text_color(theme.text_primary)
        })
        .when(!is_disabled && !is_selected, |el| {
            el.bg(theme.surface)
                .text_color(theme.text_primary)
                .hover(|style| style.bg(theme.surface_hover))
        })
        .child(label)
}

fn conditional_section(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(theme.text_secondary)
                .child("when() - Apply styles conditionally"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(list_item("item-1", "Normal item", false, false, theme))
                .child(list_item("item-2", "Selected item", true, false, theme))
                .child(list_item("item-3", "Disabled item", false, true, theme)),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.text_secondary)
                .mt_2()
                .child("Status badges with variant-based styling"),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(status_badge("Success", StatusVariant::Success, theme))
                .child(status_badge("Warning", StatusVariant::Warning, theme))
                .child(status_badge("Error", StatusVariant::Error, theme))
                .child(status_badge("Neutral", StatusVariant::Neutral, theme)),
        )
}

// ============================================================================
// Group Hover Example
// ============================================================================

fn card_with_group_hover(
    id: impl Into<gpui::ElementId>,
    title: &'static str,
    description: &'static str,
    theme: &Theme,
) -> impl IntoElement {
    div()
        .id(id)
        .group("card")
        .p_4()
        .rounded_lg()
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .cursor_pointer()
        .hover(|style| style.border_color(theme.accent))
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text_primary)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_secondary)
                        .opacity(0.)
                        .group_hover("card", |style| style.opacity(1.))
                        .child("→"),
                ),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(theme.text_secondary)
                .child(description),
        )
}

fn group_hover_section(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_color(theme.text_secondary)
                .child("group() / group_hover() - Parent hover affects children"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(card_with_group_hover(
                    "card-1",
                    "Documents",
                    "View and manage your documents",
                    theme,
                ))
                .child(card_with_group_hover(
                    "card-2",
                    "Settings",
                    "Configure application settings",
                    theme,
                )),
        )
}

// ============================================================================
// Main Application View
// ============================================================================

struct StylingExample {
    focus_handle: FocusHandle,
    buttons: Vec<FocusHandle>,
}

impl StylingExample {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        let buttons = vec![
            cx.focus_handle().tab_index(1).tab_stop(true),
            cx.focus_handle().tab_index(2).tab_stop(true),
            cx.focus_handle().tab_index(3).tab_stop(true),
        ];

        Self {
            focus_handle,
            buttons,
        }
    }

    fn on_tab(&mut self, _: &Tab, window: &mut Window, _: &mut Context<Self>) {
        window.focus_next();
    }

    fn on_tab_prev(&mut self, _: &TabPrev, window: &mut Window, _: &mut Context<Self>) {
        window.focus_prev();
    }
}

impl Render for StylingExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();

        div()
            .id("app")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_tab))
            .on_action(cx.listener(Self::on_tab_prev))
            .size_full()
            .p_6()
            .bg(theme.background)
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .max_w(px(500.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child("Styling Patterns"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.text_secondary)
                                    .child("Interactive states, conditional styling, and theming"),
                            ),
                    )
                    .child(section(
                        "Interactive States",
                        interactive_states_section(&theme),
                        &theme,
                    ))
                    .child(section(
                        "Focus States (Tab to navigate)",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_secondary)
                                    .child("focus() / focus_visible() - Keyboard navigation"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(focus_button(
                                        "focus-1",
                                        "Button 1",
                                        &self.buttons[0],
                                        &theme,
                                    ))
                                    .child(focus_button(
                                        "focus-2",
                                        "Button 2",
                                        &self.buttons[1],
                                        &theme,
                                    ))
                                    .child(focus_button(
                                        "focus-3",
                                        "Button 3",
                                        &self.buttons[2],
                                        &theme,
                                    )),
                            ),
                        &theme,
                    ))
                    .child(section(
                        "Conditional Styling",
                        conditional_section(&theme),
                        &theme,
                    ))
                    .child(section("Group Hover", group_hover_section(&theme), &theme))
                    .child(section(
                        "Theme Colors",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_secondary)
                                    .child("Using Global<Theme> for consistent colors"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(color_swatch("background", theme.background))
                                    .child(color_swatch("surface", theme.surface))
                                    .child(color_swatch("accent", theme.accent))
                                    .child(color_swatch("success", theme.success))
                                    .child(color_swatch("warning", theme.warning))
                                    .child(color_swatch("error", theme.error))
                                    .child(color_swatch("border", theme.border)),
                            ),
                        &theme,
                    )),
            )
    }
}

fn section(title: &'static str, content: impl IntoElement, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .bg(theme.surface.opacity(0.3))
        .rounded_lg()
        .border_1()
        .border_color(theme.border.opacity(0.5))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_primary)
                .child(title),
        )
        .child(content)
}

fn color_swatch(name: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .child(
            div()
                .size_8()
                .rounded_md()
                .bg(color)
                .border_1()
                .border_color(gpui::white().opacity(0.2)),
        )
        .child(
            div()
                .text_xs()
                .text_color(gpui::white().opacity(0.7))
                .child(name),
        )
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.set_global(Theme::dark());

        cx.bind_keys([
            KeyBinding::new("tab", Tab, None),
            KeyBinding::new("shift-tab", TabPrev, None),
        ]);

        let bounds = Bounds::centered(None, size(px(550.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| StylingExample::new(window, cx)),
        )
        .expect("Failed to open window");

        cx.activate(true);
    });
}
