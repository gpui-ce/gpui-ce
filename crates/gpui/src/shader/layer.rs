//! An element painted by a shader that reacts to the pointer and to time.

use std::{cell::RefCell, rc::Rc, time::Instant};

use refineable::Refineable as _;
use wgsl_rs::std::Vec2f;

use crate::{
    App, Bounds, DispatchPhase, Element, ElementId, Fill, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Style, StyleRefinement, Styled, Window,
};

use super::Paint;

/// A [`layer`]'s current inputs. Values become uniforms in the paint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Seconds since the layer first painted.
    pub time: f32,
    /// Logical-pixel size of the layer.
    pub size: Vec2f,
    /// Eased pointer position in logical pixels from the top-left corner.
    /// Starts at the center and retains its last position when the pointer leaves.
    pub pointer: Vec2f,
    /// Hover amount, eased between `0` outside and `1` inside the layer.
    pub hover: f32,
    /// Whether a mouse button went down on the layer and is still held.
    pub pressed: bool,
}

/// An element filled with a paint that is rebuilt from [`Input`] every frame:
/// the pointer, hover, presses, and time.
///
/// Supports ordinary element styles, including borders and shadows. Use an
/// absolutely positioned layer to paint behind sibling content.
///
/// ```ignore
/// shader::layer("spotlight", |input| {
///     paint(|px| {
///         let distance = (px.position() - input.pointer).length();
///         let glow = (1.0 - distance / 240.0).max(0.0) * input.hover;
///         color(white()).mask(glow).over(color(black()))
///     })
/// })
/// .size_full()
/// ```
pub fn layer(id: impl Into<ElementId>, paint: impl Fn(&Input) -> Paint + 'static) -> Layer {
    Layer {
        id: id.into(),
        paint: Box::new(paint),
        animate: false,
        style: StyleRefinement::default(),
    }
}

/// A [`layer`] element.
pub struct Layer {
    id: ElementId,
    paint: Box<dyn Fn(&Input) -> Paint>,
    animate: bool,
    style: StyleRefinement,
}

impl Layer {
    /// Repaint every frame, so paints can animate with [`Input::time`].
    /// Otherwise the layer repaints only while the pointer or hover moves.
    pub fn animate(mut self) -> Self {
        self.animate = true;
        self
    }
}

/// Input state that persists across frames.
struct Tracking {
    started: Instant,
    last_frame: Instant,
    pointer: Option<Vec2f>,
    hover: f32,
    pressed: bool,
}

impl Tracking {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_frame: now,
            pointer: None,
            hover: 0.0,
            pressed: false,
        }
    }

    /// Advance the easing towards the pointer, returning the frame's input and
    /// whether easing is still in motion.
    fn advance(&mut self, size: Vec2f, target: Option<Vec2f>) -> (Input, bool) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        // Exponential easing, independent of frame rate.
        let blend = 1.0 - (-elapsed * 14.0).exp();
        let pointer = self.pointer.get_or_insert(size * 0.5);
        if let Some(target) = target {
            *pointer = *pointer + (target - *pointer) * blend;
        }
        let hover_target = if target.is_some() { 1.0 } else { 0.0 };
        self.hover += (hover_target - self.hover) * blend;

        let gap = target.map_or(0.0, |target| {
            let delta = target - *pointer;
            delta.x.abs().max(delta.y.abs())
        });
        let moving = gap > 0.05 || (hover_target - self.hover).abs() > 0.002;
        if !moving {
            self.hover = hover_target;
        }
        let input = Input {
            time: now.duration_since(self.started).as_secs_f32(),
            size,
            pointer: *pointer,
            hover: self.hover,
            pressed: self.pressed,
        };
        (input, moving)
    }
}

impl IntoElement for Layer {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Layer {
    type RequestLayoutState = Style;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Style) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style.clone(), [], cx), style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _style: &mut Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> Hitbox {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let id = id.expect("layers have ids");
        let tracking = window.with_element_state(id, |tracking, _| {
            let tracking: Rc<RefCell<Tracking>> =
                tracking.unwrap_or_else(|| Rc::new(RefCell::new(Tracking::new())));
            (tracking.clone(), tracking)
        });

        let local = |position: Point<Pixels>| {
            let offset = position - bounds.origin;
            Vec2f {
                x: f32::from(offset.x),
                y: f32::from(offset.y),
            }
        };
        let size = Vec2f {
            x: f32::from(bounds.size.width),
            y: f32::from(bounds.size.height),
        };
        let target = hitbox
            .is_hovered(window)
            .then(|| local(window.mouse_position()));
        let (input, moving) = tracking.borrow_mut().advance(size, target);
        if self.animate || moving {
            window.request_animation_frame();
        }

        style.background = Some(Fill::Shader((self.paint)(&input)));
        style.paint(bounds, window, cx, |_, _| {});

        let hovered = target.is_some();
        let moved = hitbox.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
            // Follow the pointer over the layer, and notice it leaving.
            if phase == DispatchPhase::Bubble && (hovered || moved.is_hovered(window)) {
                window.refresh();
            }
        });
        let pressing = tracking.clone();
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |_: &MouseDownEvent, phase, window, _| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                pressing.borrow_mut().pressed = true;
                window.refresh();
            }
        });
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
            let mut tracking = tracking.borrow_mut();
            if phase == DispatchPhase::Bubble && tracking.pressed {
                tracking.pressed = false;
                window.refresh();
            }
        });
    }
}

impl Styled for Layer {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
