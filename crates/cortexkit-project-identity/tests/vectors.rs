use cortexkit_project_identity::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn vector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/project-resolve-v1")
}

#[derive(Deserialize)]
struct Record<'a> {
    #[serde(borrow)]
    request: &'a RawValue,
    #[serde(borrow)]
    reply: &'a RawValue,
}

fn records(file: &str) -> String {
    std::fs::read_to_string(vector_dir().join(file)).unwrap()
}

fn check_reply<T: DeserializeOwned + Serialize>(raw: &RawValue) -> Outcome<T> {
    let reply: Outcome<T> = serde_json::from_str(raw.get()).unwrap();
    // The captures use compact, sorted object keys; struct declaration order is
    // not part of the wire contract. Value supplies that sorted-key encoding.
    let sorted = serde_json::to_value(&reply).unwrap();
    assert_eq!(serde_json::to_string(&sorted).unwrap(), raw.get());
    reply
}

fn resolve_record(index: usize, via: Via, gone: bool) -> ResolveReply {
    let bytes = records("resolve.json");
    let rows: Vec<Record<'_>> = serde_json::from_str(&bytes).unwrap();
    assert_eq!(rows.len(), 6);
    let request: Request<ResolveParams> = serde_json::from_str(rows[index].request.get()).unwrap();
    assert_eq!(request.method, "resolve");
    assert!(request.params.execution_binding.is_none());
    let Outcome::Success(reply) = check_reply::<ResolveReply>(rows[index].reply) else {
        panic!("expected success");
    };
    assert_eq!(reply.result.via, via);
    assert_eq!(reply.result.gone, gone);
    assert_eq!(reply.result.generation, 4);
    assert_eq!(reply.result.incarnation, "0123456789abcdef");
    assert_eq!(
        reply.result.canonical_root,
        Some(request.params.canonical_root)
    );
    reply.result
}

#[test]
fn sample_resolve_root() {
    let reply = resolve_record(0, Via::Root, false);
    assert_eq!(reply.project_id, "pj-resolve-root");
    assert_eq!(
        reply.matched_root,
        Presence::new(Some("/fixture/root-checkout".into()))
    );
    assert_eq!(reply.source_root, Presence::new(None));
    assert_eq!(reply.binding_status, Presence::new(None));
    let Presence::Present(roots) = reply.root_records else {
        panic!("missing roots")
    };
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].identity, RootIdentity::Bound);
    assert_eq!(roots[0].approval.state, ApprovalState::Unapproved);
    assert_eq!(roots[0].approval.journal_seq, None);
    assert_eq!(
        roots[0].incarnation.as_ref().unwrap().kind,
        RootIncarnationKind::Token
    );
}

#[test]
fn sample_resolve_walk() {
    let reply = resolve_record(1, Via::Walk, false);
    assert_eq!(
        reply.matched_root,
        Presence::new(Some("/fixture/root-checkout".into()))
    );
}

#[test]
fn sample_resolve_worktree_omits_root_fields() {
    let reply = resolve_record(2, Via::Worktree, false);
    assert_eq!(reply.worktree_of.as_deref(), Some("/fixture/root-checkout"));
    assert_eq!(reply.matched_root, Presence::Missing);
    assert_eq!(reply.source_root, Presence::Missing);
    assert_eq!(reply.root_records, Presence::Missing);
    assert_eq!(reply.binding_status, Presence::Missing);
}

#[test]
fn sample_resolve_implicit() {
    let reply = resolve_record(3, Via::Implicit, false);
    assert_eq!(reply.project_id, "pj-implicit1-30f9d5fcd6aa4ef2");
    assert_eq!(reply.workspace_id, None);
    assert_eq!(reply.project_name, None);
    assert_eq!(reply.matched_root, Presence::new(None));
    assert_eq!(reply.root_records, Presence::new(vec![]));
}

#[test]
fn sample_resolve_gone_path_is_still_a_root() {
    let reply = resolve_record(4, Via::Root, true);
    let Presence::Present(roots) = reply.root_records else {
        panic!("missing roots")
    };
    assert_eq!(roots[0].identity, RootIdentity::Unverifiable);
    assert_eq!(roots[0].incarnation, None);
    assert_eq!(roots[0].registration_epoch, None);
}

#[test]
fn sample_resolve_invalid_params() {
    let bytes = records("resolve.json");
    let rows: Vec<Record<'_>> = serde_json::from_str(&bytes).unwrap();
    assert!(serde_json::from_str::<Request<ResolveParams>>(rows[5].request.get()).is_err());
    let Outcome::Refusal(reply) = check_reply::<ResolveReply>(rows[5].reply) else {
        panic!("expected refusal");
    };
    assert_eq!(reply.code, RefusalCode::InvalidParams);
    assert_eq!(reply.message, "missing field `canonicalRoot`");
    assert_eq!(reply.detail, None);
}

fn id_record(index: usize, via: Via, project_id: &str, gone: bool) {
    let bytes = records("resolve-project-id.json");
    let rows: Vec<Record<'_>> = serde_json::from_str(&bytes).unwrap();
    assert_eq!(rows.len(), 3);
    let request: Request<ResolveProjectIdParams> =
        serde_json::from_str(rows[index].request.get()).unwrap();
    assert_eq!(request.method, "resolve_project_id");
    let Outcome::Success(reply) = check_reply::<ResolveProjectIdReply>(rows[index].reply) else {
        panic!("expected success");
    };
    assert_eq!(reply.result.via, via);
    assert_eq!(reply.result.project_id, project_id);
    assert_eq!(reply.result.gone, gone);
    assert_eq!(reply.result.generation, 4);
    assert_eq!(reply.result.incarnation, "0123456789abcdef");
}

