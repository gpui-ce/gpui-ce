# GPUI iOS Example App

A demo application showcasing GPUI rendering on iOS/iPadOS.

## Quick Start

```bash
# Build and run on iOS Simulator
./build_and_run.sh

# Or manually:
# 1. Build Rust static library
cargo build --target aarch64-apple-ios-sim -p gpui --features font-kit

# 2. Open and run in Xcode
open GpuiIOS.xcodeproj
# Select iPhone simulator, press Cmd+R
```

## Requirements

- Xcode 16.0+
- iOS Simulator (iOS 18.0+)
- Rust toolchain with `aarch64-apple-ios-sim` target:
  ```bash
  rustup target add aarch64-apple-ios-sim
  ```

## Demos Included

- **Animation Playground** - Physics-based bouncing balls with particle effects
- **Shader Showcase** - Dynamic gradients, floating orbs, parallax effects
- **Text Editor** - Text input via UITextInput protocol, cursor/selection handling

## Architecture

The app uses an Objective-C/Rust FFI bridge:

```
main.m (UIApplicationDelegate)
    ↓
gpui_ios.h (C FFI declarations)
    ↓
ffi.rs (Rust exports)
    ↓
GPUI Application (Rust)
```

Key FFI functions:
- `gpui_ios_initialize()` - Create GPUI app
- `gpui_ios_did_finish_launching()` - App lifecycle
- `gpui_ios_run_demo()` - Start rendering loop
- `gpui_ios_handle_touch_*()` - Touch event forwarding

## Troubleshooting

**Build fails with missing target**: Run `rustup target add aarch64-apple-ios-sim`

**Linker errors**: Ensure `libgpui.a` is in the Xcode project's library search paths

**Blank screen**: Check that Metal is available (requires real device or Apple Silicon Mac)
