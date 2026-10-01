# Paint API adversarial review

This review used concrete consumer scenarios and read the paint core, functions,
fields, retained canvas, WGPU paint renderer, and scene insertion code. Production
code was not changed. Findings describe the snapshot reviewed on 26 September 2026;
implementation fixes may subsequently supersede them.

## Resolution after the implementation rereview

The following resolutions were confirmed by reading the changed production files.
These statements describe the implementation; they do not claim integration tests or
new benchmark results. The initial measurements below remain evidence for the old
snapshot, not measurements of the corrected implementation.

- Function calls now memoize the first 128 distinct calls with nonempty input and
  output leaves. Keys include a unique immutable function ID and every ordered
  argument node ID. The cache belongs to the destination builder. Foreign leaf
  provenance still marks the graph invalid before a possible cache hit. This fixes
  the repeated identical argument-bearing call case. Calls beyond the bounded memo
  budget, and unit-input functions, still use instruction interning and O(N) remapping.
- Shader and drawing APIs now accept typed parameter batches. Shader batches validate
  every key before changing data and copy a changed table once. Drawing batches validate
  every requested key against the entire drawing before applying replacements; they
  memoize replacement value tables using program identity plus source value-table and binding-
  table identities. Equal numeric values with incompatible handles cannot collide.
  Drawing commands are shared through Arc storage; empty and unchanged batches reuse
  that storage. Key-validation scans are still O(KD), with at most 64 live shader slots.
  A follow-up hot-path review removed temporary whole-Shader clones from that memo and
  restored direct single-parameter updates without an intermediate patch hash table.
- Root caches now share one Arc WGSL source between their key, insertion-order entry,
  and Program. Eviction prefers descriptions without a live Shader. The original
  hot-program case is addressed while capacity remains bounded. If all 256 retained
  descriptions are live, the oldest must still be evicted; identity reconstruction
  reuse beyond that bounded live working set is not promised. Rebuilding equivalent
  shaders still allocates their own compact node/dependency and handle tables.
- Missing handles can initially create unresolved bindings. Explicit material import
  resolves those bindings regardless of read/import order, for both root-declared
  and builder-local handles. Finishing rejects a reachable unresolved binding; dead
  unresolved reads disappear. Only a root declaration under its own root supplies
  a default without an imported material. The parent found and corrected the remaining
  builder-local ordering case during the final reread.
- Finished shaders now retain only reachable uniform bindings and enforce the 64-slot
  limit on those live bindings. Canonical first-use packing rewrites the emitted
  uniform indices. Hidden dead declarations no longer change shader layout or remain
  updateable. Construction still has a separate combined 4,096-node/binding budget.
- Public static Shape fields now contribute constants rather than private uniforms,
  and `field_at` takes an explicit local point. Closed functions can therefore use
  the same fixed geometry descriptor. Canvas masks keep their deliberate private
  geometry uniforms so changing dimensions continues to reuse pipelines.
- Renderer pruning collects and sorts inactive candidates once, removing the former
  repeated full-cache scan. It preserves every current-frame pipeline.

Remaining limitations are the fallible canvas closure, O(V) retained path replay,
alignment padding, one shader draw per primitive, area-based shader-border rasterization,
per-finish immutable graph storage, and bounded memo/cache working sets. Canvas masks
add one internal geometry slot to the composed material's 64-slot budget; a source
material using all 64 slots therefore cannot also fit that mask. None was
established as a new blocking correctness bug during the final source rereview.

The findings below are retained as the audit trail and motivation for these changes.

## Highest-value small fixes

### Repeated function calls share emitted nodes, but repeat all construction work

[ShaderBuilder::call](../crates/gpui/src/paint/function.rs)
allocates flattened argument leaves, an instruction mapping, output IDs, and a new
dependency vector for each instruction on every call. Interning prevents duplicate
emitted nodes; it does not prevent the remapping and hashing work.

