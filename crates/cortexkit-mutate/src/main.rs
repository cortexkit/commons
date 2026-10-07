#![forbid(unsafe_code)]

use clap::{Args, Parser, Subcommand};
use cortexkit_mutate::*;
use std::{fs, path::PathBuf, sync::atomic::Ordering};

#[derive(Parser)]
#[command(
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
    Check,
    /// Replay selected catalogue rows.
    Run {
        #[arg(long, conflicts_with = "diff")]
        all: bool,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long)]
        shard: Option<String>,
        #[arg(long)]
        only: Option<String>,
        /// Expensive audit: run every package test target and grade broad catches.
        #[arg(long)]
        broad: bool,
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
struct Exploration {
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
struct Proof {
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
    #[arg(long)]
    test_file: String,
    #[arg(long)]
    runner: Option<String>,
    #[arg(long, required_unless_present = "command", conflicts_with = "command")]
    package: Option<String>,
    #[arg(long, allow_hyphen_values = true, conflicts_with = "command")]
    target: Option<String>,
    /// Command argv with one {test} element. Put this option last: all following
    /// values (including flags) belong to the command, not to ck-mutate.
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
fn write_report(path: Option<PathBuf>, rows: &[Report]) -> Result<()> {
    for row in rows {
        println!(
            "{}: {}{}{}{}",
            row.id,
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
                row.id,
                row.reason.as_deref().unwrap_or_default()
            );
        }
    }
    if !hubs.is_empty() {
        println!("HUB rows ({}):", hubs.len());
        for row in hubs {
            println!(
                "  {}: {}",
                row.id,
                row.reason.as_deref().unwrap_or_default()
            );
        }
    }
    if !desk.is_empty() {
        println!("DESK_ONLY rows ({}):", desk.len());
        for row in desk {
            println!(
                "  {}: {}",
                row.id,
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
        fs::write(
            path,
            serde_json::to_vec_pretty(rows).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn run() -> Result<bool> {
    let cli = Cli::parse();
    let root = repository()?;
    std::env::set_current_dir(&root).map_err(|e| e.to_string())?;
    let _lock = TreeLock::acquire(&root)?;
    let stop = signal_flag()?;
    match cli.command {
        Action::Check => {
            check(&root, &load(&cli.catalogue)?, &stop)?;
            println!("Catalogue anchors and exact test names verified");
            Ok(true)
        }
        Action::Run {
            all,
            diff,
            shard,
            only,
            broad,
            allow_dirty,
            report,
        } => {
            if !all && diff.is_none() && only.is_none() {
                return Err("select --all, --diff <base>, or --only <id>".into());
            }
            let catalogue = load(&cli.catalogue)?;
            validate(&root, &catalogue)?;
            if only
                .as_ref()
                .is_some_and(|id| !catalogue.control.iter().any(|c| &c.id == id))
            {
                return Err("unknown --only id".into());
            }
            let selected = diff
                .map(|base| {
                    let relative = cli.catalogue.strip_prefix(&root).unwrap_or(&cli.catalogue);
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
            let shard: Vec<&Control> = controls
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i % count == index)
                .map(|(_, c)| c)
                .collect();
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
            write_report(report, &rows)?;
            Ok(!stop.load(Ordering::SeqCst) && rows.iter().all(Report::passes))
        }
        Action::Prove(p) => {
            let c = Control {
                id: p.id,
                guards: p.guards,
                file: Some(p.file),
                old: Some(p.old),
                new: Some(p.new),
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
                command: p.command,
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
            let mut catalogue = if cli.catalogue.exists() {
                load(&cli.catalogue)?
            } else {
                Catalogue::default()
            };
            catalogue.control.push(c.clone());
            validate(&root, &catalogue)?;
            let mut session =
                ReplaySession::prepare(&root, &catalogue.prebuild, &[&c], p.allow_dirty, &stop)?;
            session.package_baselines(&root, &[&c], p.allow_dirty, &stop)?;
            let first = session.run_row(&root, &c, p.allow_dirty, &stop, false)?;
            let caught = first.outcome.is_caught();
            let recorded = caught || first.outcome == Outcome::DeskOnly;
            let mut rows = vec![first];
            let work = (|| -> Result<()> {
                if rows[0].outcome == Outcome::Survived && c.runner == "command" {
                    println!("An expected command test stayed green: inspect the guard or mutant equivalence. Command rows have no broader replay.");
                } else if rows[0].outcome == Outcome::Survived && !stop.load(Ordering::SeqCst) {
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
                append_control(&cli.catalogue, &c)?;
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
            write_report(p.report, &rows)?;
            Ok(!stop.load(Ordering::SeqCst)
                && (recorded || rows[0].outcome == Outcome::SkippedPlatform))
        }
        Action::Explore(x) => explore(&root, &cli.catalogue, x, &stop),
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
        id: x.id.unwrap_or_else(|| "explore".into()),
        guards: x.guards.unwrap_or_default(),
        file: x.file,
        old: x.old,
        new: x.new,
        edits,
        test_file: x.test_file.unwrap_or_default(),
        runner: x.runner,
        package: Some(x.package),
        // An appended row replays the whole package, as explore ran it.
        target: None,
        command: None,
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
    write_report(x.report, &[row])?;
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
    println!("Appended row {} to {}", c.id, catalogue_path.display());
    Ok(true)
}
fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("ck-mutate: {e}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    // Mutation text is source code, which often starts with `--` (a flag in a
    // shell script, a SQL comment). It must parse as the value, not as an
    // unknown option.
    #[test]
    fn old_and_new_accept_values_starting_with_hyphens() {
        let cli = Cli::try_parse_from([
            "ck-mutate",
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
            "ck-mutate",
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
}
