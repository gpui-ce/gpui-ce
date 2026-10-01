# Paint verification

This records the experimental implementation's actual coverage, rather than
equating type checking with a functioning GPU renderer. Commands were run on
2026-09-26 using Rust/Cargo 1.98.1, WGPU 29.0.4 and Naga 29.0.4.

## Evidence and limits

Final host results after the allocation fixes:

| Check | Result |
| --- | --- |
| GPUI library | 406 passed, including 47 paint tests, 17 scene tests and headless capability delegation |
| WGPU library | 52 passed; 2 independently reproduced baseline failures described below |
| Shader validation/rendering integration | 14 passed |
| Public composition stress | 6 passed |
| Existing headless integration | 2 passed |
| Real GPUI hero interaction | 1 passed |
| Paint module documentation | 8 passed, including 4 compile-fail cases |
| Guide examples | 2 passed |
| Workspace check, all host targets with WGPU test support | Passed |
| Clippy, GPUI and WGPU all targets with test support | Passed without warnings |
| Rustfmt on all new Rust files; `git diff --check` | Passed |

The core/library counts include the named subgroups; do not add them twice.
The workspace check reported an existing `block 0.1.6` future-incompatibility
notice. No baseline code or pixel tolerances were changed to make checks pass.

Representative reproduction commands:

```sh
cargo test -p gpui-ce -p gpui_ce_wgpu --no-default-features \
  --features gpui-ce/test-support,gpui_ce_wgpu/test-support --lib --offline
cargo test -p gpui_ce_wgpu --features test-support --offline \
  --test paint_shaders --test paint_composition --test headless_primitives
cargo test -p gpui_ce_wgpu --features test-support --example vercel_hero --offline
cargo check --workspace --all-targets --features gpui_ce_wgpu/test-support --offline
cargo clippy -p gpui-ce -p gpui_ce_wgpu --no-default-features --all-targets \
  --features gpui-ce/test-support,gpui_ce_wgpu/test-support --offline
```

The integration suite creates a real headless WGPU renderer and reads back pixels.
Failure to acquire a GPU fails the tests. These tests cover:

- Scalar and vector operator permutations and complete WGSL validation.
- Multiple instances sharing code with independent parameter values.
- Native/shader draw interleaving and retained scene replay.
- Logical coordinates, normalized UV, fractional positions and display scales.
- Content clipping, inherited opacity, alpha clamping and scissor restoration.
- Circles, rounded rectangles, Boolean distance operations and shader borders.
- Zero/negative animated stroke widths, including exact boundary pixels.
- Uniform alignment, arena growth, 64 parameter slots and 512 draws.
- Pipeline reuse, inactive cache eviction and revival after eviction.
- Importing materials with remapped parameters and coordinates.
- Coexistence with isolated content blur and backdrop blur.
- Function composition producing identical WGSL, pixels and pipeline identity to
  handwritten expressions.

Core tests additionally cover construction budgets, foreign expression ownership,
every nested tuple leaf, capture rejection, portable function calls, multiple
outputs sharing one graph, semantic value types, dead derivative removal,
program interning, retained drawing updates and scaled geometry overflow.
Compile-fail documentation checks numeric, semantic and function-signature errors.

The harder API audit additionally exercises 128 animation frames with permuted
mixed-type updates, independent instances, portable nested domains, repeated and
distinct function arguments, conflicting bindings, and both read/import orders.
Regression checks cover atomic batches, unchanged storage sharing, signed zero,
live-aware cache eviction, shared WGSL allocation, dead binding elimination and
the separate construction/live-uniform budgets. See the
[audit and resolutions](paint-api-review.md) and the
[ten-framework comparison](paint-comparisons.md).

## Full GPUI hero

`vercel_hero` builds a real GPUI view with native text, buttons and one retained
canvas primitive. Its triangle distance, glow and grain are typed functions; the
example contains no WGSL or raw scene shader insertion. Actual WGPU snapshots at
1280×720 and 640×480 logical pixels were visually inspected by both the parent
and implementation agent. On this host their 2× display scale produces 2560×1440
and 1280×960 images.
The six-panel `paint_gallery` was also rendered on the real GPU and visually
inspected, including fills, borders, circles, combined fields, domain warping
and independently parameterized instances.

The real-window interaction test dispatches mouse movement and button down/up
events. Pointer movement changes more than 1,000 pixels and dimming more than
4,000 pixels in a shader-only region, with a minimum three-level channel change.
That region is converted from logical coordinates to screenshot pixels, so label
changes and DPI mismatches cannot produce false passes. Resume/pause events and
animation-frame delivery are also exercised. Program identity and the retained
drawing's single primitive remain stable. The final strengthened test passed.

Reproduce a deterministic frame:

```sh
cargo run -p gpui_ce_wgpu --features test-support --example vercel_hero -- \
  --headless --output /tmp/gpui-vercel-hero.png \
  --time 3 --pointer 720 350 --size 1280 720
```

## Known baseline failures

Two existing native/WGPU pixel parity tests fail on this machine. Both were
reproduced independently in a managed worktree of untouched commit
`1086d4de39f364559bbdd1ac120ccd1a9f7f28fe`:

