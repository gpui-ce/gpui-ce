#!/usr/bin/env nu
const here = (path self | path dirname)
const user_agent = "gpui-ce release tooling"

const unreleased_skeleton = "## [Unreleased]

### Highlights

(only filled out when preparing the release)

### Breaking Changes

### Additions

### Removals

### Fixes

### Security Changes

### Deprecations

"

def root [] { $here | path dirname | path dirname }

def die [message: string] { error make --unspanned { msg: $message } }

# Append `key=value` to the GitHub Actions step output, when running in one.
def set-output [key: string, value: string] {
    print $"($key)=($value)"
    if "GITHUB_OUTPUT" in $env { $"($key)=($value)\n" | save --append $env.GITHUB_OUTPUT }
}

def workspace-packages [] {
    cd (root)
    ^cargo metadata --no-deps --format-version 1 | from json | get packages
}

# `publish = false` is `[]` in cargo metadata; null or an allowlist can publish.
def is-publishable [package: record] { $package.publish != [] }

def semver-rank [version: string] {
    let parts = ($version | parse --regex '^(\d+)\.(\d+)\.(\d+)$')
    if ($parts | is-empty) { die $"($version) is not stable MAJOR.MINOR.PATCH SemVer" }
    let p = ($parts | first)
    ($p.capture0 | into int) * 1_000_000_000_000 + ($p.capture1 | into int) * 1_000_000 + ($p.capture2 | into int)
}

def committed-version [] {
    workspace-packages | where name == "gpui-ce" | first | get version
}

def crates-io-has [crate: string, version: string] {
    let response = (http get --full --allow-errors --headers [User-Agent $user_agent] $"https://crates.io/api/v1/crates/($crate)/($version)")
    match $response.status {
        200 => true
        404 => false
        $status => (die $"Could not check crates.io for ($crate) ($version): HTTP ($status)")
    }
}

# Every published version of a crate, [] if it has never been published.
def crates-io-versions [crate: string] {
    let response = (http get --full --allow-errors --headers [User-Agent $user_agent] $"https://crates.io/api/v1/crates/($crate)")
    match $response.status {
        200 => ($response.body.versions | get num)
        404 => []
        $status => (die $"Could not list ($crate) on crates.io: HTTP ($status)")
    }
}

# Publishable workspace crates, dependencies first.
#
# Ordering is relevant to normal + build deps primarily, otherwise you'll get cycles from dev deps
def publish-order [] {
    let packages = (workspace-packages | where { is-publishable $in })
    let names = ($packages | get name)
    mut order = []
    mut remaining = ($names | sort)
    while ($remaining | is-not-empty) {
        let done = $order
        let ready = ($remaining | where { |name|
            $packages | where name == $name | first | get dependencies
                | where { |d| $d.name in $names and $d.name != $name and ($d.kind == null or $d.kind == "build") }
                | all { |d| $d.name in $done }
        })
        if ($ready | is-empty) {
            die $"dependency cycle among normal/build deps: ($remaining | str join ', ')"
        }
        $order = ($order | append $ready)
        $remaining = ($remaining | where { |name| $name not-in $ready })
    }
    let order = $order
    let position = ($order | enumerate | reduce --fold {} { |it, acc| $acc | insert $it.item $it.index })
    $order | each { |name|
        let package = ($packages | where name == $name | first)
        let later_dev = ($package.dependencies | any { |d|
            if $d.kind != "dev" or $d.req == "*" { false } else {
                let at = ($position | get --optional $d.name)
                $at != null and $at >= ($position | get $name)
            }
        })
        {
            name: $name
            version: $package.version
            manifest: ($package.manifest_path | path relative-to (root))
            no_verify: $later_dev
        }
    }
}

# Give every publishable crate `version` and repoint every internal path+version pin at it.
def set-versions [version: string, --exact] {
    let packages = (workspace-packages)
    let publishable = ($packages | where { is-publishable $in } | get manifest_path)
    let manifests = ($packages | get manifest_path | append (root | path join "Cargo.toml") | uniq)
    let requirement = if $exact { $"=($version)" } else { $version }
    for manifest in $manifests {
        let lines = (open --raw $manifest | lines | enumerate)
        # In a package manifest, the first top-level `version = "..."` is [package]'s.
        let package_line = if $manifest in $publishable {
            $lines | where item =~ '^version = "' | first | get index
        } else { -1 }
        let rewritten = ($lines | each { |line|
            if $line.index == $package_line {
                $'version = "($version)"'
            } else if ($line.item | str contains 'path = "') and ($line.item | str contains 'version = "') {
                $line.item | str replace --regex 'version = "[^"]*"' $'version = "($requirement)"'
            } else {
                $line.item
            }
        })
        $"($rewritten | str join "\n")\n" | save --force $manifest
    }
    $publishable | length
}

def changelog-section [version: string] {
    let escaped = ($version | str replace --all '.' '\.')
    open --raw (root | path join "CHANGELOG.md")
        | parse --regex ('(?s)## \[' + $escaped + '\][^\n]*\n(.*?)(?:\n## \[|\z)')
        | get capture0?
        | first
        | default ""
}

