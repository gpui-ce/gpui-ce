//! Run with `cargo bench -p gpui-ce --features bench-support --bench reflection`.

use criterion::Criterion;
use gpui::reflection::benchmarks;

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    benchmarks::run(&mut criterion);
    criterion.final_summary();
}
