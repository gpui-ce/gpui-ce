# Writing GPUI documentation

Start with a question someone has while using GPUI. A caller looking up
`rem_size` needs its unit; someone whose view stays stale needs the relationship
between mutation, notification and rendering. Give each explanation a home
where that reader can find it:

| Kind | Purpose | Lives in |
| --- | --- | --- |
| Tutorial | Teach by building one app, step by step | `crates/gpui/docs/tutorial/` |
| Guides | Get one task done | `crates/gpui/docs/guides/` |
| Concepts | Explain how and why GPUI works | `crates/gpui/docs/concepts/` |
| Reference | Describe one item exactly | `///` comments in `crates/gpui/src` |
| Examples | Complete programs to run and read | `crates/gpui/examples/` |

## Describe the contract

Begin a public item's docs with what it is or does, keeping any condition or
unit needed to interpret that sentence:

```rust
/// A handle to state owned by the [`App`].
pub struct Entity<T> { /* ... */ }

/// Returns the number of pixels in one rem for this window.
pub fn rem_size(&self) -> Pixels { /* ... */ }
```

Give callers enough to choose the API, then enough to use it. Document what
the signature leaves open: defaults and units, order of operations, callback
timing, state retained between frames, empty input and dropped handles. An
example belongs after those rules, under `# Examples` or a more specific
`# Example: <what it shows>`. Finish with `# Panics` and `# Errors` where they
apply, and a link to the guide or concept page. Move a failure earlier when
it determines whether the reader should use the API at all.

This order is a starting point, not a quota of sections. `Point::x` needs a
line; entities, contexts, focus and painting warrant more explanation.

Put cause and consequence together. A task's owner determines when it is
canceled; focus determines where a key can dispatch; notification connects a
changed value to observers and rendering. Explain the local rule before
linking to the wider model. Background work, for example, needs a description
of its input data and the return to the view before thread-pool details.

Write "call `notify` after changing the value" instead of "notification should
be performed following mutation". Active verbs, present tense and "you" make
the operation easier to follow. Name the method, event or number; cut empty
phrases such as "note that", "simply", "easily" and "in order to". Repeating
the parameter names uses space better spent on behavior a caller cannot infer.

Claims must survive inspection of the implementation. Avoid promises of smooth
frames, identical text across platforms or zero-cost operations unless there
is evidence. Give a measurement its conditions and source. If the behavior
is uncertain, read the code or prove it with an example before describing it.

Link an item at its first useful mention: ``[`Entity`]`` or
``[`Context::notify`]``. Qualify names that are out of scope, as in
``[`Entity`](crate::Entity)``. Pages in `crates/gpui/docs` can use those same
item links. To connect pages, use the full path:
`[the frame](crate::_documentation::concepts::the_frame)`.

## Make examples prove the explanation

Prefer a running example with an assertion. `assert_eq!` states the expected
result for the reader and checks it in CI. `TestApp` runs entity, app and window
code headlessly. Hide its harness with `#` while keeping the application code
visible:

  ````rust
  /// ```
  /// # use gpui::{AppContext, TestApp};
  /// # let mut app = TestApp::new();
  /// # app.update(|cx| {
  /// let count = cx.new(|_| 0);
  /// count.update(cx, |count, cx| {
  ///     *count += 1;
  ///     cx.notify();
  /// });
  /// assert_eq!(*count.read(cx), 1);
  /// # });
  /// ```
  ````

Setup can be hidden when it teaches nothing, but the complete example must
contain everything needed to run. Use `no_run` for a display or external
service, such as `gpui_platform::application().run(...)`; that code still
compiles. `ignore` must not conceal a broken example.

Take care with effects. Observers and event subscriptions run after the
outermost update. `TestApp::update` flushes them before returning. An assertion
immediately after `notify` or `emit` inside the same update can see the old
effect state; assert after that `update` returns, or in a later `update`.

