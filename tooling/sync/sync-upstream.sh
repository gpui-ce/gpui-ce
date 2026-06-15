#!/usr/bin/env bash
#
# Incrementally port upstream Zed `gpui*` commits into gpui-ce.
#
# This script walks Zed's history forward from the last-synced commit, and for 
# each commit that touches one of the forked crates it builds a path-filtered patch
# and applies it with `git am --3way`. That preserves the *original author, date and
# commit message*. A trailer with `Upstream-commit:` adds the source SHA for traceability.
#
# Usage:
#   tooling/sync/sync-upstream.sh status            # cursor + pending commits
#   tooling/sync/sync-upstream.sh next              # apply next commit (stops on conflict)
#   tooling/sync/sync-upstream.sh continue          # finish a conflicted `git am`
#   tooling/sync/sync-upstream.sh abort             # abort an in-progress `git am`
#   tooling/sync/sync-upstream.sh verify            # fmt + clippy + tests
#   tooling/sync/sync-upstream.sh show <sha>        # preview a commit's filtered patch
#
# Env:
#   ZED_REPO   path to a local Zed checkout
#   TARGET     upstream ref to sync up to (defaults to HEAD)

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(git -C "$HERE" rev-parse --show-toplevel)"
AM_CURSOR="$ROOT/.git/sync-am-current"   # remembers the SHA of an in-flight `git am`

c_red=$'\033[31m'; c_grn=$'\033[32m'; c_yel=$'\033[33m'; c_dim=$'\033[2m'; c_off=$'\033[0m'
say()  { printf '%s\n' "$*" >&2; }
die()  { printf '%s%s%s\n' "$c_red" "$*" "$c_off" >&2; exit 1; }
ok()   { printf '%s%s%s\n' "$c_grn" "$*" "$c_off" >&2; }
warn() { printf '%s%s%s\n' "$c_yel" "$*" "$c_off" >&2; }

[ -n "${ZED_REPO:-}" ] || die "ZED_REPO is not set. Point it at a local Zed checkout: ZED_REPO=/path/to/zed $(basename "$0") ..."
[ -d "$ZED_REPO/.git" ] || die "ZED_REPO=$ZED_REPO is not a git repo."

# Forked crate paths = gpui* crates present in BOTH repos.
# TODO: A list here instead?
# (Excludes gpui-ce-only crates like gpui_elements and Zed-only ones like gpui_util.)
crate_paths() {
  comm -12 \
    <(cd "$ROOT"      && ls -d crates/gpui*/ 2>/dev/null | sed 's#/$##' | sort) \
    <(cd "$ZED_REPO"  && ls -d crates/gpui*/ 2>/dev/null | sed 's#/$##' | sort)
}

# The newest Upstream-commit trailer in our history, or the
# rev pinned for Zed git deps in Cargo.toml.
current_rev() {
  local rev
  rev="$(git -C "$ROOT" log -50 --format='%(trailers:key=Upstream-commit,valueonly)' \
         | grep -m1 -oE '[0-9a-f]{7,40}' || true)"
  if [ -z "$rev" ]; then
    rev="$(grep -m1 -oE 'rev = "[0-9a-f]{40}"' "$ROOT/Cargo.toml" | grep -oE '[0-9a-f]{40}')"
  fi
  [ -n "$rev" ] || die "Could not determine the current synced rev."
  git -C "$ZED_REPO" rev-parse --verify "${rev}^{commit}" 2>/dev/null \
    || die "Rev $rev not found in $ZED_REPO. Is your Zed checkout up to date? (git -C $ZED_REPO fetch)"
}

target_rev() { git -C "$ZED_REPO" rev-parse --verify "${TARGET:-HEAD}^{commit}"; }

# SHAs of commits touching the forked crates, oldest first.
pending() {
  local from="$1" to="$2"
  # shellcheck disable=SC2046
  git -C "$ZED_REPO" log --reverse --format='%H' "${from}..${to}" -- $(crate_paths)
}

# Path-filtered patch for one commit (only the forked crates).
patch_for() {
  # shellcheck disable=SC2046
  git -C "$ZED_REPO" format-patch -1 --stdout "$1" -- $(crate_paths)
}

# Bump the Zed git-dependency rev in Cargo.toml
bump_cargo_rev() {
  local newsha="$1" oldsha
  oldsha="$(grep -m1 -oE 'zed-industries/zed", rev = "[0-9a-f]{40}"' "$ROOT/Cargo.toml" \
            | grep -oE '[0-9a-f]{40}' || true)"
  [ -n "$oldsha" ] || { warn "No zed rev found in Cargo.toml; skipping pin bump."; return 0; }
  [ "$oldsha" = "$newsha" ] && { git -C "$ROOT" add Cargo.toml Cargo.lock 2>/dev/null || true; return 0; }
  sed -i "s/$oldsha/$newsha/g" "$ROOT/Cargo.toml"
  # Let cargo re-resolve Cargo.lock
  if ( cd "$ROOT" && cargo metadata --format-version 1 >/dev/null 2>&1 ); then
    git -C "$ROOT" add Cargo.toml Cargo.lock
  else
    warn "cargo couldn't refresh Cargo.lock (offline / objects not fetched). Staged Cargo.toml only;"
    warn "the 'verify' build will refresh Cargo.lock. Use 'git commit --amend' to add it back in."
    git -C "$ROOT" add Cargo.toml
  fi
}

