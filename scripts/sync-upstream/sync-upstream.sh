#!/usr/bin/env bash
#
# sync-upstream.sh — bring upstream Zed GPUI changes into the standalone gpui-ce fork.
#
# Strategy: a vendor-branch 3-way (subtree-style) merge. We keep a local branch
# (vendor/zed-gpui) holding *filtered* snapshots of upstream's tracked crates/gpui*
# trees. Merging a new snapshot replays exactly the upstream delta since the last
# sync; git's merge base makes already-cherry-picked changes no-op. Conflicts and
# resulting build breakage are handed to `claude -p`, bounded by a retry count.
#
# Local-only. This script NEVER pushes and never lets claude push.
#
# Usage:
#   sync-upstream.sh bootstrap [BASELINE_SHA]   # one-time setup (see README)
#   sync-upstream.sh sync [REF] [options]       # default subcommand
#   sync-upstream.sh status
#
# Options (sync):
#   --ref REF        upstream ref to sync to (default: main / SYNC_ZED_REF)
#   --model NAME     claude model (default: opus / SYNC_MODEL)
#   --retries N      max claude passes per phase (default: 3 / SYNC_RETRIES)
#   --no-bump        don't bump pinned zed git-dep revs
#   --dry-run        show what would be synced, then stop
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# shellcheck source=config.sh
source "$SCRIPT_DIR/config.sh"
STATE_FILE="$REPO_ROOT/$STATE_FILE_REL"

BUILD_LOG="$(mktemp)"
BUILD_OK=0
cleanup() { rm -f "$BUILD_LOG" "$BUILD_LOG".* 2>/dev/null || true; }
trap cleanup EXIT

# ── output helpers ───────────────────────────────────────────────────────────
if [ -t 1 ]; then
  C_B=$'\e[34m'; C_G=$'\e[32m'; C_Y=$'\e[33m'; C_R=$'\e[31m'; C_D=$'\e[2m'; C_0=$'\e[0m'
else C_B=; C_G=; C_Y=; C_R=; C_D=; C_0=; fi
log()  { printf '%s %s\n' "${C_B}▶${C_0}" "$*"; }
ok()   { printf '%s %s\n' "${C_G}✓${C_0}" "$*"; }
warn() { printf '%s %s\n' "${C_Y}⚠${C_0}" "$*" >&2; }
die()  { printf '%s %s\n' "${C_R}✗${C_0}" "${C_R}$*${C_0}" >&2; exit 1; }

