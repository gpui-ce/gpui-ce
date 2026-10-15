# Use platform services

[`App`] provides clipboard access, menus, file pickers and other host services;
[`Window`] provides prompts and the window's appearance. Support depends on the
backend, and the return shape tells you what kind of operation you started:
some calls finish immediately, some return a task or response channel, and
screen capture returns a stream whose lifetime controls capture. The
[platform overview](crate::_documentation::platforms) lists availability; the
sections below describe how to consume each result.

The runnable examples use `TestApp`. Examples marked `no_run` need a real
backend or user permission.

## Copy and paste with the clipboard

The clipboard holds a [`ClipboardItem`], which can contain several entries.
[`App::write_to_clipboard`] replaces it. [`App::read_from_clipboard`] reads it
synchronously where the backend permits; a synchronous `None` means that no
usable item was returned, whether because the clipboard is empty or because
the backend cannot read it.

For text paste, call [`ClipboardItem::text`] on the item. It concatenates the
string entries and returns `None` if there are none, including an image-only
item. Here the test copies one string and reads it back:

```rust
# use gpui::{ClipboardItem, TestApp};
# let mut app = TestApp::new();
// Copy text, for example from a "Copy" button handler.
app.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("Copied text".to_string())));

// Paste: returns `None` when there is no clipboard item to read.
let item = app.read(|cx| cx.read_from_clipboard()).expect("the clipboard has an item");
assert_eq!(item.text().as_deref(), Some("Copied text"));
```

Use [`App::read_from_clipboard_async`] in code that can await. It also works on
synchronous backends, so the caller need not branch on clipboard timing. The
[`Task`] yields `Ok(Some(item))`, `Ok(None)` for no item, or a
[`ClipboardReadError`] when the read fails. On the web it can wait for
permission; the synchronous method always returns `None`. A headless or
unsupported backend may also ignore clipboard writes.

## Open a URL in the browser

[`App::open_url`] delegates to the host's default browser on desktop. On the
web it uses browser window APIs, subject to popup rules. In `TestApp`, the
request is recorded rather than opened; [`TestApp::opened_url`] returns the
last requested URL:

```rust
# use gpui::TestApp;
# let mut app = TestApp::new();
app.update(|cx| cx.open_url("https://github.com/gpui-ce/gpui-ce"));

assert_eq!(
    app.opened_url().as_deref(),
    Some("https://github.com/gpui-ce/gpui-ce"),
);
```

## Add application menus

Menus and shortcuts can invoke the same command. A [`MenuItem::action`]
dispatches its action through [`App`]; a key binding can dispatch that action
too. This example sends Quit and Undo to [`App::on_action`] handlers and
installs [`Menu`]s built from [`MenuItem`]s with [`App::set_menus`]:

```standalone_crate
use gpui::{App, KeyBinding, Menu, MenuItem, TestApp, actions};

actions!(platform_services, [Quit, Undo]);

fn set_up(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Undo, _cx| {
        // Undo the last edit here.
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-z", Undo, None),
    ]);
    cx.set_menus([
        Menu::new("App").items([MenuItem::action("Quit", Quit)]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::separator(),
        ]),
    ]);
}
# TestApp::new().update(set_up);
```

`set_menus` replaces the whole menu bar, so include every menu to retain.
`TestApp` ignores menus; test the handlers instead. For commands belonging to a
focused editor rather than the whole app, see the
[keyboard guide](crate::_documentation::guides::keyboard): menu actions use the
same dispatch path as shortcut actions.

## Show an alert

[`Window::prompt`] returns before the user answers. It presents a message and
an ordered set of buttons, then sends the chosen button's zero-based index
through a `oneshot::Receiver`; the receiver is the response, not the prompt
call itself. With `["Cancel", "Delete"]`, a Delete response is `1`:

```rust
# use gpui::{Context, PromptLevel, Render, TestApp, Window, div, prelude::*};
# struct Root;
# impl Render for Root {
#     fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div() }
# }
# let mut app = TestApp::new();
let mut window = app.open_window(|_, _| Root);

// Ask before deleting. The answers are the button labels, in order.
let mut answer = window.update(|_root, window, cx| {
    window.prompt(
        PromptLevel::Warning,
        "Delete this file?",
        Some("This cannot be undone."),
        &["Cancel", "Delete"],
        cx,
    )
});

// The test harness answers the prompt by label.
app.simulate_prompt_answer("Delete");
assert_eq!(answer.try_recv().unwrap(), Some(1));
```

`Info`, `Warning` and `Critical` in [`PromptLevel`] choose the icon and emphasis.
The labels `"OK"` and `"Cancel"` (ignoring case) select the corresponding platform button roles;
other labels create plain buttons. GPUI tries the desktop's native dialog and
uses an in-window prompt when none is available. The web uses that fallback.
Either kind replies with the same button index.

Await the `oneshot::Receiver<usize>` in a task so input processing can continue
while the prompt is open. Its output is `Ok(button_index)`. If the channel
closes, awaiting it returns `Err(oneshot::Canceled)`—a closed prompt channel,
distinct from a button response and from the file picker's `Ok(None)` user
cancellation.

