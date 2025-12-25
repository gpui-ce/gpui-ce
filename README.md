# gpui - Community Edition

A community fork of [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework.

## Cross-platform Support and external APIs

### Platform Matrix
| Platform | Windowing | Graphics | Text |
|----------|-----------|----------|------|
| Windows | Win32 | DirectX 11 | DirectWrite |
| macOS | Cocoa/AppKit | Metal | Core Text |
| macOS (Blade) | Cocoa/AppKit | Metal via Blade | Core Text |
| Linux (X11) | X11 | Vulkan via Blade | cosmic-text |
| Linux (Wayland) | Wayland | Vulkan via Blade | cosmic-text |
| Test | Virtual | Virtual | Virtual |

### Runtime Configuration (Environment Variables)
| Variable | Purpose |
|----------|---------|
| `ZED_HEADLESS` | Force headless mode (macOS/Linux) |
| `WAYLAND_DISPLAY` | Use Wayland compositor (Linux) |
| `DISPLAY` | Use X11 compositor (Linux) |

### Compile-time Configuration (Feature Flags)
| Feature | Purpose |
|---------|---------|
| `x11` | Enable X11 support on Linux |
| `wayland` | Enable Wayland support on Linux |
| `macos-blade` | Use Blade renderer on macOS (replaces native Metal) |
| `test-support` | Enable virtual test platform |
| `screen-capture` | Enable screen capture (Windows/Linux) |

### Platform Detection
1. **Windows** → Always Win32 + DirectX 11
2. **macOS** → Always Cocoa + Metal (or Metal via Blade with `macos-blade`)
3. **Linux** → Checks: `ZED_HEADLESS` → `WAYLAND_DISPLAY` → `DISPLAY` → Headless fallback

### Graphics Backends
- **Windows**: DirectX 11 (native)
- **macOS**: Metal (native) or Metal via Blade (`macos-blade` feature)
- **Linux**: Vulkan via Blade (always)
- **Test**: Virtual (for testing)

### Text Rendering Stack
- **Windows**: DirectWrite (native)
- **macOS**: Core Text (native)  
- **Linux**: cosmic-text (uses RustyBuzz for shaping, fontdb for font management)