# Static pre-flight: what crates.io would reject, found before anything is uploaded.
def publish-problems [] {
    let packages = (workspace-packages)
    let names = ($packages | get name)
    let problems = ($packages | where { is-publishable $in } | each { |package|
        $package.dependencies | each { |dep|
            if $dep.kind == "dev" and $dep.req == "*" {
                return []  # path-only dev-dependencies are stripped when packaging
            }
            let local = ($packages | where name == $dep.name)
            mut found = []
            if ($local | is-not-empty) and not (is-publishable ($local | first)) {
                $found = ($found | append $"($package.name) depends on ($dep.name), which has `publish = false`")
            }
            if ($dep.source? | default "" | str starts-with "git+") and $dep.req == "*" {
                $found = ($found | append $"($package.name) depends on git-only `($dep.name)` with no crates.io version")
            }
            $found
        } | flatten
    } | flatten | uniq | sort)
    $problems
}

def check-publishable [] {
    let problems = (publish-problems)
    if ($problems | is-not-empty) {
        $problems | each { |problem| print --stderr $"error: ($problem)" }
        die "a publishable crate depends on something crates.io cannot resolve"
    }
    print "Every publishable crate depends only on crates.io-resolvable crates."
}

# Is `sha` still the tip of origin/main? Older queued runs must never publish.
def is-main-tip [sha: string] {
    ^git fetch origin main
    (^git rev-parse origin/main | str trim) == $sha
}

def main [] {
    print "usage: nu release.nu <subcommand>; see the `release` group in the justfile"
}

# Print the committed gpui-ce version.
def "main version" [] { committed-version }

# JSON: publishable crates, dependencies first.
def "main publish-order" [] { publish-order | to json --raw }

# Fail if a publishable crate depends on something crates.io cannot resolve.
def "main publishable" [] { check-publishable }

# Record VERSION on every crate without touching the CHANGELOG.
def "main sync" [version: string] {
    let version = ($version | str replace --regex '^v' '')
    semver-rank $version | ignore
    print $"Set (set-versions $version) crates to ($version)."
}

# Rewrite every publishable crate to a crates.io-only snapshot version.
def "main alpha" [version: string] {
    print $"Rewrote (set-versions $version --exact) crates to ($version)."
}

# Bump every crate to VERSION and roll the CHANGELOG's [Unreleased] section into it.
def "main prepare" [version: string] { prepare-release $version }

def prepare-release [version: string] {
    let version = ($version | str replace --regex '^v' '')
    let current = (committed-version)
    # The committed version is the release being worked towards, so re-preparing
    # it is fine; only going backwards or re-releasing a published one is not.
    if (semver-rank $version) < (semver-rank $current) {
        die $"new version ($version) must not be lower than the current ($current)"
    }
    if (crates-io-has gpui-ce $version) {
        die $"gpui-ce ($version) is already on crates.io"
    }
    let changelog = (root | path join "CHANGELOG.md")
    let text = (open --raw $changelog)
    if not ($text | str contains "## [Unreleased]\n") { die "CHANGELOG.md has no [Unreleased] section" }
    if ($text | str contains $"## [($version)]") { die $"CHANGELOG.md already has a [($version)] section" }

    set-versions $version | ignore
    let today = (date now | format date "%Y-%m-%d")
    $text | str replace "## [Unreleased]\n" $"($unreleased_skeleton)## [($version)] - ($today)\n" | save --force $changelog
    print $"Prepared ($current) -> ($version). Fill in the Highlights, then run `cargo update --workspace`."
}

# Verify the tree is a coherent, unreleased release candidate.
def "main check" [--tag-free] {
    let version = (committed-version)
    semver-rank $version | ignore
    let packages = (workspace-packages)
    let names = ($packages | get name)
    let section = (changelog-section $version)
    mut problems = []
    if ($section | str trim | is-empty) { $problems = ($problems | append $"CHANGELOG.md has no populated [($version)] section") }
    if ($section | str contains "(only filled out when preparing the release)") {
        $problems = ($problems | append $"the [($version)] Highlights are still the placeholder")
    }
    if not (open --raw (root | path join "CHANGELOG.md") | str contains "## [Unreleased]") {
        $problems = ($problems | append "CHANGELOG.md is missing a fresh [Unreleased] section")
    }
    for package in $packages {
        if (is-publishable $package) and $package.version != $version {
            $problems = ($problems | append $"($package.name) is ($package.version); releases are lockstep, expected ($version)")
        }
        for dep in $package.dependencies {
            if $dep.name in $names and $dep.req not-in ["*" $"^($version)"] {
                $problems = ($problems | append $"($package.name) requires ($dep.name) ($dep.req), expected ^($version)")
            }
        }
    }
    $problems = ($problems | append (publish-problems))
    if $tag_free and (crates-io-has gpui-ce $version) {
        $problems = ($problems | append $"gpui-ce ($version) is already on crates.io; bump the version")
    }
    if ($problems | is-not-empty) {
        $problems | each { |problem| print --stderr $"error: ($problem)" }
        die "the tree is not a coherent release"
    }
    print $"Release ($version) looks coherent."
}

