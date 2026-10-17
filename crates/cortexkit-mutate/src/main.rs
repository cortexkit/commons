#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand};
use cortexkit_mutate::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

#[derive(Parser)]
#[command(
    name = "ckdev-mutate",
    version,
    about = "Replay mutation proofs. Never git checkout a target mid-run: it removes the mutation before tests and fakes SURVIVED."
)]
struct Cli {
    #[arg(long, global = true, default_value = "mutations.toml")]
    catalogue: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Validate anchors and exact full test names without mutating source.
    Check {
        /// Refuse active command rows without list_command; legitimate skipped rows are exempt.
        #[arg(long)]
        strict: bool,
    },
    /// Replay selected catalogue rows.
    Run {
        #[arg(long, conflicts_with = "diff")]
        all: bool,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long)]
        shard: Option<String>,
        /// Assign rows by estimated cost. Every shard job must read the same report file.
        #[arg(long, value_name = "REPORT.json")]
        shard_weights: Option<PathBuf>,
        #[arg(long)]
        only: Option<String>,
        /// List selected row ids without building or running tests.
        #[arg(long)]
        list: bool,
        /// Expensive audit: run every package test target and grade broad catches.
        #[arg(long)]
        broad: bool,
        /// Run only each row's expect_red tests. Rows with their own select, `only = true`
        /// rows (sibling tests must stay green), hub rows and command rows run as written.
        #[arg(long, conflicts_with = "broad")]
        narrow: bool,
        #[arg(long)]
        allow_dirty: bool,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Prove one edit and append it only when caught.
    Prove(Proof),
    /// Find which tests, if any, catch one mutant: runs the whole package (or
    /// workspace) and lists every red test. Unscoped by design.
    Explore(Exploration),
}
#[derive(Args)]
struct FeatureSelection {
    /// Enable Cargo features (repeat or separate names with commas).
    #[arg(long, value_delimiter = ',', conflicts_with = "all_features")]
    features: Vec<String>,
    #[arg(long, conflicts_with = "all_features")]
    no_default_features: bool,
    #[arg(long)]
    all_features: bool,
    /// Also run `#[ignore]`d tests (include), or run only them (only).
    #[arg(long, value_name = "SELECTION", value_parser = ["include", "only"])]
    ignored: Option<String>,
}

/// Clap has already limited the value to the two catalogue spellings.
fn ignored_selection(value: Option<&str>) -> Option<IgnoredSelection> {
    match value? {
        "include" => Some(IgnoredSelection::Include),
        _ => Some(IgnoredSelection::Only),
    }
}

#[derive(Args)]
struct Exploration {
    #[command(flatten)]
    feature_selection: FeatureSelection,
    #[arg(long)]
    package: String,
    /// Restrict this row to Rust target-OS names (repeat for multiple OSes).
    #[arg(long = "platform")]
    platforms: Vec<String>,
    /// Run every test in the workspace instead of only the package.
    #[arg(long)]
    workspace: bool,
    #[arg(long, required_unless_present = "edits", conflicts_with = "edits", requires_all = ["old", "new"])]
    file: Option<String>,
    #[arg(long, requires = "file", allow_hyphen_values = true)]
    old: Option<String>,
    #[arg(long, requires = "file", allow_hyphen_values = true)]
    new: Option<String>,
    /// Inline TOML or JSON edits (or a path to a file holding them), instead of
    /// --file/--old/--new.
    #[arg(long)]
    edits: Option<String>,
    #[arg(long, default_value = "cargo")]
    runner: String,
    #[arg(long, default_value_t = 600)]
    timeout_s: u64,
    /// Bounds the mutant's build; --timeout-s bounds only the test run.
    #[arg(long, default_value_t = default_build_timeout())]
    build_timeout_s: u64,
    /// Record why this code has no production caller, without running a mutant.
    #[arg(long, value_name = "REASON", conflicts_with = "desk_only")]
    unreachable: Option<String>,
    /// Record a proof that requires a real desktop, without automated replay.
    #[arg(long, value_name = "REASON")]
    desk_only: Option<String>,
    /// Append a caught proof, or an explicitly reasoned UNREACHABLE row.
    #[arg(long, requires_all = ["id", "guards", "test_file"])]
    append: bool,
    #[arg(long)]
    id: Option<String>,
    #[arg(long, requires = "append")]
    guards: Option<String>,
    /// The appended row's test_file (required with --append).
    #[arg(long, requires = "append")]
    test_file: Option<String>,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    report: Option<PathBuf>,
}
#[derive(Args)]
struct ProofSelection {
    /// Run only the exact --expect-red tests on the baseline and mutant.
    #[arg(long, value_name = "SELECTION", value_parser = ["expected"], conflicts_with_all = ["only", "select_filter", "select_tests"])]
    select: Option<String>,
    /// Run tests whose full harness name contains this literal substring.
    #[arg(long, value_name = "TEXT", conflicts_with_all = ["select", "select_tests", "command"])]
    select_filter: Option<String>,
    /// Run these exact full harness names, including expected-green siblings.
    #[arg(long, value_delimiter = ',', num_args = 1, conflicts_with_all = ["select", "select_filter", "command"])]
    select_tests: Option<Vec<String>>,
}

