# Custom Draw (Metal and Blade)

The custom draw API is supported on Metal (default on macOS) and Blade (`macos-blade`).

## Features

- Custom WGSL render pipelines.
- Vertex buffers (inline and static GPU buffers).
- Index buffers and indexed draws.
- Instanced rendering.
- Uniform bindings (per-draw data).
- Push constants.
- Storage buffers (read/write) with buffer slices for dynamic offsets.
- Storage textures (compute write) and sampled textures.
- Texture and sampler bindings.
- Binding arrays (texture/buffer).
- Texture arrays and cubemaps.
- Block-compressed textures (BC1, BC3, BC7, ETC2, ASTC 4x4/5x5/6x6/8x8, PVRTC 2bpp/4bpp).
- sRGB formats and mipmapped textures.
- Compute pipelines and dispatch.
- Configurable pipeline state (blend, cull, front face, depth).
- Offscreen render targets and depth testing.
- Multiple color attachments (MRT).
- MSAA sample counts for offscreen render targets.
- Batching by pipeline and bindings.
- Stress harness for throughput testing.

## Quick start

```sh
# Base example (Metal default on macOS; add --features macos-blade for Blade)
cargo run --example custom_draw_api

# Precompiled .metallib pipeline example (Metal backend)
cargo run --example custom_draw_api_metallib

# Animated uniform example
cargo run --example custom_draw_api_animated

# GPU profiling example
cargo run --example custom_draw_api_gpu_profiling

# Instanced rendering example
cargo run --example custom_draw_api_instanced

# Offscreen render target, depth, and MSAA example
cargo run --example custom_draw_api_offscreen

# Compute pipeline and storage buffer example
cargo run --example custom_draw_api_compute

# Storage texture example
cargo run --example custom_draw_api_storage_texture

# Binding arrays example
cargo run --example custom_draw_api_binding_arrays

# Texture array example
cargo run --example custom_draw_api_texture_arrays

# Cubemap example
cargo run --example custom_draw_api_cubemap

# Compressed texture example
cargo run --example custom_draw_api_compressed_texture

# Streaming texture upload example
cargo run --example custom_draw_api_streaming_texture

# Explicit @location and @group/@binding example
cargo run --example custom_draw_api_conformance

# Stress harness (many instanced quads)
cargo run --example custom_draw_stress
```

## WGSL constraints (Blade reflection)

- Vertex inputs can use `a0..a7` naming (implicit mapping) or explicit `@location` with
  `CustomVertexAttribute.location`.
- Bindings use `b0..b15` naming when you are **not** using explicit `@group/@binding`.

## Explicit location/binding support

Explicit bindings are supported on Metal and Blade.

You can opt into explicit `@location` and `@group/@binding` if you follow these rules:

- Set `CustomVertexAttribute.location` for any attribute that should map to an explicit `@location`.
  Field names in WGSL can then be arbitrary.
- Bindings can use explicit `@group/@binding` via `CustomBindingDesc.slot` (group 0 and higher supported).
- Binding indices can be sparse; unused slots are permitted.
- Binding indices are capped at 4096 to prevent accidental huge layouts.

See `custom_draw_api_conformance` (explicit), `custom_draw_api_mixed` (mixed explicit/implicit),
`custom_draw_api_multi_group` (multiple explicit bind groups), and
`custom_draw_api_missing_binding` (logs missing binding warnings) for examples.
The mixed example computes its quad from the canvas bounds so it stays inside the rounded border on resize.

## Metal pipeline caching and precompiled MSL

- Metal custom draw pipelines are cached by shader source, entry points, layouts, and bindings.
- You can persist custom draw pipeline cache data across runs with a Metal binary archive path:

```rust
window.set_custom_pipeline_cache_path("/tmp/gpui_custom_draw_pipeline_cache.binarchive")?;
```

Call `window.clear_custom_pipeline_cache_path()?` to disable persistent cache usage.
Persistent cache uses Metal binary archives and requires macOS 11 or newer.

- You can bypass WGSL→MSL translation with precompiled MSL source:

```rust
let pipeline = window.create_custom_pipeline_msl(desc, msl_source)?;
```

- You can also load precompiled `.metallib` bytes:

```rust
let pipeline = window.create_custom_pipeline_metallib_file(desc, "path/to/custom.metallib")?;
```

See `custom_draw_api_metallib` for a complete example that compiles MSL to `.metallib`
and loads it via this API. Set `GPUI_EXAMPLE_ENABLE_PIPELINE_CACHE=1` when running that
example to exercise persistent pipeline cache path setup.

