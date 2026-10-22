//! Profiles Zed tests marked `#[perf]` and prints timings as Markdown.
//! See `util_macros::perf` for attribute usage.
//!
//! # Setup
//! Make sure `hyperfine` is installed and in the shell path.
//!
//! # Usage
//! Rebuilds target crates with perf macro cfg flags and the optimized `release-fast` profile.
//!
//! To test an individual crate, run:
//! ```sh
//! cargo perf-test -p $CRATE
//! ```
//!
//! To test the workspace, which is slow, run:
//! ```sh
//! cargo perf-test --workspace
//! ```
//!
//! To filter out tests below an importance level such as `important`, run:
//! ```sh
//! cargo perf-test $WHATEVER -- --important
//! ```
//!
//! Pass `-- --quiet` to hide progress. These flags can be combined.
//!
//! ## Comparing runs
//! `--json=ident` saves per-crate results in `.perf-runs` unless no tests ran.
//! For example, `cargo perf-test -p gpui -- --json=blah` creates `.perf-runs/blah.gpui.json`.
//! Compare results with `cargo perf-compare new-ident old-ident`.
//!
//! To save the markdown output to a file instead, run `cargo perf-compare --save=$FILE
//! new-ident old-ident`.
//!
//! Comparisons include every `.perf-runs/ident.*.json` file. Remove stale matches first.
//!
//! # Notes
//! Build this crate, then use the `cargo perf-test` alias. Invoke the binary directly
//! only when working on the profiler.

mod implementation;

use implementation::{FailKind, Importance, Output, TestMdata, Timings, consts};

use std::{
    fs::OpenOptions,
    io::{Read, Write},
    num::NonZero,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// How many iterations to attempt the first time a test is run.
const DEFAULT_ITER_COUNT: NonZero<usize> = NonZero::new(3).unwrap();
/// Multiplier for the iteration count when a test doesn't pass the noise cutoff.
const ITER_COUNT_MUL: NonZero<usize> = NonZero::new(4).unwrap();

/// Do we keep stderr empty while running the tests?
static QUIET: AtomicBool = AtomicBool::new(false);

/// Report a failure into the output and skip an iteration.
macro_rules! fail {
    ($output:ident, $name:expr, $kind:expr) => {{
        $output.failure($name, None, None, $kind);
        continue;
    }};
    ($output:ident, $name:expr, $mdata:expr, $kind:expr) => {{
        $output.failure($name, Some($mdata), None, $kind);
        continue;
    }};
    ($output:ident, $name:expr, $mdata:expr, $count:expr, $kind:expr) => {{
        $output.failure($name, Some($mdata), Some($count), $kind);
        continue;
    }};
}

/// How does this perf run return its output?
enum OutputKind<'a> {
    /// Print markdown to the terminal.
    Markdown,
    /// Save JSON to a file.
    Json(&'a Path),
}

impl OutputKind<'_> {
    /// Logs the output of a run as per the `OutputKind`.
    fn log(&self, output: &Output, t_bin: &str) {
        match self {
            OutputKind::Markdown => println!("{output}"),
            OutputKind::Json(ident) => {
                // We're going to be in tooling/perf/$whatever.
                let wspace_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                    .join("..")
                    .join("..");
                let runs_dir = PathBuf::from(&wspace_dir).join(consts::RUNS_DIR);
                std::fs::create_dir_all(&runs_dir).unwrap();
                assert!(
                    !ident.to_string_lossy().is_empty(),
                    "FATAL: Empty filename specified!"
                );
                // Extract the crate name, such as gpui from deps/gpui-061ff76c9b7af5d7.
                let test_bin_stripped = Path::new(t_bin)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .rsplit_once('-')
                    .unwrap()
                    .0;
                let mut file_path = runs_dir.join(ident);
                file_path
                    .as_mut_os_string()
                    .push(format!(".{test_bin_stripped}.json"));
                let mut out_file = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&file_path)
                    .unwrap();
                out_file
                    .write_all(&serde_json::to_vec(&output).unwrap())
                    .unwrap();
                if !QUIET.load(Ordering::Relaxed) {
                    eprintln!("JSON output written to {}", file_path.display());
                }
            }
        }
    }
}