## Pick files and folders

[`App::prompt_for_paths`] selects existing paths;
[`App::prompt_for_new_path`] selects a save location. Both return
`oneshot::Receiver<Result<Option<T>>>`: `T` is `Vec<PathBuf>` for opening and
`PathBuf` for saving. There are two independent failure layers—the picker can
fail, or its response channel can close:

| Result of awaiting the receiver | Meaning |
| --- | --- |
| `Ok(Ok(Some(value)))` | The user selected paths or a save location. |
| `Ok(Ok(None))` | The user cancelled. |
| `Ok(Err(error))` | The picker failed. |
| `Err(error)` | The response channel closed without a result. |

Linux needs a working desktop portal. This example handles selections only:

```rust,no_run
use std::path::Path;

use gpui::{App, PathPromptOptions};

fn open_and_save(cx: &mut App) {
    let open = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Open".into()),
    });
    let save = cx.prompt_for_new_path(Path::new("."), Some("notes.txt"));

    cx.spawn(async move |_cx| {
        if let Ok(Ok(Some(paths))) = open.await {
            println!("Open {paths:?}");
        }
        if let Ok(Ok(Some(path))) = save.await {
            println!("Save to {}", path.display());
        }
    })
    .detach();
}
```

[`PathPromptOptions`] controls whether the open picker accepts `files`,
`directories` and `multiple` selections. Before enabling both files and
directories, check [`App::can_select_mixed_files_and_dirs`]: only macOS supports
selecting both in one picker. Other platforms need separate pickers. The task
retains the receivers while the user decides, leaving the input handler free to
return.

## Capture a screen or window

Screen capture is available with `gpui_platform`'s `screen-capture` feature on
macOS 12.3 or later and Windows. [`App::is_screen_capture_supported`] reports
capability, not permission. Enumerating with [`App::screen_capture_sources`]
can still fail if the operating system denies access, and a successful result
can contain no sources.

Each [`ScreenCaptureSource`] describes a screen or window. After selecting one,
request frames with [`ScreenCaptureSource::stream`] and await the returned
receiver. Only `Ok(Ok(stream))` gives the caller a [`ScreenCaptureStream`] to
retain; an inner error or closed response channel gives it no stream. Keep the
stream alive for as long as frames are needed: dropping it stops capture.

The platform chooses the callback thread. Queue the frame or schedule
foreground work to update a view, as illustrated by the callback below:

```rust,no_run
use gpui::{App, ScreenCaptureFrame, ScreenCaptureSource};

fn start_capture(cx: &mut App) {
    if !cx.is_screen_capture_supported() {
        return;
    }

    let sources = cx.screen_capture_sources();
    let foreground = cx.foreground_executor().clone();
    cx.spawn(async move |_cx| {
        let Ok(Ok(mut sources)) = sources.await else {
            return;
        };
        let Some(source) = sources.pop() else {
            return;
        };
        let Ok(metadata) = source.metadata() else {
            return;
        };
        println!("capturing {:?} at {:?}", metadata.label, metadata.resolution);

        let frames = source.stream(
            &foreground,
            Box::new(|_frame: ScreenCaptureFrame| {
                // Forward the frame to an owned queue or schedule main-thread work.
            }),
        );
        let Ok(Ok(_stream)) = frames.await else {
            return;
        };

        // Keep `_stream` alive while the application consumes frames.
        std::future::pending::<()>().await;
    })
    .detach();
}
```

macOS frames contain a `CVImageBuffer`; Windows uses
`WindowsScreenCaptureFrame`. Headless and other unsupported targets report no
support or fail enumeration, so an application using capture needs another
path for those builds.

## Send system notifications

Register [`App::on_system_notification_response`] before posting a notification.
The handler runs on the main thread with `&mut App`, where it can update an
entity or dispatch an action. Registration replaces the previous handler, so
install it where the app owns notification handling.

[`App::show_system_notification`] posts to the operating system's notification
center. Give it a stable [`SystemNotification::tag`] when later posts should
replace it; the tag requests replacement but does not make it portable across
platforms. A user's action arrives as
[`SystemNotificationResponse::action_id`]. Permission settings may prevent
delivery, and some platforms display fewer actions than requested.

```rust,no_run
use gpui::{App, SystemNotification, SystemNotificationAction, SharedString};

fn notifications(cx: &mut App) {
    cx.on_system_notification_response(|response, cx| {
        if response.action_id.as_deref() == Some("open") {
            cx.open_url("https://gpui-ce.github.io/gpui-ce");
        }
    });

    cx.show_system_notification(SystemNotification {
        tag: "sync-finished".into(),
        title: "Sync finished".into(),
        body: "Your files are up to date.".into(),
        actions: vec![SystemNotificationAction {
            id: SharedString::from("open"),
            label: "Open".into(),
        }],
    });
}
```

Unsupported backends treat posting and dismissing as no-ops. For web apps, use
an in-window notification or the browser's own API.
The [system notifications example](crate::_documentation::examples::notifications)
shows posting, replacement, dismissal and response handling in a desktop app.

## Store credentials in the platform keychain

