# Upstream sync

Automated, local-only tooling to pull GPUI changes from the upstream Zed monorepo
(`zed-industries/zed`) into this standalone fork. Conflicts and resulting build
breakage are resolved with `claude -p`. **Nothing is ever pushed.**

## TL;DR

```sh
just sync-upstream-bootstrap   # ONE TIME — records the baseline to sync from
just sync-upstream             # pull upstream changes onto a fresh sync/ branch
just sync-upstream-status      # how far behind upstream are we?
```

`just sync-upstream` leaves a `sync/zed-<date>-<sha>` branch with the upstream delta
merged, conflicts resolved, and the workspace compiling (or a clear report of what's
left). Review it, then fast-forward `main` onto it or open a PR — by hand.

## How it works

Upstream keeps the GPUI crates **in-tree** in the monorepo under `crates/gpui*`. This
fork keeps the same crates at the **same relative paths**, so upstream changes apply at
identical paths here. The script uses a **vendor-branch 3-way merge** (a generalized
`git subtree` merge):

1. A local branch `vendor/zed-gpui` holds a *filtered replay* of upstream's actual
   gpui-touching commits — each keeps its original author/date/message (plus a
   `zed-upstream: <sha>` trailer) but carries only the tracked `crates/gpui*` trees,
   built via a throwaway git index (no working-tree churn). Non-gpui and merge commits
   are dropped.
2. Each sync extends that chain with the new commits and `git merge`s its tip. Git's
   merge base is the previous tip, so the merge replays exactly the upstream delta since
   the last sync — anything this fork already cherry-picked upstream produces **no
   conflict** — and every upstream commit is preserved in the merge's **second-parent
   history** (so `git log` shows both histories; `git log --first-parent` shows just the
   -ce line).
3. The merge is committed as **two commits** for reviewability:
   - **Commit 1 — raw merge:** git's auto-merges applied, conflict markers committed in
     as-is (deterministic add/delete conflicts settled by policy: gpui-ce's deletions
     kept). This captures exactly what git could *not* resolve.
   - **Commit 2 — resolution:** `claude -p` (`resolve-conflicts.prompt.md`, looped up to
     `--retries`) edits out the markers. Because it's a separate commit, its diff shows
     *exactly* what was chosen — auditable in isolation, distinct from git's auto-merge.
   (A conflict-free sync is just one clean merge commit.)
4. The pinned `zed-industries/zed` git-dep revs in the root `Cargo.toml` are bumped to
   the synced commit, then `just check` runs; on failure `claude -p`
   (`fix-build.prompt.md`) is looped up to `--retries` times to make it compile —
   committed as a **third** commit. **Tests are not run or fixed automatically.**

### Tracked vs. untracked

Synced 1:1: `gpui`, `gpui_linux`, `gpui_macos`, `gpui_macros`, `gpui_platform`,
`gpui_shared_string`, `gpui_tokio`, `gpui_web`, `gpui_wgpu`, `gpui_windows`.

Left untouched: `gpui_util` (consumed here as a git dep, not vendored),
`crates/gpui_elements` (fork-only stub), `tooling/perf` (fork-only).

## One-time bootstrap

The script needs to know which upstream commit this fork currently corresponds to, to
use as the merge base. `just sync-upstream-bootstrap` defaults to the
`zed-industries/zed` rev already pinned in `Cargo.toml` (currently `876ec5a8…`), which
is the best automatic guess. If you know a more accurate baseline (e.g. the commit of
the last "re-re-fork"), pass it:

```sh
just sync-upstream-bootstrap <upstream-sha>
```

Bootstrap adds the `zed` remote, builds the baseline vendor snapshot, records it in the
current branch's history with a **no-op** `-s ours` merge (no files change), and writes
`state.json`. Run it once on `main`. A too-old baseline just means the first real sync
has more to merge; correctness is unaffected.

## Files

| File | Purpose |
|------|---------|
| `sync_upstream.py` | orchestrator (git plumbing + `claude -p` loops); stdlib-only, fully typed |
| `resolve-conflicts.prompt.md` | rules for the conflict-resolution `claude -p` pass |
| `fix-build.prompt.md` | rules for the build-fix `claude -p` pass |
| `state.json` | committed: last synced upstream sha + vendor tip |

## Config / env overrides

Every default near the top of `sync_upstream.py` is overridable via a `SYNC_*` env var:

```sh
SYNC_MODEL=sonnet just sync-upstream          # cheaper model
SYNC_RETRIES=5    just sync-upstream           # more claude passes
SYNC_VERIFY_CMD="just ci-test" just sync-upstream
just sync-upstream --ref some-tag --no-bump --dry-run
```

## Caveats

- The compile gate is host-only (`just check` / `cargo check --workspace`). macOS- and
  Windows-specific changes can't be fully verified on a Linux host — verify those on the
  platform or in CI. The build-fix prompt asks claude to flag such changes.
- Requires Python 3 (stdlib only — no pip installs), the `claude` CLI on `PATH`, and a
  working `just` + Rust toolchain. The `just` recipes shell out to `sync_upstream.py`.
- If conflict resolution exhausts its retries, the branch is left mid-merge for you to
  finish. If the build can't be fixed in time, the merge is committed and the branch is
  left with the remaining errors plus a clear report.
- `--allowedTools` is passed as a single space-separated string; if your `claude` CLI
  version expects a different format, adjust `SYNC_CLAUDE_ALLOWED_TOOLS`.