usage() { sed -n '3,23p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

# ── state helpers (no jq dependency) ─────────────────────────────────────────
state_get() {
  [ -f "$STATE_FILE" ] || return 0
  sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" "$STATE_FILE" | head -1
}
write_state() { # <last_synced_sha> <vendor_tip>
  local now; now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  cat > "$STATE_FILE" <<EOF
{
  "last_synced_sha": "$1",
  "vendor_tip": "$2",
  "last_synced_date": "$now",
  "upstream_url": "$ZED_REMOTE_URL",
  "upstream_ref": "$ZED_REF"
}
EOF
}

# ── git / upstream helpers ───────────────────────────────────────────────────
require_clean_tree() {
  [ -z "$(git status --porcelain)" ] || \
    die "working tree is not clean; commit or stash changes first"
}

ensure_remote() {
  if ! git remote get-url "$ZED_REMOTE_NAME" >/dev/null 2>&1; then
    log "adding remote '$ZED_REMOTE_NAME' → $ZED_REMOTE_URL"
    git remote add "$ZED_REMOTE_NAME" "$ZED_REMOTE_URL"
  fi
}

fetch_ref() { # <ref> -> sets FETCH_HEAD
  log "fetching $ZED_REMOTE_NAME/$1 (blobless partial fetch)…"
  git fetch --filter=blob:none "$ZED_REMOTE_NAME" "$1"
}

fetch_object() { # <sha>
  git cat-file -e "$1^{commit}" 2>/dev/null && return 0
  log "fetching object ${1:0:12} from $ZED_REMOTE_NAME…"
  git fetch --filter=blob:none "$ZED_REMOTE_NAME" "$1" 2>/dev/null \
    || git fetch --filter=blob:none "$ZED_REMOTE_NAME" || true
  git cat-file -e "$1^{commit}" 2>/dev/null || die "could not fetch $1 from upstream"
}

# Default baseline = the zed rev currently pinned in the root Cargo.toml.
default_baseline() {
  sed -n 's#.*github\.com/zed-industries/zed".*rev = "\([0-9a-fA-F]\{7,40\}\)".*#\1#p' \
    "$REPO_ROOT/Cargo.toml" | head -1
}

# Build a tree object containing ONLY the tracked crates from <sha>, at their correct
# relative paths. Echoes the tree sha. Uses a throwaway index — no working-tree churn.
filtered_tree() { # <sha> -> tree sha (stdout); returns 1 if no tracked crate present
  local sha="$1" idx tree c added=0
  idx="$(mktemp -u)"
  GIT_INDEX_FILE="$idx" git read-tree --empty
  for c in "${TRACKED_CRATES[@]}"; do
    if git cat-file -e "$sha:crates/$c" 2>/dev/null; then
      GIT_INDEX_FILE="$idx" git read-tree --prefix="crates/$c/" "$sha:crates/$c"
      added=$((added + 1))
    fi
  done
  if [ "$added" -eq 0 ]; then rm -f "$idx"; return 1; fi
  tree="$(GIT_INDEX_FILE="$idx" git write-tree)"
  rm -f "$idx"
  printf '%s' "$tree"
}

# Bootstrap baseline: a single filtered snapshot commit (no upstream history precedes the
# baseline). Optional <parent> chains it. Echoes the commit sha.
build_vendor_snapshot() { # <sha> [parent] -> commit sha (stdout)
  local sha="$1" parent="${2:-}" tree
  tree="$(filtered_tree "$sha")" || die "no tracked crates found at ${sha:0:12}"
  if [ -n "$parent" ]; then
    git commit-tree "$tree" -p "$parent" -m "vendor: zed gpui baseline @ ${sha:0:12}"
  else
    git commit-tree "$tree" -m "vendor: zed gpui baseline @ ${sha:0:12}"
  fi
}

# Replay one upstream commit as a filtered commit, preserving its author, committer, dates
# and message (plus a zed-upstream sha trailer). The tree is the filtered snapshot at that
# commit, so the diff vs <parent> is exactly that commit's gpui change. Echoes the new sha.
replay_commit() { # <tree> <parent> <orig_commit> -> commit sha (stdout)
  local tree="$1" parent="$2" oc="$3" msg
  msg="$(git show -s --format=%B "$oc")"
  GIT_AUTHOR_NAME="$(git show -s --format=%an "$oc")" \
  GIT_AUTHOR_EMAIL="$(git show -s --format=%ae "$oc")" \
  GIT_AUTHOR_DATE="$(git show -s --format=%aI "$oc")" \
  GIT_COMMITTER_NAME="$(git show -s --format=%cn "$oc")" \
  GIT_COMMITTER_EMAIL="$(git show -s --format=%ce "$oc")" \
  GIT_COMMITTER_DATE="$(git show -s --format=%cI "$oc")" \
  git commit-tree "$tree" -p "$parent" -m "$msg

zed-upstream: $oc"
}

# Replay upstream's gpui-touching commits from <from>..<to> as a filtered, metadata-
# preserving chain onto <parent>. Merging the resulting tip keeps every upstream commit in
# history (second-parent ancestry) while the cumulative delta equals a single snapshot.
# Empty (non-gpui) commits and merge commits are dropped. Echoes the new tip sha.
build_vendor_history() { # <parent_commit> <from_sha> <to_sha> -> tip sha (stdout)
  local parent="$1" from="$2" to="$3" prev="$1" c tree ptree n=0 commits
  # shellcheck disable=SC2046
  commits="$(git rev-list --reverse --topo-order --no-merges "$from..$to" -- $(tracked_pathspec))"
  for c in $commits; do
    tree="$(filtered_tree "$c")" || continue
    ptree="$(git rev-parse "$prev^{tree}")"
    [ "$tree" = "$ptree" ] && continue   # no net change inside the tracked crates
    prev="$(replay_commit "$tree" "$prev" "$c")"
    n=$((n + 1))
  done
  log "replayed $n upstream gpui commit(s) into vendor history" >&2
  printf '%s' "$prev"
}

tracked_pathspec() { # echoes "crates/gpui crates/gpui_linux ..."
  local c out=""
  for c in "${TRACKED_CRATES[@]}"; do out="$out crates/$c"; done
  printf '%s' "${out# }"
}

# ── claude invocation ────────────────────────────────────────────────────────
render_prompt() { # <resolve|build> <payload>
  case "$1" in
    resolve)
      printf '%s\n\n' "You are resolving git merge conflicts from syncing upstream Zed's GPUI crates into the standalone \`gpui-ce\` fork. The working tree is mid-merge."
      printf 'Conflicted / unresolved files:\n'
      printf '%s\n' "$2" | sed 's/^/  - /'
      printf '\n'
      cat "$SCRIPT_DIR/resolve-conflicts.prompt.md"
      ;;
    build)
      printf '%s\n\n' "You are fixing compile errors after merging upstream Zed GPUI changes into \`gpui-ce\`. The merge is already committed; only fix what is needed to compile."
      printf 'Build command: `%s`\n' "$VERIFY_CMD"
      printf 'Recent build output (tail):\n```\n%s\n```\n\n' "$(tail -n 300 "$BUILD_LOG")"
      cat "$SCRIPT_DIR/fix-build.prompt.md"
      ;;
  esac
}

