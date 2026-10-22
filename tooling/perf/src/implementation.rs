//! Separate implementation module for publishing as a GPUI dependency.

use collections::HashMap;
use serde::{Deserialize, Serialize};
use std::{num::NonZero, time::Duration};

pub mod consts {
    //! Constants shared by the profiler and proc macro protocol.

    /// The suffix on the actual test function.
    pub const SUF_NORMAL: &str = "__ZED_PERF_FN";
    /// The suffix on an extra function which prints metadata about a test to stdout.
    pub const SUF_MDATA: &str = "__ZED_PERF_MDATA";
    /// The env var in which we pass the iteration count to our tests.
    pub const ITER_ENV_VAR: &str = "ZED_PERF_ITER";
    /// Metadata line prefix that distinguishes benchmark output from test harness output.
    pub const MDATA_LINE_PREF: &str = "ZED_MDATA_";
    /// The version number for the data returned from the test metadata function.
    /// Increment on non-backwards-compatible changes.
    pub const MDATA_VER: u32 = 0;
    /// The default weight, if none is specified.
    pub const WEIGHT_DEFAULT: u8 = 50;
    /// How long a test must have run to be assumed to be reliable-ish.
    pub const NOISE_CUTOFF: std::time::Duration = std::time::Duration::from_millis(250);

    /// Identifier for the iteration count of a test metadata.
    pub const ITER_COUNT_LINE_NAME: &str = "iter_count";
    /// Identifier for the weight of a test metadata.
    pub const WEIGHT_LINE_NAME: &str = "weight";
    /// Identifier for importance in test metadata.
    pub const IMPORTANCE_LINE_NAME: &str = "importance";
    /// Identifier for the test metadata version.
    pub const VERSION_LINE_NAME: &str = "version";

    /// Where to save json run information.
    pub const RUNS_DIR: &str = ".perf-runs";
}

/// How relevant a benchmark is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Importance {
    /// Regressions shouldn't be accepted without good reason.
    Critical = 4,
    /// Regressions should be paid extra attention.
    Important = 3,
    /// Regressions need no extra attention but may indicate a problem.
    #[default]
    Average = 2,
    /// It's unclear whether regressions here are meaningful, but still worth watching.
    /// The lowest importance checked by default.
    Iffy = 1,
    /// Regressions are likely spurious or outside core functionality. Useful in large
    /// numbers or to support higher-importance regressions. Not checked by default.
    Fluff = 0,
}

impl std::fmt::Display for Importance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Importance::Critical => f.write_str("critical"),
            Importance::Important => f.write_str("important"),
            Importance::Average => f.write_str("average"),
            Importance::Iffy => f.write_str("iffy"),
            Importance::Fluff => f.write_str("fluff"),
        }
    }
}

/// Why or when did this test fail?
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FailKind {
    /// Failed while triaging it to determine the iteration count.
    Triage,
    /// Failed while profiling it.
    Profile,
    /// Failed due to an incompatible version for the test.
    VersionMismatch,
    /// Could not parse metadata for a test.
    BadMetadata,
    /// Skipped due to filters applied on the perf run.
    Skipped,
}

impl std::fmt::Display for FailKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailKind::Triage => f.write_str("errored in triage"),
            FailKind::Profile => f.write_str("errored while profiling"),
            FailKind::VersionMismatch => f.write_str("test version mismatch"),
            FailKind::BadMetadata => f.write_str("bad test metadata"),
            FailKind::Skipped => f.write_str("skipped"),
        }
    }
}

/// Information about a given perf test.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestMdata {
    /// A version number for when the test was generated. If this is greater
    /// than the version this test handler expects, one of the following will
    /// happen in an unspecified manner:
    /// - The test is skipped silently.
    /// - The handler exits with an error message indicating the version mismatch
    ///   or inability to parse the metadata.
    ///
    /// INVARIANT: If `version` <= `MDATA_VER`, this tool *must* be able to
    /// correctly parse the output of this test.
    pub version: u32,
    /// How many iterations to pass this test if this is preset, or how many
    /// iterations a test ended up running afterwards if determined at runtime.
    pub iterations: Option<NonZero<usize>>,
    /// The test's [`Importance`].
    pub importance: Importance,
    /// The weight of this particular test within its importance category. Used
    /// when comparing across runs.
    pub weight: u8,
}

/// The actual timings of a test, as measured by Hyperfine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timings {
    /// Mean runtime for `self.iter_total` runs of this test.
    pub mean: Duration,
    /// Standard deviation for the above.
    pub stddev: Duration,
}

impl Timings {
    /// How many iterations does this test seem to do per second?
    #[expect(
        clippy::cast_precision_loss,
        reason = "We only care about a couple sig figs anyways"
    )]
    #[must_use]
    pub fn iters_per_sec(&self, total_iters: NonZero<usize>) -> f64 {
        (1000. / self.mean.as_millis() as f64) * total_iters.get() as f64
    }
}

