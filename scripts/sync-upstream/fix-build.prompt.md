## Repository context

`gpui-ce` is a standalone fork of Zed's GPUI. A 3-way merge of upstream Zed GPUI changes was just
committed, and the pinned `zed-industries/zed` git-dependency revisions were bumped to match the
synced commit. The workspace no longer compiles. Fix it so the build command passes.

## Rules

1. **Fix only what's needed to compile.** Address the errors in the build output: items
   moved/renamed upstream, changed function signatures or trait bounds, added/removed enum
   variants, and API changes from the bumped zed git-deps (`collections`, `util`, `gpui_util`,
   `sum_tree`, `refineable`, `scheduler`, `util_macros`, `media`).

2. **Prefer minimal, idiomatic changes** consistent with how upstream intends the new API to be
   used, and matching the surrounding gpui-ce code style.

3. **Do not** disable features, comment out code, add blanket `#[allow(...)]`, or stub out
   functions just to silence errors. Preserve gpui-ce's existing features (blur, kinetic
   scrolling, wgpu device-loss API, etc.). If an upstream API change requires updating a gpui-ce
   patch, update the patch correctly.

4. **Do not** edit `tooling/perf` or `crates/gpui_elements` unless one of them is the actual source
   of an error. Do not run `git commit`, `git merge`, or `git push` (the surrounding script commits
   and re-runs the build). You may run `cargo check` / `cargo build` to verify your fixes.

5. If an error stems from the **root `Cargo.toml`** (a workspace dependency that must be added or
   updated to match upstream's new requirements), fix it there using gpui-ce's sourcing convention
   (git deps from `zed-industries/zed` for the crates listed above; crates.io versions otherwise).

When finished, briefly summarize the fixes and anything a human should double-check (especially
changes to macOS/Windows-only code that this host can't fully compile).
