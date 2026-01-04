# gpui - Community Edition

A community fork of [GPUI](https://gpui.rs), Zed's GPU-accelerated UI framework.

## Usage

```toml
[dependencies]
gpui = { package = "gpui-ce", version = "0.3" }

# for test support...
[dev-dependencies]
gpui = { package = "gpui-ce", version = "0.3", features = ["test-support"] }
```

Then use `gpui::{import}` as normal.

---

todo: rewrite below...

# Welcome to GPUI!

GPUI is a hybrid immediate and retained mode, GPU accelerated, UI framework
for Rust, designed to support a wide variety of applications.

Everything in GPUI starts with an `Application`. You can create one with `Application::new()`, and kick off your application by passing a callback to `Application::run()`. Inside this callback, you can create a new window with `App::open_window()`, and register your first root view. See [gpui.rs](https://www.gpui.rs/) for a complete example.

### Dependencies

GPUI has various system dependencies that it needs in order to work.

#### macOS

On macOS, GPUI uses Metal for rendering. In order to use Metal, you need to do the following:

- Install [Xcode](https://apps.apple.com/us/app/xcode/id497799835?mt=12) from the macOS App Store, or from the [Apple Developer](https://developer.apple.com/download/all/) website. Note this requires a developer account.

> Ensure you launch Xcode after installing, and install the macOS components, which is the default option.

- Install [Xcode command line tools](https://developer.apple.com/xcode/resources/)

  ```sh
  xcode-select --install
  ```

- Ensure that the Xcode command line tools are using your newly installed copy of Xcode:

  ```sh
  sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
  ```

## The Big Picture

GPUI offers three different [registers](<https://en.wikipedia.org/wiki/Register_(sociolinguistics)>) depending on your needs:

- State management and communication with `Entity`'s. Whenever you need to store application state that communicates between different parts of your application, you'll want to use GPUI's entities. Entities are owned by GPUI and are only accessible through an owned smart pointer similar to an `Rc`. See the `app::context` module for more information.

- High level, declarative UI with views. All UI in GPUI starts with a view. A view is simply an `Entity` that can be rendered, by implementing the `Render` trait. At the start of each frame, GPUI will call this render method on the root view of a given window. Views build a tree of `elements`, lay them out and style them with a tailwind-style API, and then give them to GPUI to turn into pixels. See the `div` element for an all purpose swiss-army knife of rendering.

- Low level, imperative UI with Elements. Elements are the building blocks of UI in GPUI, and they provide a nice wrapper around an imperative API that provides as much flexibility and control as you need. Elements have total control over how they and their child elements are rendered and can be used for making efficient views into large lists, implement custom layouting for a code editor, and anything else you can think of. See the `element` module for more information.

Each of these registers has one or more corresponding contexts that can be accessed from all GPUI services. This context is your main interface to GPUI, and is used extensively throughout the framework.

## Other Resources

In addition to the systems above, GPUI provides a range of smaller services that are useful for building complex applications:

- Actions are user-defined structs that are used for converting keystrokes into logical operations in your UI. Use this for implementing keyboard shortcuts, such as cmd-q. See the `action` module for more information.

- Platform services, such as `quit the app` or `open a URL` are available as methods on the `app::App`.

- An async executor that is integrated with the platform's event loop. See the `executor` module for more information.,

- The `[gpui::test]` macro provides a convenient way to write tests for your GPUI applications. Tests also have their own kind of context, a `TestAppContext` which provides ways of simulating common platform input. See `app::test_context` and `test` modules for more details.

Currently, the best way to learn about these APIs is to read the Zed source code or drop a question in the [Zed Discord](https://zed.dev/community-links). We're working on improving the documentation, creating more examples, and will be publishing more guides to GPUI on our [blog](https://zed.dev/blog).

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