| Existing test | Untouched baseline result |
| --- | --- |
| `quad_backgrounds_match_legacy_metal_pixels` | Pixel `(2, 2)`, channel 2: 151 versus 152 |
| `quad_border_backgrounds_use_the_fill_coordinate_space` | Pixel `(1, 6)`: expected `[166, 58, 186, 255]`, got `[168, 58, 187, 255]` |

Baseline command:

```sh
cargo test -p gpui_ce_wgpu --features test-support --lib quad_ --offline
```

The baseline had two failures and one pass. No comparison tolerances were changed.
The temporary baseline worktree was archived after verification.

## Performance methodology

Criterion runs use optimized release code, 10 samples, one second of warmup and
one second of measurement per case. This is a local diagnostic sample, not a
cross-device performance guarantee. The suite separates:

1. Graph construction, equivalent reconstruction, function calls and parameter
   updates. These are CPU operations and perform no GPU work.
2. Scene recording at 1, 64 and 512 primitives.
3. Cold paint pipeline creation and rendering on a fresh renderer. Renderer and
   device setup happen outside the timed closure; driver-level caches may persist.
4. Warm rendering at 1, 64 and 512 primitives with native quads, one shared paint
   program, or a program per distinct root. Warmup asserts no further pipeline
   compilation. The distinct-root case intentionally defeats root interning.

Warm rendering uses a 256×256 target and disjoint 8×8 primitives. It waits for GPU
completion, but performs no pixel readback. Reported time therefore includes CPU
encoding, submission and synchronization, not only GPU execution. No GPU timestamp
queries are collected. Comparing these numbers with a frame rate for a real UI
would require additional representative workloads.

The strongest abstraction-cost check is deterministic: a function-composed and
handwritten material produce exactly the same generated program and share one
GPU pipeline. Function construction/calling still costs CPU time. Applications
should retain the resulting shader and change parameter values during animation.

The final verification host also had unrelated compilations running. That workload
was left untouched. Treat short timings as diagnostic measurements with their
confidence intervals, not clean-machine performance guarantees or proof of small
cross-run improvements. Repeated expansion and multi-parameter updates have
dedicated benchmarks so their costs remain visible after the audit fixes.

## Measured results

Final CPU measurements on an Apple M3 Pro used 20 samples, one second of warmup
and three seconds of measurement per case. The table reports Criterion slope
estimates and 95% confidence intervals from
[paint-performance-final.json](paint-performance-final.json). These operations
construct descriptions or update retained values; they perform no GPU rendering.

| Final implementation operation | Estimate | 95% interval |
| --- | ---: | ---: |
| Update one parameter in a retained shader | 38.4 ns | 37.4–39.3 ns |
| Update one parameter across 64 canvas primitives | 13.17 µs | 12.59–13.87 µs |
| Update three parameters across 64 primitives in one batch | 14.46 µs | 14.18–14.84 µs |
| The same three updates chained separately | 62.32 µs | 53.27–74.82 µs |
| Build a shader making 1,000 identical calls to a retained 1,000-operation function | 877 µs | 845–921 µs |

The batch/chained comparison uses the same executable. Single-shader updates
avoid an intermediate hash table. Changed value tables use `Arc::make_mut`, and
the canvas update memo stores only replacement value tables, avoiding transient
whole-shader reference-count traffic. No-op storage sharing and unchanged pipeline
identity are asserted independently of timings.

The initial broader [rendering sample](paint-performance.json) measured 512 native
quads at 2.54 ms (2.10–2.84 ms) and 512 shared-program shader quads at 2.61 ms
(2.42–2.87 ms), including GPU completion waiting. Their intervals overlap; this
does not establish a meaningful speed difference. A cold shared-program 64-draw
render measured 13.93 ms, excluding device setup. These were earlier snapshots;
the final value-update fixes were remeasured separately. The hero's end-to-end
interactive frame budget was not benchmarked.

[paint-performance-after-audit.json](paint-performance-after-audit.json) retains
the intermediate implementation's measurements. Its extra patch allocation and
temporary Shader clones prompted the final hot-path fixes. The earlier scratch
165 ms repeated-call result used a different harness; do not turn it into a
cross-run speedup claim. Current memoization and program equivalence have direct
structural tests.

Memory costs remain explicit: dynamic uniform alignment pads records, retained
paths still copy vertices on replay, caches bound entry counts rather than exact
driver memory, and the active frame can exceed the inactive pipeline cache budget.
The [API review](paint-api-review.md) details those tradeoffs.

## Platform coverage

| Path | Status |
| --- | --- |
| Native headless WGPU | Real GPU rendering and readback tested |
| WGPU Linux/FreeBSD windows | Integration implemented; not run on this host |
| WebGPU browser windows | Integration implemented; not run in a browser |
| Native macOS Metal windows | Explicitly unsupported for custom paint; native rendering preserved |
| Native Windows D3D11 windows | Explicitly unsupported for custom paint; native rendering preserved |
| WASM target compilation | Not verified; this toolchain has no WASM standard library installed |
| Forced device loss/recovery | Resource ownership reviewed; not exercised by fault injection |

The renderer still issues one draw per shader primitive. Geometry masks may shade
the entire scissored bounding rectangle. Shader-painted paths/text, sampled images,
compute entry points and replacement filter scheduling are not implemented.
Those limits are separate from the correctness of the tested fragment-paint path.
