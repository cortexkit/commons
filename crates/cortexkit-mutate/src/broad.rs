//! Build a broad selection once, then give each test target its own deadline.

use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct TargetReport {
    pub target: String,
    pub outcome: String,
    pub red: Vec<String>,
    pub green: Vec<String>,
    pub ms: u128,
}

pub(super) struct Run {
    pub output: Output,
    pub results: Option<TestResults>,
}

fn enabled(c: &Control, scope: Scope) -> bool {
    scope == Scope::Broad && c.select.is_none()
}

pub(super) fn build(root: &Path, c: &Control, scope: Scope, stop: &AtomicBool) -> Result<Output> {
    let mut cmd = command(c, "build", scope)?;
    if enabled(c, scope) {
        if c.runner == "cargo" {
            cmd.arg("--message-format=json");
        } else {
            cmd = command(c, "list", scope)?;
            cmd.args(["--list-type", "binaries-only"]);
        }
        let start = Instant::now();
        let mut output = execute_output(root, cmd, c.build_timeout_s, stop, true)?;
        if c.runner == "nextest" && output.success {
            // Package metadata belongs to build preparation, not a target's
            // test deadline. Include it in the same build budget and timing.
            let mut metadata_command = Command::new("cargo");
            metadata_command.args(["metadata", "--locked", "--no-deps", "--format-version", "1"]);
            let mut metadata = execute_output(
                root,
                metadata_command,
                c.build_timeout_s.saturating_sub(start.elapsed().as_secs()),
                stop,
                true,
            )?;
            if !metadata.success {
                metadata.ms += output.ms;
                return Ok(metadata);
            }
            output.ms += metadata.ms;
            output.cargo_metadata = Some(metadata.stdout);
        }
        Ok(output)
    } else {
        execute(root, cmd, c.build_timeout_s, stop)
    }
}

struct Target {
    id: String,
    selector: Vec<String>,
}

fn field<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("broad target metadata missing {key}"))
}

fn cargo_targets(c: &Control, build: &Output) -> Result<Vec<Target>> {
    let mut targets = BTreeMap::new();
    for line in build.stdout.lines().filter(|line| line.starts_with('{')) {
        let artifact: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if artifact["reason"] != "compiler-artifact" || artifact["executable"].is_null() {
            continue;
        }
        let target = &artifact["target"];
        let kinds = target["kind"]
            .as_array()
            .ok_or("test artifact missing target kind")?;
        let kind = kinds
            .first()
            .and_then(|kind| kind.as_str())
            .ok_or("empty target kind")?;
        if kind == "custom-build" {
            continue;
        }
        // `profile.test` is false for custom harnesses. Use Cargo's target
        // selection too, without mistaking helper examples/binaries for tests.
        let selected = artifact["profile"]["test"] == true
            || kind == "test"
            || kind == "bench"
            || target["test"] == true
            || explicitly_selected(c, kind, field(target, "name")?)?;
        if !selected {
            continue;
        }
        let executable = PathBuf::from(field(&artifact, "executable")?);
        let binary = executable
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("broad test executable has no UTF-8 filename")?;
        let selector = target_selector(target)?;
        targets.insert(
            selector.clone(),
            Target {
                id: stable_target("cargo", binary).to_owned(),
                selector,
            },
        );
    }
    if targets.is_empty() {
        return Err("broad build produced no test executables".into());
    }
    Ok(targets.into_values().collect())
}

