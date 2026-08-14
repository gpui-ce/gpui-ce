# gpui_wasm_thread

A vendored copy of [`wasm_thread`](https://github.com/chemicstry/wasm_thread) 0.3.3
by Jurgis Balciunas, dual-licensed Apache-2.0 OR MIT (see `LICENSE-APACHE` and
`LICENSE-MIT`).

It is vendored rather than depended on because upstream is unmaintained and the
published 0.3.3 worker bootstrap still calls the wasm-bindgen init function with
the positional arguments deprecated in wasm-bindgen 0.2.93 — gpui-ce is on
0.2.126.

## Local patch

The only change against published 0.3.3, applied to both
`src/wasm32/js/web_worker.js` and `src/wasm32/js/web_worker_module.js`:

```diff
-    let [ module, memory, work ] = event.data;
+    const [module_or_path, memory, work] = event.data;

-    init(module, memory).catch(err => {
+    init({ module_or_path, memory }).catch(err => {
```

(`web_worker.js` calls the global `wasm_bindgen(...)` rather than `init(...)`;
the shape of the fix is identical.)

Keep this list current if the crate is patched further, so a future upgrade to a
maintained release can be checked off against it.

## Requires nightly on wasm32

`src/lib.rs` opens with upstream's own
`#![cfg_attr(target_arch = "wasm32", feature(stdarch_wasm_atomic_wait))]`, so
anything enabling `gpui_web/multithreaded` for `wasm32-unknown-unknown` has to
build on nightly:

```
cargo +nightly check --target wasm32-unknown-unknown -p gpui_web --features multithreaded
```

On stable it fails with `E0554`. This is inherent rather than a consequence of
vendoring — wasm atomics are nightly-only anyway, which is why `just
check-wasm-atomics` also runs under `+nightly`.

## Upgrading

The vendored tree is rustfmt-normalized by this workspace's `cargo fmt --all`,
so it does not match published 0.3.3 byte-for-byte. To compare against a newer
release: extract it, apply the patch above, run `cargo fmt` on it, then diff
against this directory.

If upstream ever ships the fix, drop this crate and depend on the release
directly — the `wasm_thread` workspace alias means no call site has to change.