Examples declaring actions with `actions!` or `#[derive(Action)]` need
```` ```standalone_crate ````. Ordinary doctests share a binary, and actions
register by name. Two declarations of `editor::Save` would make every example
that creates an app panic with "Action with name `editor::Save` already
registered".

## Build a page around the operation

A heading should tell the reader which operation or decision follows.
Introduce the first example's assumptions, show it, and explain what to observe
before moving on. A tutorial introduces one idea at a time and carries its
setup forward. Guides should link to concept pages when further internals
would interrupt the task. Concept pages can start with a model when that is
the prerequisite, including why it exists and what it does not do.

Reserve a plain blockquote for a condition or failure that needs to interrupt
the explanation. Begin it with **Note:**, **Tip:** or **Warning:**.

## Add a complete example program

A gallery example lives in one file in `crates/gpui/examples`. Its `fn main`
creates the application; `pub fn run(cx: &mut App)` does everything else.
Cargo builds the program, `site/player` runs it in the browser and renders it
in tests, and `_documentation.rs` creates a page containing its full source.
Register a new program in both places: the example list in
`crates/gpui/src/_documentation.rs` and the `examples!` list in
`site/player/src/main.rs`.

To embed a live example in another page, write:

```html
<div class="gpui-example" data-example="counter">

Run it with `cargo run --example counter`.

</div>
```

The GPUI site replaces that text with the live example. On docs.rs, readers
see the command instead.

## Review the reader's inference

Read the page in order, including tables, snippet comments, captions and
navigation. Does each paragraph answer the question raised by its predecessor
or prepare the next example? Follow links and check the relationship the
sentence claims. Compare consecutive tutorial programs for renamed fields,
missing setup and state that must survive between steps.

Then try to break the explanation. Choose applicable cases from this list:

- Change a value without notifying, or read another entity from a cached view.
- Drop a task, subscription, entity or window partway through an operation.
- Re-enter an update from a callback, or expect a deferred effect immediately.
- Use multibyte code points, combining marks or mixed text directions. Identify
  each offset's unit: UTF-8 bytes, UTF-16 code units, code points or grapheme
  boundaries.
- Change a measured list's membership or row height.
- Remove a required Cargo feature or use an unsupported backend.
- Give a control an accessibility role but no focus, activation or disabled guard.

For each finding, record the claim, a concrete misuse, the source or test, and
the corrected wording. Revise only as far as the evidence supports. Show both
cases when a distinction is easily missed: mutation and notification, list
structure and measurement, task cancellation and running synchronous code.

Read the prose once more without the snippets. Replace vague pronouns and
stock transitions with the operation or result they refer to. Repeated sentence
openings can reveal repeated information; remove the duplication without losing
the conditions that make the example work.

[Jiff's documentation](https://docs.rs/jiff/latest/jiff/) is a useful reference
for examples and API relationships; [Wikipedia's Manual of Style](https://en.wikipedia.org/wiki/Wikipedia:Manual_of_Style)
for clear, consistent wording; and [Gwern's style guide](https://gwern.net/style-guide)
for specificity and useful cross-references. Apply the principles to GPUI's
readers; the sites' formatting conventions are not project requirements.
Grammar and prose tools can suggest passages to inspect. Check their findings
in context, where identifiers and Markdown can cause false positives. An
AI-detector score establishes neither accuracy nor readability. Report a check
as unavailable when it could not run.

## Run the corresponding checks

Rustdoc combines source comments and the Markdown registered in
`crates/gpui/src/_documentation.rs`. Building the documentation checks its
links; doctests compile Rust snippets and execute those that permit it. Cargo
builds complete example programs separately:

```sh
just docs        # build the API docs and guides, failing on broken links
just test-doc    # test the Rust documentation examples
just site        # build the website, including the live examples
```

`just docs` fails on broken links. Passing these checks establishes that the
examples and links work under the tested conditions; the adversarial reading
must still establish that the explanation is useful and complete.
