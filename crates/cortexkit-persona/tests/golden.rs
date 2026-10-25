//! The golden vectors in `test-vectors/persona-v1/cases/`: each case names
//! its tiers, approvals, context and supplied fragments, and pins the exact
//! rendered bytes and the full resolution report. The expected files are
//! written by `test-vectors/generate.py`, a second implementation of SPEC.md's
//! digest and report rules in Python, so a match is evidence for the rules,
//! not just for this crate agreeing with itself. See test-vectors/README.md
//! for how to rerun it.

use std::{
    collections::BTreeMap,
    fs,
    num::{NonZeroU64, NonZeroUsize},
};

use cortexkit_persona::{
    Approval, ApprovalSet, Digest, Extension, FragmentSources, LoadRequest, PersonaEngine,
    ProjectKey, RenderContext, RenderLimits, Rendered, Tier, TrustClass,
};
use serde::Deserialize;

const VECTORS: &str = "test-vectors/persona-v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    module: String,
    role: String,
    project_key: Option<String>,
    extensions: Vec<String>,
    tiers: Vec<CaseTier>,
    approvals: Vec<Approval>,
    context: String,
    supplied: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseTier {
    name: String,
    trust: TrustClass,
    root: String,
    roles_path: String,
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn render_case(name: &str) -> Rendered {
    render_case_with_limits(name, None)
}

fn render_case_with_limits(name: &str, limits: Option<RenderLimits>) -> Rendered {
    let case: Case = serde_json::from_str(&read(&format!("{VECTORS}/cases/{name}/case.json")))
        .expect("case.json parses");
    let mut engine = PersonaEngine::new(&case.module).unwrap();
    for key in &case.extensions {
        engine
            .register_extension(Extension::<serde_json::Value>::new(key))
            .unwrap();
    }
    let tiers: Vec<Tier> = case
        .tiers
        .iter()
        .map(|t| {
            // Joined with `/` rather than Path::join so the report's root is
            // the same string on every OS.
            Tier::directory(
                &t.name,
                t.trust,
                format!("{VECTORS}/{}", t.root),
                &t.roles_path,
            )
            .unwrap()
        })
        .collect();
    let approvals: ApprovalSet = case.approvals.into_iter().collect();
    let project_key = case.project_key.map(|k| ProjectKey::new(k).unwrap());
    let mut request = LoadRequest::new(&case.role, &tiers, &approvals);
    if let Some(key) = &project_key {
        request = request.project_key(key);
    }
    let role = engine.load(request).expect("the case loads");
    let context: RenderContext<serde_json::Value> =
        serde_json::from_str(&read(&format!("{VECTORS}/{}", case.context))).unwrap();
    let mut supplied = FragmentSources::new();
    for (id, path) in &case.supplied {
        supplied
            .insert(id, read(&format!("{VECTORS}/{path}")))
            .unwrap();
    }
    match limits {
        Some(limits) => role.render_with_limits(&context, &supplied, limits),
        None => role.render(&context, &supplied),
    }
    .expect("the case renders")
}

fn case_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(format!("{VECTORS}/cases"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn every_golden_case_renders_its_expected_bytes_digest_and_report() {
    let names = case_names();
    // Vacuity floor: the five shipped cases must all be found and run.
    assert!(names.len() >= 5, "only {} golden cases found", names.len());
    for name in names {
        let out = render_case(&name);
        let dir = format!("{VECTORS}/cases/{name}");
        let expected = fs::read(format!("{dir}/expected.txt")).unwrap();
        assert_eq!(
            out.text.as_bytes(),
            expected.as_slice(),
            "case {name}: rendered bytes differ:\n{}",
            out.text
        );
        assert_eq!(out.digest, Digest::of(&expected), "case {name}: digest");
        let expected_report: serde_json::Value =
            serde_json::from_str(&read(&format!("{dir}/expected-report.json"))).unwrap();
        let actual_report = serde_json::to_value(&out.report).unwrap();
        assert_eq!(
            actual_report,
            expected_report,
            "case {name}: report differs:\n{}",
            serde_json::to_string_pretty(&out.report).unwrap()
        );
    }
}

#[test]
fn render_limits_preserve_every_golden_bytes_digest_and_report() {
    let names = case_names();
    assert!(names.len() >= 5, "only {} golden cases found", names.len());
    let limits = RenderLimits::default()
        .with_recursion(NonZeroUsize::new(200).unwrap())
        .with_fuel(NonZeroU64::new(100_000).unwrap());
    for name in names {
        let default = render_case(&name);
        let configured = render_case_with_limits(&name, Some(limits));
        assert_eq!(configured.bytes(), default.bytes(), "case {name}: bytes");
        assert_eq!(configured.digest, default.digest, "case {name}: digest");
        assert_eq!(configured.report, default.report, "case {name}: report");
    }
}

#[test]
fn crlf_and_lf_golden_sources_give_the_same_bytes_and_digests() {
    let lf = render_case("echo-lf");
    let crlf = render_case("echo-crlf");
    assert_eq!(lf.text, crlf.text);
    assert_eq!(lf.digest, crlf.digest);
    assert_eq!(lf.report.source_revision, crlf.report.source_revision);
    assert_eq!(
        lf.report.tiers[0].layer_digest,
        crlf.report.tiers[0].layer_digest
    );
    assert_eq!(lf.report.fragments, crlf.report.fragments);
    // The fixture really is CRLF on disk; otherwise this test proves nothing.
    let raw = fs::read(format!(
        "{VECTORS}/echo-crlf/tiers/base/roles/echo/PERSONA.md"
    ))
    .unwrap();
    assert!(raw.windows(2).any(|w| w == b"\r\n"));
}