/// Runs a given metadata-returning function from a test handler, parsing its
/// output into a `TestMdata`.
fn parse_mdata(t_bin: &str, mdata_fn: &str) -> Result<TestMdata, FailKind> {
    let mut cmd = Command::new(t_bin);
    cmd.args([mdata_fn, "--exact", "--nocapture"]);
    let out = cmd
        .output()
        .expect("FATAL: Could not run test binary {t_bin}");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut version = None;
    let mut iterations = None;
    let mut importance = Importance::default();
    let mut weight = consts::WEIGHT_DEFAULT;
    for line in stdout
        .lines()
        .filter_map(|l| l.strip_prefix(consts::MDATA_LINE_PREF))
    {
        let mut items = line.split_whitespace();
        // For v0, we know the ident always comes first, then one field.
        match items.next().ok_or(FailKind::BadMetadata)? {
            consts::VERSION_LINE_NAME => {
                let v = items
                    .next()
                    .ok_or(FailKind::BadMetadata)?
                    .parse::<u32>()
                    .map_err(|_| FailKind::BadMetadata)?;
                if v > consts::MDATA_VER {
                    return Err(FailKind::VersionMismatch);
                }
                version = Some(v);
            }
            consts::ITER_COUNT_LINE_NAME => {
                // This should never be zero!
                iterations = Some(
                    items
                        .next()
                        .ok_or(FailKind::BadMetadata)?
                        .parse::<usize>()
                        .map_err(|_| FailKind::BadMetadata)?
                        .try_into()
                        .map_err(|_| FailKind::BadMetadata)?,
                );
            }
            consts::IMPORTANCE_LINE_NAME => {
                importance = match items.next().ok_or(FailKind::BadMetadata)? {
                    "critical" => Importance::Critical,
                    "important" => Importance::Important,
                    "average" => Importance::Average,
                    "iffy" => Importance::Iffy,
                    "fluff" => Importance::Fluff,
                    _ => return Err(FailKind::BadMetadata),
                };
            }
            consts::WEIGHT_LINE_NAME => {
                weight = items
                    .next()
                    .ok_or(FailKind::BadMetadata)?
                    .parse::<u8>()
                    .map_err(|_| FailKind::BadMetadata)?;
            }
            _ => unreachable!(),
        }
    }

    Ok(TestMdata {
        version: version.ok_or(FailKind::BadMetadata)?,
        // Iterations may be determined by us and thus left unspecified.
        iterations,
        // Importance should always be reported. Accept missing values for compatibility.
        importance,
        // Same with weight.
        weight,
    })
}

/// Compares the perf results of two profiles as per the arguments passed in.
fn compare_profiles(args: &[String]) {
    let mut save_to = None;
    let mut ident_idx = 0;
    args.first().inspect(|a| {
        if a.starts_with("--save") {
            save_to = Some(
                a.strip_prefix("--save=")
                    .expect("FATAL: save param formatted incorrectly"),
            );
            ident_idx = 1;
        }
    });
    let ident_new = args
        .get(ident_idx)
        .expect("FATAL: missing identifier for new run");
    let ident_old = args
        .get(ident_idx + 1)
        .expect("FATAL: missing identifier for old run");
    let wspace_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let runs_dir = PathBuf::from(&wspace_dir)
        .join("..")
        .join("..")
        .join(consts::RUNS_DIR);

    // Use the blank outputs initially, so we can merge into these with prefixes.
    let mut outputs_new = Output::blank();
    let mut outputs_old = Output::blank();

    for e in runs_dir.read_dir().unwrap() {
        let Ok(entry) = e else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };

            // Read and merge JSON output.
            let read_into = |output: &mut Output| {
                let mut elems = name.split('.').skip(1);
                let prefix = elems.next().unwrap();
                assert_eq!("json", elems.next().unwrap());
                assert!(elems.next().is_none());
                let mut buffer = Vec::new();
                let _ = OpenOptions::new()
                    .read(true)
                    .open(entry.path())
                    .unwrap()
                    .read_to_end(&mut buffer)
                    .unwrap();
                let o_other: Output = serde_json::from_slice(&buffer).unwrap();
                output.merge(o_other, prefix);
            };

            if name.starts_with(ident_old) {
                read_into(&mut outputs_old);
            } else if name.starts_with(ident_new) {
                read_into(&mut outputs_new);
            }
        }
    }

    let res = outputs_new.compare_perf(outputs_old);
    if let Some(filename) = save_to {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(filename)
            .expect("FATAL: couldn't save run results to file");
        file.write_all(format!("{res}").as_bytes()).unwrap();
    } else {
        println!("{res}");
    }
}

