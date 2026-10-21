# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/). This project somewhat adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html), aside from the major version being 0.

## [Unreleased]

### Highlights
(only filled out when preparing the release)

### Breaking Changes
- Switched text system to [parley](https://github.com/linebender/parley). [#267](https://github.com/gpui-ce/gpui-ce/pull/267)
- Replaced `DefiniteLength::Fraction(f32)` with `DefiniteLength::Relative(Relative)`. [#230](https://github.com/gpui-ce/gpui-ce/pull/230), [#255](https://github.com/gpui-ce/gpui-ce/pull/255)
- Renamed `Window::activate_window` to `Window::activate`. [#272](https://github.com/gpui-ce/gpui-ce/pull/272)
- Moved notify, emit, and `global_entities` into `AppContext`. [#127](https://github.com/gpui-ce/gpui-ce/pull/127)
- Border and shadow color fields now use `Background` instead of `Hsla`. [#252](https://github.com/gpui-ce/gpui-ce/pull/252)
- Image, shadow, and filter painting methods now require a `corner_smoothing` argument, `PaintQuad` gains a `corner_smoothing` field. [#338](https://github.com/gpui-ce/gpui-ce/pull/338)
- Removed `Deferred::with_priority` in favor of the existing `Deferred::priority` method. [#245](https://github.com/gpui-ce/gpui-ce/pull/245)
- Replaced `asset_source` with new `AssetRegistry` API. [#259](https://github.com/gpui-ce/gpui-ce/pull/259)

### Additions
- Changelog established.
- Hot reloading via sub-second. [#292](https://github.com/gpui-ce/gpui-ce/pull/292)
- `.aria_disabled()` and `.aria_modal()` to expose element states to screen readers. [#233](https://github.com/gpui-ce/gpui-ce/pull/233)
- `.aria_live()` and `.aria_live_atomic()` for announcing content updates. [#233](https://github.com/gpui-ce/gpui-ce/pull/233)
- `.aria_hidden()` and `Element::is_a11y_hidden()` to hide elements from screen readers. [#234](https://github.com/gpui-ce/gpui-ce/pull/234)
- `Application::with_accessibility_forced()` and `Window::set_a11y_forced()` to generate accessibility trees without a screen reader. [#274](https://github.com/gpui-ce/gpui-ce/pull/274)
- `Window::a11y_tree()`, `a11y_node_bounds()`, and `a11y_frame_number()` for accessibility tree inspection. [#279](https://github.com/gpui-ce/gpui-ce/pull/279)
- `Window::perform_a11y_action()` to trigger accessibility actions programmatically. [#293](https://github.com/gpui-ce/gpui-ce/pull/293)
- Inline layout for elements via `.inline()` and `.inline_flex()`, and `.vertical_align()` for alignment. [#262](https://github.com/gpui-ce/gpui-ce/pull/262)
- Bidirectional text support for elements via `.rtl()` and `.ltr()`, with `.text_start()` and `.text_end()` for semantic alignment. [#263](https://github.com/gpui-ce/gpui-ce/pull/263)
- Unified renderer shaders using WGSL and added opt-in macOS WGPU renderer. [#215](https://github.com/gpui-ce/gpui-ce/pull/215) [#287](https://github.com/gpui-ce/gpui-ce/pull/287)
- Surface API for custom shaders. [#237](https://github.com/gpui-ce/gpui-ce/pull/237)
- Declarative style transitions API via `.transitions` method. [#125](https://github.com/gpui-ce/gpui-ce/pull/125) [#231](https://github.com/gpui-ce/gpui-ce/pull/231)
- New `Motion` API for timed and spring animations, with `Motion::new` and `Motion::spring`. [#125](https://github.com/gpui-ce/gpui-ce/pull/125) [#281](https://github.com/gpui-ce/gpui-ce/pull/281) [#284](https://github.com/gpui-ce/gpui-ce/pull/284)
- Customizable caret width & height for `EditableText`. [#218](https://github.com/gpui-ce/gpui-ce/pull/218)
- Tailwind-style ring api's. [#216](https://github.com/gpui-ce/gpui-ce/pull/216) [#231](https://github.com/gpui-ce/gpui-ce/pull/231)
- Gradient support for borders and shadows. [#252](https://github.com/gpui-ce/gpui-ce/pull/252)
- Customizable dashed border `length` and `gap`. [#254](https://github.com/gpui-ce/gpui-ce/pull/254)
- Figma flavoured corner smoothing. [#235](https://github.com/gpui-ce/gpui-ce/pull/235) [#338](https://github.com/gpui-ce/gpui-ce/pull/338)
- Overflow fade-clipping. [#298](https://github.com/gpui-ce/gpui-ce/pull/298)
- `Styled::whitespace` method that takes a `WhiteSpace` enum variant. [#243](https://github.com/gpui-ce/gpui-ce/pull/243/changes)
- `Styled::refine_style` method to refine an element's style. [#302](https://github.com/gpui-ce/gpui-ce/pull/302)
- `BaseSwitch` element. [#329](https://github.com/gpui-ce/gpui-ce/pull/329)
- `PaintQuad::new` and property-building functions. [#338](https://github.com/gpui-ce/gpui-ce/pull/338)
- Windows can now be hidden via `set_visible`. [#239](https://github.com/gpui-ce/gpui-ce/pull/239)
- Window icons are now supported on Wayland. [#133](https://github.com/gpui-ce/gpui-ce/pull/133)
- Character palette now works on Windows and Linux. [#308](https://github.com/gpui-ce/gpui-ce/pull/308)
- Directional resize cursors. [#322](https://github.com/gpui-ce/gpui-ce/pull/322)
- `WindowOptions` is now a builder. [#301](https://github.com/gpui-ce/gpui-ce/pull/301) [#299](https://github.com/gpui-ce/gpui-ce/pull/299)
- Improved Drag-and-drop capabilities. [#227](https://github.com/gpui-ce/gpui-ce/pull/227) [#270](https://github.com/gpui-ce/gpui-ce/pull/270)
- reworked HitTest data tracking to give users access to iterate information about Hitboxes included in the HitTest. [#247](https://github.com/gpui-ce/gpui-ce/pull/247)
- Detect mouse move outside of an elements hitbox via new `on_mouse_move_all` method. [#244](https://github.com/gpui-ce/gpui-ce/pull/244)
- `on_hover` now supports multiple listeners. [#131](https://github.com/gpui-ce/gpui-ce/pull/131)
- `Interactivity::on_prepaint`. [#129](https://github.com/gpui-ce/gpui-ce/pull/129)
- Deferred clipping masks and nested overlay priorities. [#245](https://github.com/gpui-ce/gpui-ce/pull/245)
- `Size::at_anchor` method to evaluate a point along the edge of a box at an anchor. [#130](https://github.com/gpui-ce/gpui-ce/pull/130)
- Opt-out for image crate features via `image-default-formats`. [#331](https://github.com/gpui-ce/gpui-ce/pull/331)
- Convenience derive macro's for `Styled`, `ParentElement`, `InteractiveElement`, `StatefulInteractiveElement`. [#300](https://github.com/gpui-ce/gpui-ce/pull/300)
- Variable font width via `.font_width(...)`. [#316](https://github.com/gpui-ce/gpui-ce/pull/316)
- Disable an element via new `disabled` method, and set style for a disabled element via `disabled_style` method. [#339](https://github.com/gpui-ce/gpui-ce/pull/339)
- `Window::activate_with_token` to activate a window with an external Wayland token. [#272](https://github.com/gpui-ce/gpui-ce/pull/272)

### Removals
- The opt-in Windows wgpu renderer. [#215](https://github.com/gpui-ce/gpui-ce/pull/215)
- Linux/FreeBSD screen capture. [#215](https://github.com/gpui-ce/gpui-ce/pull/215)
- The `gpui_apple/runtime_shaders`, `gpui_macos/runtime_shaders`, and `gpui_platform/runtime_shaders` features. [#215](https://github.com/gpui-ce/gpui-ce/pull/215)
- `WindowOptions::window_background` in favor of `macos_window_background`, `windows_window_background`, `linux_window_background`, and `web_window_background`. [#299](https://github.com/gpui-ce/gpui-ce/pull/299)

### Fixes
- EditableText selection remaining visible after losing focus. [#267](https://github.com/gpui-ce/gpui-ce/pull/267)
- Proportional pixel snapping for fractional padding on automatically sized axes. [#217](https://github.com/gpui-ce/gpui-ce/pull/217)
- Linux-to-macOS cross-compilation in gpui_media. [#232](https://github.com/gpui-ce/gpui-ce/pull/232)
- Duplicate characters during web IME composition. [#236](https://github.com/gpui-ce/gpui-ce/pull/236)
- Element opacity now applies to all descendants as one group. [#330](https://github.com/gpui-ce/gpui-ce/pull/330)
- Incorrect RTL glyph ordering in EditableText. [#267](https://github.com/gpui-ce/gpui-ce/pull/267)

### Security Changes

### Deprecations
- `quad`, `fill`, & `outline`. [#338](https://github.com/gpui-ce/gpui-ce/pull/338)
- `hover`, `group_hover`, `focus`, `in_focus`, `focus_visible`, `active`, `group_active`, and `group_drag_over` methods in favor of variants with a `_style` suffix. [#339](https://github.com/gpui-ce/gpui-ce/pull/339)
- `InteractiveElement::drag_over` in favor of `on_drag_over`. [#227](https://github.com/gpui-ce/gpui-ce/pull/227)

### Contributors
Thanks to the people who made this release happen!

- Björn Harrtell (@bjornharrtell)
- Calin P (@wyvernbw)
- Cameron Campbell (@cameronpcampbell)
- Christopher Addison (@chris-addison)
- David Acevedo (@dacevedo12)
- Domen Kožar (@domenkozar)
- Dustin Yost (@temportalflux)
- François (@francois-unity)
- Jayson Saavedra (@saavy1)
- Kayla Fortin (@kaylafortin)
- Miles Wirht (@philocalyst)
- Sabin Regmi (@wheregmis)
- Sampo Kivistö (@Havunen)
- Skye Ibrahim (@DeSitterUniverse)
- Takashi Fujita (@tgfjt)
- William Whittaker (@143mailliw)
- Zachary Freed (@FuZzledd)

## [0.2.2] - 2026-08-28
*Existed before changelog.*
