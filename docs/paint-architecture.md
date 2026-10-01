# A shared canvas and shader language

The central abstraction is a typed function, not a programmable rectangle.
Compose functions, bind changing values, and draw geometry. Color, distance,
coverage and coordinate transformations use the same expression graph. Ordinary
canvas backgrounds retain their existing optimized lowering. Existing GPUI
elements are one host for this language.

The design favors a small vocabulary that composes. It does not need a public
render-graph framework, a scheduler DSL, a catalog of effect classes, or a new
coordinate marker for every use case. Those would make the user pay for internal
machinery before it earns its place.

The code in this change is an experimental vertical slice, not the entire design
below. The distinction matters: arbitrary image effects, compute, texture
bindings, and shader-painted text/path coverage are not implemented yet. The
existing blur APIs therefore remain available and are not yet deprecated.

## What the existing code tells us

- `gpui/src/elements/canvas.rs` supplies layout and prepaint/paint callbacks. It
  does not currently define an independent drawing language.
- `Window` handles logical-to-device coordinates, inherited opacity and content
  masks. Bypassing it with an offscreen texture would duplicate these semantics.
- `Scene` stores primitive descriptions, cached paint replay and spatial draw
  order. `scene/plan.rs` compiles them into backend-neutral ordered work.
- `gpui_render/src/shaders` contains Rust-authored built-in shaders, with generated
  layouts and build-time validation. User composition should complement this
  optimized implementation, rather than require users to author its ABI.
- `gpui_wgpu` owns device recovery, pipeline state, intermediate textures and
  upload arenas. Compiled user programs belong in that device-owned lifetime.
- `Filter` and `ScaledFilter` contain only blur. The scene plan limits isolated
  content filters to two targets; deeper groups render inline. Blur currently
  lowers to downsample, horizontal, vertical and composite passes.
- The default macOS and Windows windows still use separate native Metal and
  D3D11 renderers. Working on a WGPU Metal adapter does not make an API available
  in those native window renderers.

## Design commitments

1. **One authoring model.** Typed Rust values and ordinary Rust functions compose
   shader expressions. There is no mandatory WGSL string, binding number or
   second shader language in application code. Rust closures construct a graph;
   they are not an implicit compiler for arbitrary Rust control flow.
2. **Geometry and paint are independent.** Solid colors, gradients, procedural
   materials, images and effect outputs should all shade the same geometry.
   A border is coverage applied to paint, not a special shader subsystem.
3. **Programs and values have different identities.** Changing animation data
   must not compile a pipeline. Matching generated programs may share code, but
   independent parameter handles must never alias by accident.
4. **Descriptions survive devices.** The public root owns composition and program
   identity. A renderer realization owns resources for one device generation.
   Recovery rebuilds GPU state without destroying application paint descriptions.
5. **Expensive work is explicit and measurable.** Sampling another result creates
   a resource dependency. A pure color transform need not create a texture.
   Uploads, compilation, passes and retained bytes should have separate counters.
6. **Unsupported work fails explicitly.** A capability check and typed error are
   preferable to a successful draw that silently omits an effect.

## The small public core

| Operation | Contract |
| --- | --- |
| `root.function::<I, O>(...)` | Describe reusable computation with explicit typed arguments and results |
| `cx.call(&function, arguments)` | Substitute arguments into that same expression graph |
| `root.parameter(value)` / `cx.uniform(handle)` | Supply changing data without changing code |
| `root.shader(...)` | Choose the fragment domain and produce a drawable color program |
| `root.canvas(...)` | Record ordered geometry with those paints |

`Function<I, O>` is the general composition primitive. Numeric leaves and tuple
structure are different type layers: a tuple describes a function signature, not
a GPU buffer layout. `Distance`, `Coverage` and `Predicate` preserve their
meaning through calls. Functions have no captured uniforms or implicit fragment
coordinates; supply them as arguments. The same function can consequently serve
two independently parameterized instances in one shader without a binding
collision. A completed `Shader` additionally packages its fragment inputs and
parameter bindings for drawing.

Calls substitute into one DAG. Equal expressions share nodes; unused results
disappear when a shader is finished. A composed function and the equivalent
handwritten expression should yield the **same generated program**, not merely
similar timing. This is the primary test of whether the abstraction costs GPU
work. Building a graph still has CPU cost; retain it and update parameters.

Coordinates can be explicit vector arguments. A warp is a function returning a
vector; a shape is a function returning distance; a mask is a function returning
coverage. There is no need for separate warping, geometry-node and shader-node
languages. Semantic wrappers earn their place by preventing actual mistakes:
coverage cannot be accidentally combined as a distance, and predicates cannot
participate in numeric arithmetic.

Derivatives are an execution requirement, not evidence that a function is valid
in every stage. Reachable derivative use is tracked. This implementation executes
fragment paints only; supporting compute later needs an actual entry-point
validator, not an assertion that every fragment expression works in compute.

## Two composition boundaries, not a feature catalog

