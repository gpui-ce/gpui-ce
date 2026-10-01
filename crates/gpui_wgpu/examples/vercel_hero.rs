//! A retained GPUI hero: typed material functions, one canvas, native text and controls.
//!
//! cargo run -p gpui_ce_wgpu --example vercel_hero --features test-support -- --headless
//! --output /tmp/hero.png --time 3 --pointer 720 350 --size 1280 720
//! macOS defaults to a headless screenshot because its native renderer is Metal.

use gpui::{
    AppContext, Bounds, Context, FontWeight, HeadlessAppContext, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, Size, StatefulInteractiveElement, Styled, Window, canvas, div,
    paint::{CanvasDrawing, Distance, PaintRoot, Parameter, Shader, ShaderError},
    point, px, rgb, size,
};
use gpui_ce_wgpu::{CosmicTextSystem, WgpuHeadlessRenderer};
use std::{path::PathBuf, sync::Arc, time::Instant};

/// Parameters belong to the material, independently of canvas geometry.
struct HeroMaterial {
    root: PaintRoot,
    shader: Shader,
    time: Parameter<f32>,
    pointer: Parameter<[f32; 2]>,
    intensity: Parameter<f32>,
    center: Parameter<[f32; 2]>,
    tint: Parameter<[f32; 4]>,
}
impl HeroMaterial {
    fn new() -> Result<Self, ShaderError> {
        let root = PaintRoot::new();
        // A distance to a finite edge; projection clamps correctly at both corners.
        let edge = root.function::<([f32; 2], [f32; 2], [f32; 2]), f32>(|cx, (p, a, b)| {
            let v = b - a.clone();
            let w = p - a;
            let t = (w.clone().dot(v.clone()) / v.clone().dot(v.clone()))
                .clamp(cx.scalar(0.0), cx.scalar(1.0));
            (w - v.scale(t)).length()
        })?;
        let triangle = root.function::<[f32; 2], Distance>(|cx, p| {
            let a = cx.vec2([0.0, -110.0]);
            let b = cx.vec2([-95.26, 55.0]);
            let c = cx.vec2([95.26, 55.0]);
            let d = cx
                .call(&edge, (p.clone(), a.clone(), b.clone()))
                .min(cx.call(&edge, (p.clone(), b, c.clone())))
                .min(cx.call(&edge, (p.clone(), c, a)));
            let outside = (p.clone().x().abs() * cx.scalar(0.8660254)
                - p.clone().y() * cx.scalar(0.5)
                - cx.scalar(55.0))
            .max(p.y() - cx.scalar(55.0))
            .gt(cx.scalar(0.0));
            Distance::new(outside.select(d.clone(), -d))
        })?;
        // A compact inverse-square falloff, reusable with any distance or radius.
        let glow = root.function::<(f32, f32), f32>(|cx, (distance, radius)| {
            let x = distance / radius;
            cx.scalar(1.0) / (cx.scalar(1.0) + x.clone() * x)
        })?;
        let grain = root.function::<([f32; 2], f32), f32>(|cx, (p, phase)| {
            (p.dot(cx.vec2([12.9898, 78.233])) + phase)
                .sin()
                .scale(cx.scalar(43758.547))
                .fract()
        })?;
        let time = root.parameter(0.0)?;
        let pointer = root.parameter([0.0, 0.0])?;
        let intensity = root.parameter(1.0)?;
        let center = root.parameter([640.0, 422.0])?;
        let tint = root.parameter([0.94, 0.97, 1.0, 1.0])?;
        let shader = root.shader(|cx| {
            // Centered logical pixels preserve an equilateral triangle at every size.
            let p = cx.position() - cx.uniform(center);
            let d = cx.call(&triangle, p.clone());
            let phase = cx.uniform(time);
            let pointer = cx.uniform(pointer).scale(cx.scalar(0.14));
            let light = cx.xy(
                phase.clone().sin() * cx.scalar(20.0) - cx.scalar(65.0),
                phase.clone().cos() * cx.scalar(12.0) - cx.scalar(60.0),
            ) + pointer;
            let illumination = cx.call(&glow, ((p.clone() - light).length(), cx.scalar(95.0)));
            let halo = cx.call(&glow, (d.expression().abs(), cx.scalar(18.0)))
                * illumination.clone()
                * cx.scalar(0.46);
            let rim = cx.call(&glow, (d.expression().abs(), cx.scalar(1.5)))
                * illumination
                * cx.scalar(0.75);
            let floor = cx.call(
                &glow,
                ((p.clone().y() - cx.scalar(60.0)).abs(), cx.scalar(1.5)),
            ) * cx.call(&glow, (p.clone().x().abs(), cx.scalar(130.0)))
                * cx.scalar(0.36);
            let mist = cx.call(
                &glow,
                ((p - cx.vec2([-35.0, 42.0])).length(), cx.scalar(110.0)),
            ) * cx.scalar(0.025);
            let exterior = cx.scalar(1.0) - d.coverage().expression();
            let noise = cx.call(&grain, (cx.position(), phase * cx.scalar(0.1))) * cx.scalar(0.006);
            let value = ((halo + rim + floor + mist) * exterior * cx.uniform(intensity) + noise)
                .clamp(cx.scalar(0.0), cx.scalar(1.0));
            let tint = cx.uniform(tint);
            cx.rgba(
                value.clone() * tint.clone().x(),
                value.clone() * tint.clone().y(),
                value * tint.z(),
                cx.scalar(1.0),
            )
        })?;
        Ok(Self {
            root,
            shader,
            time,
            pointer,
            intensity,
            center,
            tint,
        })
    }
    fn drawing(&self, viewport: Size<Pixels>) -> Result<CanvasDrawing, ShaderError> {
        self.root.canvas(|canvas| {
            canvas
                .rect(Bounds::new(point(px(0.0), px(0.0)), viewport))
                .fill(&self.shader);
        })
    }
}

