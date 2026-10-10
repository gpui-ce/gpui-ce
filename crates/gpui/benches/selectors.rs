//! Run with `cargo bench -p gpui-ce --features bench-support --bench selectors`.
//! `-- --allocations` counts warm operations separately from timing runs.

use criterion::Criterion;
use gpui::{bench_platform, selector_benchmarks};
use smol::{
    block_on,
    process::{Command, Output},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    collections::HashSet,
    env,
    time::Duration,
};

#[derive(Clone, Copy, Default)]
struct Allocations {
    count: usize,
    bytes: usize,
}

thread_local! {
    static ALLOCATIONS: Cell<Option<Allocations>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn record(bytes: usize) {
    let _recorded = ALLOCATIONS.try_with(|counter| {
        if let Some(mut allocations) = counter.get() {
            allocations.count += 1;
            allocations.bytes += bytes;
            counter.set(Some(allocations));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());

        // SAFETY: Forward the caller's allocation contract to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());

        // SAFETY: Forward the caller's allocation contract to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The pointer and layout came from the system allocator.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);

        // SAFETY: Forward the caller's reallocation contract to the system allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn command_output(command: &mut Command) -> Output {
    block_on(command.output()).unwrap()
}

fn main() {
    let revision = command_output(Command::new("git").args(["rev-parse", "HEAD"]));
    let status = command_output(Command::new("git").args(["status", "--porcelain"]));
    let rustc = command_output(Command::new("rustc").arg("-V"));

    eprintln!(
        "revision={} dirty={}; {}; target={}-{}; backend=CPU TestPlatform; GPU=none; debug_assertions={}; font-kit={}; wayland={}; x11={}; windows-manifest={}",
        String::from_utf8_lossy(&revision.stdout).trim(),
        !status.stdout.is_empty(),
        String::from_utf8_lossy(&rustc.stdout).trim(),
        env::consts::ARCH,
        env::consts::OS,
        cfg!(debug_assertions),
        cfg!(feature = "font-kit"),
        cfg!(feature = "wayland"),
        cfg!(feature = "x11"),
        cfg!(feature = "windows-manifest"),
    );

    let allocations = env::args().any(|argument| argument == "--allocations");
    let mut criterion = if allocations {
        Criterion::default().profile_time(Some(Duration::from_millis(1)))
    } else {
        Criterion::default().configure_from_args()
    };
    let platform = bench_platform(None, gpui_platform::current_platform(true).text_system());
    let mut recorded = HashSet::new();

    selector_benchmarks::run(&mut criterion, platform, |name, operation| {
        if !allocations || !recorded.insert(name.to_owned()) {
            return;
        }

        ALLOCATIONS.with(|counter| counter.set(Some(Allocations::default())));
        operation();
        let result = ALLOCATIONS.with(|counter| counter.replace(None).unwrap());
        println!(
            "{name}: allocations={} bytes={}",
            result.count, result.bytes
        );
    });

    if !allocations {
        criterion.final_summary();
    }
}