fn explicitly_selected(c: &Control, kind: &str, name: &str) -> Result<bool> {
    let targets = c.targets()?;
    if targets.contains(&"--all-targets")
        || (kind == "example" && targets.contains(&"--examples"))
        || (kind == "bin" && targets.contains(&"--bins"))
    {
        return Ok(true);
    }
    for pair in targets.windows(2) {
        if pair[0] != format!("--{kind}") {
            continue;
        }
        let mut pattern = String::from("^");
        let mut in_class = false;
        for ch in pair[1].chars() {
            match ch {
                '*' if !in_class => pattern.push_str(".*"),
                '?' if !in_class => pattern.push('.'),
                '[' => {
                    in_class = true;
                    pattern.push(ch);
                }
                ']' => {
                    in_class = false;
                    pattern.push(ch);
                }
                '!' if pattern.ends_with('[') => pattern.push('^'),
                '-' if in_class => pattern.push(ch),
                _ => pattern.push_str(&regex::escape(&ch.to_string())),
            }
        }
        pattern.push('$');
        if regex::Regex::new(&pattern)
            .map_err(|e| e.to_string())?
            .is_match(name)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn target_selector(target: &serde_json::Value) -> Result<Vec<String>> {
    let kinds = target["kind"]
        .as_array()
        .ok_or("test artifact missing target kind")?;
    let kind = kinds
        .first()
        .and_then(|kind| kind.as_str())
        .ok_or("empty target kind")?;
    let flag = match kind {
        "test" => "--test",
        "bench" => "--bench",
        "bin" => "--bin",
        "example" => "--example",
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => {
            return Ok(vec!["--lib".into()])
        }
        _ => return Err(format!("unsupported broad test target kind: {kind}")),
    };
    Ok(vec![flag.into(), field(target, "name")?.into()])
}

fn merge(events: &mut TestEvents, results: TestResults) -> Result<()> {
    for (identity, target) in results.targets {
        let name = &results.names[&identity];
        events.record(&target, name, results.red.contains(&identity))?;
        let event = events.0.get_mut(&(target, name.clone())).unwrap();
        event.output = results
            .failure_output
            .get(&identity)
            .cloned()
            .unwrap_or_default();
        event.signal = results.signals.get(&identity).cloned();
    }
    Ok(())
}

fn rebuilt_artifact(text: &str) -> Option<String> {
    text.lines()
        // After Cargo starts the harness, JSON can be ordinary test output.
        .take_while(|line| !line.trim_start().starts_with("Running "))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|message| message["reason"] == "compiler-artifact" && message["fresh"] == false)
        .map(|message| {
            message["target"]["name"]
                .as_str()
                .unwrap_or("<unknown>")
                .to_owned()
        })
}

fn check_freshness(target: &str, text: &str) -> Result<()> {
    match rebuilt_artifact(text) {
        Some(artifact) => Err(format!(
            "{target}: Cargo rebuilt a target after the broad build ({artifact})"
        )),
        None => Ok(()),
    }
}

pub(super) fn run(
    root: &Path,
    c: &Control,
    scope: Scope,
    build: &Output,
    stop: &AtomicBool,
    reports: &mut Vec<TargetReport>,
) -> Result<Run> {
    if !enabled(c, scope) {
        return Ok(Run {
            output: execute(root, command(c, "run", scope)?, c.timeout_s, stop)?,
            results: None,
        });
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let cargo_metadata = temp.path().join("cargo.json");
    let binaries_metadata = temp.path().join("binaries.json");
    let targets = if c.runner == "cargo" {
        cargo_targets(c, build)?
    } else {
        fs::write(
            &cargo_metadata,
            build
                .cargo_metadata
                .as_ref()
                .ok_or("nextest build missing Cargo metadata")?,
        )
        .map_err(|e| e.to_string())?;
        fs::write(&binaries_metadata, &build.stdout).map_err(|e| e.to_string())?;
        let binaries: serde_json::Value =
            serde_json::from_str(&build.stdout).map_err(|e| e.to_string())?;
        binaries["rust-binaries"]
            .as_object()
            .ok_or("nextest binary metadata missing rust-binaries")?
            .keys()
            .map(|id| Target {
                id: id.clone(),
                selector: vec![],
            })
            .collect()
    };
    if targets.is_empty() {
        return Err("broad build produced no test targets".into());
    }
    let mut events = TestEvents::default();
    let mut text = String::new();
    let mut ms = 0;
    let mut success = true;
    let mut last = None;
    let mut failure = None;
    for target in targets {
        let cmd = if c.runner == "cargo" {
            let mut selected = c.clone();
            selected.target = Some(target.selector.join(" "));
            // Cargo launches the already-built executable with its package cwd,
            // library paths and configured runner. JSON freshness is checked below.
            let original = command(&selected, "run", Scope::Row)?;
            let args: Vec<_> = original.get_args().collect();
            let harness = args.iter().position(|arg| *arg == "--").unwrap();
            let mut cmd = Command::new("cargo");
            cmd.args(&args[..harness])
                .arg("--message-format=json")
                .args(&args[harness..]);
            cmd
        } else {
            // Reuse nextest's binary list rather than asking it to build again.
            // Its runner still owns per-test configuration, environment and signals.
            let original = command(c, "run", scope)?;
            let args: Vec<_> = original.get_args().collect();
            let options = args
                .iter()
                .position(|arg| *arg == "--no-fail-fast")
                .unwrap();
            let mut cmd = Command::new("cargo");
            cmd.args(["nextest", "run", "--binaries-metadata"])
                .arg(&binaries_metadata)
                .arg("--cargo-metadata")
                .arg(&cargo_metadata)
                .args(&args[options..])
                .args(["--no-tests", "pass"])
                .args(["-E", &format!("binary_id(={})", escape_filter(&target.id))]);
            cmd.envs(
                original
                    .get_envs()
                    .filter_map(|(name, value)| value.map(|value| (name, value))),
            );
            cmd
        };
        let mut output = execute(root, cmd, c.timeout_s, stop)?;
        if c.runner == "cargo" {
            check_freshness(&target.id, &output.text)?;
        }
        // The header also identifies a nextest timeout, whose output need not
        // contain a Cargo-style `Running` line.
        if output.timeout && c.runner != "cargo" {
            output
                .text
                .push_str(&format!("\nRunning {} ({})\n", target.id, target.id));
        }
        let results = if output.timeout || output.interrupted {
            None
        } else {
            Some(
                parse_test_results(&output.text, &c.runner)
                    .map_err(|e| format!("{}: {e}", target.id))?,
            )
        };
        reports.push(TargetReport {
            target: target.id,
            outcome: if output.timeout {
                "TIMED_OUT"
            } else if output.interrupted {
                "INTERRUPTED"
            } else if output.success {
                "PASSED"
            } else {
                "FAILED"
            }
            .into(),
            red: results
                .as_ref()
                .map(|results| results.red.clone())
                .unwrap_or_default(),
            green: results
                .as_ref()
                .map(|results| results.green.clone())
                .unwrap_or_default(),
            ms: output.ms,
        });
        if let Some(results) = results {
            merge(&mut events, results)?;
        }
        text.push_str(&output.text);
        text.push('\n');
        ms += output.ms;
        success &= output.success;
        let done = output.timeout || output.interrupted;
        if !output.success && failure.is_none() {
            failure = Some(output.clone());
        }
        last = Some(output);
        if done {
            break;
        }
    }
    let mut output = last.unwrap();
    if !output.timeout && !output.interrupted {
        output = failure.unwrap_or(output);
    }
    output.stdout = text.clone();
    output.stderr = None;
    output.text = text;
    output.ms = ms;
    output.success = success;
    if !success && !output.timeout && !output.interrupted {
        output.code = Some(1);
    }
    Ok(Run {
        output,
        results: Some(events.finish(&c.runner)),
    })
}

fn escape_filter(name: &str) -> String {
    name.replace('\\', "\\\\")
        .replace('/', "\\/")
        .replace(')', "\\)")
        .replace(',', "\\,")
}

#[cfg(test)]
mod freshness_tests {
    use super::{check_freshness, rebuilt_artifact};

    #[test]
    fn identifies_rebuilt_artifact_before_running() {
        let text = concat!(
            "    Finished `test` profile\n",
            "{\"reason\":\"compiler-artifact\",\"fresh\":false,\"target\":{\"name\":\"rebuilt-dependency\"}}\n",
            "     Running tests/guard.rs (guard-0123456789abcdef)\n",
        );
        assert_eq!(
            rebuilt_artifact(text).as_deref(),
            Some("rebuilt-dependency")
        );
        assert_eq!(
            check_freshness("guard", text).unwrap_err(),
            "guard: Cargo rebuilt a target after the broad build (rebuilt-dependency)"
        );
    }

    #[test]
    fn fresh_artifacts_are_not_rebuilds() {
        let text = concat!(
            "{\"reason\":\"compiler-artifact\",\"fresh\":true,\"target\":{\"name\":\"guard\"}}\n",
            "{\"reason\":\"build-script-executed\",\"fresh\":false}\n",
            "     Running tests/guard.rs (guard-0123456789abcdef)\n",
        );
        assert_eq!(rebuilt_artifact(text), None);
        assert!(check_freshness("guard", text).is_ok());
    }

    #[test]
    fn test_output_after_running_is_never_a_rebuild() {
        let text = concat!(
            "{\"reason\":\"compiler-artifact\",\"fresh\":true,\"target\":{\"name\":\"guard\"}}\n",
            "     Running tests/guard.rs (guard-0123456789abcdef)\n",
            "{\"reason\":\"compiler-artifact\",\"fresh\":false,\"target\":{\"name\":\"test-output\"}}\n",
        );
        assert_eq!(rebuilt_artifact(text), None);
        assert!(check_freshness("guard", text).is_ok());
    }
}