#[derive(Args)]
struct Proof {
    #[command(flatten)]
    selection: Box<ProofSelection>,
    #[command(flatten)]
    feature_selection: FeatureSelection,
    #[arg(long = "platform")]
    platforms: Vec<String>,
    /// Record why this proof requires a real desktop, without running a mutant.
    #[arg(long, value_name = "REASON")]
    desk_only: Option<String>,
    #[arg(long)]
    id: String,
    #[arg(long)]
    guards: String,
    #[arg(long)]
    file: String,
    #[arg(long, allow_hyphen_values = true)]
    old: String,
    #[arg(long, allow_hyphen_values = true)]
    new: String,
    /// Require the old anchor to occur after this unique text in the file.
    #[arg(long, allow_hyphen_values = true)]
    scope_after: Option<String>,
    /// End the anchor region at the first occurrence of this text.
    #[arg(long, allow_hyphen_values = true, requires = "scope_after")]
    scope_before: Option<String>,
    #[arg(long)]
    test_file: String,
    #[arg(long)]
    runner: Option<String>,
    #[arg(long, required_unless_present = "command", conflicts_with = "command")]
    package: Option<String>,
    #[arg(long, allow_hyphen_values = true, conflicts_with = "command")]
    target: Option<String>,
    /// Command argv with one {test} element. Put this option last: all following
    /// values (including flags) belong to the command, not to ckdev-mutate.
    #[arg(long, num_args = 1.., allow_hyphen_values = true, conflicts_with = "only")]
    command: Option<Vec<String>>,
    /// Literal runner-output pattern with one {count} decimal placeholder.
    #[arg(long)]
    test_count_pattern: Option<String>,
    #[arg(long, requires = "command")]
    catch_on: Option<String>,
    #[arg(long, requires = "catch_on")]
    output_normalize: Vec<String>,
    #[arg(long, required_unless_present = "catch_on", num_args = 1..)]
    expect_red: Vec<String>,
    /// Substring or /regex/ required in each expected failure's output.
    #[arg(long)]
    expect_message: Option<String>,
    /// Explain why an abort or other signal is the intended catch.
    #[arg(long, value_name = "REASON")]
    signal_is_catch: Option<String>,
    #[arg(long)]
    only: bool,
    #[arg(long, default_value_t = 600)]
    timeout_s: u64,
    /// Bounds the mutant's build; --timeout-s bounds only the test run.
    #[arg(long, default_value_t = default_build_timeout())]
    build_timeout_s: u64,
    #[arg(long)]
    allow_dirty: bool,
    #[arg(long)]
    report: Option<PathBuf>,
}
#[derive(Default, Serialize)]
struct NarrowCounts {
    narrowed: usize,
    own_select: usize,
    only: usize,
    command_runner: usize,
    hub: usize,
    not_run: usize,
}

#[derive(Serialize)]
struct NarrowReport<'a> {
    narrow: bool,
    selection_counts: NarrowCounts,
    selections: BTreeMap<&'a str, NarrowSelection>,
    rows: &'a [Report],
}

impl<'a> NarrowReport<'a> {
    fn new(rows: &'a [Report], selections: &BTreeMap<String, NarrowSelection>) -> Self {
        let mut report = Self {
            narrow: true,
            selection_counts: NarrowCounts::default(),
            selections: BTreeMap::new(),
            rows,
        };
        for row in rows {
            let selection = selections[&row.id];
            report.selections.insert(&row.id, selection);
            let count = match selection {
                NarrowSelection::Narrowed => &mut report.selection_counts.narrowed,
                NarrowSelection::OwnSelect => &mut report.selection_counts.own_select,
                NarrowSelection::Only => &mut report.selection_counts.only,
                NarrowSelection::CommandRunner => &mut report.selection_counts.command_runner,
                NarrowSelection::Hub => &mut report.selection_counts.hub,
                NarrowSelection::NotRun => &mut report.selection_counts.not_run,
            };
            *count += 1;
        }
        report
    }
}

