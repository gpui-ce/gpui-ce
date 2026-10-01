# Composable paint: current API

`gpui::paint` is an experimental, device-independent composition API. A `PaintRoot`
constructs typed functions, parameters, procedural materials and retained canvas
drawings. The WGPU renderer realizes these descriptions on its own device.

## Declare, compose, draw, update

```rust
use gpui::{Bounds, point, px, size};
use gpui::paint::{PaintRoot, ShaderError};

fn drawing() -> Result<gpui::paint::CanvasDrawing, ShaderError> {
    let root = PaintRoot::new();
    let tint = root.parameter([0.2, 0.5, 1.0, 1.0])?;
    let phase = root.parameter(0.0_f32)?;

    // A reusable function has explicit arguments; it captures no uniform bindings.
    let waves = root.function::<([f32; 2], f32, [f32; 4]), [f32; 4]>(|s, (uv, phase, tint)| {
        let wave = (uv.x() * s.scalar(12.0) + phase).sin();
        let amount = wave * s.scalar(0.5) + s.scalar(0.5);
        tint.mix(s.vec4([1.0, 0.3, 0.1, 1.0]), amount)
    })?;

    let fill = root.shader(|s| s.call(&waves, (s.uv(), s.uniform(phase), s.uniform(tint))))?;

    // A coordinate change is just another argument. Function calls fuse into the
    // shader; they allocate no intermediate texture and launch no extra draw.
    let mirrored = root.shader(|s| {
        let uv = s.xy(s.scalar(1.0) - s.uv().x(), s.uv().y());
        s.call(&waves, (uv, s.uniform(phase), s.uniform(tint)))
    })?;

    let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(180.0), px(96.0)));
    let drawing = root.canvas(|c| {
        c.rounded_rect(bounds, px(16.0))
            .fill(&fill)
            .stroke(&mirrored, px(4.0));
    })?;

    // Updates every occurrence, including inside imported materials and masks.
    // The original drawing and compiled programs remain intact.
    drawing.with_parameters(|p| {
        p.set(phase, 0.75);
        p.set(tint, [0.8, 0.3, 0.2, 1.0]);
    })
}
```

Call `drawing.paint_at(bounds.origin, window)` during the existing `gpui::canvas`
paint callback. The drawing uses logical coordinates; the window supplies the
display scale, inherited opacity and content clip. Handle its `Result` or choose
an explicit fallback using `window.supports_shader_paint()`.

The supported window implementations are WGPU-backed Linux/FreeBSD and web
windows. Native macOS Metal and Windows D3D11 window renderers currently return
`UnsupportedBackend`. Headless WGPU works on a supported native WGPU adapter,
including Metal; that is separate from the native macOS window renderer.

## Geometry is reusable shader code

`Distance` describes negative-inside geometry. Its `union`, `intersect`,
`subtract`, `smooth_union`, `offset`, `inset`, `dilate`, `inward_stroke` and
`centered_stroke` operations produce geometry. `coverage()` converts it into
the separate `Coverage` type. `Coverage::mask` changes alpha once, preserving
straight RGB.

```rust
use gpui::paint::{Distance, PaintRoot, Shader, ShaderError};

fn cutout(root: &PaintRoot) -> Result<Shader, ShaderError> {
    let disc = root.function::<([f32; 2], f32), Distance>(|s, (point, radius)| {
        s.circle_field(point, s.vec2([0.0, 0.0]), radius)
    })?;
    root.shader(|s| {
        let outer = s.call(&disc, (s.position() - s.vec2([32.0, 32.0]), s.scalar(28.0)));
        let hole = s.call(&disc, (s.position() - s.vec2([42.0, 32.0]), s.scalar(18.0)));
        outer.subtract(hole)
            .coverage()
            .mask(s.vec4([0.1, 0.8, 0.9, 1.0]))
    })
}
```

The same function abstraction supports scalars, vectors, semantic values and
nested tuples. Returning both a distance and a color does not create two graphs.
Taking a `Distance` or `Coverage` argument preserves that semantic type. A
function returning coordinates is a reusable warp; a function returning color
can be called with different phases and tints in the same material.

`Shape::field` uses the same geometry recipe as canvas shape masks. A fixed shape
contributes constants; `Shape::field_at` accepts an explicit local point, including
inside a reusable function. Canvas recording supplies size, radius and border
width as uniform data so differently sized canvas shapes share a compiled program.
Procedural borders use one draw. They still rasterize their bounding rectangle;
they are not yet optimized into a border mesh.

Zero and negative animated field stroke widths produce empty coverage. The
canvas builder rejects negative static widths and skips zero-width strokes.
Custom distance fields must use consistent units; the type distinguishes a
distance from opacity but cannot prove an arbitrary expression is a true metric.

## Composition and ownership rules

- Functions are reusable recipes with explicit inputs. Calls use the same graph
  primitives as handwritten expressions, including common-expression sharing
  and removal of unused outputs. Reachable implicit fragment coordinates and
  captured uniforms are rejected when building a function; pass them as arguments.
  Functions can be called under a different root. Derivative use is tracked and
  currently executes in fragment shaders only.