| Layer | Meaning | Reusable building blocks |
| --- | --- | --- |
| Field | A typed function of coordinates and resources | Color, distance, coverage, vector displacement, noise, image sample |
| Drawing | Ordered geometry shaded with paints | Paths, shapes, text runs, images, mesh, clips, transforms, layers |
| Effect graph | Dependencies between rendered or computed results | Sampling, blur, displacement, reduction, simulation, history |

Pure functions fuse. Ordered drawing preserves blending, clipping and scene
order. These are different semantics and should not be flattened into an
unrestricted bag of nodes. The missing bridge is a sampled image: a drawing can
be materialized into an image, and sampling an image can return to the function
graph. That pair of operations is the next useful extension. It closes the loop
needed for most image effects without making users author passes or allocate
textures themselves.

The effect graph below describes compiler obligations behind that bridge, not a
requirement to publish all of its machinery. Blur, displacement and backdrop
recipes must demonstrate the bridge before adding reductions, simulations or a
general compute API. They are not prerequisites for a useful canvas.

### Fields and geometry

Keep expression result types sealed: invalid scalar/vector operations must fail
to compile. Graph membership and parameter ownership are also checked; those are
runtime construction errors in the current implementation. A future generative
scope can make more ownership errors compile-time errors, provided it does not
make reusable materials or retained parameters awkward.

Build distance operations from the same expression vocabulary as color. Circle,
rounded rectangle, stroke and Boolean combinations are useful starting points.
Coverage is distinct from color: masking straight-alpha paint multiplies alpha
once. Multiplying all four channels before ordinary alpha blending would darken
edges twice. Evaluate derivative-based coverage before divergent clipping.

A distance field is one geometry representation, not the only representation.
Bezier paths require robust tessellation or coverage rasterization, and text
requires shaped glyph coverage. Keep a common coverage contract so the same paint
can shade all three. Do not force arbitrary paths into giant analytic shaders.

Document local logical pixels and normalized UV explicitly. Introduce more
domain types only where they prevent mistakes without making ordinary composition
verbose. A transformation changes the evaluation domain once; it must not
independently distort color, clipping and stroke width. A nonuniform affine
transform does not preserve signed distances: coverage needs derivatives or an
explicitly documented metric correction.

### Reusable composition

Composition should support ordinary functions over expressions, typed parameter
handles, and transformation of a completed material. Mapping a material must
preserve its original parameter handles while allocating new ones without
collision. This enables reusable tint, mask, contour, lighting and alpha recipes.

Program interning should compare canonical structure after reachability analysis,
including output type, parameter layout and stage requirements. Uniform values,
object bounds and time do not belong in a pipeline key. Use structural equality
after hashing; a hash collision must never select a different shader.

Interning an already constructed graph avoids GPU compilation but does not make
CPU graph construction free. Applications should retain materials, and the API
should make that natural. Benchmarks must distinguish first construction,
equivalent reconstruction, value update, shader-module creation and pipeline
creation.

### A real effect graph

Do not generalize `Vec<Filter>` into a bag of opaque callbacks. Use typed,
versioned resources and explicit read/write edges. The following is design
notation, not a claim that these methods exist today:

```rust,ignore
let image = root.drawing(|c| {
    c.shape(panel).fill(material.clone());
    c.text(label).fill(ink);
});
let blurred = image.effect(gaussian_blur(radius));
let distorted = blurred.effect(displace(flow_field, amplitude));
let final_image = distorted.over(backdrop);
```

An effect declares its input/output type, required capabilities, sample footprint,
extent rule, resolution rule, color interpretation and alpha interpretation.
The graph compiler then owns scheduling, lifetimes and resource allocation.

- **Versioned writes:** a pass consuming image version N produces version N+1.
  It cannot sample the same subresource it is currently writing as an attachment.
- **Backdrop is a snapshot:** capture the prior scene at the exact ordered point
  of use. It is not a reference to a mutable final scene texture.
- **History is explicit:** feedback means previous-frame data with initialization,
  resize, invalidation and recovery behavior. It is not a cycle in one frame DAG.
- **Bounds are semantic:** blur expands required input; displacement needs a
  maximum footprint or explicitly requests the whole source. Cropping without
  these declarations creates seams.
- **Budget is explicit:** limit graph nodes, passes, texture bytes and dispatches.
  Exceeding a budget returns an error or invokes a caller-selected quality policy;
  it does not silently erase deep effects.
- **Fusion is conditional:** combine pure pointwise operations with compatible
  sampling/alpha semantics; do not reorder blur, clipping, nonlinear color
  operations or backdrop reads simply because they look composable.
- **Allocation follows liveness:** alias transient textures only when descriptors
  agree and live ranges do not overlap. Persistent history and external resources
  cannot enter the same alias pool without explicit ownership rules.

Compute then fits the same graph rather than becoming a separate escape hatch.
Typed buffers and textures declare access and element format. Dispatch sizes,
workgroup storage, integer overflow and device limits are validated. Fragment-only
devices receive an explicit unsupported-capability result. WebGL fallback cannot
silently emulate every WebGPU operation.

