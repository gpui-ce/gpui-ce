# gpui - Community Edition

Welcome, to the community fork of [GPUI](https://gpui.rs).
For now, it is mostly API compatible, but this is changing! Join the discord if you'd like to help build the next generation of GUIs!

A GUI application framework that's:
- as simple as you need it to be.
- the fastest, if you dare to take it there.
- universally consistent, on all levels.

It's deeply inspired by the web, with tailwind syntax, familiar builtin elements, and reactivity. It blends intermediate and retained approaches, and is (working) to be nearly-dependency free and non-pessimized.

It delegates work to the GPU at a *deep* level, hence the name!

## Quickstart

Everything in GPUI starts with an `Application`. You can create one with `gpui_platform::application()`, and kick off your application by passing a callback to `Application::run()`. Inside this callback, you can create a new window with `App::open_window()`, and register your first root view. See [gpui.rs](https://www.gpui.rs/) for a complete example.

### Usage

`cargo add gpui-ce`, and you're ready to go! 

From then on, `gpui::{import}` to pull in whatever you need. Most of it will be under `gpui::prelude::*`. 

### High level concepts

GPUI offers three different [registers](<https://en.wikipedia.org/wiki/Register_(sociolinguistics)>) depending on your needs:

- State management and communication with `Entity`'s. Whenever you need to store application state that communicates between different parts of your application, you'll want to use GPUI's entities. Entities are owned by GPUI and are only accessible through an owned smart pointer similar to an `Rc`. See the `app::context` module for more information.

- High level, declarative UI with views. All UI in GPUI starts with a view. A view is simply an `Entity` that can be rendered, by implementing the `Render` trait. At the start of each frame, GPUI will call this render method on the root view of a given window. Views build a tree of `elements`, lay them out and style them with a tailwind-style API, and then give them to GPUI to turn into pixels. See the `div` element for an all purpose swiss-army knife of rendering.

- Low level, imperative UI with Elements. Elements are the building blocks of UI in GPUI, and they provide a nice wrapper around an imperative API that provides as much flexibility and control as you need. Elements have total control over how they and their child elements are rendered and can be used for making efficient views into large lists, implement custom layouting for a code editor, and anything else you can think of. See the `element` module for more information.

Each of these registers has one or more corresponding contexts that can be accessed from all GPUI services. This context is your main interface to GPUI, and is used extensively throughout the framework.

### Other Resources

In addition to the systems above, GPUI provides a range of smaller services that are useful for building complex applications:

- Actions are user-defined structs that are used for converting keystrokes into logical operations in your UI. Use this for implementing keyboard shortcuts, such as cmd-q. See the `action` module for more information.

- Platform services, such as `quit the app` or `open a URL` are available as methods on the `app::App`.

- An async executor that is integrated with the platform's event loop. See the `executor` module for more information.,

- The `[gpui::test]` macro provides a convenient way to write tests for your GPUI applications. Tests also have their own kind of context, a `TestAppContext` which provides ways of simulating common platform input. See `app::test_context` and `test` modules for more details.

Currently, the best way to learn about these APIs is to read the Zed source code or drop a question in the [Zed Discord](https://zed.dev/community-links). We're working on improving the documentation, creating more examples, and will be publishing more guides to GPUI on our [blog](https://zed.dev/blog).


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

## FAQ
#### Where does work happen?
- Join the [discord](https://discord.gg/vNdskJSfWd)
- File an issue
- Submit a PR.

We strive to be a welcoming community, and even the simplest of issues or worst PRs, we will work with, if you're willing to rise to the occasion.

Our AI policy is [that of the Rust foundations](https://rustfoundation.org/policy/internal-ai-usage-policy/), that GPUI is a framework "Made by Humans". This extends beyond the AI policy, and into community participation. We won't hate you for a drive-by PR, but we'll always take the time to understand your intentions, and provide education and support, if you have the time for it. This is a community project!

#### What is the long-term goal of GPUI-CE?
To be the premiere Rust GUI library.

We were born out of Zed's hard work, and we will always pull their fixes and hard work, they remain a huge inspiration.

And yet, our ambitious are larger than supporting the use-cases of two applications. We look to become the home for applications, big and small, and make GUI authoring as simple as TUI and CLI development, and allowing GUI-specific concerns and considerations painless to work with, while offering users the power to dive deep when it matters.

As a whole, we'd like to be a framework for real applications, to allow for deep support and code-sharing between GPUI projects. Our roots are in the web, and we want to take on the current application monster that is Electron, head to head, but with order-of-magnitude performance improvements and platform integrations. 

#### How does the project compare to other forks in the ecosystem?
Other efforts (namely WGPUI) are actively maintained, but have diverged quite a bit from mainline usage. They typically serve the interests of the projects that they're used within, leading to a diverse yet fragmented ecosystem. GPUI-CE focuses on stability, and continuously monitors the other forks for good ideas worth pulling in.


