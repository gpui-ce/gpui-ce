# Maintenance

Misc info for maintainers on complex project setup/automation details.

## Vouch

[gpui-ce vouch](https://github.com/gpui-ce/vouch) forked from [ghostty vouch](https://github.com/mitchellh/vouch)

Provides github actions for automating processing of issues/PRs against vouched users. Requires a [github app](https://docs.github.com/apps/creating-github-apps/setting-up-a-github-app/creating-a-github-app) to [acquire a token](https://github.com/actions/create-github-app-token) in order to apply changes to tickets/vouch list.

A bot exists under the [gpui-ce org](https://github.com/organizations/gpui-ce/settings/apps/gpuice-vouch-bot) and is installable via https://github.com/apps/gpuice-vouch-bot. The client-id is accessible in the bot settings (first link) and the private key is locked. The former must be configured on this repository as a repository variable under the id `VOUCH_CLIENT_ID`, and the latter as a repository secret under the id `VOUCH_APP_PRIVATE_KEY`. If the bot needs to be installed into additional repositories, it will need to be regenerated and updated in all existing installed repos. Private keys stored in the credential key-pairs, as [documented here](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/managing-private-keys-for-github-apps). The vouch system requires Read access to Discussions (to respond to discussion macros/commands), Read/Write on contents (to write commits with vouch updates), & Read/Write for Issues and Pull Requests (to close/lock them).

## Releases

We use a release PR system to collaborate on official releases. If merged, it is auto tagged, and published to crates.

If you'd like to do this, it's simple:

1. Run the **Prepare release PR** workflow (Actions tab) with the new stable version, e.g. `0.4.0`. It opens `release/v0.4.0` labelled `release`, with every publishable crate bumped to the new version
2. On that PR, fill in the **Highlights**. The **Release check** workflow verifies the branch/label/version agree, the CHANGELOG section is filled in, the version is unpublished, and `just publish true` passes.
3. Merge. **Tag release** sees the merged `release` PR, pushes the annotated tag `vX.Y.Z`, and calls **Release**, which runs the full CI recipes, a dry-run publish, `just publish`, then drafts the GitHub Release.

And! As a fallback, you'll see `Release` still runs for a `v*` tag pushed the old way.
