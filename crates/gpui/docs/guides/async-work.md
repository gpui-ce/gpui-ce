# Run work in the background

It's frequent that an application will need to load data, or compute a result,
while its window is responsible for working with input. The preferred way of handling this
is through starting such work as a task, and then later updating the entity
when the result is ready.

The examples use Rust's `async` blocks and `.await`. If these are new to you,
start with [the Rust Book's introduction to async](https://doc.rust-lang.org/book/ch17-01-futures-and-syntax.html).
A future describes work that an executor advances by polling it. When an awaited future
is not ready, the task yields so other work can run; awaiting an already-ready
future does not yield.

## Choose an executor

GPUI provides two executors. The foreground executor runs tasks on the main
thread and gives them access to entities through an async context. Use it to
wait for a timer or an asynchronous network response, then update the interface.
The background executor runs tasks on native worker threads. Use it for synchronous
work such as parsing a large file, passing owned input and returning an owned
result. Marking a computation `async` does not move it off the main thread.

In a single-threaded web build, background futures also run on the browser's
main thread. Split long computations into short polls to let input and drawing
progress. [Executors and tasks](crate::_documentation::concepts::executors)
explains scheduling and how a threaded web build differs.

## Compute in the background and update the entity

As an example, to count the words in the document is one you'll want to clone its text while the entity is available.
Start a foreground task with [`Context::spawn`], and have it await the count
from [`AppContext::background_spawn`]. The background computation uses the
cloned text without borrowing the entity. Once the count is ready, the
foreground task updates the document and notifies GPUI to redraw it.

```rust
use gpui::{AppContext, Context, Task, TestApp};

struct Document {
    text: String,
    word_count: Option<usize>,
    _count: Task<()>,
}

impl Document {
    fn count_words(&mut self, cx: &mut Context<Self>) {
        let text = self.text.clone();
        self._count = cx.spawn(async move |document, cx| {
            let count = cx
                .background_spawn(async move { text.split_whitespace().count() })
                .await;
            document
                .update(cx, |document, cx| {
                    document.word_count = Some(count);
                    cx.notify();
                })
                .ok();
        });
    }
}

let mut app = TestApp::new();
let document = app.update(|cx| {
    cx.new(|_| Document {
        text: "one two three".into(),
        word_count: None,
        _count: Task::ready(()),
    })
});

app.update(|cx| document.update(cx, |document, cx| document.count_words(cx)));
app.run_until_parked();

app.update(|cx| assert_eq!(document.read(cx).word_count, Some(3)));
```

The `document` argument is a [`WeakEntity`], so the task does not keep the
document alive. Its `update` returns an error if the document was released while
the work ran. The example uses `.ok()` to discard that shutdown error.

## Store tasks to own and cancel work

Every spawn returns a [`Task`] handle. Keeping the handle keeps the work alive;
dropping it cancels the future. Choose an owner based on when the work should
stop:

* **Store it in an entity field** when the work belongs to that entity.
  Dropping the entity drops the task. Assigning a new task to the field cancels
  the old one.
* **Detach it** with [`Task::detach`] when the work should continue after the
  caller drops its handle, such as saving a file. The executor then owns the
  task until completion or shutdown. For work that returns a `Result`, use
  [`TaskExt::detach_and_log_err`] to log failures.

```rust
use gpui::{AppContext, Context, TaskExt};

struct Editor {
    text: String,
}

impl Editor {
    fn save(&mut self, cx: &mut Context<Self>) {
        let text = self.text.clone();
        cx.background_spawn(async move { write_draft(&text) })
            .detach_and_log_err(cx);
    }
}

// Stands in for a file write; a real app would return its I/O error.
fn write_draft(text: &str) -> Result<usize, String> {
    Ok(text.len())
}
```

Cancellation prevents further polls of the future; it cannot interrupt
synchronous code already running inside a poll. If a task is doing a long
computation, that computation can continue until the poll returns. A spawned
task that you neither await, store, nor detach is dropped when it leaves scope.

## Debounce by replacing a stored task

Debouncing delays a search until the user pauses typing. Store the task in a
field and replace it on every change. Each task first awaits a
[`BackgroundExecutor::timer`]; replacing it before the timer fires cancels that
search. A background computation that has already started may finish its current
poll, but the cancelled foreground task will not apply its result.

```rust
use std::time::Duration;

use gpui::{AppContext, Context, Task, TestApp};

const DEBOUNCE: Duration = Duration::from_millis(300);

struct Search {
    results: Vec<String>,
    _search: Task<()>,
}

impl Search {
    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self._search = cx.spawn(async move |search, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let results = cx
                .background_spawn(async move { find(&query) })
                .await;
            search
                .update(cx, |search, cx| {
                    search.results = results;
                    cx.notify();
                })
                .ok();
        });
    }
}

fn find(query: &str) -> Vec<String> {
    ["apple", "banana", "cherry"]
        .into_iter()
        .filter(|fruit| fruit.contains(query))
        .map(String::from)
        .collect()
}

let mut app = TestApp::new();
let search = app.update(|cx| {
    cx.new(|_| Search {
        results: Vec::new(),
        _search: Task::ready(()),
    })
});

app.update(|cx| search.update(cx, |search, cx| search.set_query("app".into(), cx)));
app.run_until_parked();
app.advance_clock(Duration::from_millis(100));

// Replacing the task cancels the "app" search before its timer fires.
app.update(|cx| search.update(cx, |search, cx| search.set_query("ban".into(), cx)));
app.run_until_parked();
app.advance_clock(Duration::from_millis(400));
app.run_until_parked();

app.update(|cx| assert_eq!(search.read(cx).results, vec!["banana".to_string()]));
```

The timer starts when the task is first polled, which is why the example calls
`run_until_parked` after each change. In an app, the event loop does that for
you.

## Set a timeout on a future

Use [`FutureExt::with_timeout`] to stop waiting after a deadline. It returns
`Ok(output)` if the future completes first, or `Err(Timeout)` if the timer wins.
Pass `cx.background_executor()` as the executor to use GPUI's timers.

When the timeout wins, the wrapped future is dropped. As with task cancellation,
this does not interrupt synchronous code already running in a poll or cancel
work that was detached separately.

```rust
use std::time::Duration;

use gpui::{AppContext, FutureExt, Timeout, TestApp};

struct Status {
    message: String,
}

let mut app = TestApp::new();
let status = app.update(|cx| cx.new(|_| Status { message: String::new() }));

app.update(|cx| {
    let status = status.clone();
    cx.spawn(async move |cx| {
        // A stand-in for a slow request that takes ten seconds.
        let request = cx.background_executor().timer(Duration::from_secs(10));
        let message = match request
            .with_timeout(Duration::from_secs(2), cx.background_executor())
            .await
        {
            Ok(()) => "finished".to_string(),
            Err(Timeout) => "timed out".to_string(),
        };
        status.update(cx, |status, cx| {
            status.message = message;
            cx.notify();
        });
    })
    .detach();
});

app.run_until_parked();
app.advance_clock(Duration::from_secs(3));
app.run_until_parked();

app.update(|cx| assert_eq!(status.read(cx).message, "timed out"));
```

## Work with a window from async code

Use [`Context::spawn_in`] when a task needs to return to a particular window,
such as to change its title. Pass that window when spawning; the task receives
an [`AsyncWindowContext`] instead of the [`AsyncApp`] supplied by
[`Context::spawn`]. Its [`update`](AsyncWindowContext::update) method provides
`&mut Window` and `&mut App` while the closure runs, and returns an error if the
window has closed.

```rust
use std::time::Duration;

use gpui::{Context, Render, Task, Window, div, prelude::*};

struct Clock {
    ticks: usize,
    _ticker: Task<()>,
}

impl Clock {
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._ticker = cx.spawn_in(window, async move |clock, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(ticks) = clock.update(cx, |clock, cx| {
                clock.ticks += 1;
                cx.notify();
                clock.ticks
            }) else {
                break;
            };
            // Fails once the window is gone, which ends the loop.
            if cx
                .update(|window, _cx| window.set_window_title(&format!("{ticks} s")))
                .is_err()
            {
                break;
            }
        });
    }
}

impl Render for Clock {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(format!("{} s", self.ticks))
    }
}
```

`start` takes the window and passes it to `spawn_in`. Keep the task in a field
so that dropping the view stops the clock.

## Use tokio libraries through `gpui_tokio`

Some networking libraries require a [tokio](https://tokio.rs) runtime. Use
`gpui_ce_tokio`, imported as `gpui_tokio` in this workspace, to run their
futures and await the results as GPUI tasks.

At startup, call `gpui_tokio::init(cx)` to create a runtime with two worker
threads, or `gpui_tokio::init_from_handle` to use an existing runtime.
Initialize it before calling the functions below; they panic if no runtime is
registered.

* `Tokio::spawn(cx, future)` runs a `Send` future on tokio and returns a
  `Task<Result<R, JoinError>>`.
* `Tokio::spawn_result(cx, future)` does the same for a future that returns
  `anyhow::Result<R>`, and returns a `Task<anyhow::Result<R>>`.
* `Tokio::handle(cx)` returns the runtime's `tokio::runtime::Handle`, for code that needs
  one directly.

Dropping the GPUI task aborts the tokio task. Await the GPUI task, or detach it,
the same way as any other task. The futures you pass to tokio can't use entities
or windows, so move results back with `Context::spawn` as in the first section.
This crate has no doctests, so this page describes its API rather than running
an example.

## Test async code deterministically

In tests, both executors run on a single simulated scheduler, and time only
moves when the test moves it. [`TestApp::run_until_parked`] runs all work that
can make progress. [`TestApp::advance_clock`] moves the simulated time forward
and wakes any timers that are due. Together they make the timers in this guide
run instantly and repeatably.

```rust
use std::time::Duration;

use gpui::{AppContext, Context, Task, TestApp};

struct Ticker {
    ticks: usize,
    _task: Task<()>,
}

let mut app = TestApp::new();
let ticker = app.update(|cx| {
    cx.new(|cx: &mut Context<Ticker>| Ticker {
        ticks: 0,
        _task: cx.spawn(async move |ticker, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            if ticker
                .update(cx, |ticker, cx| {
                    ticker.ticks += 1;
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        }),
    })
});

// Let the task start its first timer, then advance one second at a time.
app.run_until_parked();
for _ in 0..3 {
    app.advance_clock(Duration::from_secs(1));
    app.run_until_parked();
}

app.update(|cx| assert_eq!(ticker.read(cx).ticks, 3));
```

Each [`TestApp::update`] flushes queued effects before returning. Run the
scheduled work, then check the result in a later update. The
[testing guide](crate::_documentation::guides::testing) covers seeds and
`#[gpui::test]`, which runs a test with many random task orderings.

## See also

- [Executors and tasks](crate::_documentation::concepts::executors), for the foreground
  and background executors.
- [Contexts](crate::_documentation::concepts::contexts#async-contexts), for `AsyncApp` and
  `AsyncWindowContext`.
- [Share state between views](crate::_documentation::guides::sharing_state), for updating
  entities from tasks.
- [The tasks example](crate::_documentation::examples::tasks), a stopwatch and a
  cancellable prime search.
