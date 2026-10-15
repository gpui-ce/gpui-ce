# Examples

Each page contains a complete program for one area of GPUI. Many also run in
the browser on the [GPUI site](https://gpui-ce.github.io). Examples that require
desktop platform services, such as notifications, run locally.

To run one yourself, clone the repository and run it by name:

```sh
git clone https://github.com/gpui-ce/gpui-ce
cd gpui-ce
cargo run --example counter
```

Start with [`hello_world`](crate::_documentation::examples::hello_world) to see
how an application opens a window, then [`counter`](crate::_documentation::examples::counter)
for state and click handlers. The [tutorial](crate::_documentation::tutorial)
combines these ideas into [`todos`](crate::_documentation::examples::todos),
adding text input, components, background work, and tests.
