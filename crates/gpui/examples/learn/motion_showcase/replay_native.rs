//! Opt-in Windows check of the production animation wrapper and native input.
use gpui::{prelude::FluentBuilder, *};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, rc::Rc, time::Duration};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE},
};

struct NativeNotification {
    event: usize,
    clicks: usize,
    renders: usize,
    emphasis: Rc<Cell<f32>>,
    focus: FocusHandle,
}

impl Render for NativeNotification {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let emphasis = self.emphasis.clone();
        div()
            .size_full()
            .bg(rgb(0x101820))
            .text_color(rgb(0xffffff))
            .when(self.event != 0, |root| {
                root.child(
                    div()
                        .id("notification")
                        .relative()
                        .w(px(300.0))
                        .h(px(140.0))
                        .p_3()
                        .border_1()
                        .child("Build finished successfully")
                        .child(
                            div()
                                .id("action")
                                .absolute()
                                .top(px(60.0))
                                .left(px(20.0))
                                .w(px(120.0))
                                .h(px(40.0))
                                .bg(rgb(0x365587))
                                .track_focus(&self.focus)
                                .tab_index(0)
                                .role(Role::Button)
                                .child("Open activity")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.clicks += 1;
                                    cx.notify();
                                })),
                        )
                        .with_animation(
                            "stable-notification",
                            Animation::new(
                                Motion::new(millis(200))
                                    .with_reverse_pass(MotionPass::new(millis(300)))
                                    .iterations(2)
                                    .alternate()
                                    .with_easing(ease_in_out),
                            ),
                            move |row, value| {
                                emphasis.set(value);
                                row.bg(rgb(0x202b3d).lerp(&rgb(0x4875b0), value))
                                    .border_color(rgb(0x35435a).lerp(&rgb(0x9bbcff), value))
                            },
                        )
                        .replay_on(self.event),
                )
            })
    }
}

fn refresh(window: &WindowHandle<NativeNotification>, cx: &mut AsyncApp) {
    cx.update_window((*window).into(), |_, w, _| {
        w.refresh();
    })
    .unwrap();
}

pub(super) fn run() {
    gpui_platform::application().run(|cx| {
        let bounds = Bounds::centered(None, size(px(480.0), px(240.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    show: true,
                    focus: false,
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |w, cx| {
                    w.set_window_title("GPUI replay acceptance");
                    cx.new(|cx| NativeNotification {
                        event: 0,
                        clicks: 0,
                        renders: 0,
                        emphasis: Rc::new(Cell::new(0.0)),
                        focus: cx.focus_handle(),
                    })
                },
            )
            .unwrap();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(millis(100)).await;
            for event in [1usize, 2] {
                window.update(cx, |v, _, cx| { v.event = event; cx.notify(); }).unwrap();
                refresh(&window, cx);
                window.update(cx, |v, w, cx| w.focus(&v.focus, cx)).unwrap();
                cx.background_executor().timer(millis(100)).await;
                refresh(&window, cx);
                window.update(cx, |v, w, _| {
                    assert!(v.emphasis.get() > 0.1 && v.emphasis.get() < 1.0);
                    assert!(v.focus.is_focused(w));
                }).unwrap();
                let (hwnd, scale) = window.update(cx, |_, w, _| {
                    let RawWindowHandle::Win32(raw) = HasWindowHandle::window_handle(w).unwrap().as_raw() else {
                        panic!("native Windows handle");
                    };
                    (HWND(raw.hwnd.get() as *mut _), w.scale_factor())
                }).unwrap();
                let x = (70.0 * scale).round() as u32;
                let y = (80.0 * scale).round() as u32;
                let position = LPARAM(((y << 16) | x) as isize);
                unsafe {
                    PostMessageW(Some(hwnd), WM_MOUSEMOVE, WPARAM(0), position).unwrap();
                    PostMessageW(Some(hwnd), WM_LBUTTONDOWN, WPARAM(1), position).unwrap();
                    PostMessageW(Some(hwnd), WM_LBUTTONUP, WPARAM(0), position).unwrap();
                }
                cx.background_executor().timer(millis(550)).await;
                refresh(&window, cx);
                window.update(cx, |v, w, _| {
                    assert_eq!(v.clicks, event);
                    assert_eq!(v.emphasis.get(), 0.0);
                    let physical = w.viewport_size().scale(w.scale_factor());
                    eprintln!("native DirectX replay: event={event}, clicks={}, {}x{} physical, scale={}",
                        v.clicks, physical.width.0, physical.height.0, w.scale_factor());
                }).unwrap();
                cx.background_executor().timer(millis(150)).await;
            }
            window.update(cx, |v, _, cx| { v.event = 3; cx.notify(); }).unwrap();
            refresh(&window, cx);
            cx.background_executor().timer(millis(80)).await;
            cx.update(|cx| cx.set_reduce_motion(true));
            refresh(&window, cx);
            cx.background_executor().timer(millis(50)).await;
            cx.update(|cx| cx.set_reduce_motion(false));
            refresh(&window, cx);
            cx.background_executor().timer(millis(50)).await;
            window.update(cx, |v, _, _| assert_eq!(v.emphasis.get(), 0.0)).unwrap();
            window.update(cx, |v, _, cx| { v.event = 4; cx.notify(); }).unwrap();
            refresh(&window, cx);
            cx.background_executor().timer(millis(100)).await;
            window.update(cx, |v, _, _| assert!(v.emphasis.get() > 0.0)).unwrap();
            cx.update(|cx| {
                cx.set_reduce_motion(true);
                cx.set_reduce_motion(false);
            });
            refresh(&window, cx);
            cx.background_executor().timer(millis(50)).await;
            window.update(cx, |v, _, _| assert_eq!(v.emphasis.get(), 0.0)).unwrap();
            cx.background_executor().timer(millis(100)).await;
            let renders = window.update(cx, |v, _, _| v.renders).unwrap();
            cx.background_executor().timer(Duration::from_millis(600)).await;
            window.update(cx, |v, _, _| assert_eq!(v.renders, renders, "no idle redraw")).unwrap();
            eprintln!("native replay, action, focus, sustained/transient reduced-motion and idle checks passed");
            cx.update(|cx| cx.quit());
        }).detach();
    });
}
