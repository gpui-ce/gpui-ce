# Executors and tasks

To count a document's lines without holding its entity open, clone the text,
compute from that snapshot in a background task, and write the count back on
the foreground. Here `cx.spawn` owns the return to GPUI, while
`cx.background_spawn` owns the parse:

```
# use gpui::{AppContext, Context, Task};
# fn parse(text: &str) -> usize { text.lines().count() }
struct Document {
    text: String,
    line_count: Option<usize>,
    _count_lines: Task<()>,
}

impl Document {
    fn count_lines(&mut self, cx: &mut Context<Self>) {
        let text = self.text.clone();
        self._count_lines = cx.spawn(async move |document, cx| {
            // Background computation: owned data, with no entities.
            let count = cx.background_spawn(async move { parse(&text) }).await;
            // Back on the main thread, with access to the entity.
            document
                .update(cx, |document, cx| {
                    document.line_count = Some(count);
                    cx.notify();
                })
                .ok();
        });
    }
}
```

`self.text.clone()` runs while the entity is available. After that, `parse`
uses its own string and holds no mutable lease on the document. The result is
a `usize`; assigning it to `line_count` and calling `notify` happen after the
foreground task resumes.

## Choose where the future runs

| | Foreground executor | Background executor |
| --- | --- | --- |
| Runs futures on | The main thread | Native worker threads; the main thread in a single-threaded web build |
| Future bounds | `'static` | Future and result: `Send + 'static` |
| Can reach entities and windows | Yes, through an async context | No |
| Start one with | [`App::spawn`], [`Context::spawn`], [`Context::spawn_in`] | [`background_spawn`](AppContext::background_spawn) |

The usual pattern is a foreground task awaiting a background parse. If the
count is not ready, the await returns `Pending`; the foreground future yields,
and the main thread can handle events and draw. When the background task has a
result, it wakes the foreground future so the executor can poll it again.

An await that is already ready does not yield. Execution continues in the same
poll until the future reaches `Pending` or finishes with `Ready`. Putting
synchronous parsing directly inside `cx.spawn` would still block the main
thread until it returns; `async` supplies a future, not a worker thread. Rust's
[`Future` documentation](https://doc.rust-lang.org/std/future/trait.Future.html)
describes this poll-and-wake contract in more detail.

On native targets, background futures run in parallel on worker threads. Move or
copy their inputs into the future and return plain data to the foreground task.
An entity handle alone cannot update an entity from a worker: the update also
needs a GPUI context, which belongs on the foreground side. Tokio or another
runtime can own the computation instead, but its scheduling and cancellation
rules remain that runtime's rules.

## Return to GPUI state

The foreground future uses an [`AsyncApp`] to borrow the app for each update;
the borrow ends before it awaits again. If the work belongs to one window,
`spawn_in` supplies an [`AsyncWindowContext`] instead. Operations through that
window context return errors after the window closes, so closing a window is a
normal completion path for work that outlives it.

Both contexts hold a weak app reference: they do not keep the application
alive, and a call through `AsyncApp` panics after the app is released. The
[contexts page](crate::_documentation::concepts::contexts#async-contexts)
explains these access boundaries.

[`Context::spawn`] passes a [`WeakEntity`] to the future. Storing the returned
task therefore does not keep the document alive: when the document is released,
the weak update fails. The example discards that expected error because there
is no document left to display the result. Capturing a separate strong handle
would defeat the lifetime boundary and can create a reference cycle.

The count describes the cloned version of the text. If the document changes
while parsing, the task still returns the old version's count. Start another
count after an edit, or check a revision number before applying the result when
the application permits several requests to remain in flight.

## Decide who owns the work

Every spawn returns a [`Task<T>`]. In `cx.spawn(...);`, the temporary task is
dropped at the semicolon, so its future might never receive a first poll. Give
the handle an owner that matches the intended lifetime:

* **Store it in a field** when the work belongs to an entity. Releasing the
  entity drops its tasks; replacing a search task on the next keystroke cancels
  the previous search.
* **Await it** when another async operation needs its result.
* **[`detach`](Task::detach) it** when it should continue after the local handle
  goes away, such as a file save with no caller waiting for the result.
  [`detach_and_log_err`](TaskExt::detach_and_log_err) logs failures from tasks
  returning `Result`. Neither method promises completion through app or process
  shutdown.

The `_count_lines` field makes replacement concrete. Calling `count_lines`
again drops the previous foreground task, including the background `Task` it
was awaiting, so the old result is not applied later. A parse that is already
running may still finish its synchronous work: dropping a task cannot preempt
Rust code. Cancellation takes effect when that code returns control to the
scheduler.

For prompt cancellation, split long work into chunks with opportunities to
become pending. A separately retained or detached task has its own owner, so
cancelling the future that waits for it does not cancel the retained task.

## Wait for a timer

[`BackgroundExecutor::timer`] completes after a duration. The loop below waits
for one timer per iteration, then briefly updates a counter. While the timer is
pending, the foreground task holds no app borrow. Once a timer has expired, its
await may already be ready and the loop continues in that same poll, just like
any other ready operation:

```
use std::time::Duration;
use gpui::{AppContext, TestApp};

let mut app = TestApp::new();
let ticks = app.update(|cx| cx.new(|_| 0));

let _clock = app.update(|cx| {
    // A weak handle, so the loop does not keep the entity alive. Updating it
    // fails once the entity is dropped, which ends the loop.
    let ticks = ticks.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let updated = ticks.update(cx, |ticks, cx| {
                *ticks += 1;
                cx.notify();
            });
            if updated.is_err() {
                break;
            }
        }
    })
});

app.advance_clock(Duration::from_secs(3));
assert_eq!(app.read(|cx| *ticks.read(cx)), 3);
```

There are two ways to end this clock. Drop `_clock` to cancel its task, or
release the counter so the next weak update returns an error and breaks the
loop. Keeping a strong `ticks` handle inside the loop would keep the counter
alive instead.

## Drive tasks in tests

Tests run both executors on a simulated single-threaded scheduler. A ready
task still needs the test to give it a turn, and a timer still needs simulated
time to advance. [`TestApp::run_until_parked`] drives work that is ready now;
[`TestApp::advance_clock`] moves the clock and wakes timers. The example can
therefore assert three ticks deterministically, without sleeping for three
seconds.

The scheduler can vary the order of ready work from a seed, which helps expose
ordering bugs. `#[gpui::test(iterations = 100)]` tries a hundred seeds; `SEED`
reproduces a failure. See the
[testing guide](crate::_documentation::guides::testing) for both controls.

## Account for the web target

In single-threaded WebAssembly, the background executor still requires
`Send + 'static`, but its polls run on the browser's main thread. A long parse
there still competes with rendering, so splitting work at meaningful await
points matters. Worker threads require WebAssembly threads, the Cargo feature,
a shared-memory build, and a cross-origin isolated page with the APIs needed
to instantiate workers. Instantiation can fail before a runtime fallback is
possible.

The [web guide](crate::_documentation::guides::web) gives the build and serving
requirements, including Trunk headers. The
[platforms page](crate::_documentation::platforms) describes fallback behavior.
Use the single-threaded configuration when the requirements for shared-memory
WebAssembly cannot be met.
