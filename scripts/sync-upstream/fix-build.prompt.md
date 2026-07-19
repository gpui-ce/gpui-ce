## Repository context

`gpui-ce` is a standalone fork of Zed's GPUI. A 3-way merge of upstream Zed GPUI changes was just
committed, and the pinned `zed-industries/zed` git-dependency revisions were bumped to match the
synced commit. The result has a problem the sync introduced — a compile error, a compile warning,
or a test failure (see the output above). Fix it so the gate passes.

## Rules

1. **Fix only what the merge/sync caused.** Address the issues in the output: items moved/renamed
   upstream, changed function signatures or trait bounds, added/removed enum variants, and any
   fallout in gpui-ce's own patches. NOTE: the util crates (`collections`, `util`, `gpui_util`,
   `sum_tree`, `refineable`, `scheduler`, `media`) are now **vendored in-tree** as `gpui_collections`,
   `gpui_zed_util`, `gpui_ce_util`, `gpui_sum_tree`, `gpui_refineable`, `gpui_scheduler`,
   `gpui_media` and are **synced by this same tool** — so if gpui needs a new API from one of them,
   it should already be present from the merge. Prefer using that API; only hand-add to a vendored
   crate if the merge genuinely didn't bring it (and say so in your summary).

2. **Compile warnings.** Fix every compile warning the merge introduced (unused imports/variables,
   unreachable code, deprecated APIs, etc.) by addressing the **root cause** — the synced branch
   must be warning-clean to pass CI. Do **not** silence warnings with `#[allow(...)]`, `_`-prefixes,
   or `#[allow(dead_code)]` unless that is genuinely the correct fix.

3. **Test failures.** Fix the underlying cause. Do **not** delete tests, add `#[ignore]`, weaken or
   delete assertions, or otherwise change a test just to make it pass. If an upstream change
   legitimately changes behavior, update the test to match upstream's intent — and call that out in
   your summary. Note that some tests may fail for environmental reasons (e.g. no display); flag
   those rather than "fixing" them.

4. **Prefer minimal, idiomatic changes** consistent with how upstream intends the new API to be
   used, and matching the surrounding gpui-ce code style. Preserve gpui-ce's existing features
   (blur, kinetic scrolling, wgpu device-loss API, etc.); if an upstream API change requires
   updating a gpui-ce patch, update the patch correctly.

5. **Do not** edit `tooling/perf` or `crates/gpui_elements` unless one of them is the actual source
   of an issue. Do not run `git commit`, `git merge`, or `git push` (the surrounding script commits
   and re-runs the gate). You may run `cargo check` / `cargo build` / `cargo test` to verify.

6. If an issue stems from the **root `Cargo.toml`** (a workspace dependency that must be added or
   updated to match upstream's new requirements), fix it there using gpui-ce's sourcing convention
   (git deps from `zed-industries/zed` for the crates listed above; crates.io versions otherwise).

When finished, briefly summarize the fixes and anything a human should double-check (especially
changes to macOS/Windows-only code that this host can't fully compile, and any tests you judged to
be failing for environmental rather than correctness reasons).
