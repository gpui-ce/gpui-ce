# Custom Draw (Blade-only)

The custom draw API is currently supported only when using the Blade backend.

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
# Base example
cargo run --example custom_draw_api --features macos-blade

# Animated uniform example
cargo run --example custom_draw_api_animated --features macos-blade

# Instanced rendering example
cargo run --example custom_draw_api_instanced --features macos-blade

# Stress harness (many instanced quads)
cargo run --example custom_draw_stress --features macos-blade
```

## WGSL constraints (Blade reflection)

- Vertex inputs must be named `a0..a7` **without** `@location`.
- Bindings must be named `b0..b3` **without** `@group/@binding`.

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
cargo run --example custom_draw_stress --features macos-blade -- \
  --instances 2500 --quad-size 10 --grid-pad 6 --bounds-inset 1
```

### Instanced example flags

```sh
cargo run --example custom_draw_api_instanced --features macos-blade -- \
  --instances 25 --quad-size 24 --grid-pad 16 --bounds-inset 1
```

## Perf testing

- Run with `--release`.
- Increase `--instances` until FPS drops below display refresh.

```sh
cargo run --release --example custom_draw_stress --features macos-blade -- \
  --instances 10000
```