MSL slot mapping follows the binding order used in `CustomPipelineDesc`:
- Textures/samplers use the binding index (0..N)
- Buffers/uniforms use `vertex_fetch_count` offset by the binding index
- Binding groups are ignored on Metal (slots are flat)

## Custom GPU profiling (timestamps)

You can enable per-frame GPU profiling for custom draw and custom compute work:

```rust
window.set_custom_gpu_profiling_enabled(true)?;

if let Some(sample) = window.take_last_custom_gpu_profile()? {
    log::info!(
        "custom draws={}, custom computes={}, render passes={}, compute passes={}, gpu_time_ns={:?}",
        sample.custom_draw_count,
        sample.custom_compute_count,
        sample.custom_render_pass_count,
        sample.custom_compute_pass_count,
        sample.gpu_time_ns,
    );
}
```

`gpu_time_ns` is currently sourced from Metal command buffer timestamps.

## Custom frame diagnostics

You can enable frame pacing diagnostics for custom draw and custom compute work:

```rust
window.set_custom_frame_diagnostics_enabled(true)?;

if let Some(sample) = window.take_last_custom_frame_diagnostics()? {
    log::info!(
        "cpu_encode_ns={}, retries={}, submit_to_scheduled_ns={:?}, submit_to_completed_ns={:?}, scheduled_to_completed_ns={:?}",
        sample.cpu_encode_time_ns,
        sample.retry_count,
        sample.submit_to_scheduled_ns,
        sample.submit_to_completed_ns,
        sample.scheduled_to_completed_ns,
    );
}
```

These diagnostics are currently sourced from Metal command buffer callbacks and timestamps.

## Custom resource diagnostics

You can snapshot custom draw resource counts and estimated memory usage:

```rust
let stats = window.custom_draw_resource_stats()?;
log::info!(
    "custom pipelines={}, compute pipelines={}, buffers={} ({} bytes), textures={} ({} bytes), depth targets={} ({} bytes)",
    stats.pipeline_count,
    stats.compute_pipeline_count,
    stats.buffer_count,
    stats.buffer_bytes,
    stats.texture_count,
    stats.texture_bytes,
    stats.depth_target_count,
    stats.depth_target_bytes,
);
```

The memory values are estimates based on texture formats, mip chains, array layers, and sample counts.

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

## Push constants

Declare a push constant block in WGSL:

```wgsl
var<push_constant> Params: Params;
```

Set `push_constants` on the pipeline descriptor with the block size (16-byte aligned) and provide
per-draw data via `CustomDrawParams.push_constants` or per-dispatch data via
`CustomComputeDispatch.push_constants`.

## Buffer slices (dynamic offsets)

Use `CustomBufferSource::BufferSlice { id, offset, size }` to bind a sub-range of a larger buffer
for uniforms, storage buffers, or vertex/index data. Offsets and sizes are in bytes; the slice must
fit inside the buffer and uniform slices must be at least the declared uniform size.

## Mipmap textures

Provide one `Arc<[u8]>` per mip level in `CustomTextureDesc.data` (level 0 first). Use
`CustomTextureUpdate { level, data, bytes_per_row: None }` to update a specific level. When
your data includes row padding, set `bytes_per_row` to the stride in bytes.

For streaming updates, upload into a custom buffer and call
`update_custom_texture_from_buffer` with `CustomTextureBufferUpdate`. The buffer must contain
packed data for all layers in the mip level.

## Storage textures

Create textures with `CustomTextureUsage::STORAGE` (optionally combined with `SAMPLED`). Bind them
using `CustomBindingKind::StorageTexture` and `CustomBindingValue::Texture`.

## Texture arrays and cubemaps

Set `CustomTextureDesc.dimension` to `CustomTextureDimension::D2Array { layers }` or
`CustomTextureDimension::Cube` (cube textures must be square). Array and cube textures expect each
mip level’s data to pack all layers sequentially (layer 0 first). Storage textures currently only
support `CustomTextureDimension::D2`.

## Compressed textures

