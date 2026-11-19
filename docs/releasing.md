# Releasing `gpui-ce`

Stable releases are release pull requests (see MAINTENANCE.md). The committed
`gpui-ce` version only changes in those PRs. Do not change it for alpha
publishes, commit an alpha suffix, or create an alpha Git tag.

Which crates are published, and in what order, is computed from `cargo metadata`
by `just` via `scripts/release/release.nu` (`nu scripts/release/release.nu publish-order`). A new workspace crate is picked up
automatically, and `just publishable` fails before anything is uploaded if
a publishable crate depends on a `publish = false` crate or a git-only dependency.

## Alpha snapshots

After CI succeeds, only the latest commit on `main` is published as a crates.io
development snapshot named `X.Y.Z-alpha.N`: `X.Y.Z` is the committed version
(the release being worked towards, so snapshots sort below it), and `N` is
discovered from crates.io. **Every** publishable
crate is published at that same version, in dependency order, with internal
dependencies pinned to `=X.Y.Z-alpha.N`. `N` is one past the highest alpha any
crate has reached, so a partly published earlier run is never reused. A smoke
job then compiles a fresh crate against the published snapshot. Older queued CI runs
self-discard when main has advanced. It neither reads nor creates Git tags. It
temporarily substitutes the alpha version while packaging (`just alpha-publish`); the repository and
its Git history continue to record the unprefixed release version.

Cargo excludes prereleases from ordinary dependency resolution. Consequently,
`cargo add gpui-ce` continues to select the latest stable crate. Consumers who
want a snapshot must opt in with an explicit alpha version, for example
`cargo add gpui-ce@0.4.0-alpha.3`.

Stable tagged releases continue to use the existing tag workflow and do not
change the alpha publisher's committed-version record.

## SemVer checks

SemVer CI compares the PR head directly with the PR base commit. That isolates
the API changes introduced by the PR and does not re-report incompatible API
changes that were already on `main`.