run_claude() { # <prompt-text>
  # Never fatal: claude hitting --max-turns or the timeout exits non-zero AFTER doing
  # useful work. We warn and return 0; the surrounding loop re-checks actual progress
  # (markers/unmerged paths) and retries up to RETRIES, so a partial pass isn't wasted.
  local prompt="$1" rc=0
  local -a cmd=("$CLAUDE_BIN" -p "$prompt"
    --model "$MODEL"
    --permission-mode acceptEdits
    --allowedTools "$CLAUDE_ALLOWED_TOOLS")
  [ "${CLAUDE_MAX_TURNS:-0}" -gt 0 ] && cmd+=(--max-turns "$CLAUDE_MAX_TURNS")
  if [ "${CLAUDE_TIMEOUT:-0}" -gt 0 ] && command -v timeout >/dev/null 2>&1; then
    timeout "$CLAUDE_TIMEOUT" "${cmd[@]}" || rc=$?
  else
    "${cmd[@]}" || rc=$?
  fi
  case "$rc" in
    0)   ;;
    124) warn "claude hit the ${CLAUDE_TIMEOUT}s timeout — re-checking progress, may retry" ;;
    *)   warn "claude exited non-zero ($rc; likely --max-turns) — re-checking progress, may retry" ;;
  esac
  return 0
}

# ── conflict resolution ──────────────────────────────────────────────────────
conflicted_files() {
  { git diff --name-only --diff-filter=U
    grep -rlE '^(<{7}|>{7}|\|{7})' crates/ 2>/dev/null || true
  } | sort -u
}
has_unresolved() {
  [ -n "$(git diff --name-only --diff-filter=U)" ] && return 0
  grep -rqE '^(<{7}|>{7}|\|{7})' crates/ 2>/dev/null && return 0
  return 1
}
# Deterministically settle add/delete conflicts that claude can't express via edits:
#   DU = deleted by us (gpui-ce), modified by them (upstream) → honor gpui-ce's deletion
#   UD = modified by us, deleted by them                      → keep gpui-ce's version (flag)
auto_resolve_add_delete() {
  local line code path handled=0
  while IFS= read -r line; do
    code="${line:0:2}"; path="${line:3}"
    case "$code" in
      DU) log "  modify/delete — keeping gpui-ce's deletion of $path"
          git rm -q --force -- "$path" >/dev/null 2>&1 || true; handled=1 ;;
      UD) warn "  delete/modify — upstream deleted $path; keeping gpui-ce's version (review)"
          git add -- "$path" >/dev/null 2>&1 || true; handled=1 ;;
    esac
  done < <(git status --porcelain)
  [ "$handled" = 1 ] && ok "auto-resolved add/delete conflicts"
  return 0
}

resolve_conflicts_loop() { # <branch> (for the error message)
  local branch="$1" attempt=0 files before after
  auto_resolve_add_delete
  while has_unresolved; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt "$RETRIES" ]; then
      warn "still unresolved after $RETRIES attempt(s):"
      conflicted_files | sed 's/^/    /' >&2
      die "conflict resolution failed; branch '$branch' left mid-merge for manual finishing"
    fi
    files="$(conflicted_files)"
    before="$(printf '%s\n' "$files" | grep -c . || true)"
    log "claude conflict-resolution pass $attempt/$RETRIES ($before file(s) remaining)"
    printf '%s\n' "$files" | sed "s/^/    ${C_D}conflict:${C_0} /"
    run_claude "$(render_prompt resolve "$files")"
    git add -A
    after="$(conflicted_files | grep -c . || true)"
    if [ "$after" -gt 0 ] && [ "$after" -ge "$before" ]; then
      warn "no progress this pass ($before → $after files) — claude may be stuck on these"
    fi
  done
  return 0   # never let the while/no-progress status trip `set -e` in the caller
}