struct Hero {
    material: HeroMaterial,
    drawing: Option<(Size<Pixels>, CanvasDrawing)>,
    started: Instant,
    time: f32,
    pointer: [f32; 2],
    animate: bool,
    bright: bool,
}
impl Hero {
    fn new(options: &Options, animate: bool) -> Self {
        Self {
            material: HeroMaterial::new().expect("valid hero material"),
            drawing: None,
            started: Instant::now(),
            time: options.time,
            pointer: options.pointer,
            animate,
            bright: true,
        }
    }
}
impl Render for Hero {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        if self.animate {
            window.request_animation_frame();
        }
        if self
            .drawing
            .as_ref()
            .is_none_or(|(previous, _)| *previous != viewport)
        {
            self.drawing = Some((
                viewport,
                self.material.drawing(viewport).expect("valid canvas"),
            ));
        }
        let time = self.time
            + if self.animate {
                self.started.elapsed().as_secs_f32() * 0.35
            } else {
                0.0
            };
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let compact = width < 1000.0;
        let center = if compact {
            [width * 0.76, height * 0.56]
        } else {
            [width * 0.5, height * 0.5 + 62.0]
        };
        let drawing = self
            .drawing
            .as_ref()
            .unwrap()
            .1
            .with_parameters(|p| {
                p.set(self.material.time, time);
                p.set(self.material.center, center);
                p.set(self.material.pointer, self.pointer);
                p.set(self.material.intensity, if self.bright { 1.0 } else { 0.3 });
                p.set(self.material.tint, [0.94, 0.97, 1.0, 1.0]);
            })
            .expect("valid live parameters");
        let headline_size = if compact { 40.0 } else { 60.0 };
        div()
            .relative()
            .size_full()
            .bg(rgb(0x000000))
            .text_color(rgb(0xffffff))
            .font_family("sans-serif")
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    this.pointer = [
                        f32::from(event.position.x) - width * 0.5,
                        f32::from(event.position.y) - height * 0.5,
                    ];
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        drawing
                            .paint_at(bounds.origin, window)
                            .expect("renderer supports typed paints");
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.0))
                    .top(px(20.0))
                    .text_size(px(25.0))
                    .child("▲"),
            )
            .child(
                div()
                    .absolute()
                    .left(px(72.0))
                    .top(px(25.0))
                    .flex()
                    .gap(px(28.0))
                    .text_size(px(14.0))
                    .child("GPUI  /  Graphics laboratory"),
            )
            .child(
                div()
                    .absolute()
                    .right(px(24.0))
                    .top(px(25.0))
                    .text_size(px(14.0))
                    .child(if compact {
                        "01 / Hero"
                    } else {
                        "01 / Interactive material study"
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(if compact { 24.0 } else { width * 0.5 - 122.0 }))
                    .top(px(if compact { 70.0 } else { 115.0 }))
                    .text_size(px(13.0))
                    .text_color(rgb(0xa1a1a1))
                    .child("GPUI + WebGPU  /  Composable by design"),
            )
            .child(
                div()
                    .absolute()
                    .left(px(24.0))
                    .top(px(height * 0.5 - if compact { 140.0 } else { 60.0 }))
                    .flex()
                    .flex_col()
                    .text_size(px(headline_size))
                    .line_height(px(headline_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Composable")
                    .child("Graphics")
                    .child(
                        div()
                            .mt(px(27.0))
                            .flex()
                            .gap(px(12.0))
                            .text_size(px(14.0))
                            .line_height(px(20.0))
                            .child(
                                div()
                                    .id("light")
                                    .px(px(24.0))
                                    .py(px(12.0))
                                    .rounded_full()
                                    .bg(rgb(0xffffff))
                                    .text_color(rgb(0x000000))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.bright = !this.bright;
                                        cx.notify();
                                    }))
                                    .child(if self.bright {
                                        "Dim light"
                                    } else {
                                        "Brighten light"
                                    }),
                            )
                            .child(
                                div()
                                    .id("motion")
                                    .px(px(24.0))
                                    .py(px(12.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(0x383838))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.animate {
                                            this.time +=
                                                this.started.elapsed().as_secs_f32() * 0.35;
                                        }
                                        this.started = Instant::now();
                                        this.animate = !this.animate;
                                        cx.notify();
                                    }))
                                    .child(if self.animate {
                                        "Pause motion"
                                    } else {
                                        "Resume motion"
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(if compact { 24.0 } else { width * 0.71 }))
                    .top(px(height * 0.5 + if compact { 90.0 } else { 3.0 }))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .text_size(px(if compact { 16.0 } else { 18.0 }))
                    .line_height(px(25.0))
                    .children(["Typed functions.", "Retained canvas.", "Shared primitives."]),
            )
    }
}

