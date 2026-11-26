use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use cortexkit_doctor::EvidenceKind::{Bool, Code, Count};
use cortexkit_doctor::{
    build_report, template_id, CheckOutput, Manifest, ReportContext, ReportInput,
};
use serde_json::{json, Value};

fn run(command: &mut Command) {
    let output = command.output().expect("start fixture command");
    assert!(
        output.status.success(),
        "{command:?} exited with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn matched(level: &str, template: String, text: &str, fields: Value) -> Value {
    json!({"age": "3 s before", "level": level, "template": template, "text": text, "fields": fields})
}

#[test]
fn log_manifest_join_matches_real_log_tail() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/join");
    let cargo_tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let tmp = tempfile::tempdir_in(cargo_tmp).unwrap();
    let out = tmp.path().join("doctor-manifest.json");
    let logs = tmp.path().join("logs");

    let generator = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortexkit-log-manifest"));
        command.current_dir(&fixture).arg("--out").arg(&out);
        command
    };
    run(generator().args([
        "--module-id",
        "join-fixture",
        "--repository",
        "https://github.com/example/join-fixture",
        "--log-glob",
        "logs/*.log",
    ]));
    run(generator().arg("--check"));
    let manifest = Manifest::decode(&fs::read(out).unwrap()).unwrap();

    let nested = "nested {braces}\\path";
    let shared = "shared event";
    let bin = "join_fixture_bin";
    let templates: BTreeMap<_, _> = manifest
        .templates()
        .iter()
        .map(|t| {
            (
                (t.target(), t.text()),
                t.fields()
                    .iter()
                    .map(|f| (f.name(), f.kind()))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        templates,
        BTreeMap::from([
            (
                ("join_fixture::nested::leaf", nested),
                vec![("n_bytes", Count)]
            ),
            (("join_fixture::inline", "inline event"), vec![("ok", Bool)]),
            ((bin, "bin event"), vec![]),
            ((bin, "check"), vec![("n_bytes", Count)]),
            (
                ("join.first", shared),
                vec![("n_bytes", Count), ("state", Code)]
            ),
            (
                ("join.second", shared),
                vec![("n_bytes", Count), ("ok", Bool)]
            ),
        ])
    );

    // Use a separate reusable build directory so the nested Cargo run does not
    // contend with the Cargo process running this integration test.
    run(Command::new(env!("CARGO"))
        .current_dir(&fixture)
        .args(["run", "--quiet", "--locked", "--offline", "--target-dir"])
        .arg(cargo_tmp.join("log-manifest-join-fixture"))
        .args(["--bin", "join-fixture-bin", "--"])
        .arg(&logs));

    let segments: Vec<_> = fs::read_dir(&logs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "log"))
        .collect();
    assert_eq!(segments.len(), 1, "fixture log segments: {segments:?}");
    let tail = fs::read_to_string(&segments[0]).unwrap();
    for raw in [
        " join-fixture.dep.join-dependency: dependency event n_bytes=17",
        " check n_bytes=2",
        " join-fixture.join.first: shared event n_bytes=5 state=ready",
        " join-fixture.join.second: shared event n_bytes=6 ok=true",
    ] {
        assert!(
            tail.lines().any(|line| line.ends_with(raw)),
            "missing {raw:?} in {tail}"
        );
    }

    let ctx = ReportContext::new(
        [7; 32],
        1_735_000_003_000,
        tmp.path(),
        tmp.path(),
        tmp.path(),
    );
    let check = CheckOutput::new(vec![]).unwrap();
    let tails = [tail.as_str()];
    let input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        "Join",
        "",
        Ok(&check),
        usize::MAX,
    )
    .with_log_tails(&tails);
    let report: Value = serde_json::from_slice(&build_report(&ctx, &input).unwrap()).unwrap();

    let shared_id = template_id("join.first", shared).min(template_id("join.second", shared));
    assert_eq!(report["omitted_lines"], 0);
    assert_eq!(report["dropped_values"], 0);
    let expected = [
        matched(
            "info",
            template_id("join_fixture::nested::leaf", nested),
            nested,
            json!({"n_bytes": 3}),
        ),
        matched(
            "info",
            template_id("join_fixture::inline", "inline event"),
            "inline event",
            json!({"ok": true}),
        ),
        matched(
            "info",
            template_id(bin, "bin event"),
            "bin event",
            json!({}),
        ),
        json!({"age": "3 s before", "level": "error", "text": "text omitted"}),
        json!({"age": "3 s before", "level": "warn", "text": "text omitted"}),
        matched(
            "info",
            template_id(bin, "check"),
            "check",
            json!({"n_bytes": 4}),
        ),
        // The formatted warning has no template of its own, but its rendered
        // text and field syntax match the literal check template.
        matched(
            "warn",
            template_id(bin, "check"),
            "check",
            json!({"n_bytes": 2}),
        ),
        matched("info", shared_id.clone(), shared, json!({"n_bytes": 5})),
        matched("info", shared_id, shared, json!({"n_bytes": 6})),
    ];
    assert_eq!(report["logs"], json!(expected));
}
