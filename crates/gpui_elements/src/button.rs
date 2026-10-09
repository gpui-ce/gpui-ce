use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, InteractiveElement, Interactivity,
    IntoElement, MouseButton, ParentElement, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, div, prelude::FluentBuilder,
};

pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// An unstyled button that owns activation, focus, and accessibility behavior.
///
/// Callers provide all layout and appearance through GPUI styles.
#[derive(IntoElement, Styled, ParentElement, StatefulInteractiveElement)]
pub struct BaseButton {
    element_id: ElementId,
    base: Stateful<Div>,
    #[style]
    style: StyleRefinement,
    disabled: bool,
    disabled_style: Option<Box<StyleRefinement>>,
    focusable_when_disabled: bool,
    #[children]
    children: Vec<AnyElement>,
    on_click: Option<ClickHandler>,
    aria_label: Option<SharedString>,
}

impl BaseButton {
    pub fn new(element_id: impl Into<ElementId>) -> Self {
        let element_id = element_id.into();

        Self {
            element_id: element_id.clone(),
            base: div().id(element_id),
            style: StyleRefinement::default(),
            disabled: false,
            disabled_style: None,
            focusable_when_disabled: false,
            children: Vec::new(),
            on_click: None,
            aria_label: None,
        }
    }

    /// Sets whether the button ignores pointer and keyboard activation.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;

        self
    }

    /// Applies these styles after the regular styles while the button is disabled.
    pub fn disabled_style(
        mut self,
        styles: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.disabled_style = Some(Box::new(styles(StyleRefinement::default())));

        self
    }

    /// Keeps a disabled button in focus traversal and retains focus when its
    /// disabled state changes, while still preventing activation.
    pub fn focusable_when_disabled(mut self, focusable: bool) -> Self {
        self.focusable_when_disabled = focusable;

        self
    }

    /// Sets the name exposed to accessibility clients.
    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());

        self
    }

    /// Handles pointer, Enter, and Space activation.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));

        self
    }
}

impl InteractiveElement for BaseButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for BaseButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus_handle = use_focus_handle(self.element_id.clone(), window, cx, None);

        let disabled = self.disabled;
        let focusable = !disabled || self.focusable_when_disabled;
        let disabled_style = if disabled { self.disabled_style } else { None };
        let on_click = if disabled { None } else { self.on_click };

        self.base
            .role(Role::Button)
            .refine_style(&self.style)
            .when_some(disabled_style, |this, styles| this.refine_style(&styles))
            .when_some(self.aria_label, |this, label| this.aria_label(label))
            .when(focusable, |this| this.track_focus(&focus_handle))
            .when(disabled, |this| {
                this.on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
            })
            .when_some(on_click, |this, on_click| {
                this.on_click(move |event, window, cx| {
                    on_click(event, window, cx);
                })
            })
            .children(self.children)
    }
}