/// When the row's run on the unmutated tree failed, print its command, exit
/// status and output below the row's ERROR line, so that a one-off CI failure
/// can be diagnosed from the log alone.
fn print_baseline_failure(id: &str, failure: &BaselineFailure) {
    let command = failure
        .command
        .iter()
        .map(|arg| {
            if !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || "\"'\\".contains(c))
            {
                arg.clone()
            } else {
                format!("{arg:?}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    println!("  {id}: baseline command: {command}");
    println!("  {id}: baseline status: {}", failure.exit);
    println!(
        "  {id}: baseline output ({}):",
        if failure.truncated {
            format!(
                "last {BASELINE_TAIL_LINES} lines or {} KiB; earlier output dropped",
                BASELINE_TAIL_BYTES / 1024
            )
        } else {
            "complete".to_owned()
        }
    );
    if failure.output_tail.is_empty() {
        println!("    <no output>");
    }
    for line in failure.output_tail.lines() {
        println!("    {line}");
    }
}

fn write_report(
    path: Option<PathBuf>,
    rows: &[Report],
    selections: Option<&BTreeMap<String, NarrowSelection>>,
) -> Result<()> {
    let narrow_report = selections.map(|selections| NarrowReport::new(rows, selections));
    if let Some(report) = &narrow_report {
        let counts = &report.selection_counts;
        println!(
            "NARROW run summary (reported row selections): {} narrowed; unchanged: own select {}, only {}, command runner {}, hub {}; not run {}. Narrowed rows cannot detect WRONG_TEST or collateral failures.",
            counts.narrowed, counts.own_select, counts.only, counts.command_runner, counts.hub, counts.not_run
        );
    }
    for row in rows {
        println!(
            "{}: {}{}{}{}",
            row.label(),
            serde_json::to_value(&row.outcome)
                .map_err(|e| e.to_string())?
                .as_str()
                .ok_or("invalid outcome")?,
            if row.outcome.is_caught() && row.collateral.count > 0 {
                format!(
                    " ({}collateral: {} tests in targets: {})",
                    if row.outcome == Outcome::CaughtBroadly {
                        "warning; "
                    } else {
                        ""
                    },
                    row.collateral.count,
                    row.collateral.targets.join(", ")
                )
            } else {
                String::new()
            },
            row.reason
                .as_ref()
                .map_or(String::new(), |r| format!(" ({r})")),
            if row.outcome.is_caught() && !row.breadth_observed {
                " (breadth not observed; use run --broad to audit)"
            } else {
                ""
            }
        );
        if let Some(scope) = &row.scope {
            println!(
                "  anchor scope: after={:?}, before={:?}",
                scope.after, scope.before
            );
        }
        if let Some(failure) = &row.baseline_failure {
            print_baseline_failure(&row.id, failure);
        }
        if matches!(
            row.outcome,
            Outcome::WrongTest | Outcome::RedForAnotherReason
        ) {
            for (name, output) in row.unexpected_failures() {
                println!("  unexpected red: {name}");
                if output.is_empty() {
                    println!("    <no failure output>");
                } else {
                    for line in output.lines().take(6) {
                        println!("    {line}");
                    }
                }
            }
        }
    }
    let broad = rows
        .iter()
        .filter(|r| r.outcome == Outcome::CaughtBroadly)
        .count();
    let unreachable: Vec<_> = rows
        .iter()
        .filter(|r| r.outcome == Outcome::Unreachable)
        .collect();
    let hubs: Vec<_> = rows.iter().filter(|r| r.outcome == Outcome::Hub).collect();
    let skipped = rows
        .iter()
        .filter(|r| r.outcome == Outcome::SkippedPlatform)
        .count();
    let desk: Vec<_> = rows
        .iter()
        .filter(|r| r.outcome == Outcome::DeskOnly)
        .collect();
    if broad > 0
        || !unreachable.is_empty()
        || !hubs.is_empty()
        || skipped > 0
        || !desk.is_empty()
        || rows
            .iter()
            .any(|r| matches!(r.outcome, Outcome::Equivalent | Outcome::EquivalentCaught))
    {
        println!(
            "Summary: {} CAUGHT, {broad} CAUGHT_BROADLY (warning), {} EQUIVALENT, {} EQUIVALENT_CAUGHT, {} UNREACHABLE, {} HUB, {skipped} SKIPPED_PLATFORM, {} DESK_ONLY",
            rows.iter().filter(|r| r.outcome == Outcome::Caught).count(),
            rows.iter()
                .filter(|r| r.outcome == Outcome::Equivalent)
                 .count(),
            rows.iter().filter(|r| r.outcome == Outcome::EquivalentCaught).count(),
            unreachable.len(),
            hubs.len(),
            desk.len()
        );
    }
    if !unreachable.is_empty() {
        println!("UNREACHABLE rows ({}):", unreachable.len());
        for row in unreachable {
            println!(
                "  {}: {}",
                row.label(),
                row.reason.as_deref().unwrap_or_default()
            );
        }
    }
    if !hubs.is_empty() {
        println!("HUB rows ({}):", hubs.len());
        for row in hubs {
            println!(
                "  {}: {}",
                row.label(),
                row.reason.as_deref().unwrap_or_default()
            );
        }
    }
    if !desk.is_empty() {
        println!("DESK_ONLY rows ({}):", desk.len());
        for row in desk {
            println!(
                "  {}: {}",
                row.label(),
                row.reason.as_deref().unwrap_or_default()
            );
        }
    }
    let mut baseline_red: std::collections::BTreeMap<&str, std::collections::BTreeSet<&str>> =
        std::collections::BTreeMap::new();
    for row in rows {
        for (target, names) in &row.baseline_red {
            baseline_red
                .entry(target)
                .or_default()
                .extend(names.iter().map(String::as_str));
        }
    }
    if !baseline_red.is_empty() {
        println!("red at baseline (ignored):");
        for (target, names) in baseline_red {
            for name in names {
                println!("  {target}::{name}");
            }
        }
    }
    eprintln!(
        "Baseline timing: build {} ms, test {} ms, prebuild {} ms",
        rows.iter().map(|r| r.baseline_build_ms).sum::<u128>(),
        rows.iter().map(|r| r.baseline_test_ms).sum::<u128>(),
        rows.iter().map(|r| r.baseline_prebuild_ms).sum::<u128>()
    );
    if let Some(path) = path {
        let bytes = if let Some(report) = narrow_report {
            serde_json::to_vec_pretty(&report)
        } else {
            serde_json::to_vec_pretty(rows)
        }
        .map_err(|e| e.to_string())?;
        let mut temporary = tempfile::NamedTempFile::new_in(output_parent(&path))
            .map_err(|e| format!("cannot write report {}: {e}", path.display()))?;
        temporary
            .write_all(&bytes)
            .map_err(|e| format!("cannot write report {}: {e}", path.display()))?;
        temporary.persist(&path).map_err(|e| {
            format!(
                "cannot atomically write report {}: {}",
                path.display(),
                e.error
            )
        })?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct ReplayTiming {
    id: String,
    build_ms: u128,
    test_ms: u128,
}

#[derive(Debug, PartialEq, Eq)]
struct ShardPlan {
    rows: Vec<Vec<usize>>,
    loads: Vec<u128>,
}

fn read_shard_weights(path: &Path) -> Result<BTreeMap<String, u128>> {
    #[derive(Deserialize)]
    struct WrappedTimings {
        rows: Vec<ReplayTiming>,
    }
    let bytes = fs::read(path)
        .map_err(|e| format!("cannot read shard weight report {}: {e}", path.display()))?;
    let report: Vec<ReplayTiming> = serde_json::from_slice(&bytes)
        .or_else(|_| serde_json::from_slice::<WrappedTimings>(&bytes).map(|report| report.rows))
        .map_err(|e| format!("invalid shard weight report {}: {e}", path.display()))?;
    let mut weights = BTreeMap::new();
    for row in report {
        let weight = row
            .build_ms
            .checked_add(row.test_ms)
            .ok_or_else(|| format!("shard weight overflows for row {}", row.id))?;
        if weights.insert(row.id.clone(), weight).is_some() {
            return Err(format!(
                "duplicate row id {} in shard weight report {}",
                row.id,
                path.display()
            ));
        }
    }
    Ok(weights)
}

fn median_weight(weights: &BTreeMap<String, u128>) -> u128 {
    let mut sorted: Vec<_> = weights.values().copied().collect();
    if sorted.is_empty() {
        return 1;
    }
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        let low = sorted[middle - 1];
        low + (sorted[middle] - low) / 2
    }
}

fn weights_for_selection(
    report_weights: &BTreeMap<String, u128>,
    row_ids: &[String],
) -> (BTreeMap<String, u128>, usize) {
    let median = median_weight(report_weights);
    let mut missing = 0;
    let selected = row_ids
        .iter()
        .map(|id| {
            let weight = report_weights.get(id).copied().unwrap_or_else(|| {
                missing += 1;
                median
            });
            (id.clone(), weight)
        })
        .collect();
    (selected, missing)
}

fn shard_plan(
    row_ids: &[String],
    count: usize,
    weights: Option<&BTreeMap<String, u128>>,
) -> Result<ShardPlan> {
    let mut rows = vec![Vec::new(); count];
    let mut loads = vec![0u128; count];
    if let Some(weights) = weights {
        let mut order: Vec<_> = (0..row_ids.len()).collect();
        order.sort_by(|&left, &right| {
            weights[&row_ids[right]]
                .cmp(&weights[&row_ids[left]])
                .then_with(|| row_ids[left].cmp(&row_ids[right]))
        });
        for row in order {
            let shard = (0..count)
                .min_by_key(|&shard| (loads[shard], shard))
                .ok_or("shard count must be greater than zero")?;
            loads[shard] = loads[shard]
                .checked_add(weights[&row_ids[row]])
                .ok_or("estimated shard weight overflows")?;
            rows[shard].push(row);
        }
        for shard in &mut rows {
            shard.sort_unstable();
        }
    } else {
        if count == 0 {
            return Err("shard count must be greater than zero".into());
        }
        for row in 0..row_ids.len() {
            rows[row % count].push(row);
        }
    }
    Ok(ShardPlan { rows, loads })
}

fn weighted_summary(plan: &ShardPlan, index: usize, missing: usize) -> String {
    let estimates = plan
        .loads
        .iter()
        .enumerate()
        .map(|(shard, load)| {
            format!(
                "{}/{}={:.2} min",
                shard + 1,
                plan.loads.len(),
                *load as f64 / 60_000.0
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let note = if missing > 0 {
        format!("; {missing} selected row(s) missing from report, assigned the median weight")
    } else {
        String::new()
    };
    format!(
        "Shard {}/{}: {} rows, estimated {:.2} minutes; estimates: {estimates}{note}",
        index + 1,
        plan.rows.len(),
        plan.rows[index].len(),
        plan.loads[index] as f64 / 60_000.0,
    )
}

fn output_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn prepare_output_path(path: &Path) -> Result<()> {
    let parent = output_parent(path);
    fs::create_dir_all(parent)
        .map_err(|e| format!("cannot prepare output path {}: {e}", path.display()))?;
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(format!(
                "cannot prepare output path {}: destination is a directory",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot prepare output path {}: {e}",
                path.display()
            ));
        }
    }
    tempfile::NamedTempFile::new_in(parent)
        .and_then(|file| file.close())
        .map_err(|e| format!("cannot prepare output path {}: {e}", path.display()))
}

fn prepare_optional_output(path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        prepare_output_path(path)?;
    }
    Ok(())
}

fn run() -> Result<bool> {
    let cli = Cli::parse();
    let root = repository()?;
    std::env::set_current_dir(&root).map_err(|e| e.to_string())?;
    let catalogue_path = catalogue_path(&cli.catalogue)?;
    let _lock = TreeLock::acquire(&root)?;
    let stop = signal_flag()?;
    match cli.command {
        Action::Check { strict } => {
            let report = check(&root, &load(&catalogue_path)?, &stop)?;
            for (id, reason) in &report.not_verified {
                println!("{id}: test names not verified: {reason}");
            }
            let ids = report
                .not_verified
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>();
            println!(
                "Catalogue anchors checked; verified {} rows; {} rows not verified{}",
                report.verified.len(),
                ids.len(),
                if ids.is_empty() {
                    String::new()
                } else {
                    format!(": {}", ids.join(", "))
                }
            );
            if strict && !report.missing_list_commands.is_empty() {
                return Err(format!(
                    "command row has no list_command: {}",
                    report.missing_list_commands.join(", ")
                ));
            }
            Ok(true)
        }
        Action::Run {
            all,
            diff,
            shard,
            shard_weights,
            only,
            list,
            broad,
            narrow,
            allow_dirty,
            report,
        } => {
            if !list {
                prepare_optional_output(report.as_deref())?;
            }
            if !all && diff.is_none() && only.is_none() {
                return Err("select --all, --diff <base>, or --only <id>".into());
            }
            let catalogue = load(&catalogue_path)?;
            validate(&root, &catalogue)?;
            if only
                .as_ref()
                .is_some_and(|id| !catalogue.control.iter().any(|c| &c.id == id))
            {
                return Err("unknown --only id".into());
            }
            let selected = diff
                .map(|base| {
                    let relative = catalogue_path
                        .strip_prefix(&root)
                        .unwrap_or(&catalogue_path);
                    select_diff(
                        &root,
                        relative.to_str().ok_or("non-UTF-8 catalogue path")?,
                        &catalogue,
                        &base,
                    )
                })
                .transpose()?;
            let (index, count) = if let Some(shard) = shard {
                let (i, n) = shard.split_once('/').ok_or("shard must be i/n (1-based)")?;
                let i: usize = i.parse().map_err(|_| "invalid shard index")?;
                let n: usize = n.parse().map_err(|_| "invalid shard count")?;
                if i == 0 || n == 0 || i > n {
                    return Err("shard must satisfy 1 <= i <= n".into());
                }
                (i - 1, n)
            } else {
                (0, 1)
            };
            let mut controls: Vec<_> = catalogue
                .control
                .iter()
                .filter(|c| {
                    only.as_ref().is_none_or(|id| id == &c.id)
                        && selected.as_ref().is_none_or(|s| s.contains(&c.id))
                })
                .collect();
            controls.sort_by(|a, b| a.id.cmp(&b.id));
            let row_ids: Vec<_> = controls.iter().map(|control| control.id.clone()).collect();
            let (weights, missing_weights) = if let Some(path) = shard_weights.as_deref() {
                let report_weights = read_shard_weights(path)?;
                let (weights, missing) = weights_for_selection(&report_weights, &row_ids);
                (Some(weights), missing)
            } else {
                (None, 0)
            };
            let plan = shard_plan(&row_ids, count, weights.as_ref())?;
            let shard: Vec<&Control> = plan.rows[index].iter().map(|&row| controls[row]).collect();
            if list {
                for control in &shard {
                    println!("{}", control.id);
                }
                if weights.is_some() {
                    eprintln!("{}", weighted_summary(&plan, index, missing_weights));
                } else {
                    eprintln!(
                        "Listed {} selected row(s) for shard {}/{}",
                        shard.len(),
                        index + 1,
                        count
                    );
                }
                return Ok(true);
            }
            if weights.is_some() {
                eprintln!("{}", weighted_summary(&plan, index, missing_weights));
            }
            // Apply narrow selection only after row assignment so sharding is unchanged.
            // Both baselines and mutants must receive the same effective controls.
            let mut effective: Vec<Control> = shard.iter().map(|c| (*c).clone()).collect();
            let selections = narrow.then(|| {
                effective
                    .iter_mut()
                    .map(|c| (c.id.clone(), c.narrow()))
                    .collect::<BTreeMap<_, _>>()
            });
            let shard: Vec<&Control> = effective.iter().collect();
            if broad {
                for control in &shard {
                    if let Some(name) = &control.broad_report {
                        prepare_output_path(&root.join(name))?;
                    }
                }
            }
            // Rows execute grouped by package to save rebuilds, but each report
            // lands in its sorted-ID slot, so output order never depends on it.
            let mut slots: Vec<Option<Report>> = shard.iter().map(|_| None).collect();
            let mut session =
                ReplaySession::prepare(&root, &catalogue.prebuild, &shard, allow_dirty, &stop)?;
            let work = (|| -> Result<()> {
                if broad {
                    session.broad_baselines(&root, &shard, allow_dirty, &stop)?;
                } else {
                    session.baselines(&root, &shard, allow_dirty, &stop)?;
                }
                for i in execution_order(&shard) {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    slots[i] = Some(if broad {
                        session.run_broad_row(&root, shard[i], allow_dirty, &stop)?
                    } else {
                        session.run_row(&root, shard[i], allow_dirty, &stop, false)?
                    });
                }
                Ok(())
            })();
            let mut rows: Vec<Report> = slots.into_iter().flatten().collect();
            session.finish(&root, &mut rows, allow_dirty)?;
            work?;
            write_report(report, &rows, selections.as_ref())?;
            Ok(!stop.load(Ordering::SeqCst) && rows.iter().all(Report::passes))
        }
        Action::Prove(p) => {
            let selection = *p.selection;
            prepare_optional_output(p.report.as_deref())?;
            prepare_output_path(&control_destination(&catalogue_path, &p.id)?)?;
            let c = Control {
                source: control_source(&catalogue_path, &p.id)?,
                id: p.id,
                guards: p.guards,
                file: Some(p.file),
                old: Some(p.old),
                new: Some(p.new),
                scope: p.scope_after.map(|after| AnchorScope {
                    after,
                    before: p.scope_before,
                }),
                edits: vec![],
                test_file: p.test_file,
                runner: p.runner.unwrap_or_else(|| {
                    if p.command.is_some() {
                        "command"
                    } else {
                        "cargo"
                    }
                    .into()
                }),
                package: p.package,
                target: p.target,
                features: (!p.feature_selection.features.is_empty())
                    .then_some(p.feature_selection.features),
                no_default_features: p.feature_selection.no_default_features.then_some(true),
                all_features: p.feature_selection.all_features.then_some(true),
                ignored: ignored_selection(p.feature_selection.ignored.as_deref()),
                select: selection
                    .select
                    .as_deref()
                    .map(|_| TestSelection::Expected)
                    .or_else(|| selection.select_filter.map(TestSelection::Filter))
                    .or_else(|| selection.select_tests.map(TestSelection::Tests)),
                command: p.command,
                list_command: None,
                broad_command: None,
                broad_report: None,
                broad_id: None,
                test_count_pattern: p.test_count_pattern,
                catch_on: p.catch_on,
                output_normalize: p.output_normalize,
                expect_red: p.expect_red,
                expect_message: p.expect_message,
                signal_is_catch: p.signal_is_catch,
                only: p.only,
                equivalent: None,
                equivalent_guard: None,
                unreachable: None,
                desk_only: p.desk_only,
                hub: None,
                hub_targets: None,
                platforms: (!p.platforms.is_empty()).then_some(p.platforms),
                timeout_s: p.timeout_s,
                build_timeout_s: p.build_timeout_s,
            };
            let mut catalogue = if catalogue_path.exists() {
                load(&catalogue_path)?
            } else {
                Catalogue::default()
            };
            catalogue.control.push(c.clone());
            validate(&root, &catalogue)?;
            let mut session =
                ReplaySession::prepare(&root, &catalogue.prebuild, &[&c], p.allow_dirty, &stop)?;
            if c.select.is_some() {
                // A selected proof runs its chosen target and test set even if
                // the mutant survives. Replaying the whole package to diagnose
                // that survivor would still run this set, so skip that replay.
                session.baselines(&root, &[&c], p.allow_dirty, &stop)?;
            } else {
                session.package_baselines(&root, &[&c], p.allow_dirty, &stop)?;
            }
            let first = session.run_row(&root, &c, p.allow_dirty, &stop, false)?;
            let caught = first.outcome.is_caught();
            let recorded = caught || first.outcome == Outcome::DeskOnly;
            let mut rows = vec![first];
            let work = (|| -> Result<()> {
                if rows[0].outcome == Outcome::Survived && c.runner == "command" {
                    println!("An expected command test stayed green: inspect the guard or mutant equivalence. Command rows have no broader replay.");
                } else if rows[0].outcome == Outcome::Survived
                    && c.select.is_none()
                    && !stop.load(Ordering::SeqCst)
                {
                    let broader = session.run_row(&root, &c, p.allow_dirty, &stop, true)?;
                    if !broader.red.is_empty() {
                        println!("Unscoped package tests caught the mutant: the original command scope omitted covering tests. Update target and expect_red.");
                    } else if broader.outcome == Outcome::Survived {
                        println!("No package test caught this mutant: a real coverage gap or an equivalent mutant. Inspect semantics; use equivalent = <reason> only when justified.");
                    } else {
                        println!(
                            "Unscoped replay could not establish a cause: {:?}",
                            broader.outcome
                        );
                    }
                    rows.push(broader);
                }
                Ok(())
            })();
            session.finish(&root, &mut rows, p.allow_dirty)?;
            work?;
            if recorded && !stop.load(Ordering::SeqCst) {
                append_control(&catalogue_path, &c)?;
                if caught {
                    // The proved file is restored by now, so the hint reads its
                    // unmutated text. `prove` always takes exactly one edit.
                    let hint = c.edits()?.first().and_then(|edit| {
                        let text = fs::read_to_string(root.join(&edit.file)).ok()?;
                        call_site_hint(&text, edit)
                    });
                    if let Some(hint) = hint {
                        println!("{hint}");
                    }
                }
            }
            write_report(p.report, &rows, None)?;
            Ok(!stop.load(Ordering::SeqCst)
                && (recorded || rows[0].outcome == Outcome::SkippedPlatform))
        }
        Action::Explore(x) => explore(&root, &catalogue_path, x, &stop),
    }
}
fn explore(
    root: &std::path::Path,
    catalogue_path: &std::path::Path,
    x: Exploration,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<bool> {
    if x.runner == "command" {
        return Err("explore refuses command rows: only expect_red ids can be observed; use prove --command instead".into());
    }
    prepare_optional_output(x.report.as_deref())?;
    if x.append {
        prepare_output_path(&control_destination(
            catalogue_path,
            x.id.as_deref().unwrap_or("explore"),
        )?)?;
    }
    let edits = match x.edits {
        Some(text) => {
            let path = std::path::Path::new(&text);
            if !text.contains('\n') && path.is_file() {
                parse_edits(&fs::read_to_string(path).map_err(|e| e.to_string())?)?
            } else {
                parse_edits(&text)?
            }
        }
        None => vec![],
    };
    let mut c = Control {
        source: control_source(catalogue_path, x.id.as_deref().unwrap_or("explore"))?,
        id: x.id.unwrap_or_else(|| "explore".into()),
        guards: x.guards.unwrap_or_default(),
        file: x.file,
        old: x.old,
        new: x.new,
        scope: None,
        edits,
        test_file: x.test_file.unwrap_or_default(),
        runner: x.runner,
        package: Some(x.package),
        // An appended row replays the whole package, as explore ran it.
        target: None,
        features: (!x.feature_selection.features.is_empty())
            .then_some(x.feature_selection.features),
        no_default_features: x.feature_selection.no_default_features.then_some(true),
        all_features: x.feature_selection.all_features.then_some(true),
        ignored: ignored_selection(x.feature_selection.ignored.as_deref()),
        select: None,
        command: None,
        list_command: None,
        broad_command: None,
        broad_report: None,
        broad_id: None,
        test_count_pattern: None,
        catch_on: None,
        output_normalize: vec![],
        expect_message: None,
        signal_is_catch: None,
        expect_red: vec![],
        only: false,
        equivalent: None,
        equivalent_guard: None,
        unreachable: x.unreachable,
        desk_only: x.desk_only,
        hub: None,
        hub_targets: None,
        platforms: (!x.platforms.is_empty()).then_some(x.platforms),
        timeout_s: x.timeout_s,
        build_timeout_s: x.build_timeout_s,
    };
    validate_mutant(root, &c)?;
    let mut catalogue = if catalogue_path.exists() {
        load(catalogue_path)?
    } else {
        Catalogue::default()
    };
    if c.platforms.is_none() {
        c.platforms = infer_platforms(root, &c.edits()?);
    }
    if x.append {
        // Reject a bad id, guard text or test file before the (long) run. The
        // placeholder stands in for the red names, which only the run supplies.
        let mut candidate = catalogue.control.clone();
        candidate.push(Control {
            expect_red: vec!["explore-pending".into()],
            ..c.clone()
        });
        validate(
            root,
            &Catalogue {
                control: candidate,
                prebuild: catalogue.prebuild.clone(),
            },
        )?;
    }
    let mut session =
        ReplaySession::prepare(root, &catalogue.prebuild, &[&c], x.allow_dirty, stop)?;
    let row = session.explore_row(root, &c, x.allow_dirty, stop, x.workspace)?;
    let mut rows = vec![row];
    session.finish(root, &mut rows, x.allow_dirty)?;
    let row = rows.remove(0);
    let caught = row.outcome.is_caught();
    let recorded = caught || matches!(row.outcome, Outcome::Unreachable | Outcome::DeskOnly);
    let skipped = row.outcome == Outcome::SkippedPlatform;
    if caught {
        println!("Red tests ({}):", row.red.len());
        for name in &row.red {
            println!("  {name}");
        }
    } else if row.outcome == Outcome::Survived {
        println!(
            "{}",
            survivor_diagnosis(c.package.as_deref().unwrap_or_default(), x.workspace)
        );
    }
    let outcome = row.outcome.clone();
    let red = row.red.clone();
    write_report(x.report, &[row], None)?;
    if !x.append {
        return Ok(recorded || skipped);
    }
    if !recorded || stop.load(Ordering::SeqCst) {
        println!("--append: nothing appended; only caught proofs or reasoned UNREACHABLE/DESK_ONLY rows append (outcome {outcome:?})");
        return Ok(skipped);
    }
    c.expect_red = red;
    catalogue.control.push(c.clone());
    validate(root, &catalogue)?;
    // Preparation already ran in this session; name checking must not run it again.
    check(root, &Catalogue { control: vec![c.clone()], ..Catalogue::default() }, stop).map_err(|e| {
        format!("explore row failed check, nothing appended (with --workspace, red tests outside --package cannot be named in its row): {e}")
    })?;
    append_control(catalogue_path, &c)?;
    println!(
        "Appended row {} to {}",
        c.label(),
        control_destination(catalogue_path, &c.id)?.display()
    );
    Ok(true)
}
fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("ckdev-mutate: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn narrow_conflicts_with_broad_by_name() {
        let error = Cli::try_parse_from(["ckdev-mutate", "run", "--all", "--narrow", "--broad"])
            .err()
            .expect("narrow and broad must conflict")
            .to_string();
        assert!(error.contains("--narrow"), "{error}");
        assert!(error.contains("--broad"), "{error}");
        for selection in [
            vec!["--all"],
            vec!["--diff", "HEAD"],
            vec!["--only", "guard"],
        ] {
            let mut args = vec!["ckdev-mutate", "run", "--narrow"];
            args.extend(selection);
            assert!(Cli::try_parse_from(args).is_ok());
        }
    }

    // Mutation text is source code, which often starts with `--` (a flag in a
    // shell script, a SQL comment). It must parse as the value, not as an
    // unknown option.
    #[test]
    fn prove_expected_selection_conflicts_with_only() {
        let result = Cli::try_parse_from([
            "ckdev-mutate",
            "prove",
            "--id",
            "r",
            "--guards",
            "g",
            "--file",
            "f.rs",
            "--old",
            "old",
            "--new",
            "new",
            "--test-file",
            "t.rs",
            "--package",
            "p",
            "--expect-red",
            "test_name",
            "--select",
            "expected",
            "--only",
        ]);
        assert!(
            result.is_err(),
            "--select expected must be refused with --only"
        );
    }

    #[test]
    fn old_and_new_accept_values_starting_with_hyphens() {
        let cli = Cli::try_parse_from([
            "ckdev-mutate",
            "prove",
            "--id",
            "r",
            "--guards",
            "g",
            "--file",
            "f.sh",
            "--old",
            "--data-dir \"$d\"",
            "--new",
            "-- disabled",
            "--test-file",
            "t",
            "--package",
            "p",
            "--expect-red",
            "t1",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        let Action::Prove(p) = cli.command else {
            panic!("expected prove")
        };
        assert_eq!(p.old, "--data-dir \"$d\"");
        assert_eq!(p.new, "-- disabled");

        let cli = Cli::try_parse_from([
            "ckdev-mutate",
            "explore",
            "--package",
            "p",
            "--file",
            "f.sh",
            "--old",
            "--data-dir",
            "--new",
            "-x",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        let Action::Explore(x) = cli.command else {
            panic!("expected explore")
        };
        assert_eq!(x.old.as_deref(), Some("--data-dir"));
        assert_eq!(x.new.as_deref(), Some("-x"));
    }

    #[test]
    fn prove_parses_anchor_scope_options() {
        let cli = Cli::try_parse_from([
            "ckdev-mutate",
            "prove",
            "--id",
            "scoped",
            "--guards",
            "the guard is in this function",
            "--file",
            "guard.rs",
            "--old",
            "value > 0",
            "--new",
            "value >= 0",
            "--scope-after",
            "fn guarded(",
            "--scope-before",
            "\nfn ",
            "--test-file",
            "tests.rs",
            "--package",
            "fixture",
            "--expect-red",
            "guard_test",
        ])
        .unwrap_or_else(|e| panic!("{e}"));
        let Action::Prove(proof) = cli.command else {
            panic!("expected prove")
        };
        assert_eq!(proof.scope_after.as_deref(), Some("fn guarded("));
        assert_eq!(proof.scope_before.as_deref(), Some("\nfn "));

        let missing_after = Cli::try_parse_from([
            "ckdev-mutate",
            "prove",
            "--id",
            "scoped",
            "--guards",
            "g",
            "--file",
            "guard.rs",
            "--old",
            "old",
            "--new",
            "new",
            "--scope-before",
            "end",
            "--test-file",
            "tests.rs",
            "--package",
            "fixture",
            "--expect-red",
            "guard_test",
        ]);
        assert!(missing_after.is_err());
    }
}

#[cfg(test)]
mod shard_tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn weighted_shards_use_report_costs_and_cover_selection() {
        let row_ids = ids(&["alpha", "bravo", "charlie", "delta", "echo"]);
        let weights = BTreeMap::from([
            ("alpha".into(), 70),
            ("bravo".into(), 60),
            ("charlie".into(), 50),
            ("delta".into(), 40),
            ("echo".into(), 30),
        ]);
        let plan = shard_plan(&row_ids, 3, Some(&weights)).unwrap();
        assert_eq!(plan, shard_plan(&row_ids, 3, Some(&weights)).unwrap());
        let assigned: Vec<_> = plan
            .rows
            .iter()
            .flat_map(|shard| shard.iter().map(|&row| row_ids[row].as_str()))
            .collect();
        assert_eq!(assigned, ["alpha", "bravo", "echo", "charlie", "delta"]);
        assert_eq!(plan.loads, [70, 90, 90]);
        let all_rows: Vec<_> = plan.rows.iter().flatten().copied().collect();
        let unique_rows: std::collections::BTreeSet<_> = all_rows.iter().copied().collect();
        assert_eq!(unique_rows.len(), row_ids.len());
        assert_eq!(unique_rows, (0..row_ids.len()).collect());
    }

    #[test]
    fn weighted_ties_go_to_lowest_shard_index() {
        let row_ids = ids(&["alpha", "bravo", "charlie"]);
        let weights = BTreeMap::from([
            ("alpha".into(), 10),
            ("bravo".into(), 10),
            ("charlie".into(), 10),
        ]);
        let plan = shard_plan(&row_ids, 2, Some(&weights)).unwrap();
        assert_eq!(plan.rows, [vec![0, 2], vec![1]]);
        assert_eq!(plan.loads, [20, 10]);
    }

    #[test]
    fn lpt_balances_one_hundred_and_ten_ten_minute_rows_exactly() {
        let mut row_ids = vec!["00-heavy".to_owned()];
        row_ids.extend((1..=10).map(|i| format!("{i:02}-small")));
        let weights = row_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    if id == "00-heavy" { 6_000_000 } else { 600_000 },
                )
            })
            .collect();
        let plan = shard_plan(&row_ids, 2, Some(&weights)).unwrap();
        assert_eq!(plan.rows[0], [0]);
        assert_eq!(plan.rows[1], (1..=10).collect::<Vec<_>>());
        assert_eq!(plan.loads, [6_000_000, 6_000_000]);
    }

    #[test]
    fn missing_report_rows_use_median_or_one_for_an_empty_report() {
        let report = BTreeMap::from([("known-a".into(), 100), ("known-b".into(), 300)]);
        let (weights, missing) = weights_for_selection(
            &report,
            &ids(&["known-a", "unknown-a", "known-b", "unknown-b"]),
        );
        assert_eq!(missing, 2);
        assert_eq!(weights["unknown-a"], 200);
        assert_eq!(weights["unknown-b"], 200);

        let (weights, missing) = weights_for_selection(&BTreeMap::new(), &ids(&["unknown"]));
        assert_eq!(missing, 1);
        assert_eq!(weights["unknown"], 1);
    }

    #[test]
    fn unweighted_shards_keep_round_robin_for_fixed_sorted_catalogue() {
        let row_ids = ids(&["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]);
        let plan = shard_plan(&row_ids, 3, None).unwrap();
        assert_eq!(
            plan.rows,
            [vec![0, 3], vec![1, 4], vec![2, 5]],
            "the no-weight option preserves the fixed catalogue's old index sharding"
        );
        assert_eq!(plan.loads, [0, 0, 0]);
    }

    #[test]
    fn report_rows_supply_build_plus_test_costs() {
        let path = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            path.path(),
            r#"[{"id":"alpha","build_ms":125,"test_ms":75}]"#,
        )
        .unwrap();
        assert_eq!(read_shard_weights(path.path()).unwrap()["alpha"], 200);
    }
}