# ── dependency bump ──────────────────────────────────────────────────────────
bump_zed_deps() { # <target-sha>
  local target="$1" f="$REPO_ROOT/Cargo.toml" n
  log "bumping zed-industries/zed git-dep revs → ${target:0:12}"
  awk -v new="$target" '
    /github\.com\/zed-industries\/zed"/ {
      if (match($0, /rev = "[0-9a-fA-F]+"/)) {
        $0 = substr($0, 1, RSTART - 1) "rev = \"" new "\"" substr($0, RSTART + RLENGTH)
      }
    }
    { print }
  ' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  n="$(grep -c 'github.com/zed-industries/zed"' "$f" || true)"
  ok "rewrote $n zed dep rev(s) in Cargo.toml"
}

# ── build fixing ─────────────────────────────────────────────────────────────
run_verify() {
  : > "$BUILD_LOG"
  eval "$VERIFY_CMD" 2>&1 | tee "$BUILD_LOG"
  return "${PIPESTATUS[0]}"
}
build_fix_loop() {
  local attempt=0
  while true; do
    log "verifying build: $VERIFY_CMD"
    if run_verify; then ok "build passes"; BUILD_OK=1; return 0; fi
    attempt=$((attempt + 1))
    if [ "$attempt" -gt "$RETRIES" ]; then
      warn "build still failing after $RETRIES fix attempt(s); leaving branch for manual finishing"
      BUILD_OK=0; return 1
    fi
    log "claude build-fix pass $attempt/$RETRIES"
    run_claude "$(render_prompt build "")"
  done
}

# ── subcommands ──────────────────────────────────────────────────────────────
cmd_bootstrap() { # [baseline-sha]
  require_clean_tree
  ensure_remote
  local baseline="${1:-}"
  [ -n "$baseline" ] || baseline="$(default_baseline)"
  [ -n "$baseline" ] || die "no baseline sha given and none pinned in Cargo.toml; pass one explicitly"
  log "bootstrapping sync baseline at ${baseline:0:12}"
  fetch_object "$baseline"
  baseline="$(git rev-parse "$baseline^{commit}")"

  local v0; v0="$(build_vendor_snapshot "$baseline" "")"
  git update-ref "refs/heads/$VENDOR_BRANCH" "$v0"
  ok "vendor branch '$VENDOR_BRANCH' → ${v0:0:12}"

  if git merge-base --is-ancestor "$v0" HEAD 2>/dev/null; then
    warn "baseline already recorded in this branch's history; skipping ours-merge"
  else
    git merge -s ours --allow-unrelated-histories --no-edit \
      -m "chore(sync): record zed gpui baseline ${baseline:0:12}

Establishes the merge base for automated upstream syncs. No file changes." "$v0"
    ok "recorded baseline in $(git symbolic-ref --quiet --short HEAD || echo HEAD) history (no file changes)"
  fi

  write_state "$baseline" "$v0"
  git add "$STATE_FILE"
  git commit -m "chore(sync): initialize upstream sync state at ${baseline:0:12}" >/dev/null
  ok "bootstrap complete — run 'just sync-upstream' to pull upstream changes"
}

