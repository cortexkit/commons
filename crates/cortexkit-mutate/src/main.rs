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
    /// Run every test in the workspace instead of only the package.
    #[arg(long)]
    workspace: bool,
    #[arg(long, required_unless_present = "edits", conflicts_with = "edits", requires_all = ["old", "new"])]
    file: Option<String>,
    #[arg(long, requires = "file")]
    old: Option<String>,
    #[arg(long, requires = "file")]
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
    /// On CAUGHT only, append a row naming the red tests as expect_red.
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
    #[arg(long)]
    id: String,
    #[arg(long)]
    guards: String,
    #[arg(long)]
    file: String,
    #[arg(long)]
    old: String,
    #[arg(long)]
    new: String,
    #[arg(long)]
    test_file: String,
    #[arg(long, default_value = "cargo")]
    runner: String,
    #[arg(long)]
    package: String,
    #[arg(long, default_value = "", allow_hyphen_values = true)]
    target: String,
    #[arg(long, required = true, num_args = 1..)]
    expect_red: Vec<String>,
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
            "{}: {}{}",
            row.id,
            serde_json::to_value(&row.outcome)
                .map_err(|e| e.to_string())?
                .as_str()
                .ok_or("invalid outcome")?,
            row.reason
                .as_ref()
                .map_or(String::new(), |r| format!(" ({r})"))
        );
    }
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
            for i in execution_order(&shard) {
                slots[i] = Some(run_row(&root, shard[i], allow_dirty, &stop, false)?);
                if stop.load(Ordering::SeqCst) {
                    break;
                }
            }
            let rows: Vec<Report> = slots.into_iter().flatten().collect();
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
                runner: p.runner,
                package: p.package,
                target: p.target,
                expect_red: p.expect_red,
                only: p.only,
                equivalent: None,
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
            let first = run_row(&root, &c, p.allow_dirty, &stop, false)?;
            let caught = first.outcome == Outcome::Caught;
            let mut rows = vec![first];
            if caught {
                append_control(&cli.catalogue, &c)?;
                // The proved file is restored by now, so the hint reads its
                // unmutated text. `prove` always takes exactly one edit.
                let hint = c.edits()?.first().and_then(|edit| {
                    let text = fs::read_to_string(root.join(&edit.file)).ok()?;
                    call_site_hint(&text, edit)
                });
                if let Some(hint) = hint {
                    println!("{hint}");
                }
            } else if rows[0].outcome == Outcome::Survived && !stop.load(Ordering::SeqCst) {
                let broader = run_row(&root, &c, p.allow_dirty, &stop, true)?;
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
            write_report(p.report, &rows)?;
            Ok(caught)
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
        package: x.package,
        // An appended row replays the whole package, as explore ran it.
        target: String::new(),
        expect_red: vec![],
        only: false,
        equivalent: None,
        timeout_s: x.timeout_s,
        build_timeout_s: x.build_timeout_s,
    };
    validate_mutant(root, &c)?;
    let mut catalogue = Catalogue::default();
    if x.append {
        if catalogue_path.exists() {
            catalogue = load(catalogue_path)?;
        }
        // Reject a bad id, guard text or test file before the (long) run. The
        // placeholder stands in for the red names, which only the run supplies.
        let mut candidate = catalogue.control.clone();
        candidate.push(Control {
            expect_red: vec!["explore-pending".into()],
            ..c.clone()
        });
        validate(root, &Catalogue { control: candidate })?;
    }
    let row = explore_row(root, &c, x.allow_dirty, stop, x.workspace)?;
    let caught = row.outcome == Outcome::Caught;
    if caught {
        println!("Red tests ({}):", row.red.len());
        for name in &row.red {
            println!("  {name}");
        }
    } else if row.outcome == Outcome::Survived {
        println!("{}", survivor_diagnosis(&c.package, x.workspace));
    }
    let outcome = row.outcome.clone();
    let red = row.red.clone();
    write_report(x.report, &[row])?;
    if !x.append {
        return Ok(caught);
    }
    if !caught || stop.load(Ordering::SeqCst) {
        println!("--append: nothing appended; only CAUGHT appends (outcome {outcome:?})");
        return Ok(false);
    }
    c.expect_red = red;
    catalogue.control.push(c.clone());
    validate(root, &catalogue)?;
    check(root, &Catalogue { control: vec![c.clone()] }, stop).map_err(|e| {
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