/// Aggregate results, meant to be used for a given importance category. Each
/// test name corresponds to its benchmark results, iteration count, and weight.
type CategoryInfo = HashMap<String, (Timings, NonZero<usize>, u8)>;

/// Aggregate output of all tests run by this handler.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Output {
    /// A list of test outputs. Format is `(test_name, mdata, timings)`.
    /// The latter being `Ok(_)` indicates the test succeeded.
    ///
    /// INVARIANT: If the test succeeded, the second field is `Some(mdata)` and
    /// `mdata.iterations` is `Some(_)`.
    tests: Vec<(String, Option<TestMdata>, Result<Timings, FailKind>)>,
}

impl Output {
    /// Instantiates an empty "output". Useful for merging.
    #[must_use]
    pub fn blank() -> Self {
        Output { tests: Vec::new() }
    }

    /// Reports a success and adds it to this run's `Output`.
    pub fn success(
        &mut self,
        name: impl AsRef<str>,
        mut mdata: TestMdata,
        iters: NonZero<usize>,
        timings: Timings,
    ) {
        mdata.iterations = Some(iters);
        self.tests
            .push((name.as_ref().to_string(), Some(mdata), Ok(timings)));
    }

    /// Adds a failure to this run's output. Include the iteration count if the test was
    /// attempted, excluding version mismatches and skipped tests.
    ///
    /// Prefer the `fail!()` macro.
    pub fn failure(
        &mut self,
        name: impl AsRef<str>,
        mut mdata: Option<TestMdata>,
        attempted_iters: Option<NonZero<usize>>,
        kind: FailKind,
    ) {
        if let Some(ref mut mdata) = mdata {
            mdata.iterations = attempted_iters;
        }
        self.tests
            .push((name.as_ref().to_string(), mdata, Err(kind)));
    }

    /// True if no tests executed this run.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    /// Sorts the runs in the output in the order that we want them printed.
    pub fn sort(&mut self) {
        self.tests.sort_unstable_by(|a, b| match (a, b) {
            // Tests where we got no metadata go at the end.
            ((_, Some(_), _), (_, None, _)) => std::cmp::Ordering::Greater,
            ((_, None, _), (_, Some(_), _)) => std::cmp::Ordering::Less,
            // Then sort by importance, then weight.
            ((_, Some(a_mdata), _), (_, Some(b_mdata), _)) => {
                let c = a_mdata.importance.cmp(&b_mdata.importance);
                if matches!(c, std::cmp::Ordering::Equal) {
                    a_mdata.weight.cmp(&b_mdata.weight)
                } else {
                    c
                }
            }
            // Lastly by name.
            ((a_name, ..), (b_name, ..)) => a_name.cmp(b_name),
        });
    }