/// Lists tests marked for perf triage as `test_fn_name, test_mdata_name` pairs.
fn get_tests(t_bin: &str) -> impl ExactSizeIterator<Item = (String, String)> {
    let mut cmd = Command::new(t_bin);
    // --format=json is nightly-only :(
    cmd.args(["--list", "--format=terse"]);
    let out = cmd
        .output()
        .expect("FATAL: Could not run test binary {t_bin}");
    assert!(
        out.status.success(),
        "FATAL: Cannot do perf check - test binary {t_bin} returned an error"
    );
    if !QUIET.load(Ordering::Relaxed) {
        eprintln!("Test binary ran successfully; starting profile...");
    }
    // Parse the test harness output to look for tests we care about.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut test_list: Vec<_> = stdout
        .lines()
        .filter_map(|line| {
            // This should split only in two; e.g.,
            // "app::test::test_arena: test" => "app::test::test_arena:", "test"
            let line: Vec<_> = line.split_whitespace().collect();
            match line[..] {
                // Final byte of t_name is ":", which we need to ignore.
                [t_name, kind] => (kind == "test").then(|| &t_name[..t_name.len() - 1]),
                _ => None,
            }
        })
        // Exclude tests that aren't marked for perf triage based on suffix.
        .filter(|t_name| {
            t_name.ends_with(consts::SUF_NORMAL) || t_name.ends_with(consts::SUF_MDATA)
        })
        .collect();

    // Deduplicate with Vec to avoid an itertools dependency for one operation.
    test_list.sort_unstable();
    test_list.dedup();

    // Tests should come in pairs with their mdata fn!
    assert!(
        test_list.len().is_multiple_of(2),
        "Malformed tests in test binary {t_bin}"
    );

    let out = test_list
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .map(|pair| {
            // Be resilient against changes to these constants.
            if consts::SUF_NORMAL < consts::SUF_MDATA {
                (pair[0].to_owned(), pair[1].to_owned())
            } else {
                (pair[1].to_owned(), pair[0].to_owned())
            }
        })
        .collect::<Vec<_>>();
    out.into_iter()
}

/// Runs the specified test `count` times, returning the time taken if the test
/// succeeded.
#[inline]
fn spawn_and_iterate(t_bin: &str, t_name: &str, count: NonZero<usize>) -> Option<Duration> {
    let mut cmd = Command::new(t_bin);
    cmd.args([t_name, "--exact"]);
    cmd.env(consts::ITER_ENV_VAR, format!("{count}"));
    // Don't let the child muck up our stdin/out/err.
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    let pre = Instant::now();
    // Discard the output beyond ensuring success.
    let out = cmd.spawn().unwrap().wait();
    let post = Instant::now();
    out.iter().find_map(|s| s.success().then_some(post - pre))
}

/// Triage a test to determine the correct number of iterations that it should run.
/// Specifically, repeatedly runs the given test until its execution time exceeds
/// `thresh`, calling `step(iterations)` after every failed run to determine the new
/// iteration count. Returns `None` if the test errored or `step` returned `None`,
/// else `Some(iterations)`.
///
/// # Panics
/// This will panic if `step(usize)` is not monotonically increasing, or if the test
/// binary is invalid.
fn triage_test(
    t_bin: &str,
    t_name: &str,
    thresh: Duration,
    mut step: impl FnMut(NonZero<usize>) -> Option<NonZero<usize>>,
) -> Option<NonZero<usize>> {
    let mut iter_count = DEFAULT_ITER_COUNT;
    // Skip the first run to exclude cache warmup outliers.
    let duration_once = spawn_and_iterate(t_bin, t_name, NonZero::new(1).unwrap())?;
    loop {
        let duration = spawn_and_iterate(t_bin, t_name, iter_count)?;
        if duration.saturating_sub(duration_once) > thresh {
            break Some(iter_count);
        }
        let new = step(iter_count)?;
        assert!(
            new > iter_count,
            "FATAL: step must be monotonically increasing"
        );
        iter_count = new;
    }
}

/// Try to find the hyperfine binary the user has installed.
fn hyp_binary() -> Option<Command> {
    const HYP_PATH: &str = "hyperfine";
    const HYP_HOME: &str = "~/.cargo/bin/hyperfine";
    if Command::new(HYP_PATH).output().is_err() {
        if Command::new(HYP_HOME).output().is_err() {
            None
        } else {
            Some(Command::new(HYP_HOME))
        }
    } else {
        Some(Command::new(HYP_PATH))
    }
}

