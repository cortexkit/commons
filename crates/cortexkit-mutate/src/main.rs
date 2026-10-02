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
            let mut rows = Vec::new();
            for (i, c) in controls.into_iter().enumerate() {
                if i % count != index {
                    continue;
                }
                rows.push(run_row(&root, c, allow_dirty, &stop, false)?);
                if stop.load(Ordering::SeqCst) {
                    break;
                }
            }
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
                let row =
                    toml::to_string(&Catalogue { control: vec![c] }).map_err(|e| e.to_string())?;
                use std::io::Write;
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(cli.catalogue)
                    .map_err(|e| e.to_string())?;
                writeln!(file, "\n{row}").map_err(|e| e.to_string())?;
                println!("A mutation inside a function proves nothing about callers reaching it. Add a second row removing its call site.");
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
    }
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