pub fn use_focus_handle(
    base_id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
    focus_handle: Option<FocusHandle>,
) -> FocusHandle {
    focus_handle.unwrap_or_else(|| {
        window
            .use_keyed_state((base_id.into(), "state:focus_handle"), cx, |_window, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::BaseButton;

    use gpui::{
        AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
        KeyUpEvent, Keystroke, Modifiers, ParentElement, Pixels, Render,
        StatefulInteractiveElement, Styled, TestAppContext, VisualTestContext, Window,
        accesskit::{Action, Node, Role},
        div,
        prelude::FluentBuilder,
        px, size,
    };

    #[derive(Default)]
    struct ButtonHarness {
        disabled: bool,
        focusable_when_disabled: bool,
        disabled_width: Option<Pixels>,
        callback: bool,
        label: Option<&'static str>,
        clicks: Vec<bool>,
        parent_clicks: usize,
    }

    impl Render for ButtonHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("button-parent")
                .tab_group()
                .size_full()
                .on_click(cx.listener(|this, _event, _window, _app| this.parent_clicks += 1))
                .child(
                    BaseButton::new("button-under-test")
                        .disabled(self.disabled)
                        .focusable_when_disabled(self.focusable_when_disabled)
                        .when_some(self.disabled_width, |button, width| {
                            button.disabled_style(|styles| styles.w(width))
                        })
                        .w(px(100.))
                        .h(px(30.))
                        .when_some(self.label, |button, label| button.aria_label(label))
                        .when(self.callback, |button| {
                            button.on_click(cx.listener(|this, event, _window, _app| {
                                this.clicks
                                    .push(matches!(event, gpui::ClickEvent::Keyboard(_event)));
                            }))
                        }),
                )
        }
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<ButtonHarness>, &mut VisualTestContext) {
        cx.add_window_view(|window, _cx| {
            window.set_a11y_forced(true);

            ButtonHarness {
                callback: true,
                ..ButtonHarness::default()
            }
        })
    }

    fn configure(
        view: &Entity<ButtonHarness>,
        cx: &mut VisualTestContext,
        change: impl FnOnce(&mut ButtonHarness),
    ) {
        view.update(cx, |view, cx| {
            change(view);
            cx.notify();
        });
    }

    fn snapshot(cx: &mut VisualTestContext) -> (Node, Bounds<Pixels>) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);

            let (node_id, node) = window
                .a11y_tree()
                .unwrap()
                .nodes
                .iter()
                .find(|(_node_id, node)| node.role() == Role::Button)
                .expect("button should be in the accessibility tree");

            (node.clone(), window.a11y_node_bounds(*node_id).unwrap())
        })
    }

    fn state(view: &Entity<ButtonHarness>, cx: &VisualTestContext) -> (Vec<bool>, usize) {
        cx.read_entity(view, |view, _cx| (view.clicks.clone(), view.parent_clicks))
    }

    fn activate(cx: &mut VisualTestContext) {
        let center = snapshot(cx).1.center();

        cx.simulate_click(center, Modifiers::none());

        for key in ["enter", "space"] {
            let keystroke = Keystroke::parse(key).unwrap();

            cx.simulate_event(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });

            cx.simulate_event(KeyUpEvent { keystroke });
        }
    }

    #[gpui::test]
    fn pointer_enter_and_space_activate_once_each(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx);

        activate(cx);

        assert_eq!(state(&view, cx), (vec![false, true, true], 1));
    }

    #[gpui::test]
    fn disabled_activation_and_focus_policy(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx);

        for focusable in [false, true] {
            configure(&view, cx, |view| {
                view.disabled = true;
                view.focusable_when_disabled = focusable;
            });

            snapshot(cx);
            cx.update(|window, cx| {
                window.blur(cx);
                window.focus_next(cx);
                assert_eq!(window.focused(cx).is_some(), focusable);
            });

            activate(cx);

            assert_eq!(state(&view, cx), (Vec::new(), 0));
        }
    }

    #[gpui::test]
    fn focus_is_retained_across_disabled_state_changes(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx);
        snapshot(cx);
        let focused =
            cx.update(|window, cx| {
                window.focus_next(cx);
                window.draw(cx).clear(cx);

                let tree = window.a11y_tree().unwrap();

                assert!(tree.nodes.iter().any(|(node_id, node)| {
                    *node_id == tree.focus && node.role() == Role::Button
                }));

                tree.focus
            });

        for disabled in [true, false] {
            configure(&view, cx, |view| {
                view.disabled = disabled;
                view.focusable_when_disabled = true;
            });

            snapshot(cx);
            cx.update(|window, _cx| assert_eq!(window.a11y_tree().unwrap().focus, focused));
        }
    }

    #[gpui::test]
    fn disabled_style_overrides_and_restores_layout(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx);

        for (disabled, width, expected) in [
            (false, Some(px(80.)), 100.),
            (true, None, 100.),
            (true, Some(px(80.)), 80.),
            (false, Some(px(80.)), 100.),
        ] {
            configure(&view, cx, |view| {
                view.disabled = disabled;
                view.disabled_width = width;
            });

            assert_eq!(snapshot(cx).1.size, size(px(expected), px(30.)));
        }
    }

    #[gpui::test]
    fn accessibility_reports_label_and_available_actions(cx: &mut TestAppContext) {
        let (view, cx) = setup(cx);

        assert_eq!(snapshot(cx).0.label(), None);

        for (disabled, focusable, callback, actions) in [
            (false, false, true, &[Action::Click, Action::Focus][..]),
            (true, false, true, &[][..]),
            (true, true, true, &[Action::Focus][..]),
            (false, false, false, &[Action::Focus][..]),
        ] {
            configure(&view, cx, |view| {
                view.disabled = disabled;
                view.focusable_when_disabled = focusable;
                view.callback = callback;
                view.label = Some("Save");
            });

            let (node, _bounds) = snapshot(cx);

            assert_eq!(node.label(), Some("Save"));

            for action in [Action::Click, Action::Focus] {
                assert_eq!(node.supports_action(action), actions.contains(&action));
            }
        }
    }
}