/// Profiles a given test with hyperfine, returning the mean and standard deviation
/// for its runtime. If the test errors, returns `None` instead.
fn hyp_profile(t_bin: &str, t_name: &str, iterations: NonZero<usize>) -> Option<Timings> {
    let mut perf_cmd = hyp_binary().expect("Couldn't find the Hyperfine binary on the system");

    // Warm up the cache and print markdown output to stdout, which we parse.
    perf_cmd.args([
        "--style",
        "none",
        "--warmup",
        "1",
        "--export-markdown",
        "-",
        // Parse json instead...
        "--time-unit",
        "millisecond",
        &format!("{t_bin} --exact {t_name}"),
    ]);
    perf_cmd.env(consts::ITER_ENV_VAR, format!("{iterations}"));
    let p_out = perf_cmd.output().unwrap();
    if !p_out.status.success() {
        return None;
    }

    let cmd_output = String::from_utf8_lossy(&p_out.stdout);
    // Skip the trailing newline instead of using .last().
    let results_line = cmd_output.lines().nth(3).unwrap();
    // Grab the values out of the pretty-print.
    // TODO: Parse json instead.
    let mut res_iter = results_line.split_whitespace();
    // Durations are given in milliseconds, so account for that.
    let mean = Duration::from_secs_f64(res_iter.nth(5).unwrap().parse::<f64>().unwrap() / 1000.);
    let stddev = Duration::from_secs_f64(res_iter.nth(1).unwrap().parse::<f64>().unwrap() / 1000.);

    Some(Timings { mean, stddev })
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    // We get passed the test we need to run as the 1st argument after our own name.
    let t_bin = args
        .get(1)
        .expect("FATAL: No test binary or command; this shouldn't be manually invoked!");

    // We're being asked to compare two results, not run the profiler.
    if t_bin == "compare" {
        compare_profiles(&args[2..]);
        return;
    }

    // Minimum test importance we care about this run.
    let mut thresh = Importance::Iffy;
    // Where to print the output of this run.
    let mut out_kind = OutputKind::Markdown;

    for arg in args.iter().skip(2) {
        match arg.as_str() {
            "--critical" => thresh = Importance::Critical,
            "--important" => thresh = Importance::Important,
            "--average" => thresh = Importance::Average,
            "--iffy" => thresh = Importance::Iffy,
            "--fluff" => thresh = Importance::Fluff,
            "--quiet" => QUIET.store(true, Ordering::Relaxed),
            s if s.starts_with("--json") => {
                out_kind = OutputKind::Json(Path::new(
                    s.strip_prefix("--json=")
                        .expect("FATAL: Invalid json parameter; pass --json=ident"),
                ));
            }
            _ => (),
        }
    }
    if !QUIET.load(Ordering::Relaxed) {
        eprintln!("Starting perf check");
    }

    let mut output = Output::default();

    // Profile sorted test/metadata-function pairs with hyperfine.
    let i = get_tests(t_bin);
    let len = i.len();
    for (idx, (ref t_name, ref t_mdata)) in i.enumerate() {
        if !QUIET.load(Ordering::Relaxed) {
            eprint!("\rProfiling test {}/{}", idx + 1, len);
        }
        // Pretty-printable stripped name for the test.
        let t_name_pretty = t_name.replace(consts::SUF_NORMAL, "");

        // Get the metadata this test reports for us.
        let t_mdata = match parse_mdata(t_bin, t_mdata) {
            Ok(mdata) => mdata,
            Err(err) => fail!(output, t_name_pretty, err),
        };

        if t_mdata.importance < thresh {
            fail!(output, t_name_pretty, t_mdata, FailKind::Skipped);
        }

        // Choose enough iterations to reduce random noise unless the count is fixed.
        let final_iter_count = t_mdata.iterations.or_else(|| {
            triage_test(t_bin, t_name, consts::NOISE_CUTOFF, |c| {
                if let Some(c) = c.checked_mul(ITER_COUNT_MUL) {
                    Some(c)
                } else {
                    // This should almost never happen, but maybe..?
                    eprintln!(
                        "WARNING: Ran nearly usize::MAX iterations of test {t_name_pretty}; skipping"
                    );
                    None
                }
            })
        });

        // Don't profile failing tests.
        let Some(final_iter_count) = final_iter_count else {
            fail!(output, t_name_pretty, t_mdata, FailKind::Triage);
        };

        if let Some(timings) = hyp_profile(t_bin, t_name, final_iter_count) {
            output.success(t_name_pretty, t_mdata, final_iter_count, timings);
        } else {
            fail!(
                output,
                t_name_pretty,
                t_mdata,
                final_iter_count,
                FailKind::Profile
            );
        }
    }
    if !QUIET.load(Ordering::Relaxed) {
        if output.is_empty() {
            eprintln!("Nothing to do.");
        } else {
            // If stdout and stderr are on the same terminal, move us after the
            // output from above.
            eprintln!();
        }
    }

    // No need making an empty json file on every empty test bin.
    if output.is_empty() {
        return;
    }

    out_kind.log(&output, t_bin);
}
