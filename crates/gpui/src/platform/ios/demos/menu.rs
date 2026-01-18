//! Main menu for iOS demos.
//!
//! Provides a menu to select between different demo views.

use super::{
    AnimationPlayground, BACKGROUND, BLUE, GREEN, MAUVE, OVERLAY, SUBTEXT, SURFACE, ShaderShowcase,
    TEXT, TextEditor,
};
use crate::{
    App, Bounds, Context, ElementInputHandler, Entity, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, ScrollDelta, ScrollWheelEvent, Window,
    div, hsla, point, prelude::*, px, rgb, size,
};

/// Which demo is currently active
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ActiveDemo {
    #[default]
    Menu,
    AnimationPlayground,
    ShaderShowcase,
    TextEditor,
}

/// Root application view that manages demo navigation
pub struct DemoApp {
    active: ActiveDemo,
    animation_playground: Option<AnimationPlayground>,
    shader_showcase: Option<ShaderShowcase>,
    text_editor: Option<Entity<TextEditor>>,
    /// Bounds for text editor input handler
    text_editor_bounds: Option<Bounds<crate::Pixels>>,
}

impl DemoApp {
    pub fn new() -> Self {
        Self {
            active: ActiveDemo::Menu,
            animation_playground: None,
            shader_showcase: None,
            text_editor: None,
            text_editor_bounds: None,
        }
    }

    fn go_to_animation_playground(&mut self, cx: &mut Context<Self>) {
        self.animation_playground = Some(AnimationPlayground::new());
        self.active = ActiveDemo::AnimationPlayground;
        cx.notify();
    }

    fn go_to_shader_showcase(&mut self, cx: &mut Context<Self>) {
        self.shader_showcase = Some(ShaderShowcase::new());
        self.active = ActiveDemo::ShaderShowcase;
        cx.notify();
    }

    fn go_to_text_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        println!("GPUI iOS: go_to_text_editor called");
        // Create TextEditor as an Entity so we can use handle_input
        let editor = cx.new(|cx| {
            let mut editor = TextEditor::new();
            editor.init_focus(cx);
            editor
        });

        // Focus the editor to enable keyboard input
        let focus = editor.read(cx).focus_handle(cx);
        println!("GPUI iOS: Focusing editor");
        window.focus(&focus);

        self.text_editor = Some(editor);
        self.text_editor_bounds = None;
        self.active = ActiveDemo::TextEditor;
        cx.notify();
        println!("GPUI iOS: go_to_text_editor completed");
    }

    fn go_to_menu(&mut self, cx: &mut Context<Self>) {
        self.active = ActiveDemo::Menu;
        self.animation_playground = None;
        self.shader_showcase = None;
        self.text_editor = None;
        cx.notify();
    }

    fn handle_animation_touch_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(playground) = &mut self.animation_playground {
            let pos = point(event.position.x.0, event.position.y.0);
            playground.touch_start = Some((pos, std::time::Instant::now()));
            playground.current_touch = Some(pos);
            cx.notify();
        }
    }

    fn handle_animation_touch_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(playground) = &mut self.animation_playground {
            let position = point(event.position.x.0, event.position.y.0);

            if let Some((start_pos, start_time)) = playground.touch_start.take() {
                let elapsed = start_time.elapsed();
                let dx = position.x - start_pos.x;
                let dy = position.y - start_pos.y;
                let distance = (dx * dx + dy * dy).sqrt();

                if elapsed < std::time::Duration::from_millis(200) && distance < 20.0 {
                    let color_rgb = super::random_color(playground.next_ball_id);
                    playground.spawn_particles(position, rgb(color_rgb).into());
                    playground.next_ball_id += 1;
                } else {
                    let dt = elapsed.as_secs_f32().max(0.01);
                    let velocity = point(dx / dt * 0.5, dy / dt * 0.5);
                    playground.spawn_ball(start_pos, velocity);
                }
            }
            playground.current_touch = None;
            cx.notify();
        }
    }

    fn handle_shader_touch_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(showcase) = &mut self.shader_showcase {
            let pos = point(event.position.x.0, event.position.y.0);
            showcase.touch_position = Some(pos);
            showcase.spawn_ripple(pos);
            cx.notify();
        }
    }

    fn handle_shader_touch_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(showcase) = &mut self.shader_showcase {
            let pos = point(event.position.x.0, event.position.y.0);
            showcase.touch_position = Some(pos);
            cx.notify();
        }
    }

    fn handle_shader_touch_up(&mut self, _event: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(showcase) = &mut self.shader_showcase {
            showcase.touch_position = None;
            cx.notify();
        }
    }
}