For R identical calls to an N-node function, construction is O(RN), even if the
finished program has only O(N) nodes. In a release-mode standalone Rust harness,
1,000 identical calls to a 1,000-operation sine function took approximately 165 ms
and emitted 1,003 nodes. This is an illustrative local measurement, not a portable
performance promise. Distinct argument values correctly emitted distinct sine
instructions in the same harness.

A bounded per-builder call-result cache keyed by function identity and ordered
argument node IDs would eliminate this repeated work. The key must include every
argument; function identity alone is incorrect. Capture-free functions make this
cache particularly straightforward. Do not add this cache to immutable functions:
the results belong to the destination graph.

### Animated multi-value updates multiply allocations and traversals

[Shader::with_parameter](../crates/gpui/src/paint.rs)
copies the entire uniform table for one value change. Updating K handles costs
O(KP) table work for P parameters and allocates K replacement tables.

[CanvasDrawing::with_parameter](../crates/gpui/src/paint/canvas.rs)
also clones D commands and scans their paints. For M matching primitives, each
single update allocates M independent tables, including when the original paints
shared one Shader instance. A hero with 1,000 repeated tiles and separately animated
phase, tint, and mask therefore performs three drawing clones/traversals and up to
3,000 uniform-table replacements per frame. Shape-specific mask uniforms mean not
every masked paint can share one replacement table; preserve that distinction.

A typed batch-update closure can validate updates and copy each table once. A
consuming/COW update method is a smaller alternative for disposable instances.
Memoize replacement tables only for truly identical source instance data, including
logical binding identities; equality based solely on program ID and numeric values
does not establish handle compatibility.

### Hot program identity is lost when the FIFO cache fills

[PaintRoot::finish](../crates/gpui/src/paint.rs)
does not refresh cache age on a hit. A hot description is evicted after enough new
programs, even while an equivalent Shader remains alive outside the cache. Rebuilding
that description then creates a new program ID, which means a new GPU pipeline.

The standalone harness kept a hot Shader alive, built 257 distinct one-off shaders,
and rebuilt the hot shader: its ID changed. This is not a rendering correctness bug,
but it contradicts consumers' expectation that retaining a material preserves
reconstruction reuse. Refresh recency on hits, or retain a weak lookup of evicted
descriptions that still have live owners. Keep the policy bounded and explicit.

## Composition pain exposed by ordinary use

### Cross-root imported handle reads depend on construction order

[ShaderBuilder::uniform](../crates/gpui/src/paint.rs)
accepts a handle already present in the destination binding table, otherwise checks
the declaring root. In a different root, reading a handle before sampling its owning
material fails `ForeignGraph`; sampling first makes the same read valid. Same-root
default/instance precedence is independent of order, but cross-root importing has
this additional constraint. The consumer stress agent independently identified this
case. Either validate imported ownership after construction, or state the import-first
requirement explicitly beside the otherwise broad order-independence documentation.

### Shape helpers hide parameter allocation and ambient coordinates

[Shape::distance](../crates/gpui/src/paint/canvas.rs)
allocates a private parameter slot each time. Calling the same static shape helper
65 times fails the parameter budget even though the values are identical and callers
have no handles for updating those slots. These slots survive graph dead-node pruning.

The helper also reads implicit position through
[distance_fields](../crates/gpui/src/paint/canvas.rs).
Consequently the shared Shape vocabulary cannot be used in a capture-free Function
with explicit position arguments. Removing hidden uniforms alone does not solve the
ambient coordinate dependency.

A static descriptor helper should use constant descriptor fields and offer an
explicit-position form such as `field_at(cx, point)`. Keep the canvas recorder's
private geometry uniforms: those deliberately allow varying dimensions without
changing pipeline identity. The convenient implicit-position form can delegate to
the explicit form.

### Fallible work cannot propagate through the canvas recording closure

[PaintRoot::canvas](../crates/gpui/src/paint/canvas.rs)
accepts a closure returning unit. A consumer constructing a shader or function
inside that closure cannot use `?`; they must prebuild everything or maintain an
outer error variable. The recorded primitives' deferred error handling does not
cover arbitrary user composition work.

