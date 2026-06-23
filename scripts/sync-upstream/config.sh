# shellcheck shell=bash
#
# Configuration for the gpui-ce upstream-sync script.
# Every value can be overridden via the matching SYNC_* environment variable,
# e.g.  SYNC_MODEL=sonnet just sync-upstream

# ── Upstream Zed monorepo ────────────────────────────────────────────────────
# NOTE: this is a SEPARATE remote from the existing `upstream` remote, which
# points at gpui-ce/gpui-ce (the fork's own canonical repo), NOT at Zed.
ZED_REMOTE_NAME="${SYNC_ZED_REMOTE_NAME:-zed}"
ZED_REMOTE_URL="${SYNC_ZED_REMOTE_URL:-https://github.com/zed-industries/zed.git}"
ZED_REF="${SYNC_ZED_REF:-main}"

# ── Tracked crates ───────────────────────────────────────────────────────────
# Crates that exist in BOTH repos at the same relative path (crates/<name>) and
# are therefore file-synced from upstream. Everything else is left untouched:
#   - gpui_util     : upstream vendors it; gpui-ce consumes it as a git dep
#   - gpui_elements : gpui-ce-only stub
#   - tooling/perf  : gpui-ce-only tooling
TRACKED_CRATES=(
  gpui
  gpui_linux
  gpui_macos
  gpui_macros
  gpui_platform
  gpui_shared_string
  gpui_tokio
  gpui_web
  gpui_wgpu
  gpui_windows
)

# Local branch holding filtered upstream snapshots (the 3-way merge base source).
VENDOR_BRANCH="${SYNC_VENDOR_BRANCH:-vendor/zed-gpui}"

# ── claude -p settings ───────────────────────────────────────────────────────
CLAUDE_BIN="${SYNC_CLAUDE_BIN:-claude}"
MODEL="${SYNC_MODEL:-opus}"
# Max claude passes per phase (conflict resolution, build fixing).
RETRIES="${SYNC_RETRIES:-3}"
# Per-invocation wall-clock cap in seconds (0 disables; needs `timeout`).
CLAUDE_TIMEOUT="${SYNC_CLAUDE_TIMEOUT:-1800}"
# Optional cap on claude's agentic turns per invocation (0 = no cap, the default).
# Resolving many files in one pass is turn-intensive, so we do not cap by default:
# the wall-clock CLAUDE_TIMEOUT bounds runaway invocations and hitting it is non-fatal
# (the loop re-checks progress and retries). Set >0 to impose a turn cap.
CLAUDE_MAX_TURNS="${SYNC_CLAUDE_MAX_TURNS:-0}"
# Tools claude may use headlessly. Edits/writes are auto-accepted via
# --permission-mode acceptEdits; the rest are read-only/build helpers. Git
# staging/commits are done by THIS script, never by claude, and push is never
# allowed. (Format is a single space-separated string; adjust if your claude
# CLI version expects commas.)
CLAUDE_ALLOWED_TOOLS="${SYNC_CLAUDE_ALLOWED_TOOLS:-Read Edit Write Grep Glob Bash(cargo check:*) Bash(cargo build:*) Bash(cargo metadata:*) Bash(git status:*) Bash(git diff:*) Bash(git log:*) Bash(rg:*) Bash(grep:*) Bash(ls:*) Bash(cat:*) Bash(sed:*) Bash(find:*)}"

# ── Verification / dep bump ──────────────────────────────────────────────────
# Compile gate run by the build-fix loop. Host-buildable crates only; macOS /
# Windows-specific changes must be verified on those platforms (or in CI).
VERIFY_CMD="${SYNC_VERIFY_CMD:-just check}"

# Bump pinned zed-industries/zed git-dep revs to the synced commit (1=yes,0=no).
BUMP_ZED_DEPS="${SYNC_BUMP_ZED_DEPS:-1}"

# Committed state file tracking the last synced upstream commit.
STATE_FILE_REL="scripts/sync-upstream/state.json"
