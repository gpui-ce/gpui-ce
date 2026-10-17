use super::WgpuHeadlessRenderer;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, Context, HeadlessAppContext, InteractiveElement, Lerp, Modifiers,
    Motion, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, Render, Window, WindowHandle,
    div, millis, point, px, rgb, size,
};
use gpui::{DevicePixels, PlatformAtlas, PlatformHeadlessRenderer, Scene, Size};
use std::{cell::Cell, rc::Rc, sync::Arc};

struct FixtureRenderer {
    inner: WgpuHeadlessRenderer,
    density: Rc<Cell<u32>>,
}

impl FixtureRenderer {
    fn physical_size(&self) -> Size<DevicePixels> {
        size(
            DevicePixels(160 * self.density.get() as i32),
            DevicePixels(100 * self.density.get() as i32),
        )
    }
}

impl PlatformHeadlessRenderer for FixtureRenderer {
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.inner.sprite_atlas()
    }
    fn render_scene(&mut self, scene: &Scene, _: Size<DevicePixels>) -> anyhow::Result<()> {
        self.inner.render_scene(scene, self.physical_size())
    }
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        _: Size<DevicePixels>,
    ) -> anyhow::Result<image::RgbaImage> {
        self.inner
            .render_scene_to_image(scene, self.physical_size())
    }
}

struct Notification {
    event: usize,
    clicks: usize,
}

impl Render for Notification {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x101820))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .id("notification-row")
                    .relative()
                    .w(px(120.0))
                    .h(px(80.0))
                    .border_1()
                    .border_color(rgb(0xffffff))
                    .child("Build finished")
                    .child(
                        div()
                            .id("action")
                            .absolute()
                            .left(px(10.0))
                            .top(px(25.0))
                            .w(px(100.0))
                            .h(px(30.0))
                            .bg(rgb(0x32445b))
                            .child("Open activity")
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.clicks += 1;
                                cx.notify();
                            })),
                    )
                    .with_animation(
                        "stable-animation",
                        Animation::new(Motion::new(millis(100)).iterations(2).alternate()),
                        |row, emphasis| row.bg(rgb(0x000000).lerp(&rgb(0x0080ff), emphasis)),
                    )
                    .replay_on(self.event),
            )
    }
}

fn capture(cx: &mut HeadlessAppContext, window: WindowHandle<Notification>) -> image::RgbaImage {
    cx.update_window(window.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.capture_screenshot(window.into()).unwrap()
}

#[test]
fn replayed_notification_pixels_and_action_at_one_and_two_times_density() {
    // Window's test scale override does not change TestPlatform's display scale.
    // Use explicit physical fixture sizes through the existing renderer seam.
    let density_setting = Rc::new(Cell::new(1));
    let renderer_density = density_setting.clone();
    let mut cx = HeadlessAppContext::with_platform(
        gpui_platform::current_platform(true).text_system(),
        Arc::new(()),
        move || {
            Some(Box::new(FixtureRenderer {
                inner: WgpuHeadlessRenderer::new().expect("headless adapter"),
                density: renderer_density.clone(),
            }))
        },
    );
    let window = cx
        .open_window(size(px(160.0), px(100.0)), |_, cx| {
            cx.new(|_| Notification {
                event: 1,
                clicks: 0,
            })
        })
        .unwrap();
    for density in [1u32, 2] {
        density_setting.set(density);
        window
            .update(&mut cx, |v, w, _| {
                w.set_scale_factor(density as f32);
                v.event = density as usize * 10;
            })
            .unwrap();
        let resting = capture(&mut cx, window);
        let pixel = |image: &image::RgbaImage| image.get_pixel(110 * density, 70 * density).0;
        assert_eq!(resting.dimensions(), (160 * density, 100 * density));
        assert_eq!(pixel(&resting), [0, 0, 0, 255]);
        cx.advance_clock(millis(50));
        let intermediate = capture(&mut cx, window);
        assert!(pixel(&intermediate)[2] > 20 && pixel(&intermediate)[2] < 240);
        window.update(&mut cx, |v, _, _| v.event += 1).unwrap();
        assert_eq!(pixel(&capture(&mut cx, window)), [0, 0, 0, 255]);
        cx.advance_clock(millis(100));
        let emphasized = capture(&mut cx, window);
        assert!(pixel(&emphasized)[2] > 245);
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: point(px(30.0), px(40.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            w.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    position: point(px(30.0), px(40.0)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        })
        .unwrap();
        assert_eq!(
            window.update(&mut cx, |v, _, _| v.clicks).unwrap(),
            density as usize
        );
        cx.advance_clock(millis(100));
        let completed = capture(&mut cx, window);
        assert_eq!(pixel(&completed), [0, 0, 0, 255]);
        assert_eq!(
            completed, resting,
            "completed pixels return exactly to rest"
        );
        if let Some(directory) = std::env::var_os("GPUI_MOTION_FIXTURE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            emphasized
                .save(
                    std::path::PathBuf::from(&directory)
                        .join(format!("replay-peak-{density}x.png")),
                )
                .unwrap();
            completed
                .save(
                    std::path::PathBuf::from(directory).join(format!("replay-rest-{density}x.png")),
                )
                .unwrap();
        }
    }
}