A `try_canvas` variant accepting `Result<(), ShaderError>` would preserve existing
ergonomics while making fallible composition ordinary Rust. Avoid introducing a
general-purpose error framework for this narrow inconvenience.

### Dead bindings remain operational after dead expressions disappear

[PaintRoot::finish](../crates/gpui/src/paint.rs)
packs referenced slots first, then appends every unreferenced binding. A dead sample
of a material with 64 declared slots leaves all 64 slots in an otherwise constant
shader. The harness observed 64 parameter slots for that case. The finished shader
also reports those handles present and its WGSL uniform array retains their count.

Keeping declared handles through transforms is an intentional current contract;
do not silently remove it as a cleanup. The user-facing tradeoff should be explicit:
construction limits and mutable binding retention are separate from reachable GPU
code. A future explicit compact operation could discard unused handles. A smaller
immediate improvement is documenting this exact scenario beside sampling, not only
beside parameter declaration.

## Performance and memory accounting

- Graph construction stores a node in both its ordered vector and its intern map;
  each unique node's dependency vector is copied by
  [Graph::push](../crates/gpui/src/paint.rs).
  Finished graphs are compacted separately. This is bounded by 4,096 nodes, but
  dependency allocations dominate repeated calls before call caching.
- Equivalent shader reconstructions share Program metadata, but each finish still
  allocates canonical node/dependency storage and logical bindings. Moving canonical
  nodes into shared Program storage would remove duplicate immutable graphs, provided
  per-instance handle mappings remain separate.
- The reviewed cache stored WGSL as its lookup key, FIFO entry, and Program string.
  That is roughly three source copies per retained description. The parent identified
  this independently; shared source ownership is an appropriate small fix.
- Retained path storage is shared, but
  [CanvasDrawing::paint_at](../crates/gpui/src/paint/canvas.rs)
  validates every vertex before replay, clones vertices, and translates them.
  [Window::paint_path](../crates/gpui/src/window.rs)
  then scales the submitted path for the scene. Replay is therefore O(V), with a
  vertex-buffer clone; it is not constant-time retained geometry. The current docs
  correctly disclose the clone. Improving this requires the existing path submission
  representation, so it is not a quick paint-only patch.
- With the usual 256-byte uniform offset alignment, a no-parameter draw uses 64 bytes
  of draw data plus one 16-byte parameter slot but advances the arena about 512 bytes
  per draw. Ten thousand such draws upload about 5.12 MB, rather than 0.8 MB of useful
  data. [WGPU prepare](../crates/gpui_wgpu/src/wgpu_renderer/paint.rs)
  intentionally trades space for simple dynamic binding. Shared parameter uploads or
  future instancing may reduce this; do not present current uploads as tightly packed.
- Draw count remains one submission per primitive, with pipeline/bind-group/scissor
  operations per shader draw. Cache reuse avoids compilation, not draw-call overhead.
  Hardware scissors now bound fragment work. Inward shader borders still rasterize
  their outer bounding rectangle, so a thin border on a large panel remains an
  area-based workload.
- Renderer cache pruning was changed during this audit to collect/sort inactive
  candidates once, resolving the previously quadratic repeated scan. The current
  frame may exceed the cache budget intentionally; active pipelines cannot be evicted
  before drawing them.

## Correctness conclusions

The reviewed function implementation checks provenance for every input/output leaf,
rejects reachable ambient captures, substitutes argument IDs correctly, shares work
between multiple outputs, preserves Coverage's bounded public construction invariant,
and summarizes reachable derivative use. Scratch distinct-argument composition matched
the expected emitted graph. No additional blocking correctness bug was established.

The scratch harness used the implementation agent's standalone core snapshot with a
stub canvas Shape. It validates core behavior and illustrative construction costs;
the Shape/canvas findings above come from direct source inspection. It is not a GPU
benchmark and does not replace the verification agent's integration/GPU tests.
