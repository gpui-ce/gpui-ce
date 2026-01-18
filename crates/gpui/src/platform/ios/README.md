# GPUI iOS Platform

iOS platform implementation for GPUI, providing GPU-accelerated UI rendering on iOS/iPadOS.

## Architecture

```
platform.rs     IosPlatform - main platform trait implementation
window.rs       IosWindow - UIWindow/UIViewController management
display.rs      IosDisplay - UIScreen wrapper
dispatcher.rs   IosDispatcher - Grand Central Dispatch integration
events.rs       Touch event handling (tap, drag → mouse events)
text_system.rs  CoreText-based text rendering (shared with macOS)
text_input.rs   UITextInput protocol classes for keyboard support
ffi.rs          C FFI exports for Objective-C interop
demos/          Interactive demo views
```

## Key Components

### Rendering
- **Metal** via Blade graphics backend
- **60fps** via CADisplayLink
- Scene primitives: quads, shadows, text, paths

### Text Input
- **UITextInput protocol** for software keyboard
- **GPUITextPosition/GPUITextRange** classes wrap UTF-16 offsets
- **IME support** via marked text handling

### Touch Events
Touch events are translated to GPUI mouse events:
| Touch | GPUI Event |
|-------|------------|
| Tap | MouseDown + MouseUp |
| Drag | MouseDown + MouseMove + MouseUp |
| Two-finger scroll | ScrollWheel |

## Adding a New Demo

1. Create `demos/my_demo.rs` implementing `Render` trait
2. Add to `demos/mod.rs` exports
3. Add navigation in `demos/menu.rs`

## Platform Differences from macOS

| Feature | macOS | iOS |
|---------|-------|-----|
| Window management | AppKit (NSWindow) | UIKit (UIWindow) |
| Menu bar | Yes | No |
| Multiple windows | Yes | Single window |
| Keyboard | Always available | Software/external |
| Mouse | Pointer device | Touch translated |

## Testing

```bash
# Build for simulator
cargo build --target aarch64-apple-ios-sim -p gpui

# Run example app
cd examples/ios_app && ./build_and_run.sh
```
