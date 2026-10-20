# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/). This project somewhat adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html), aside from the major version being 0.

## [Unreleased]

### Highlights

(only filled out when preparing the release)

### Breaking changes

- `ContentMask` gains `fade_outset`, `corner_radii` and `corner_outset` fields; struct literals need `..Default::default()` [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `MonochromeSprite.padding` is now `blur_radius` [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `overflow_hidden()` on an element with rounded corners now clips its children to those corners [#362](https://github.com/gpui-ce/gpui-ce/pull/362)

### Additions

- Changelog established
- Default text layout system switched to [parley](https://github.com/linebender/parley) [#267](https://github.com/gpui-ce/gpui-ce/pull/267)
- `PaintQuad::new` and property-building functions [#338](https://github.com/gpui-ce/gpui-ce/pull/338)
- `Window::on_present`, for applying state outside the scene (such as native view geometry) when the painted frame is presented [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `Window::enable_scene_overlay` and `Window::native_composition`, for compositing native child views between GPUI content and its menus, popovers and tooltips [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `ImageSource::evict` and `App::peek_asset`, for freeing cached images from the sprite atlas [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `Window::with_rounded_clip`, per-pixel rounded content masks [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `Window::paint_glyph_transformed`, for transformed and blurred monochrome glyphs [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- List tail reservations (`ListState::set_tail_reservation`) [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Idle macOS windows park their display link, and Metal releases idle render targets and shrinks its instance buffers [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Blur kernels are precomputed and adapt their downsample factor to the radius; idle filter targets are released on wgpu and Direct3D 11 [#362](https://github.com/gpui-ce/gpui-ce/pull/362)

### Removals

### Fixes

- Text selection is not cleared when editable-text looses focus [#250](https://github.com/gpui-ce/gpui-ce/issues/250)
- Blurred macOS window backgrounds render without blur on macOS 26 [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Images with explicit width and height are sized by their intrinsic aspect ratio [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- `ListState::scroll_by` panics after `scroll_to` with a negative item offset [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Linux desktop-setting changes panic with an already-borrowed client [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Ordinary clicks on Windows promote into drags [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- Nested clips drop an ancestor's edge fade, border-only quads lose their fade, and paths don't fade [#362](https://github.com/gpui-ce/gpui-ce/pull/362)
- macOS windows stop drawing after display sleep or a user switch [#362](https://github.com/gpui-ce/gpui-ce/pull/362)

### Security Changes

### Deprecations

- `quad`, `fill`, & `outline` [#338](https://github.com/gpui-ce/gpui-ce/pull/338)

### Contributors

Thanks to the people who made this release happen!

- Cameron Campbell (@cameronpcampbell)
- Vu Nguyen (@hoangvu12)
- Wing (@wingleeio)

## [0.2.2] - 2026-08-28

Existed before changelog