cmd_sync() { # [ref]
  require_clean_tree
  ensure_remote
  [ -n "${1:-}" ] && ZED_REF="$1"

  local last vendor_tip
  last="$(state_get last_synced_sha)"
  vendor_tip="$(state_get vendor_tip)"
  [ -n "$last" ]       || die "no sync baseline recorded — run: just sync-upstream-bootstrap [SHA]"
  [ -n "$vendor_tip" ] || die "state is missing vendor_tip — re-run bootstrap"

  fetch_ref "$ZED_REF"
  local target; target="$(git rev-parse FETCH_HEAD^{commit})"
  log "last synced ${last:0:12}  →  target ${target:0:12} ($ZED_REF)"
  if [ "$target" = "$last" ]; then ok "already up to date with $ZED_REF"; return 0; fi

  log "upstream commits touching tracked crates:"
  # shellcheck disable=SC2046
  git --no-pager log --oneline "$last..$target" -- $(tracked_pathspec) | sed 's/^/    /' || true

  if [ "${DRY_RUN:-0}" = 1 ]; then ok "dry run — no changes made"; return 0; fi

  git cat-file -e "$vendor_tip^{commit}" 2>/dev/null \
    || die "vendor_tip ${vendor_tip:0:12} missing locally — re-run bootstrap"
  git merge-base --is-ancestor "$vendor_tip" HEAD \
    || die "vendor_tip not in current branch history — run sync from the branch that holds the last sync (usually main)"

  local vnew; vnew="$(build_vendor_history "$vendor_tip" "$last" "$target")"
  git update-ref "refs/heads/$VENDOR_BRANCH" "$vnew"
  if [ "$vnew" = "$vendor_tip" ]; then
    warn "no gpui-touching upstream commits in range — only deps will be updated"
  fi

  local branch="sync/zed-$(date -u +%Y%m%d)-${target:0:7}"
  git switch -C "$branch" >/dev/null
  ok "working on branch '$branch'"

  local merge_msg="merge: sync zed gpui ${last:0:12}..${target:0:12}

Synced tracked GPUI crates from zed-industries/zed ($ZED_REF).
The individual upstream commits are preserved in this merge's second-parent
history (filtered to the tracked crates); conflicts are resolved here.
Upstream range: $last..$target"

  log "merging upstream delta…"
  if git merge --no-ff --no-edit -m "$merge_msg" "$vnew"; then
    ok "merged cleanly (no conflicts)"
  else
    git rev-parse -q --verify MERGE_HEAD >/dev/null \
      || die "git merge failed without conflicts to resolve; aborting"
    warn "conflicts detected — invoking claude to resolve"
    resolve_conflicts_loop "$branch"
    git commit --no-edit -m "$merge_msg" >/dev/null
    ok "conflicts resolved; merge committed"
  fi

  [ "$BUMP_ZED_DEPS" = 1 ] && bump_zed_deps "$target"

  build_fix_loop || true

  if [ -n "$(git status --porcelain)" ]; then
    git add -A
    git commit -m "chore(sync): post-merge fixes (zed deps bump + build)" >/dev/null || true
    ok "committed post-merge fixes"
  fi

  write_state "$target" "$vnew"
  git add "$STATE_FILE"
  git commit -m "chore(sync): advance sync state to ${target:0:12}" >/dev/null

  summary "$branch" "$last" "$target"
}

cmd_status() {
  ensure_remote
  local last; last="$(state_get last_synced_sha)"
  printf 'last synced : %s\n' "${last:-<none — run bootstrap>}"
  printf 'vendor tip  : %s\n' "$(state_get vendor_tip)"
  printf 'synced date : %s\n' "$(state_get last_synced_date)"
  git fetch --filter=blob:none "$ZED_REMOTE_NAME" "$ZED_REF" >/dev/null 2>&1 || true
  local target; target="$(git rev-parse FETCH_HEAD^{commit} 2>/dev/null || true)"
  [ -n "$target" ] && printf 'upstream %s : %s\n' "$ZED_REF" "${target:0:12}"
  if [ -n "$last" ] && [ -n "$target" ] && [ "$last" != "$target" ]; then
    local n; n="$(git rev-list --count "$last..$target" 2>/dev/null || echo '?')"
    printf '\nbehind by %s upstream commit(s) on %s\n' "$n" "$ZED_REF"
  elif [ -n "$last" ] && [ "$last" = "$target" ]; then
    ok "up to date with $ZED_REF"
  fi
}

summary() { # <branch> <last> <target>
  local branch="$1" last="$2" target="$3"
  printf '\n'
  if [ "$BUILD_OK" = 1 ]; then ok "sync complete — build passes"; else warn "sync complete — BUILD STILL FAILING, finish manually"; fi
  printf '  branch         : %s\n' "$branch"
  printf '  upstream range : %s..%s\n' "${last:0:12}" "${target:0:12}"
  printf '  build (%s) : %s\n' "$VERIFY_CMD" "$([ "$BUILD_OK" = 1 ] && echo OK || echo FAILED)"
  printf '\nReview : git log --oneline --stat main..%s\n' "$branch"
  printf 'Merge  : git switch main && git merge --ff-only %s   (or open a PR)\n' "$branch"
  printf '%s\n' "${C_D}Nothing was pushed.${C_0}"
}

# ── dispatch ─────────────────────────────────────────────────────────────────
SUBCMD="sync"
case "${1:-}" in
  bootstrap|sync|status) SUBCMD="$1"; shift ;;
  -h|--help) usage; exit 0 ;;
esac

DRY_RUN=0
POSITIONAL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ref)     ZED_REF="$2"; shift 2 ;;
    --model)   MODEL="$2"; shift 2 ;;
    --retries) RETRIES="$2"; shift 2 ;;
    --no-bump) BUMP_ZED_DEPS=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    --*)       die "unknown option: $1" ;;
    *)         [ -z "$POSITIONAL" ] && POSITIONAL="$1" || die "unexpected argument: $1"; shift ;;
  esac
done

case "$SUBCMD" in
  bootstrap) cmd_bootstrap "$POSITIONAL" ;;
  sync)      cmd_sync "$POSITIONAL" ;;
  status)    cmd_status ;;
esac