- `sample` imports another material in the same domain. `sample_uv` changes only
  its normalized coordinates; `sample_at` changes logical position and size and
  derives normalized coordinates from them. A zero sample size is undefined
  arithmetic and remains the author's responsibility.
- `map_shader`/`Shader::map` transform the result of a completed material.
- Root parameters are typed immutable defaults. `with_parameters` applies a batch
  to a shader or drawing atomically, copying each affected value table once.
  Repeated keys use the last value; unchanged batches share the original storage.
  `with_parameter` is the single-value convenience. A drawing accepts a handle
  present in any of its paints; a shader requires every handle to be reachable.
- Importing two instances with the same logical parameter and different bound
  values is an `AmbiguousParameter` error. Declare separate parameters for
  independently controlled inputs, or call one function with different explicit
  arguments. An imported instance binding overrides a
  bare root default regardless of construction order.
- Types reject scalar/vector mistakes. Graph construction rejects foreign
  expressions, missing parameter ownership, non-finite supplied values and
  excessive graph sizes. Runtime expressions can still divide by zero or
  overflow; this is a typed language, not a numerical theorem prover.
- Generated expressions are a structured DAG. Reachable nodes and packed
  uniforms receive canonical numbering; repeated dependencies do not expand
  exponentially. Completed materials can be shared between threads.

## Retention and costs

Retain the root, materials and drawings. Updating values shares programs and
path storage. Recording a canvas again still performs CPU graph work even when
the root finds an existing program. Path replay currently copies vertices to
the existing scene representation; drawing cloning and parameter updates share
the retained tessellation.

The construction budget is 4,096 nodes and parameter bindings combined. Only
reachable uniforms survive compilation and count toward the 64-slot material
limit. Updating a pruned handle returns `ForeignGraph`, including after a mapping
that discards its input. The limit applies after composition: a canvas shape mask
currently adds one geometry slot, so leave room for it in the source material.
The first 128 distinct argument-bearing function calls
are memoized within a builder; subsequent identical calls reuse the expansion.

The root caches 256 compiled descriptions, preferring the oldest description
without live shader instances for eviction. If all entries are live, it evicts
the oldest cache entry; existing shaders remain valid, but rebuilding its source
may receive a new identity. WGSL storage is shared between the program and cache.
The renderer keeps all programs needed by the current frame and trims inactive
entries toward a 128-program cache. These caches have separate lifetimes.

The renderer uploads aligned parameter records through a reused uniform arena.
Each primitive has its own record; later writes cannot overwrite the values of
an earlier draw. It uses one draw per procedural primitive, hardware scissoring
plus exact fragment clipping, and synchronous first-use pipeline compilation.
Alpha is clamped to `[0, 1]` before inherited opacity and target blending.

Naga validates the complete generated vertex/fragment module before WGPU pipeline
creation. Device compilation, adapter limits and runtime backend availability
remain a second validation boundary. Device-owned resources are rebuilt with the
renderer after recovery; actual forced device-loss testing remains future work.

## Run the visual example and benchmarks

```sh
cargo run -p gpui_ce_wgpu --features test-support --example vercel_hero -- --headless --output /tmp/gpui-vercel-hero.png --time 3 --pointer 720 350 --size 1280 720
cargo run -p gpui_ce_wgpu --features test-support --example paint_gallery -- /tmp/gpui-paint-gallery.png
cargo test -p gpui-ce --no-default-features --lib paint::
cargo test -p gpui_ce_wgpu --features test-support --test paint_shaders
cargo bench -p gpui_ce_wgpu --features test-support --bench paint -- --sample-size 10 --measurement-time 1 --warm-up-time 1
```

The hero is a complete GPUI view inspired by Vercel's triangle hero: native text
and controls over one retained shader canvas. Typed functions compose its triangle
distance, light falloff and grain. Time, pointer and light changes update parameter
values; resizing updates the canvas geometry. Linux/FreeBSD use an interactive
window without `--headless`; macOS uses a headless WGPU screenshot. The same GPUI
view and canvas paint path produce both. Headless snapshots support deterministic
time, pointer and viewport arguments, with a minimum size of 640×480.

The gallery renders six panels using only the typed shader builder. The
benchmarks separate graph construction, reconstruction, parameter updates,
canvas construction and cold/warm rendering. GPU tests fail if no adapter is
available; they do not silently skip rendering.

## What follows this slice

This API currently supports procedural fragment paints, shape masks, native
backgrounds and native-background paths. It does not yet support texture/image
sampling in the graph, arbitrary shader-filled paths or text, affine canvas
transforms, custom vertex/compute stages, or effect-pass scheduling.

The next compositional extension is a sampled image: materialize a drawing,
then sample it inside an ordinary function. The
[architecture](paint-architecture.md) describes the scheduling obligations behind
that boundary. Replacing content/backdrop blur requires it; the current filter
API remains available until a replacement preserves its behavior.

See the [framework comparison](paint-comparisons.md), [adversarial API review](paint-api-review.md)
and [verification record](paint-verification.md) for the design tradeoffs and evidence.