impl Render for DemoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active {
            ActiveDemo::Menu => self.render_menu(window, cx).into_any_element(),
            ActiveDemo::AnimationPlayground => self
                .render_animation_playground(window, cx)
                .into_any_element(),
            ActiveDemo::ShaderShowcase => {
                self.render_shader_showcase(window, cx).into_any_element()
            }
            ActiveDemo::TextEditor => self.render_text_editor(window, cx).into_any_element(),
        }
    }
}

impl DemoApp {
    fn render_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> crate::AnyElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .justify_center()
            .items_center()
            .gap_6()
            // Title
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(div().text_3xl().text_color(rgb(TEXT)).child("GPUI on iOS"))
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(SUBTEXT))
                            .child("Interactive Demos"),
                    ),
            )
            // Demo buttons
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w(px(300.0))
                    // Animation Playground button
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_6()
                            .py_4()
                            .bg(rgb(SURFACE))
                            .rounded_xl()
                            .border_l_4()
                            .border_color(rgb(BLUE))
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(TEXT))
                                    .child("Animation Playground"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(SUBTEXT))
                                    .child("Bouncing balls & particle effects"),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.go_to_animation_playground(cx);
                                }),
                            ),
                    )
                    // Shader Showcase button
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_6()
                            .py_4()
                            .bg(rgb(SURFACE))
                            .rounded_xl()
                            .border_l_4()
                            .border_color(rgb(MAUVE))
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(TEXT))
                                    .child("Shader Showcase"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(SUBTEXT))
                                    .child("Dynamic gradients & visual effects"),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.go_to_shader_showcase(cx);
                                }),
                            ),
                    )
                    // Text Editor button
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_6()
                            .py_4()
                            .bg(rgb(SURFACE))
                            .rounded_xl()
                            .border_l_4()
                            .border_color(rgb(GREEN))
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(TEXT))
                                    .child("Text Editor"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(SUBTEXT))
                                    .child("Text editing & cursor control"),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    this.go_to_text_editor(window, cx);
                                }),
                            ),
                    ),
            )
            // Footer
            .child(
                div()
                    .mt_8()
                    .text_sm()
                    .text_color(rgb(OVERLAY))
                    .child("Powered by GPUI"),
            )
            .into_any_element()
    }
}