    /// Merges a run's results with a prefix. Start from `Output::blank()` to prefix every test.
    pub fn merge<'a>(&mut self, other: Self, pref_other: impl Into<Option<&'a str>>) {
        let pref = if let Some(pref) = pref_other.into() {
            "crates/".to_string() + pref + "::"
        } else {
            String::new()
        };
        self.tests = std::mem::take(&mut self.tests)
            .into_iter()
            .chain(
                other
                    .tests
                    .into_iter()
                    .map(|(name, md, tm)| (pref.clone() + &name, md, tm)),
            )
            .collect();
    }

    /// Compares `self` with `baseline`. A positive `PerfReport` means `self` performed better.
    ///
    /// # Panics
    /// `self` and `baseline` are assumed to have the iterations field on all
    /// `TestMdata`s set to `Some(_)` if the `TestMdata` is present itself.
    #[must_use]
    pub fn compare_perf(self, baseline: Self) -> PerfReport {
        let self_categories = self.collapse();
        let mut other_categories = baseline.collapse();

        let deltas = self_categories
            .into_iter()
            .filter_map(|(cat, self_data)| {
                // Compare categories with data in both runs.
                let mut other_data = other_categories.remove(&cat)?;
                let mut max = f64::MIN;
                let mut min = f64::MAX;

                // Running totals for averaging out tests.
                let mut r_total_numerator = 0.;
                let mut r_total_denominator = 0;
                // O(n^2), but unlikely to be a bottleneck for these benchmark sets.
                for (name, (s_timings, s_iters, weight)) in self_data {
                    // Only use the new weights if they conflict.
                    let Some((o_timings, o_iters, _)) = other_data.remove(&name) else {
                        continue;
                    };
                    let shift =
                        (o_timings.iters_per_sec(o_iters) / s_timings.iters_per_sec(s_iters)) - 1.;
                    if shift > max {
                        max = shift;
                    }
                    if shift < min {
                        min = shift;
                    }
                    r_total_numerator += shift * f64::from(weight);
                    r_total_denominator += u32::from(weight);
                }
                if r_total_denominator == 0 {
                    None
                } else {
                    let mean = r_total_numerator / f64::from(r_total_denominator);
                    // TODO: Consider aggregating standard deviation. Unknown test correlations
                    // make it hard to produce a meaningful result.
                    Some((cat, PerfDelta { max, mean, min }))
                }
            })
            .collect();

        PerfReport { deltas }
    }

    /// Groups tests by `Importance` in a `HashMap`.
    fn collapse(self) -> HashMap<Importance, CategoryInfo> {
        let mut categories = HashMap::<Importance, HashMap<String, _>>::default();
        for entry in self.tests {
            if let Some(mdata) = entry.1
                && let Ok(timings) = entry.2
            {
                if let Some(handle) = categories.get_mut(&mdata.importance) {
                    handle.insert(entry.0, (timings, mdata.iterations.unwrap(), mdata.weight));
                } else {
                    let mut new = HashMap::default();
                    new.insert(entry.0, (timings, mdata.iterations.unwrap(), mdata.weight));
                    categories.insert(mdata.importance, new);
                }
            }
        }

        categories
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Don't print the header for an empty run.
        if self.tests.is_empty() {
            return Ok(());
        }

        // We want to print important tests at the top, then alphabetical.
        let mut sorted = self.clone();
        sorted.sort();
        writeln!(
            f,
            "| Command | Iter/sec | Mean [ms] | SD [ms] | Iterations | Importance (weight) |",
        )?;
        writeln!(f, "|:---|---:|---:|---:|---:|---:|")?;
        for (name, metadata, timings) in &sorted.tests {
            match metadata {
                Some(metadata) => match timings {
                    Ok(timings) => {
                        // If the test succeeded, then metadata.iterations is Some(_).
                        writeln!(
                            f,
                            "| {} | {:.2} | {} | {:.2} | {} | {} ({}) |",
                            name,
                            timings.iters_per_sec(metadata.iterations.unwrap()),
                            {
                                // Very small mean runtimes will give inaccurate
                                // results. Should probably also penalise weight.
                                let mean = timings.mean.as_secs_f64() * 1000.;
                                if mean < consts::NOISE_CUTOFF.as_secs_f64() * 1000. / 8. {
                                    format!("{mean:.2} (unreliable)")
                                } else {
                                    format!("{mean:.2}")
                                }
                            },
                            timings.stddev.as_secs_f64() * 1000.,
                            metadata.iterations.unwrap(),
                            metadata.importance,
                            metadata.weight,
                        )?;
                    }
                    // We have (some) metadata, but the test errored.
                    Err(err) => writeln!(
                        f,
                        "| ({}) {} | N/A | N/A | N/A | {} | {} ({}) |",
                        err,
                        name,
                        metadata
                            .iterations
                            .map_or_else(|| "N/A".to_owned(), |i| format!("{i}")),
                        metadata.importance,
                        metadata.weight
                    )?,
                },
                // No metadata, couldn't even parse the test output.
                None => writeln!(
                    f,
                    "| ({}) {} | N/A | N/A | N/A | N/A | N/A |",
                    timings.as_ref().unwrap_err(),
                    name
                )?,
            }
        }
        Ok(())
    }
}

/// The difference in performance between two runs within a given importance
/// category.
struct PerfDelta {
    /// The biggest improvement / least bad regression.
    max: f64,
    /// The weighted average change in test times.
    mean: f64,
    /// The worst regression / smallest improvement.
    min: f64,
}

/// Shim type for reporting all performance deltas across importance categories.
pub struct PerfReport {
    /// Inner (group, diff) pairing.
    deltas: HashMap<Importance, PerfDelta>,
}

impl std::fmt::Display for PerfReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.deltas.is_empty() {
            return write!(f, "(no matching tests)");
        }
        let sorted = self.deltas.iter().collect::<Vec<_>>();
        writeln!(f, "| Category | Max | Mean | Min |")?;
        // Avoid extra trailing newlines.
        write!(f, "|:---|---:|---:|---:|")?;
        for (cat, delta) in sorted.into_iter().rev() {
            const SIGN_POS: &str = "↑";
            const SIGN_NEG: &str = "↓";
            const SIGN_NEUTRAL_POS: &str = "±↑";
            const SIGN_NEUTRAL_NEG: &str = "±↓";

            let prettify = |time: f64| {
                let sign = if time > 0.05 {
                    SIGN_POS
                } else if time > 0. {
                    SIGN_NEUTRAL_POS
                } else if time > -0.05 {
                    SIGN_NEUTRAL_NEG
                } else {
                    SIGN_NEG
                };
                format!("{} {:.1}%", sign, time.abs() * 100.)
            };

            // Pretty-print these instead of just using the float display impl.
            write!(
                f,
                "\n| {cat} | {} | {} | {} |",
                prettify(delta.max),
                prettify(delta.mean),
                prettify(delta.min)
            )?;
        }
        Ok(())
    }
}
