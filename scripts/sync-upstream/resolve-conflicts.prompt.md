## Repository context

`gpui-ce` is a standalone community fork of Zed's GPUI. It vendors these crates from the
upstream Zed monorepo (`zed-industries/zed`) at the **same relative paths**:
`crates/gpui`, `crates/gpui_linux`, `crates/gpui_macos`, `crates/gpui_macros`,
`crates/gpui_platform`, `crates/gpui_shared_string`, `crates/gpui_tokio`, `crates/gpui_web`,
`crates/gpui_wgpu`, `crates/gpui_windows`.

A 3-way `git merge` of the upstream delta is in progress and produced conflicts. Your job is to
resolve **every** conflict so the result is correct gpui-ce code that incorporates the upstream
changes.

## Rules

1. **Resolve every conflict marker** (`<<<<<<<`, `=======`, `>>>>>>>`, `|||||||`) in the listed
   files. Leave no markers behind. Do not touch files that aren't conflicted.

2. **gpui-ce keeps its own patches.** gpui-ce carries features/fixes not yet upstream (e.g. blur
   filters, kinetic scrolling on Wayland, the wgpu device-loss API). When a conflict pits an
   upstream change against a gpui-ce patch, **keep both behaviours** — integrate the upstream
   change around gpui-ce's additions rather than dropping either. Only drop a gpui-ce line if the
   upstream change genuinely supersedes it.

3. **Already-present (cherry-picked) changes.** gpui-ce frequently contributes to and cherry-picks
   from upstream, so an upstream commit may already be present here under a different hash. If a
   conflict exists *only* because the change is **already applied** in gpui-ce (semantically
   equivalent, even if worded differently), keep gpui-ce's version and do **not** duplicate the code.

4. **Per-crate `Cargo.toml` (`crates/gpui*/Cargo.toml`):**
   - KEEP gpui-ce packaging: the internal package name stays `gpui` (it is published to crates.io
     as `gpui-ce` via registry config — do **not** rename it), plus `publish` settings, workspace
     metadata, and `edition = "2024"`.
   - KEEP gpui-ce's dependency *sources*: several crates (`collections`, `util`, `gpui_util`,
     `sum_tree`, `refineable`, `scheduler`, `util_macros`, `media`) are pulled as **git deps from
     `zed-industries/zed`** in the root `Cargo.toml`, and font-kit is the `zed-font-kit` fork. Do
     not convert these to path or crates.io deps.
   - ADOPT upstream's changes: newly added/removed dependencies, new features, version bumps, and
     new `[target.'cfg(...)']` blocks. Wire any newly-required workspace dependency through the
     same gpui-ce convention.

5. **gpui-ce-only items — never overwrite with upstream's version, never delete:**
   `crates/gpui_elements` (stub) and `tooling/perf` are gpui-ce-only. `gpui_util` is **not**
   vendored here — it's an external git dep; if an upstream change assumes `gpui_util` is a local
   path crate, keep gpui-ce's external-dep usage.

6. **Removed Zed-app / AGPL code.** gpui-ce stripped Zed-application-specific and non-Apache code.
   If an upstream change references a crate or module that doesn't exist in gpui-ce, drop that
   reference rather than reintroducing the removed code.

7. **Add/delete conflicts are handled for you** — the script settles `modify/delete` cases
   (files gpui-ce deleted that upstream changed are kept deleted) before calling you, so you
   only ever see content conflicts. Don't recreate a deleted file.

8. **Do not** run `git commit`, `git merge`, `git rebase`, or `git push`. Only edit files to
   resolve the conflicts — the surrounding script stages and commits. Do not change anything
   unrelated to the conflicts.

When finished, briefly summarize what you resolved and any decisions worth a human's attention.
