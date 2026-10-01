# Composition lessons from other renderers

This is an API and renderer-lifecycle comparison, researched against primary
documentation on 26 September 2026. It is not a performance ranking: no other
framework was benchmarked against this implementation. The local measurements
and unverified platforms are recorded in [paint-verification.md](paint-verification.md).

The useful common idea is to separate reusable computation from the decision to
draw or materialize it. A function should remain ordinary shader code until an
operation actually needs an image. Backend resources and scheduling should not
become prerequisites for drawing a custom border.

| Implementation | Useful property | Decision for GPUI paint |
| --- | --- | --- |
| [TypeGPU functions](https://docs.swmansion.com/TypeGPU/apis/functions/), [slots](https://docs.swmansion.com/TypeGPU/apis/slots/) and [roots](https://docs.swmansion.com/TypeGPU/apis/roots/) | Typed reusable shader functions, dependency substitution, and centralized resource ownership. | Keep explicit typed function arguments and one composition root. Our root owns device-independent descriptions; the existing renderer owns the device. Do not reproduce a second resource-management API. |
| [Vercel vgpu](https://github.com/vercel-labs/vgpu) and its [performance playbook](https://github.com/vercel-labs/vgpu/blob/main/docs/topics/performance-playbook.docs.md) | Replayable work, reusable pipelines and bindings, and attention to CPU-side submission costs. | Retain drawings and program identity, use a reusable aligned upload arena, and measure graph construction separately from warm submission. The hero exercises animation through values instead of per-frame graph reconstruction. |
| [Skia runtime effects](https://docs.skia.org/docs/user/sksl/) | A runtime shader contributes a function to a larger paint pipeline; child shaders compose without requiring an intermediate image. Local coordinates, color space and alpha have explicit contracts. | Inline functions and material sampling into one graph. Shape coverage and procedural color share that graph. State straight-alpha output and logical-pixel coordinates explicitly; reserve image materialization for operations that need it. |
| [Flutter fragment shaders](https://docs.flutter.dev/ui/design/graphics/fragment-shaders) | A compiled program creates shader instances usable as canvas paints; image filters add distinct input-image and backend requirements. Uniform binding and premultiplied output are explicit concerns. | Separate immutable program code from typed instance values. Reuse the same paint in fills and borders. A future sampled image must be explicit; an ordinary material cannot silently turn into a backdrop pass. |
| [Vello Scene](https://docs.rs/vello/latest/src/vello/scene.rs.html) | Retained encoding of paths, brushes, clips and layers separates scene construction from rendering. Its GPU pipeline manages coverage and composition. | Preserve GPUI's existing scene order and effect boundaries. Keep native shapes on their established path; introduce procedural paint as another ordered primitive. A full replacement path rasterizer is unnecessary for this slice. |
| [Iced WGPU primitives](https://docs.iced.rs/src/iced_wgpu/primitive.rs.html) | Custom primitives have prepare/draw/render phases and renderer-owned reusable pipelines. Same-pass rendering can avoid additional work. | Prepare pipelines and uniform storage before the pass, then draw inline with native content. Keep this lifecycle inside the renderer instead of requiring application authors to manage WGPU pipelines for every material. |
| [egui WGPU callbacks](https://docs.rs/egui-wgpu/latest/src/egui_wgpu/renderer.rs.html) | Preparation is distinct from painting, callback resources persist across frames, and viewport/clip state is part of integration. | Treat cache ownership, clipping and state restoration as correctness requirements. Test a native primitive after shader draws, not just an isolated shader image. |
| [Slint WGPU integration](https://docs.slint.dev/latest/docs/rust/slint/wgpu_30/) | Rendering notifications expose a graphics lifecycle; custom WGPU output can be composed through images. | Retain the existing raw-GPU escape hatch for specialized work. Keep common paint composition device-independent; add a sampled-image boundary later without leaking raw texture ownership into every function. |
| [Firefox WebRender](https://firefox-source-docs.mozilla.org/gfx/RenderingOverview.html) | Display lists, scenes, frames, clip structures and render tasks separate stable descriptions from frame-specific culling, batching and intermediate surfaces. Caching trades work against memory and invalidation. | Preserve spatial ordering and explicit effect dependencies. Bound caches, protect current-frame programs, and defer a public pass graph until sampled images require one. Do not promise arbitrary pass fusion or free retention. |
| [Piet RenderContext](https://docs.rs/piet/latest/piet/trait.RenderContext.html) | A compact drawing vocabulary combines geometry, brushes, clipping and scoped state, while backend resources remain associated with their render context. | Use a small canvas recorder with scoped clip, translation and opacity. Share paint and geometry vocabulary rather than adding a second shader-specific drawing language. |

Two compiler designs reinforce the same boundary. [Halide's introductory
pipeline model](https://halide-lang.org/docs/tutorial/lesson_01_basics.html)
separates a computation from realization. [Slang's interfaces and generics](https://shader-slang.org/slang/user-guide/interfaces-generics.html)
show the value of typed reusable shader components. These are design influences,
not dependencies or claims that this small expression builder implements either
language. Rust types distinguish numeric values, distances, coverage and function
signatures; the resulting expression graph remains deliberately limited.

## What the comparison changed

The adversarial review led to concrete implementation changes:

- Repeated calls now memoize graph expansion, rather than merely deduplicating
  emitted instructions after doing the same CPU work again.
- A typed parameter batch performs one immutable update per affected instance;
  an unchanged batch retains its storage. Drawings share their command array.
- Dead bindings disappear with dead expressions. They consume no final uniform
  slots and cannot make a live 64-parameter composition fail through unused input.
- Fixed geometry can be used inside a closed function without allocating hidden
  uniforms. Canvas dimensions remain data so resized shapes can share pipelines.
- Imported parameter resolution no longer depends on whether a read precedes the
  material import. Conflicting instance bindings are explicit construction errors.
- WGSL is shared by the description and cache. Root eviction prefers descriptions
  without live instances; renderer pruning collects candidates in one scan.

## Keep the next extension small

The present abstraction handles procedural color and analytic coverage. The next
useful primitive is a sampled image, including a drawing materialized as an image.
That bridge would enable content effects, backdrop effects and multipass work
through the same typed functions. It also introduces real obligations: bounds,
read/write ordering, texture lifetimes, filtering and color space, invalidation,
memory budgets, and device recovery. Those belong behind a small public boundary,
not a large family of effect-specific builders.

Until that bridge exists and reproduces current blur behavior, the shader filter
API must remain available. The [architecture proposal](paint-architecture.md)
describes the migration requirements; the current implementation does not pretend
to have completed them.