## What should be shared all the way down

| Concern | Shared representation | Specialized lowering is allowed |
| --- | --- | --- |
| Geometry | Bounds, path, shape, stroke semantics | Analytic coverage, tessellation, glyph atlas |
| Paint | Typed program plus parameter/resource bindings | Existing solid/gradient fast paths, generated WGSL |
| Clip | Ordered clip stack and coverage semantics | Scissor, stencil, mask texture |
| Transform | Typed coordinate mapping | Vertex transform or inverse field mapping |
| Effects | Versioned graph resources and pass dependencies | Render or compute passes |
| Parameters | Validated schema and stable ownership | Packed uniforms, dynamic arena, storage/instance data |
| Diagnostics | Source graph/node labels and stage requirements | WGSL/Naga/device compilation diagnostics |

This avoids making the optimized quad renderer the public model. Its current
storage layout and batching remain implementation choices. Conversely, a canvas
wrapper that only forwards method calls cannot provide the resource analysis,
retention or composition guarantees described here.

## Performance architecture

Keep device-free descriptions cheap to clone and immutable. Realize programs in
a cache keyed by program, target format, sample count, blend state, layout and
device generation. Bound unused cache entries; keep current-frame entries alive.
Prewarm known variants asynchronously before animation where the backend supports
it. A synchronous first-use path remains observable and documented.

Use aligned uniform arenas with one upload per frame, reusing bind groups across
frames and rebuilding them when the underlying buffer grows. Allocate per-draw
slots, not one mutable uniform value that every draw in the submission would
observe. Check maximum offsets and binding sizes before GPU API calls.

The first inline shader renderer still issues one draw per primitive. The next
optimization is material cohorts: adjacent compatible draws carry per-instance
bounds, clip and parameter records, and share one pipeline and binding layout.
Preserve spatial ordering and effect boundaries. Measure dynamic-uniform and
storage-buffer variants separately; storage is not portable to all fallback
devices. Stable display lists can later become render bundles when target and
resource generations match.

Do not promise that arbitrary programmable paints are as cheap as specialized
quads. The useful question is where time goes: graph construction, planning,
upload/encoding, cold compilation, submission/wait, GPU execution, or readback.
GPU timestamp queries require feature support and are distinct from CPU timings.

## Replacing filters deliberately

1. Establish typed fields, reusable parameterized paints and ordinary scene
   integration. Exercise clipping, alpha, ordering and recovery boundaries.
2. Add typed sampled images and an effect pass graph with resource validation.
3. Implement Gaussian blur as a reusable effect recipe. Match the current kernel,
   downsampling, expanded bounds, corner clipping and opacity semantics with pixel
   comparisons. Preserve existing optimized kernels where they are appropriate.
4. Lower content layers and backdrop snapshots into that graph. Remove the fixed
   two-target depth policy only when allocation budgeting and nested behavior are
   tested. Multiple effects need sequential semantics, not a maximum-radius fold.
5. Make existing `filter`, `backdrop_filter` and `blur` builders adapters to the
   effect vocabulary. Publish migration examples and equivalence evidence.
6. Deprecate redundant public types after replacement coverage exists. Remove
   adapters on the project's normal breaking-release schedule.

Removing the current filter API before step 4 would discard working behavior;
the greenfield design should succeed it with greater expressive power.

## External ideas and deliberate departures

Skia's useful precedent is composing local shading functions into one rendering
pipeline. Halide separates a computed value from the decision to materialize it.
The inference for this API is to keep function composition cheap and make the
image boundary explicit, without importing either system's entire public API.
See [Skia runtime effects](https://docs.skia.org/docs/user/sksl/) and
[Halide's function model](https://halide-lang.org/docs/tutorial/lesson_01_basics.html).

TypeGPU demonstrates typed schemas, reusable GPU functions and late-bound slots.
Its root distinguishes device-dependent resources from descriptions, which is a
useful ownership boundary. Here, the application composition root remains
device-independent so GPUI can recover a device underneath retained drawings.
See [functions](https://docs.swmansion.com/TypeGPU/apis/functions/),
[slots](https://docs.swmansion.com/TypeGPU/apis/slots/), and
[roots](https://docs.swmansion.com/TypeGPU/apis/roots/).

Vercel's vgpu performance guidance motivates explicit pass flow, pipeline warmup,
replayable work and dynamic uniform pools. GPUI additionally needs spatial scene
ordering, element opacity, deferred overlays and device recovery to remain part
of the contract. See the
[performance playbook](https://github.com/vercel-labs/vgpu/blob/main/docs/topics/performance-playbook.docs.md)
and [render package](https://github.com/vercel-labs/vgpu/blob/main/packages/render/README.md).

WebGPU feature/limit negotiation and WGSL validation remain the portability
boundary, not resemblance to JavaScript APIs. See the
[WebGPU specification](https://www.w3.org/TR/webgpu/) and
[WGSL specification](https://gpuweb.github.io/gpuweb/wgsl/).