#[test]
fn sample_id_current() {
    id_record(0, Via::Current, "pj-resolve-root", false);
}
#[test]
fn sample_id_alias() {
    id_record(1, Via::Alias, "pj-alias-target", false);
}
#[test]
fn sample_id_gone() {
    id_record(2, Via::Gone, "pj-no-such-project", true);
}

#[test]
fn sample_enumerate() {
    let bytes = records("enumerate.json");
    let rows: Vec<Record<'_>> = serde_json::from_str(&bytes).unwrap();
    assert_eq!(rows.len(), 1);
    let request: Request<EnumerateParams> = serde_json::from_str(rows[0].request.get()).unwrap();
    assert_eq!(request.method, "enumerate");
    assert_eq!(request.params.workspace_id, None);
    let Outcome::Success(reply) = check_reply::<EnumerateReply>(rows[0].reply) else {
        panic!("expected success");
    };
    assert_eq!(reply.result.generation, 2);
    assert_eq!(reply.result.workspaces.len(), 1);
    assert_eq!(reply.result.workspaces[0].root, None);
    assert_eq!(reply.result.projects.len(), 2);
    assert_eq!(reply.result.projects[0].name, "Editor");
    assert_eq!(reply.result.projects[1].name, "Terminal");
    for project in reply.result.projects {
        assert_eq!(project.ref_kind, RefKind::Local);
        assert_eq!(project.root_records, Some(vec![]));
        assert_eq!(project.workspace_id.as_deref(), Some("ws-dashboard"));
        assert_eq!(project.last_route_activity_ms, None);
        assert_eq!(project.device_fingerprint, None);
    }
}

#[test]
fn published_samples_match_sha256sums() {
    let sums = std::fs::read_to_string(vector_dir().join("SHA256SUMS")).unwrap();
    let mut names = vec![];
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(vector_dir().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
        names.push(name);
    }
    assert_eq!(
        names,
        ["enumerate.json", "resolve-project-id.json", "resolve.json"]
    );
}

#[test]
fn synthetic_unknown_values_sample() {
    let sample: Value = serde_json::from_str(include_str!(
        "../test-vectors/synthetic-forward-compatibility/unknown-values.json"
    ))
    .unwrap();
    let resolve: Success<ResolveReply> = serde_json::from_value(sample["resolve"].clone()).unwrap();
    assert!(matches!(resolve.result.via, Via::Other(ref s) if s == "future-via"));
    assert!(
        matches!(resolve.result.binding_status, Presence::Present(Some(BindingStatus::Other(ref s))) if s == "future-binding")
    );
    let Presence::Present(ref roots) = resolve.result.root_records else {
        panic!("missing roots")
    };
    assert!(matches!(roots[0].identity, RootIdentity::Other(ref s) if s == "future-identity"));
    assert!(
        matches!(roots[0].approval.state, ApprovalState::Other(ref s) if s == "future-approval")
    );
    assert!(
        matches!(roots[0].root_key.as_ref().unwrap().kind, RootKeyKind::Other(ref s) if s == "future-key")
    );
    assert!(
        matches!(roots[0].incarnation.as_ref().unwrap().kind, RootIncarnationKind::Other(ref s) if s == "future-incarnation")
    );
    assert_eq!(serde_json::to_value(resolve).unwrap(), sample["resolve"]);
    let enumerate: Success<EnumerateReply> =
        serde_json::from_value(sample["enumerate"].clone()).unwrap();
    assert!(
        matches!(enumerate.result.projects[0].ref_kind, RefKind::Other(ref s) if s == "future-reference")
    );
    assert_eq!(
        serde_json::to_value(enumerate).unwrap(),
        sample["enumerate"]
    );
    let refusal: Refusal = serde_json::from_value(sample["refusal"].clone()).unwrap();
    assert!(matches!(refusal.code, RefusalCode::Other(ref s) if s == "future-refusal"));
    assert_eq!(serde_json::to_value(refusal).unwrap(), sample["refusal"]);
}

fn remove_unknown_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("futureMember");
            for value in map.values_mut() {
                remove_unknown_keys(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                remove_unknown_keys(value);
            }
        }
        _ => {}
    }
}

#[test]
fn synthetic_unknown_keys_sample() {
    let mut sample: Value = serde_json::from_str(include_str!(
        "../test-vectors/synthetic-forward-compatibility/unknown-keys.json"
    ))
    .unwrap();
    let request: Request<ResolveParams> =
        serde_json::from_value(sample["request"].clone()).unwrap();
    let resolve: Outcome<ResolveReply> = serde_json::from_value(sample["resolve"].clone()).unwrap();
    let id: Outcome<ResolveProjectIdReply> = serde_json::from_value(sample["id"].clone()).unwrap();
    let enumerate: Outcome<EnumerateReply> =
        serde_json::from_value(sample["enumerate"].clone()).unwrap();
    let refusal: Outcome<ResolveReply> = serde_json::from_value(sample["refusal"].clone()).unwrap();
    remove_unknown_keys(&mut sample);
    for (key, decoded) in [
        ("request", serde_json::to_value(request).unwrap()),
        ("resolve", serde_json::to_value(resolve).unwrap()),
        ("id", serde_json::to_value(id).unwrap()),
        ("enumerate", serde_json::to_value(enumerate).unwrap()),
        ("refusal", serde_json::to_value(refusal).unwrap()),
    ] {
        assert_eq!(decoded, sample[key], "{key}");
    }
}
