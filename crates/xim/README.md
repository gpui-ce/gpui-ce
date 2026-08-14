# gpui_xim

XIM (X Input Method) client used by `gpui_linux` to talk to input method
servers such as fcitx and ibus on X11. Without it, IME users cannot compose
text — CJK input, dead keys and compose sequences all go through this.

Vendored, not depended on. `crates/xim_parser` and `crates/xim_ctext` are
vendored alongside it for the same reason; the three are one unit.

## Provenance

    xim 0.4.0            Riey        github.com/Riey/xim-rs   (MIT)
      └─ zed-xim 0.4.0-zed   zed-industries fork
           └─ gpui_xim       this directory

All three crates are MIT licensed, Copyright (c) 2020 Riey — see `LICENSE`.
This is *not* the workspace's Apache-2.0 license, so the manifests state MIT
explicitly rather than inheriting.

## Why it is vendored

Two reasons, and both have to hold for vendoring to be worth it.

**gpui-ce does not depend on zed-owned crates.** The published `zed-xim` is
zed's, so depending on it is out.

**The genuine upstream is not a drop-in.** `xim` 0.5.0 (Riey's latest) lacks
things gpui_linux relies on. zed's fork adds them, and that is what this copy
preserves:

* `Client::reset_ic` and the `Request::ResetIc` / `ResetIcReply` round trip,
  plus `ClientHandler::handle_reset_ic`. `x11/client.rs` calls `reset_ic` to
  clear preedit state; the method does not exist upstream.
* `Request::DestroyIcReply` dispatch to `ClientHandler::handle_destroy_ic`.
* `ClientError::NoXimServer`, raised when a `BadWindow` error names the IM
  window — i.e. the XIM server went away.
* A fcitx4 workaround in both the x11rb and xlib transports: a zero-length
  property reply returns `ClientError::InvalidReply` instead of being read as
  valid empty data.

That is the whole functional delta against Riey's 0.4.0 — 51 lines added and 5
removed, in `client.rs`, `x11rb.rs` and `xlib.rs`.

## Local changes

Against published `zed-xim` 0.4.0-zed there are **no functional changes**, only
what pulling the source into this workspace forces:

* rustfmt normalization under `cargo fmt --all`, including 2024-edition import
  ordering.
* 2024-edition lint fixes: elided lifetimes written out (`AttributeBuilder<'_>`)
  and a `let _ =` on a now-`must_use` call.
* Package renamed `zed-xim` → `gpui_xim`, version `0.4.0-zed` → `0.4.0-gpui`.
  It is `publish = false` and reached through the `xim` workspace alias, so no
  call site refers to either name.

## x11rb

This copy pins `x11rb = "0.14"`, while published `zed-xim` and `xim` 0.5.0 both
pin 0.13. That is load-bearing: `gpui_linux` hands the client an
`XCBConnection`, so if xim resolves a different x11rb than gpui_linux does, the
two `XCBConnection` types are unrelated and `HasConnection` stops being
satisfied. Keeping x11rb here in step with the workspace is the reason the
workspace can be on 0.14 at all.

If you ever switch to a published xim, x11rb has to move to whatever that
release pins, in the same commit.

## Upgrading

The vendored tree is rustfmt-normalized by this workspace, and published
tarballs use CRLF where this tree uses LF, so neither matches byte-for-byte.
To compare against a new release: extract it, convert line endings, run
`cargo fmt` on it, then diff.

Before adopting any published release, check it provides `reset_ic`,
`handle_destroy_ic`, `NoXimServer` and the fcitx4 empty-reply guard. If Riey
upstreams all four, drop these three crates and depend on the release directly
— the `xim` workspace alias means no call site has to change.