# Branch, label and committed version must agree on a release PR.
def "main pr-check" [head_ref: string, labels: string] {
    let version = (committed-version)
    if $head_ref != $"release/v($version)" {
        die $"branch ($head_ref) does not match the committed version ($version)"
    }
    if "release" not-in ($labels | split row ",") {
        die "release PRs need the 'release' label, or merging will not tag"
    }
    print $"Release PR for v($version) is consistent."
}

# Open a release PR for VERSION: bump, roll the changelog, refresh the lockfile.
def "main pr-open" [version: string] {
    let version = ($version | str replace --regex '^v' '')
    let branch = $"release/v($version)"
    semver-rank $version | ignore
    ^git fetch --tags --quiet
    if (^git rev-parse -q --verify $"refs/tags/v($version)" | complete | get exit_code) == 0 {
        die $"tag v($version) already exists"
    }
    if (^gh pr list --head $branch --state open --json number | from json | is-not-empty) {
        die $"a release PR for v($version) is already open"
    }

    prepare-release $version
    ^cargo update --workspace

    ^git config user.name "github-actions[bot]"
    ^git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
    ^git switch -c $branch
    ^git commit -am $"Release v($version)"
    ^git push origin $branch
    ^gh label create release --color 0e8a16 --description "Merging tags and publishes to crates.io" | complete | ignore
    let body = $"Merging this pull request tags `v($version)` and publishes every crate to crates.io.

Before merging:
- [ ] Fill in the `[($version)]` **Highlights** in CHANGELOG.md (replace the placeholder)
- [ ] The **Release check** and CI jobs are green"
    ^gh pr create --base main --head $branch --title $"Release v($version)" --label release --body $body
}

# On main: if this push merged a release PR, tag it. Sets the `tag` step output.
def "main tag" [] {
    let version = (committed-version)
    let tag = $"v($version)"

    # Only a merged PR carrying the `release` label counts as a release, so an
    # ordinary version edit never publishes by accident.
    let labels = (^gh api $"repos/($env.GITHUB_REPOSITORY)/commits/($env.GITHUB_SHA)/pulls" | from json
        | where merged_at != null | get labels | flatten | get name)
    if "release" not-in $labels {
        print "Not a release PR merge; nothing to tag."
        set-output tag ""
        return
    }
    ^git fetch --tags --quiet
    if (^git rev-parse -q --verify $"refs/tags/($tag)" | complete | get exit_code) == 0 {
        print $"($tag) already exists; nothing to do."
        set-output tag ""
        return
    }
    ^git config user.name "github-actions[bot]"
    ^git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
    ^git tag -a $tag -m $"gpui-ce ($version)" $env.GITHUB_SHA
    ^git push origin $tag
    set-output tag $tag
}

# Allocate the next alpha for SHA. Sets the `publish` and `version` step outputs.
def "main alpha-version" [sha: string] {
    if not (is-main-tip $sha) {
        print "main advanced; this snapshot is stale."
        set-output publish "false"
        return
    }
    let record = (committed-version)
    semver-rank $record | ignore
    # The committed version is the release being worked towards, so snapshots
    # are its prereleases: X.Y.Z-alpha.N sorts below X.Y.Z.
    let release = $record
    # One number for the whole set: the highest alpha any crate has reached, so a
    # partly published earlier run is never reused (its crates carry older code).
    let last = (publish-order | get name | each { |crate|
        crates-io-versions $crate
            | parse --regex ('^' + ($release | str replace --all '.' '\.') + '-alpha\.(\d+)$')
            | get capture0 | into int
    } | flatten | append 0 | math max)
    set-output publish "true"
    set-output version $"($release)-alpha.($last + 1)"
}

# Publish every crate at the snapshot VERSION, in dependency order.
def "main alpha-publish" [version: string, sha: string] {
    # Uploads are permanent: find unresolvable dependencies before the first one.
    check-publishable
    print $"Rewrote (set-versions $version --exact) crates to ($version)."
    if not (is-main-tip $sha) {
        print "main advanced while this snapshot was being prepared; skipping it."
        return
    }
    # Full CI passed on this commit, so tarballs are not re-verified: the
    # workspace's dev-dependency cycles cannot be resolved in isolation. cargo
    # waits for each crate to reach the index before the next one.
    for crate in (publish-order) {
        print $"::group::cargo publish ($crate.manifest)"
        ^cargo publish --manifest-path $crate.manifest --allow-dirty --no-verify
        print "::endgroup::"
    }
}

# Prove the published snapshot resolves and compiles for a consumer.
def "main alpha-smoke" [version: string] {
    let dir = (mktemp --directory)
    cd $dir
    ^cargo new --lib consumer
    cd consumer
    # The crates.io index can lag a few seconds behind the publish.
    for attempt in 1..12 {
        let result = (do { ^cargo add $"gpui-ce@=($version)" $"gpui_ce_platform@=($version)"; ^cargo check } | complete)
        if $result.exit_code == 0 { return }
        sleep 10sec
    }
    die $"($version) did not resolve and compile from crates.io"
}
