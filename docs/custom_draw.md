# Custom Draw (Metal + Blade)

The custom draw API is supported on Metal (default on macOS) and Blade (`macos-blade`).

## Features

- Custom WGSL render pipelines.
- Vertex buffers (inline + static GPU buffers).
- Instanced rendering.
- Uniform bindings (per-draw data).
- Texture + sampler bindings.
- Batching by pipeline + bindings.
- Stress harness for throughput testing.

## Quick start

```sh
# Base example (Metal default on macOS; add --features macos-blade for Blade)
cargo run --example custom_draw_api

# Animated uniform example
cargo run --example custom_draw_api_animated

# Instanced rendering example
cargo run --example custom_draw_api_instanced

# Explicit @location + @group/@binding example
cargo run --example custom_draw_api_conformance

# Stress harness (many instanced quads)
cargo run --example custom_draw_stress
```

## WGSL constraints (Blade reflection)

- Vertex inputs can use `a0..a7` naming (implicit mapping) or explicit `@location` with
  `CustomVertexAttribute.location`.
- Bindings use `b0..b3` naming when you are **not** using explicit `@group/@binding`.

## Explicit location/binding support

Explicit bindings are supported on Metal and Blade.

You can opt into explicit `@location` and `@group/@binding` if you follow these rules:

- Set `CustomVertexAttribute.location` for any attribute that should map to an explicit `@location`.
  Field names in WGSL can then be arbitrary.
- Bindings can use explicit `@group/@binding` via `CustomBindingDesc.slot` (group 0+ supported).
- Binding indices can be sparse; unused slots are permitted.
- Binding indices are capped at 4096 to prevent accidental huge layouts.

See `custom_draw_api_conformance` (explicit), `custom_draw_api_mixed` (mixed explicit/implicit),
`custom_draw_api_multi_group` (multiple explicit bind groups), and
`custom_draw_api_missing_binding` (logs missing binding warnings) for examples.
The mixed example computes its quad from the canvas bounds so it stays inside the rounded border on resize.

## Uniform helper (16-byte alignment)

Use `CustomUniformBuilder` to avoid the common "uniform size must be 16-byte aligned" error:

```rust
use gpui::CustomUniformBuilder;

let mut builder = CustomUniformBuilder::new();
builder
    .push_vec2(origin_x, origin_y)
    .push_vec2(size_x, size_y)
    .push_vec4(1.0, 0.8, 0.6, 1.0);

let uniform = builder.finish(); // padded to 16 bytes
```

## Error semantics

- **Pipeline creation** (`create_custom_pipeline`) returns `Err` for:
  - empty entry names,
  - misaligned uniform sizes (must be non-zero and 16-byte aligned),
  - WGSL compile/validation failures.
- **Runtime warnings** (non-fatal):
  - missing custom pipeline IDs,
  - missing buffers/textures/samplers,
  - missing binding values for declared slots.
  In these cases the draw may render incorrectly but will still attempt to draw.
- **Fatal mismatches**:
  - shader binding type mismatches,
  - explicit bindings that reference an unknown group or binding slot,
  - missing vertex attribute mappings.
  These are asserted and will panic.

Example vertex inputs:

```wgsl
struct VertexInput {
  a0: vec2<f32>,
  a1: vec2<f32>,
};
```

Example bindings:

```wgsl
var b0: texture_2d<f32>;
var b1: sampler;
var<uniform> b2: Uniforms;
```

## Performance tips

- Prefer static GPU buffers for vertex/instance data; only update on size changes.
- Use instancing for large numbers of similar quads.
- Keep bindings stable to maximize batching by pipeline + bindings.
- Examples inset draw bounds by 1px to keep quads inside rounded borders.

## Stress harness flags

```sh
cargo run --example custom_draw_stress -- \
  --instances 2500 --quad-size 10 --grid-pad 6 --bounds-inset 1
```

### Instanced example flags

```sh
cargo run --example custom_draw_api_instanced -- \
  --instances 25 --quad-size 24 --grid-pad 16 --bounds-inset 1
```

## Perf testing

- Run with `--release`.
- Increase `--instances` until FPS drops below display refresh.

```sh
cargo run --release --example custom_draw_stress -- \
  --instances 10000
```

## Future work

- Streaming texture uploads for large, per-frame data (e.g., video frames).
- Offscreen render targets for multi-pass effects and composition.
- Explicit compute/fragment passes to build post-processing pipelines.
- Better tooling for GPU resource lifetime and frame pacing.