impl DemoApp {
    fn render_animation_playground(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::AnyElement {
        // Request continuous animation frame
        window.request_animation_frame();

        // Update bounds
        let viewport = window.viewport_size();
        if let Some(playground) = &mut self.animation_playground {
            playground.set_bounds(Bounds {
                origin: point(0.0, 0.0),
                size: size(viewport.width.0, viewport.height.0),
            });
        }

        div()
            .size_full()
            .bg(rgb(BACKGROUND))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| {
                    this.handle_animation_touch_down(event, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| {
                    this.handle_animation_touch_up(event, cx);
                }),
            )
            .child(if let Some(playground) = &mut self.animation_playground {
                playground
                    .render_with_back_button(window, |_, _window, _cx| {
                        // Back button handled below
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(back_button(cx.listener(|this, _, _window, cx| {
                this.go_to_menu(cx);
            })))
            .into_any_element()
    }

    fn render_shader_showcase(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::AnyElement {
        // Request continuous animation frame
        window.request_animation_frame();

        // Update screen center
        if let Some(showcase) = &mut self.shader_showcase {
            let viewport = window.viewport_size();
            showcase.set_screen_center(point(viewport.width.0 / 2.0, viewport.height.0 / 2.0));
        }

        div()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| {
                    this.handle_shader_touch_down(event, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event, _window, cx| {
                this.handle_shader_touch_move(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event, _window, cx| {
                    this.handle_shader_touch_up(event, cx);
                }),
            )
            .child(if let Some(showcase) = &mut self.shader_showcase {
                showcase
                    .render_with_back_button(window, |_, _window, _cx| {
                        // Back button handled below
                    })
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(back_button(cx.listener(|this, _, _window, cx| {
                this.go_to_menu(cx);
            })))
            .into_any_element()
    }

    fn render_text_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::AnyElement {
        // Calculate viewport height for scroll handling (subtract header and footer)
        let viewport = window.viewport_size();
        let content_viewport_height = viewport.height.0 - 100.0 - 60.0; // HEADER_HEIGHT - FOOTER_HEIGHT

        // Store bounds for input handler
        let bounds = Bounds {
            origin: point(px(0.0), px(100.0)), // Below header
            size: size(viewport.width, viewport.height - px(160.0)), // Minus header and footer
        };
        self.text_editor_bounds = Some(bounds);

        // Get the entity for input handling
        let editor_entity = self.text_editor.clone();

        // Register the input handler for keyboard input
        if let Some(editor) = &self.text_editor {
            let focus = editor.read(cx).focus_handle(cx);
            let is_focused = focus.is_focused(window);
            println!("GPUI iOS: render_text_editor - focus_handle.is_focused = {}", is_focused);
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, editor.clone()),
                cx,
            );
            println!("GPUI iOS: render_text_editor - handle_input called");
        }

        div()
            .size_full()
            // Track focus for keyboard input
            .when_some(editor_entity.as_ref(), |div, editor| {
                let focus = editor.read(cx).focus_handle(cx);
                div.track_focus(&focus)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if let Some(editor) = &this.text_editor {
                        let pos = point(event.position.x.0, event.position.y.0);
                        editor.update(cx, |editor, _cx| {
                            editor.set_last_touch_y(pos.y);
                            editor.start_drag();
                            editor.handle_touch_down(pos, window);
                        });

                        // Focus the editor on touch
                        let focus = editor.read(cx).focus_handle(cx);
                        window.focus(&focus);

                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                if let Some(editor) = &this.text_editor {
                    let pos = point(event.position.x.0, event.position.y.0);
                    // Check if mouse button is pressed (dragging)
                    let is_dragging = event.pressed_button.is_some();
                    let is_selecting = editor.read(cx).is_selecting();

                    editor.update(cx, |editor, _cx| {
                        if is_dragging && is_selecting {
                            // Extend text selection
                            editor.handle_touch_move(pos, window);
                        } else {
                            // Scroll behavior (for two-finger scroll or when not selecting)
                            let delta_y = pos.y - editor.last_touch_y();
                            editor.set_last_touch_y(pos.y);
                            editor.handle_scroll(delta_y, content_viewport_height);
                        }
                    });
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseUpEvent, _window, cx| {
                    if let Some(editor) = &this.text_editor {
                        editor.update(cx, |editor, _cx| {
                            editor.end_selection();
                            editor.end_drag();
                            editor.update_momentum(content_viewport_height);
                        });
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _window, cx| {
                if let Some(editor) = &this.text_editor {
                    let delta_y = match event.delta {
                        ScrollDelta::Pixels(p) => p.y.0,
                        ScrollDelta::Lines(l) => l.y * 25.0,
                    };
                    editor.update(cx, |editor, _cx| {
                        editor.handle_scroll(delta_y, content_viewport_height);
                    });
                    cx.notify();
                }
            }))
            // Keyboard handling for hardware keyboards and key event fallback
            // Note: When insertText falls back to key events (because input handler is
            // temporarily unavailable during GPUI's rendering cycle), printable characters
            // come through here and need to be inserted into the editor.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                println!("GPUI iOS: on_key_down handler - key: {}, key_char: {:?}",
                    event.keystroke.key, event.keystroke.key_char);
                if let Some(editor) = &this.text_editor {
                    let key = event.keystroke.key.as_str();
                    let shift = event.keystroke.modifiers.shift;
                    let cmd = event.keystroke.modifiers.platform;
                    println!("GPUI iOS: on_key_down - processing key: {}", key);

                    editor.update(cx, |editor, _cx| {
                        match key {
                            "left" => {
                                if shift {
                                    editor.select_left();
                                } else {
                                    editor.move_left();
                                }
                            }
                            "right" => {
                                if shift {
                                    editor.select_right();
                                } else {
                                    editor.move_right();
                                }
                            }
                            "up" => editor.move_up(),
                            "down" => editor.move_down(),
                            // Note: backspace and delete are handled by UIKeyInput protocol
                            // (deleteBackward method in window.rs), not here.
                            // Handling them here would cause double-deletion.
                            "a" if cmd => editor.select_all(),
                            _ => {
                                // Handle printable characters (fallback from insertText when
                                // input handler is temporarily unavailable)
                                if let Some(key_char) = &event.keystroke.key_char {
                                    println!("GPUI iOS: on_key_down - inserting character via fallback: {:?}", key_char);
                                    editor.insert_text(key_char);
                                }
                            }
                        }
                    });
                    cx.notify();
                }
            }))
            .child(if let Some(editor) = &self.text_editor {
                editor.update(cx, |editor, _cx| {
                    editor
                        .render_with_back_button(window, |_, _window, _cx| {
                            // Back button handled below
                        })
                        .into_any_element()
                })
            } else {
                div().into_any_element()
            })
            .child(back_button(cx.listener(|this, _, _window, cx| {
                this.go_to_menu(cx);
            })))
            .into_any_element()
    }
}

/// Back button component for returning to menu
pub fn back_button<F>(on_click: F) -> impl IntoElement
where
    F: Fn(&(), &mut Window, &mut App) + 'static,
{
    div()
        .absolute()
        .top(px(50.0))
        .left(px(20.0))
        .px_4()
        .py_2()
        .bg(hsla(0.0, 0.0, 0.2, 0.8))
        .rounded_lg()
        .text_color(rgb(TEXT))
        .child("< Back")
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_click(&(), window, cx);
        })
}