struct Options {
    output: PathBuf,
    time: f32,
    pointer: [f32; 2],
    viewport: Size<Pixels>,
    headless: bool,
}
impl Options {
    fn parse() -> anyhow::Result<Self> {
        let mut options = Self {
            output: "/tmp/gpui-vercel-hero.png".into(),
            time: 0.0,
            pointer: [0.0, 0.0],
            viewport: size(px(1280.0), px(720.0)),
            headless: false,
        };
        let mut pointer = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => options.headless = true,
                "--output" => {
                    options.output = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing output path"))?
                        .into()
                }
                "--time" => {
                    options.time = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("missing time"))?
                        .parse()?
                }
                "--pointer" => {
                    pointer = Some([
                        args.next()
                            .ok_or_else(|| anyhow::anyhow!("missing pointer x"))?
                            .parse::<f32>()?,
                        args.next()
                            .ok_or_else(|| anyhow::anyhow!("missing pointer y"))?
                            .parse::<f32>()?,
                    ]);
                }
                "--size" => {
                    options.viewport = size(
                        px(args
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("missing width"))?
                            .parse()?),
                        px(args
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("missing height"))?
                            .parse()?),
                    )
                }
                _ => anyhow::bail!("unknown argument: {arg}"),
            }
        }
        if let Some([x, y]) = pointer {
            options.pointer = [
                x - f32::from(options.viewport.width) * 0.5,
                y - f32::from(options.viewport.height) * 0.5,
            ];
        }
        anyhow::ensure!(
            f32::from(options.viewport.width).is_finite()
                && f32::from(options.viewport.height).is_finite(),
            "viewport must be finite"
        );
        anyhow::ensure!(
            options.time.is_finite() && options.pointer.into_iter().all(f32::is_finite),
            "parameters must be finite"
        );
        anyhow::ensure!(
            f32::from(options.viewport.width) >= 640.0
                && f32::from(options.viewport.height) >= 480.0,
            "viewport must be at least 640 × 480"
        );
        Ok(options)
    }
}
fn headless(options: &Options) -> anyhow::Result<()> {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(CosmicTextSystem::new("sans-serif")),
        Arc::new(()),
        || Some(Box::new(WgpuHeadlessRenderer::new().expect("GPU renderer"))),
    );
    let window = cx.open_window(options.viewport, |_, cx| {
        cx.new(|_| Hero::new(options, false))
    })?;
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })?;
    cx.capture_screenshot(window.into())?
        .save(&options.output)?;
    println!("Saved {}", options.output.display());
    Ok(())
}
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn native(options: Options) {
    gpui_platform::application().run(move |cx: &mut gpui::App| {
        cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::centered(
                    None,
                    options.viewport,
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Hero::new(&options, true)),
        )
        .expect("hero window");
        cx.activate(true);
    });
}
fn main() -> anyhow::Result<()> {
    let options = Options::parse()?;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if !options.headless {
        native(options);
        return Ok(());
    }
    headless(&options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    };

    #[test]
    fn real_window_routes_pointer_and_buttons_and_repaints_retained_material() {
        let options = Options {
            output: "/tmp/unused-hero-test.png".into(),
            time: 0.0,
            pointer: [0.0, 0.0],
            viewport: size(px(1280.0), px(720.0)),
            headless: true,
        };
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(CosmicTextSystem::new("sans-serif")),
            Arc::new(()),
            || Some(Box::new(WgpuHeadlessRenderer::new().expect("GPU renderer"))),
        );
        let mut entity = None;
        let window = cx
            .open_window(options.viewport, |_, cx| {
                let view = cx.new(|_| Hero::new(&options, false));
                entity = Some(view.clone());
                view
            })
            .unwrap();
        let entity = entity.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let program = cx.update(|cx| entity.read(cx).material.shader.program_id());
        let before_pointer = cx.capture_screenshot(window.into()).unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(950.0), px(220.0)),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
        assert_eq!(cx.update(|cx| entity.read(cx).pointer), [310.0, -140.0]);
        let lit = cx.capture_screenshot(window.into()).unwrap();
        // These pixels contain only the procedural material, with no text or controls.
        // Require a visible channel delta across many pixels, rather than counting
        // harmless readback noise or a changed button label as successful repainting.
        fn changed_material_pixels(before: &image::RgbaImage, after: &image::RgbaImage) -> usize {
            assert_eq!(before.dimensions(), after.dimensions());
            // Headless windows retain their platform's DPI. Map the logical ROI
            // to readback pixels, so this remains the same material region at 1×/2×.
            let left = before.width() * 500 / 1280;
            let right = before.width() * 780 / 1280;
            let top = before.height() * 240 / 720;
            let bottom = before.height() * 540 / 720;
            (top..bottom)
                .flat_map(|y| (left..right).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let a = before.get_pixel(x, y);
                    let b = after.get_pixel(x, y);
                    (0..3).any(|channel| a[channel].abs_diff(b[channel]) >= 3)
                })
                .count()
        }
        assert!(
            changed_material_pixels(&before_pointer, &lit) > 1000,
            "pointer movement must visibly change the triangle lighting"
        );
        fn click(
            cx: &mut HeadlessAppContext,
            window: gpui::AnyWindowHandle,
            position: gpui::Point<Pixels>,
        ) {
            cx.update_window(window, |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent {
                        position,
                        button: MouseButton::Left,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }),
                    cx,
                );
                window.draw(cx).clear(cx);
            })
            .unwrap();
        }
        click(&mut cx, window.into(), point(px(90.0), px(470.0)));
        assert!(!cx.update(|cx| entity.read(cx).bright));
        let dimmed = cx.capture_screenshot(window.into()).unwrap();
        assert!(
            changed_material_pixels(&lit, &dimmed) > 4000,
            "the light control must dim the material, independently of its label"
        );
        click(&mut cx, window.into(), point(px(260.0), px(470.0)));
        cx.update_window(window.into(), |_, window, cx| {
            assert!(window.simulate_next_frame(cx) > 0);
        })
        .unwrap();
        assert!(cx.update(|cx| entity.read(cx).animate));
        click(&mut cx, window.into(), point(px(260.0), px(470.0)));
        assert!(!cx.update(|cx| entity.read(cx).animate));
        assert_eq!(
            program,
            cx.update(|cx| entity.read(cx).material.shader.program_id())
        );
        assert_eq!(
            1,
            cx.update(|cx| entity
                .read(cx)
                .drawing
                .as_ref()
                .unwrap()
                .1
                .primitive_count())
        );
    }
}
