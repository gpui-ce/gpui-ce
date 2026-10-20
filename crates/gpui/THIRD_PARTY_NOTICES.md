# Third-party notices

## GPUI (egoist/zed)

The scene overlay — splitting a frame into a base plane and a deferred-content plane so native
child views can composite between them (`Window::enable_scene_overlay`,
`PlatformWindow::draw_layered`) — is adapted from the GPUI crates in a fork of Zed.

Source: <https://github.com/egoist/zed>, commit `57bd4fe181639797d395978d5de17bc9e10a6219`

Licensed under the Apache License, Version 2.0, the same license as this crate
(see `LICENSE-APACHE`).