Use `CustomTextureFormat::Bc1Unorm`, `CustomTextureFormat::Bc3Unorm`,
`CustomTextureFormat::Bc7Unorm`, `CustomTextureFormat::Etc2Rgb8Unorm`,
`CustomTextureFormat::Etc2Rgba8Unorm`, `CustomTextureFormat::Astc4x4Unorm`,
`CustomTextureFormat::Astc5x5Unorm`, `CustomTextureFormat::Astc6x6Unorm`,
`CustomTextureFormat::Astc8x8Unorm`, `CustomTextureFormat::PvrtcRgb2bppUnorm`,
`CustomTextureFormat::PvrtcRgba2bppUnorm`, `CustomTextureFormat::PvrtcRgb4bppUnorm`,
or `CustomTextureFormat::PvrtcRgba4bppUnorm` (and their sRGB variants) for
block-compressed textures. Each mip level is packed using the format block size.
Compressed formats are sampled only and cannot be used as storage textures or render targets.
Creation fails if the GPU does not support the requested format. PVRTC is typically available on
Apple mobile GPU families. Blade currently supports BC formats only.

Use `window.custom_texture_format_supported(format)` to choose a format at runtime:

```rust
let format = if window.custom_texture_format_supported(CustomTextureFormat::Astc6x6Unorm)? {
    CustomTextureFormat::Astc6x6Unorm
} else if window.custom_texture_format_supported(CustomTextureFormat::Bc7Unorm)? {
    CustomTextureFormat::Bc7Unorm
} else {
    CustomTextureFormat::Rgba8Unorm
};
```

## Multiple render targets and MSAA

Set `CustomPipelineDesc.color_targets` to the list of color formats that match your fragment outputs
(`@location(0..)`). When rendering offscreen, create a `CustomRenderTarget` with
`colors: vec![...]` and an optional depth target. All color and depth targets must share the same
size and sample count.

Use `sample_count` on `CustomRenderTargetDesc`, `CustomDepthTargetDesc`, and
`CustomPipelineState.sample_count` to enable MSAA. The multisample buffer resolves into the render
target texture each frame. The window surface currently requires `sample_count` to be 1.

## Binding arrays

Use WGSL `binding_array<T, N>` (N ≤ 16) with `CustomBindingKind::TextureArray`,
`CustomBindingKind::StorageTextureArray`, or `CustomBindingKind::BufferArray`, and supply values
via `CustomBindingValue::TextureArray` or `CustomBindingValue::BufferArray`.

See `custom_draw_api_binding_arrays` for a working example. Binding arrays use Metal argument
buffers on macOS (Metal 2.0 and later). WGSL-to-MSL translation currently supports texture binding arrays;
buffer binding arrays require precompiled MSL or Blade.

## Compute dispatch

Use `CustomComputePipelineDesc` and `create_custom_compute_pipeline` to create a compute pipeline,
then call `dispatch_custom_compute` with workgroup counts and bindings. Compute dispatches run
before custom draw render passes each frame.

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
- Keep bindings stable to maximize batching by pipeline and bindings.
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

## Known limitations (current gaps)

- Depth is limited to Depth32Float; no stencil attachments.
- MSAA is only supported for offscreen render targets (the window surface uses one sample).
- Custom viewport and scissor state are not configurable.
- Binding arrays require Metal argument buffer support on macOS; WGSL buffer arrays require
  precompiled MSL (texture arrays are supported).
- ASTC, ETC2, and PVRTC textures require Metal support and are not available on Blade.
- Custom GPU timestamp profiling and frame diagnostics are currently available on Metal.
- Storage textures are limited to 2D RGBA/BGRA (with sRGB).

## Core roadmap (triage)

- **P0 (core primitives)** (done)
  - Index buffers and indexed draws.
  - Depth attachments and depth testing (Depth32Float only).
  - Offscreen render targets / render passes for multi-pass composition.
  - Configurable pipeline state (blend, cull, front face, depth).
- **P1 (feature growth)** (done)
  - Binding arrays.
  - Texture arrays and cubemaps.
  - Multiple color attachments and MSAA.
  - Compressed textures (BC1, BC3, BC7, ETC2, ASTC 4x4/5x5/6x6/8x8, PVRTC 2bpp/4bpp).
- **P2 (performance/tooling)**
  - More compressed texture formats (additional ASTC sizes, PVRTC) (done).
  - Streaming texture uploads for large, per-frame data (e.g., video frames).
  - Persistent pipeline cache for Metal (done).
  - `.metallib` loading for Metal (done).
  - GPU profiling (timestamps) for custom draw and custom compute on Metal (initial, done).
  - Resource lifetime diagnostics (initial, done).
  - Frame pacing / diagnostics tooling (initial, done).

## Streaming texture uploads plan (done)

- Add row-stride support for texture updates to avoid repacking video frames (done).
- Add buffer-backed texture uploads for per-frame streaming (done).
- Provide an example that updates a texture each frame from a buffer (done).