# Finalize the freshly-applied commit. Bump the pin and append the trailer.
finalize_commit() {
  bump_cargo_rev "$1"
  GIT_EDITOR=true git -C "$ROOT" commit --amend --no-edit \
    --trailer "Upstream-commit: $1" >/dev/null
}

cmd_status() {
  local from to ; from="$(current_rev)"; to="$(target_rev)"
  say "Zed repo:   $ZED_REPO"
  say "Crates:     $(crate_paths | tr '\n' ' ')"
  say "Synced to:  ${c_dim}$(git -C "$ZED_REPO" log -1 --format='%h %s' "$from")${c_off}"
  say "Target:     ${c_dim}$(git -C "$ZED_REPO" log -1 --format='%h %s' "$to")${c_off}"
  if [ -f "$AM_CURSOR" ]; then
    warn "An in-progress git am is recording $(cat "$AM_CURSOR"). Run 'continue' or 'abort'."
  fi
  local list count
  list="$(pending "$from" "$to")"; count="$(printf '%s' "$list" | grep -c . || true)"
  say ""
  say "Pending: $count commit(s)"
  [ "$count" -gt 0 ] && git -C "$ZED_REPO" log --reverse --oneline "${from}..${to}" -- $(crate_paths)
  return 0
}

cmd_next() {
  [ -f "$AM_CURSOR" ] && die "A git am is already in progress for $(cat "$AM_CURSOR"). Run 'continue' or 'abort' first."
  [ -z "$(git -C "$ROOT" status --porcelain --untracked-files=no)" ] || die "Tracked files have uncommitted changes. Commit or stash before syncing."
  local from to sha ; from="$(current_rev)"; to="$(target_rev)"
  sha="$(pending "$from" "$to" | head -n1)"
  [ -n "$sha" ] || { ok "In sync with $to."; return 0; }

  say "Applying ${c_yel}$(git -C "$ZED_REPO" log -1 --format='%h %s' "$sha")${c_off}"
  printf '%s' "$sha" > "$AM_CURSOR"

  local before; before="$(git -C "$ROOT" rev-parse HEAD)"
  if patch_for "$sha" | git -C "$ROOT" am --3way --keep-non-patch; then
    if [ "$(git -C "$ROOT" rev-parse HEAD)" = "$before" ]; then
      # `git am` made no commit ("Patch already applied").
      # Record an empty marker (preserving the upstream author) so the cursor advances.
      git -C "$ZED_REPO" log -1 --format='%B' "$sha" \
        | git -C "$ROOT" commit --allow-empty -q -F - \
            --author="$(git -C "$ZED_REPO" log -1 --format='%an <%ae>' "$sha")" \
            --date="$(git -C "$ZED_REPO" log -1 --format='%aD' "$sha")"
      warn "No changes. Recorded an empty marker commit."
    fi
    finalize_commit "$sha"; rm -f "$AM_CURSOR"
    ok "Applied. Now run: $(basename "$0") verify"
  else
    warn ""
    warn "Conflict applying $sha. Resolve it, then run '$(basename "$0") continue':"
    warn "  1. Edit the conflicted files (git status), 'git add' them."
    warn "  2. Run: $(basename "$0") continue"
    warn "If the commit is irrelevant to gpui-ce, run: $(basename "$0") abort."
    exit 1
  fi
}

cmd_continue() {
  [ -f "$AM_CURSOR" ] || die "No sync in progress (missing $AM_CURSOR)."
  local sha; sha="$(cat "$AM_CURSOR")"
  # If the user resolved & staged, `git am --continue` will commit. Empty patch -> skip.
  if git -C "$ROOT" am --continue; then
    finalize_commit "$sha"; rm -f "$AM_CURSOR"
    ok "Resolved and applied $sha. Now run: $(basename "$0") verify"
  else
    die "git am --continue failed. Resolve remaining conflicts (git status) and retry 'continue'."
  fi
}

cmd_abort() {
  git -C "$ROOT" am --abort 2>/dev/null || true
  rm -f "$AM_CURSOR"
  warn "Aborted in-progress git am."
}

cmd_verify() {
  cd "$ROOT"
  say "▶ cargo fmt --check"
  cargo fmt --all -- --check
  say "▶ cargo clippy --workspace --all-targets -D warnings"
  cargo clippy --workspace --all-targets -- -D warnings
  say "▶ tests"
  if cargo nextest --version >/dev/null 2>&1; then
    cargo nextest run --workspace
  else
    cargo test --workspace
  fi
  ok "verify passed (fmt + clippy + tests)."
}

cmd_show() { [ -n "${1:-}" ] || die "usage: show <sha>"; patch_for "$1"; }

# Manually re-pin the Zed git deps to a SHA. Use this to fix up a batch by hand.
cmd_bump() {
  local sha; sha="${1:-$(current_rev)}"
  sha="$(git -C "$ZED_REPO" rev-parse --verify "${sha}^{commit}")"
  bump_cargo_rev "$sha"
  ok "Pinned Zed git deps to $sha (staged). Commit or amend as needed."
}

case "${1:-status}" in
  status)   cmd_status ;;
  next)     cmd_next ;;
  continue) cmd_continue ;;
  abort)    cmd_abort ;;
  verify)   cmd_verify ;;
  bump)     shift; cmd_bump "${1:-}" ;;
  show)     shift; cmd_show "$@" ;;
  *) die "unknown command '${1}'. One of: status next continue abort verify bump show" ;;
esac