[`App::write_credentials`], [`App::read_credentials`] and
[`App::delete_credentials`] access the platform credential store using a URL
string as the key. Each returns a [`Task`]: a system service or permission
prompt may delay the answer. Await it from [`App::spawn`], keeping the input
handler free to return. A read yields `Ok(None)` for a missing entry; permission
denial or an unavailable service can produce an error.

Web and headless backends provide no persistent native store. These methods
are not browser storage. The example checks the write before reading and
deleting the entry, and keeps the password out of its log:

```rust,no_run
use gpui::App;

fn save_and_remove_credentials(cx: &mut App) {
    let write = cx.write_credentials("https://example.invalid", "demo-user", b"test-value");
    cx.spawn(async move |cx| {
        if write.await.is_err() {
            return;
        }

        let read = cx.update(|cx| cx.read_credentials("https://example.invalid"));
        let stored = read.await.ok().flatten();
        if let Some((username, _password)) = stored {
            println!("stored credentials for {username}");
        }

        let delete = cx.update(|cx| cx.delete_credentials("https://example.invalid"));
        let _ = delete.await;
    })
    .detach();
}
```

The [keyring example](crate::_documentation::examples::keyring) provides a
desktop interface for the same operations.

## Add haptic feedback

[`App::play_haptic_feedback`] plays a macOS trackpad pattern on the main thread.
Check [`App::supports_haptic_feedback`] before offering it; playback does
nothing on unsupported backends. This handler accompanies a snap into place:

```rust
use gpui::{App, HapticFeedbackStyle};

fn snap(cx: &mut App) {
    if cx.supports_haptic_feedback() {
        cx.play_haptic_feedback(HapticFeedbackStyle::Alignment);
    }
}
```

The patterns are `Generic` for a tap, `Alignment` for snapping or detents, and
`LevelChange` for a discrete slider or toggle change. Accompany haptics with
visible or audible feedback so the response works without a trackpad too.
The [haptics example](crate::_documentation::examples::haptics) lets you try
each style and shows a fallback status when the platform reports no support.

## Quit the application

[`App::quit`] asks the platform to quit gracefully. Closing the last window is a separate
policy controlled by [`QuitMode`]:

```rust,no_run
use gpui::{App, QuitMode};

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx: &mut App| {
            cx.activate(true);
        });
}

fn on_quit_button(cx: &mut App) {
    cx.quit();
}
```

[`QuitMode::Explicit`] keeps the app running after its last window closes;
[`QuitMode::LastWindowClosed`] quits then. [`QuitMode::Default`] chooses the
first policy on macOS and the second elsewhere. A browser cannot quit, so
`App::quit` only logs a warning there.

## Follow the system appearance

[`Window::appearance`] returns `Light`, `VibrantLight`, `Dark` or `VibrantDark`
as a [`WindowAppearance`]. Read the initial value when creating a view;
[`colors::Colors::for_appearance`] can choose its palette. Follow later changes
with [`Window::observe_window_appearance`], and retain the [`Subscription`]
while the view should receive them. Dropping it unregisters the callback:

```rust,no_run
use gpui::{Context, Subscription, Window, WindowAppearance, div, prelude::*};

struct Themed {
    appearance: WindowAppearance,
    _observe_appearance: Subscription,
}

impl Themed {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.entity();
        Self {
            appearance: window.appearance(),
            _observe_appearance: window.observe_window_appearance(move |window, cx| {
                let appearance = window.appearance();
                this.update(cx, |themed, cx| {
                    themed.appearance = appearance;
                    cx.notify();
                });
            }),
        }
    }
}

impl Render for Themed {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let text = match self.appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => "Dark",
            WindowAppearance::Light | WindowAppearance::VibrantLight => "Light",
        };
        div().child(format!("System appearance: {text}"))
    }
}
```

The callback runs on the next update after a platform change. It does not
recolor elements itself: the example stores the appearance and calls
[`Context::notify`] so rendering can use the new value. On the web, appearance
follows the `prefers-color-scheme` media query.

## What the web doesn't provide

For browser-specific alternatives, these are the GPUI methods' limits:

| Service | Browser capability |
| --- | --- |
| Clipboard | Async reads; writes depend on permission and browser support. |
| `open_url` | Uses browser window APIs and popup rules. |
| `set_menus` | Ignored. |
| `Window::prompt` | In-window fallback. |
| File pickers | Unsupported; return an error. |
| Screen capture | Unsupported. |
| System notifications | No GPUI operating-system notification center; posting and dismissal do nothing. |
| Platform credentials | No persistent native keychain through GPUI. |
| Haptic feedback | Unsupported. |
| `App::quit` | Logs a warning and does nothing. |
| `reveal_path`, `open_with_system` | Ignored. |
| `Window::appearance` | Follows `prefers-color-scheme`. |

For the browser's own setup, see [run on the web](crate::_documentation::guides::web).

## See also

- [Run work in the background](crate::_documentation::guides::async_work) covers
  the tasks that dialogs and clipboard reads return.
- [Test your app](crate::_documentation::guides::testing) covers [`TestApp`] in
  more depth.
- [Start a project](crate::_documentation::guides::start_a_project) shows how
  the application and its quit mode are set up.
