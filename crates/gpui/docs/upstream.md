# GPUI and GPUI Community Edition

GPUI Community Edition is a fork of the GPUI that lives in the
[Zed repository](https://github.com/zed-industries/zed). Zed's GPUI is developed
for Zed; GPUI-CE maintains that foundation for standalone applications and adds
its own APIs, and general improvements to foundational use-cases, like color, or text.

We share the entity, context, and element model, but dependency
names, text behavior, and available features can differ, so we're not *exactly* a drop-in.

## Porting code from Zed's GPUI

Start by renaming the dependencies, then check every API and text measurement your
application relies on. A port is compatible only for the revisions and enabled features
you actually compare.

**Depend on the GPUI-CE packages.** The main crate is published as `gpui-ce`.
Rename it in `Cargo.toml`, so your `use gpui::...` lines keep working:

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.2" }
gpui_platform = { package = "gpui_ce_platform", version = "0.1" }
```

The versions above select the latest compatible published releases, while this
documentation may include APIs from the repository's next release. If an example does
not exist in the version you use, switch both dependencies to the same git revision as
shown in [Start a project](crate::_documentation::guides::start_a_project), or adapt the
example to the published API.

The other GPUI crates are published as `gpui_ce_*`: `gpui_ce_platform`,
`gpui_ce_elements`, `gpui_ce_tokio` and so on. Rename them the same way.

**Start the app through `gpui_platform`.** `gpui_platform::application()`
creates an [`Application`] for whichever platform you build for, including the
web.

**Expect text to lay out slightly differently.** GPUI-CE lays out text with
[Parley](https://github.com/linebender/parley) on every platform, rather than
with each platform's text system. Shaping, fallback, and line breaks can differ
from the upstream backend; avoid assuming a fixed pixel difference. Check text
measurements and wrapping with the fonts your app ships. The
[text guide](crate::_documentation::guides::text) covers these settings.

**Use the trait methods for notifying and emitting.** [`notify`](AppContext::notify)
and [`emit`](AppContext::emit) are methods of the [`AppContext`] trait. Import
`use gpui::prelude::*` to bring the trait into scope; async contexts return a
fallible result when updating state. [Contexts](crate::_documentation::concepts::contexts)
explains those access and lifetime differences.

## What GPUI-CE adds

**The web.** GPUI-CE runs in browsers through WebAssembly. `Auto` tries WebGPU and falls
back to WebGL2; forcing a backend fails if that backend cannot initialize. See [Run on
the web](crate::_documentation::guides::web).

**Text.** Parley-based layout everywhere, with bidirectional text and
right-to-left layout ([`Styled::rtl`]), letter spacing, text transforms, and
variable font widths ([`FontWidth`]).

**Editable text.** The `gpui_ce_elements` crate provides single- and multi-line
text fields, with selection, undo, the clipboard and input methods. See [Accept
text input](crate::_documentation::guides::text_input).

**Motion.** Animations with iterations and alternating passes, and transitions
of style properties such as a hover color ([`InteractiveElement::transitions`]).
See [Animate](crate::_documentation::guides::animation).

**Styling.** Smoothed ("squircle") corners ([`Styled::rounded_smoothing`]),
gradients on borders and shadows, and transparent or blurred window backgrounds
on each desktop platform.

**Input.** Touch gestures integrate with pointer dispatch, including clicks,
scrolling, and pinches. GPUI-CE also provides character palettes on Linux and
Windows, extending the existing macOS support. See
[mouse and touch input](crate::_documentation::guides::mouse_and_touch).

**Accessibility.** APIs for performing assistive-technology actions and
inspecting the accessibility tree. The
[accessibility guide](crate::_documentation::guides::accessibility) shows how
to expose controls and test their semantics.

**Tooling.** Hot patching of render code into a running app (the `hot-patching`
feature; see [Reload code while the app runs](crate::_documentation::guides::hot_reload)),
a wgpu renderer for macOS (`macos-wgpu` on `gpui_platform`), and an API for
drawing into windows with your own wgpu code (`custom-gpu`).

**Applications.** Haptic feedback on macOS trackpads, the macOS activation
policy, hiding and showing windows at run time, and detecting a lost GPU device.

## Keeping up with upstream

GPUI-CE merges Zed's GPUI changes with the tooling in `scripts/sync-upstream`
in the repository, and keeps its crates at the same paths as upstream so that
changes apply cleanly. If you find an upstream fix missing, or want to contribute
a GPUI-CE feature back, open an issue on
[GitHub](https://github.com/gpui-ce/gpui-ce/issues).
