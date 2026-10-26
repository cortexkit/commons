use cortexkit_project_identity::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string(value).unwrap();
    let decoded: T = serde_json::from_str(&encoded).unwrap();
    assert_eq!(&decoded, value);
}

macro_rules! string_test {
    ($test:ident, $ty:ident, {$($variant:ident => $wire:literal),+ $(,)?}) => {
        #[test]
        fn $test() {
            $(
                assert_eq!($ty::new($wire), $ty::$variant);
                assert_eq!(serde_json::to_value($ty::$variant).unwrap(), json!($wire));
                assert_eq!(serde_json::from_value::<$ty>(json!($wire)).unwrap(), $ty::$variant);
                round_trip(&$ty::$variant);
            )+
            for spelling in ["future-雪", "", "FUTURE"] {
                let other = $ty::Other(spelling.into());
                assert_eq!($ty::new(spelling), other);
                assert_eq!(serde_json::from_value::<$ty>(json!(spelling)).unwrap(), other);
                assert_eq!(serde_json::to_value(&other).unwrap(), json!(spelling));
                assert_eq!(String::from(other.clone()), spelling);
                round_trip(&other);
            }
            for invalid in [Value::Null, json!(0), json!({"Other": "future"})] {
                assert!(serde_json::from_value::<$ty>(invalid).is_err());
            }
        }
    };
}

string_test!(via_strings_round_trip, Via, {
    Root => "root", Walk => "walk", Containment => "containment", Worktree => "worktree",
    Implicit => "implicit", Current => "current", Alias => "alias", Gone => "gone",
});
string_test!(binding_status_strings_round_trip, BindingStatus, {
    Active => "active", Unknown => "unknown", Replaced => "replaced", Retired => "retired",
});
string_test!(approval_state_strings_round_trip, ApprovalState, {
    Approved => "approved", Unapproved => "unapproved",
});
string_test!(root_identity_strings_round_trip, RootIdentity, {
    Bound => "bound", Unbound => "unbound", Unverifiable => "unverifiable",
    Replaced => "replaced", Retired => "retired",
});
string_test!(root_key_kind_strings_round_trip, RootKeyKind, { Remote => "remote", Label => "label" });
string_test!(root_incarnation_kind_strings_round_trip, RootIncarnationKind, { Token => "token" });
string_test!(ref_kind_strings_round_trip, RefKind, { Local => "local", Remote => "remote" });
string_test!(refusal_code_strings_round_trip, RefusalCode, {
    InvalidRequest => "invalid_request", InvalidParams => "invalid_params",
    StorageUnavailable => "storage_unavailable", StoreUnavailable => "store_unavailable",
    StorageError => "storage_error", EncodeFailed => "encode_failed",
    ModuleDraining => "module_draining", UnknownMethod => "unknown_method",
    UnknownAgent => "unknown_agent", Gone => "gone",
    ClaimIntegrityFailed => "claim_integrity_failed", StoreCorrupt => "store_corrupt",
    NotFound => "not_found",
});

#[test]
fn not_found_refusal_round_trip() {
    let refusal = Refusal::new(RefusalCode::NotFound, "no matching session".into(), None);
    assert_eq!(serde_json::to_value(&refusal).unwrap()["code"], "not_found");
    round_trip(&refusal);
}

#[test]
fn registered_is_not_current_provenance() {
    assert_eq!(Via::new("registered"), Via::Other("registered".into()));
    round_trip(&Via::Other("registered".into()));
}

fn resolve_value() -> Value {
    json!({
        "projectId":"project", "workspaceId":null, "workspaceRoot":null,
        "projectName":null, "via":"implicit", "gone":false, "generation":-1,
        "incarnation":"process", "canonicalRoot":"/query"
    })
}

#[test]
fn presence_preserves_missing_null_and_values() {
    struct NoDefault;
    assert!(Presence::<NoDefault>::default().is_missing());
    let base = resolve_value();
    let missing: ResolveReply = serde_json::from_value(base.clone()).unwrap();
    assert_eq!(missing.matched_root, Presence::missing());
    assert_eq!(missing.source_root, Presence::default());
    assert_eq!(missing.root_records, Presence::Missing);
    assert_eq!(missing.binding_status, Presence::Missing);
    assert_eq!(serde_json::to_value(missing).unwrap(), base);
    for (matched, source, status) in [
        (Value::Null, Value::Null, Value::Null),
        (json!("/matched"), json!("/source"), json!("active")),
    ] {
        let mut value = base.clone();
        value["matchedRoot"] = matched;
        value["sourceRoot"] = source;
        value["rootRecords"] = json!([]);
        value["bindingStatus"] = status;
        let reply: ResolveReply = serde_json::from_value(value.clone()).unwrap();
        assert!(!reply.matched_root.is_missing());
        assert!(!reply.source_root.is_missing());
        assert_eq!(reply.root_records, Presence::new(vec![]));
        assert!(!reply.binding_status.is_missing());
        assert_eq!(serde_json::to_value(reply).unwrap(), value);
    }
    let mut null_records = base;
    null_records["rootRecords"] = Value::Null;
    assert!(serde_json::from_value::<ResolveReply>(null_records).is_err());
    round_trip(&Presence::new(Some("root".to_string())));
    round_trip(&Presence::<Option<String>>::new(None));
    assert_eq!(
        serde_json::to_value(Presence::<String>::Missing).unwrap(),
        Value::Null
    );
}

fn require_fields<T: DeserializeOwned>(base: Value, fields: &[&str]) {
    assert!(serde_json::from_value::<T>(base.clone()).is_ok());
    for field in fields {
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove(*field);
        assert!(
            serde_json::from_value::<T>(missing).is_err(),
            "missing {field}"
        );
        let mut null = base.clone();
        null[*field] = Value::Null;
        assert!(serde_json::from_value::<T>(null).is_err(), "null {field}");
        let mut wrong_type = base.clone();
        wrong_type[*field] = json!({});
        assert!(
            serde_json::from_value::<T>(wrong_type).is_err(),
            "wrong type {field}"
        );
    }
}

#[test]
fn required_id_fields_are_not_optional() {
    require_fields::<ResolveProjectIdReply>(
        json!({"projectId":"p", "via":"current", "gone":false, "generation":0, "incarnation":"process"}),
        &["projectId", "via", "gone", "generation", "incarnation"],
    );
}

#[test]
fn required_nonnullable_fields_reject_omission_null_and_wrong_types() {
    require_fields::<ResolveReply>(
        resolve_value(),
        &["projectId", "via", "gone", "generation", "incarnation"],
    );
    require_fields::<EnumerateReply>(
        json!({"workspaces":[], "projects":[], "generation":0, "incarnation":"p"}),
        &["workspaces", "projects", "generation", "incarnation"],
    );
    require_fields::<Project>(
        json!({"projectId":"p", "name":"n", "roots":[], "implicit":false, "refKind":"local"}),
        &["projectId", "name", "roots", "implicit", "refKind"],
    );
    require_fields::<Workspace>(
        json!({"workspaceId":"w", "name":"n", "root":null}),
        &["workspaceId", "name"],
    );
    require_fields::<RootRecord>(
        json!({"root":"/r", "remotes":[], "incarnation":null, "registrationEpoch":null,
            "identity":"unbound", "approval":{"state":"unapproved", "journalSeq":null}}),
        &["root", "remotes", "identity", "approval"],
    );
    require_fields::<RootIncarnation>(json!({"kind":"token", "value":"t"}), &["kind", "value"]);
    require_fields::<RootKey>(json!({"kind":"label", "rootKey":"k"}), &["kind", "rootKey"]);
    require_fields::<Approval>(json!({"state":"unapproved", "journalSeq":null}), &["state"]);
    require_fields::<GitRemote>(
        json!({"name":"origin", "owner":"o", "repo":"r", "owned":true}),
        &["name", "owner", "repo", "owned"],
    );
    require_fields::<Refusal>(
        json!({"code":"invalid_params", "message":"m", "detail":null}),
        &["code", "message"],
    );
}

#[test]
fn requests_require_objects_and_nonnullable_children() {
    require_fields::<ResolveParams>(json!({"canonicalRoot":"/r"}), &["canonicalRoot"]);
    require_fields::<ResolveProjectIdParams>(json!({"projectId":"p"}), &["projectId"]);
    require_fields::<ExecutionBinding>(
        json!({"root":"/r", "incarnation":"checkout", "registrationEpoch":"epoch"}),
        &["root", "incarnation", "registrationEpoch"],
    );
    require_fields::<Request<ResolveParams>>(
        json!({"method":"resolve", "params":{"canonicalRoot":"/r"}}),
        &["method", "params"],
    );
    for params in [Value::Null, json!(""), json!(42)] {
        assert!(serde_json::from_value::<EnumerateParams>(params).is_err());
    }
    for request in [
        json!({"method":"enumerate"}),
        json!({"method":"enumerate", "params":null}),
    ] {
        assert!(serde_json::from_value::<Request<EnumerateParams>>(request).is_err());
    }
    assert!(serde_json::from_value::<ResolveParams>(
        json!({"canonicalRoot":"/r", "executionBinding":{}})
    )
    .is_err());
}

#[test]
fn request_aliases_nulls_filters_and_unknown_keys() {
    for key in ["canonicalRoot", "canonical_root"] {
        let value = json!({key:"/r", "executionBinding":null, "futureMember":true});
        let params: ResolveParams = serde_json::from_value(value).unwrap();
        assert_eq!(params, ResolveParams::new("/r".into(), None));
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            json!({"canonicalRoot":"/r"})
        );
    }
    for key in ["projectId", "project_id"] {
        let params: ResolveProjectIdParams =
            serde_json::from_value(json!({key:"p", "extra":0})).unwrap();
        assert_eq!(
            serde_json::to_value(params).unwrap(),
            json!({"projectId":"p"})
        );
    }
    for key in ["workspaceId", "workspace_id"] {
        for filter in [Value::Null, json!("w")] {
            let params: EnumerateParams =
                serde_json::from_value(json!({key:filter, "extra":0})).unwrap();
            assert_eq!(params.workspace_id, filter.as_str().map(str::to_owned));
            round_trip(&params);
        }
    }
    let empty = Request::new("enumerate", EnumerateParams::new(None));
    assert_eq!(
        serde_json::to_value(empty).unwrap(),
        json!({"method":"enumerate", "params":{}})
    );
    let arbitrary = Request::new("future-read", EnumerateParams::new(None));
    round_trip(&arbitrary);
}

fn nullable_member<T>(mut base: Value, field: &str, populated: Value)
where
    T: DeserializeOwned + Serialize,
{
    for value in [Value::Null, populated] {
        base[field] = value;
        let decoded: T = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap()[field], base[field]);
    }
    base.as_object_mut().unwrap().remove(field);
    let decoded: T = serde_json::from_value(base).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap().get(field),
        Some(&Value::Null)
    );
}

#[test]
fn required_nullable_members_are_lenient_but_serialize_null() {
    for field in [
        "workspaceId",
        "workspaceRoot",
        "projectName",
        "canonicalRoot",
    ] {
        nullable_member::<ResolveReply>(resolve_value(), field, json!("value"));
    }
    nullable_member::<Workspace>(json!({"workspaceId":"w", "name":"n"}), "root", json!("/w"));
    nullable_member::<Approval>(json!({"state":"approved"}), "journalSeq", json!(17));
    let root = json!({"root":"/r", "remotes":[], "identity":"bound", "approval":{"state":"unapproved", "journalSeq":null},
        "incarnation":null, "registrationEpoch":null});
    nullable_member::<RootRecord>(
        root.clone(),
        "incarnation",
        json!({"kind":"token", "value":"checkout"}),
    );
    nullable_member::<RootRecord>(root, "registrationEpoch", json!("epoch"));
}

#[test]
fn optional_members_preserve_values_and_omit_none() {
    let root = RootRecord::new(
        "/r".into(),
        vec![],
        Some("probe error".into()),
        Some(RootKey::new(RootKeyKind::Remote, "owner/repo".into())),
        Some(false),
        Some(RootIncarnation::new(
            RootIncarnationKind::Token,
            "checkout".into(),
        )),
        Some("epoch".into()),
        RootIdentity::Bound,
        Approval::new(ApprovalState::Approved, Some(17)),
    );
    round_trip(&root);
    let encoded = serde_json::to_value(&root).unwrap();
    assert_eq!(
        encoded["rootKey"],
        json!({"kind":"remote", "rootKey":"owner/repo"})
    );
    assert_eq!(encoded["rootKeyMismatch"], false);
    let project = Project::new(
        "p".into(),
        "n".into(),
        vec!["/r".into()],
        false,
        RefKind::Remote,
        Some("device".into()),
        Some("w".into()),
        Some(123),
        Some(vec![root]),
    );
    round_trip(&project);
    let mut encoded = serde_json::to_value(project).unwrap();
    for field in [
        "deviceFingerprint",
        "workspaceId",
        "lastRouteActivityMs",
        "rootRecords",
    ] {
        encoded[field] = Value::Null;
    }
    let decoded: Project = serde_json::from_value(encoded).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    for field in [
        "deviceFingerprint",
        "workspaceId",
        "lastRouteActivityMs",
        "rootRecords",
    ] {
        assert!(encoded.get(field).is_none(), "{field}");
    }
    let mut encoded = serde_json::to_value(RootRecord::new(
        "/r".into(),
        vec![],
        None,
        None,
        None,
        None,
        None,
        RootIdentity::Unbound,
        Approval::new(ApprovalState::Unapproved, None),
    ))
    .unwrap();
    for field in ["remotesError", "rootKey", "rootKeyMismatch"] {
        encoded[field] = Value::Null;
    }
    let decoded: RootRecord = serde_json::from_value(encoded).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    for field in ["remotesError", "rootKey", "rootKeyMismatch"] {
        assert!(encoded.get(field).is_none());
    }
}

#[test]
fn opaque_refusal_details_and_envelope_policy() {
    for detail in [None, Some(json!({"futureDetail":[null, {"x":true}]}))] {
        let error = Refusal::new(
            RefusalCode::Other("future".into()),
            "message".into(),
            detail,
        );
        round_trip(&Outcome::<ResolveReply>::refusal(error));
    }
    let absent: Refusal =
        serde_json::from_value(json!({"code":"storage_error", "message":"m"})).unwrap();
    assert_eq!(absent.detail, None);
    let success = Outcome::success(ResolveProjectIdReply::new(
        "p".into(),
        Via::Current,
        false,
        0,
        "process".into(),
    ));
    round_trip(&success);
    let mut both = serde_json::to_value(&success).unwrap();
    both["code"] = json!("future-code");
    both["message"] = json!("not a refusal discriminator");
    assert_eq!(
        serde_json::from_value::<Outcome<ResolveProjectIdReply>>(both).unwrap(),
        success
    );
    assert!(serde_json::from_value::<Outcome<ResolveReply>>(resolve_value()).is_err());
    assert!(serde_json::from_value::<Outcome<ResolveReply>>(json!({"result":null})).is_err());
    assert!(serde_json::from_value::<Outcome<ResolveReply>>(json!({})).is_err());
}

#[test]
fn constructors_and_nested_unknown_remote_members() {
    let binding = ExecutionBinding::new("/root".into(), "checkout".into(), "epoch".into());
    let request = Request::new(
        "resolve",
        ResolveParams::new("/query".into(), Some(binding)),
    );
    round_trip(&request);
    assert_eq!(
        serde_json::to_value(request).unwrap()["params"]["executionBinding"]["incarnation"],
        "checkout"
    );
    let remote = GitRemote::new("origin".into(), "owner".into(), "repo".into(), true);
    let mut value = serde_json::to_value(&remote).unwrap();
    value["extra"] = json!({"future":true});
    assert_eq!(serde_json::from_value::<GitRemote>(value).unwrap(), remote);
    round_trip(&remote);
    round_trip(&EnumerateReply::new(
        vec![Workspace::new("w".into(), "n".into(), Some("/w".into()))],
        vec![],
        1,
        "process".into(),
    ));
    let reply = ResolveReply::new(
        "p".into(),
        None,
        None,
        None,
        Via::Implicit,
        false,
        0,
        "process".into(),
        Some("/query".into()),
        None,
        Presence::new(None),
        Presence::new(None),
        Presence::new(vec![]),
        Presence::new(None),
    );
    round_trip(&Success::new(reply));
}

#[test]
fn resolve_remote_params_are_required_exact_strings() {
    let value = json!({"owner":"EXAMPLE-ORG", "repo":"WiDgEtS"});
    let request = Request::new(
        "resolve_remote",
        ResolveRemoteParams::new("EXAMPLE-ORG".into(), "WiDgEtS".into()),
    );
    assert_eq!(serde_json::to_value(&request).unwrap()["params"], value);
    round_trip(&request);
    for field in ["owner", "repo"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<ResolveRemoteParams>(missing).is_err());
        for invalid in [Value::Null, json!(1), json!([])] {
            let mut wrong = value.clone();
            wrong[field] = invalid;
            assert!(serde_json::from_value::<ResolveRemoteParams>(wrong).is_err());
        }
    }
    for params in [Value::Null, json!([]), json!("example-org/widgets")] {
        assert!(serde_json::from_value::<Request<ResolveRemoteParams>>(
            json!({"method":"resolve_remote", "params":params})
        )
        .is_err());
    }
    assert!(serde_json::from_value::<Request<ResolveRemoteParams>>(
        json!({"method":"resolve_remote"})
    )
    .is_err());
    assert!(serde_json::from_value::<ResolveRemoteParams>(
        json!({"Owner":"example-org", "Repo":"widgets"})
    )
    .is_err());
    let params: ResolveRemoteParams =
        serde_json::from_value(json!({"owner":"", "repo":"a/b", "future":true})).unwrap();
    assert_eq!(params, ResolveRemoteParams::new("".into(), "a/b".into()));
}

#[test]
fn resolve_remote_shapes_round_trip() {
    for (status, fields) in [
        (
            ResolveRemoteStatus::found("pj-example".into(), "/checkout".into()),
            json!({"status":"found", "projectId":"pj-example", "root":"/checkout"}),
        ),
        (ResolveRemoteStatus::None, json!({"status":"none"})),
        (
            ResolveRemoteStatus::ambiguous(vec!["pj-a".into(), "pj-b".into()]),
            json!({"status":"ambiguous", "projectIds":["pj-a", "pj-b"]}),
        ),
    ] {
        let reply = ResolveRemoteReply::new(status, -1, "process".into());
        let mut expected = fields;
        expected["generation"] = json!(-1);
        expected["incarnation"] = json!("process");
        assert_eq!(serde_json::to_value(&reply).unwrap(), expected);
        round_trip(&reply);
        round_trip(&Outcome::success(reply));
    }
}

#[test]
fn resolve_remote_tag_selects_shape() {
    let mut fields = json!({
        "generation":8, "incarnation":"process", "projectId":"pj-one",
        "root":"/one", "projectIds":["pj-a", "pj-b"], "future":true
    });
    for (tag, expected) in [
        (
            "found",
            ResolveRemoteStatus::found("pj-one".into(), "/one".into()),
        ),
        ("none", ResolveRemoteStatus::None),
        (
            "ambiguous",
            ResolveRemoteStatus::ambiguous(vec!["pj-a".into(), "pj-b".into()]),
        ),
    ] {
        fields["status"] = json!(tag);
        let reply: ResolveRemoteReply = serde_json::from_value(fields.clone()).unwrap();
        assert_eq!(reply.status, expected);
        assert!(serde_json::to_value(reply).unwrap().get("future").is_none());
    }
    for value in [
        json!({"status":"found", "projectIds":["pj-a", "pj-b"]}),
        json!({"status":"ambiguous", "projectId":"pj-one", "root":"/one"}),
    ] {
        assert!(serde_json::from_value::<ResolveRemoteStatus>(value).is_err());
    }
}

#[test]
fn resolve_remote_rejects_missing_found_root() {
    let value = json!({
        "status":"found", "projectId":"pj-one", "generation":8, "incarnation":"process"
    });
    assert!(serde_json::from_value::<ResolveRemoteReply>(value).is_err());
}

#[test]
fn resolve_remote_fields_reject_missing_null_and_wrong_types() {
    for (value, fields) in [
        (
            json!({"status":"found", "projectId":"pj-one", "root":"/one", "generation":8, "incarnation":"process"}),
            vec!["status", "projectId", "root", "generation", "incarnation"],
        ),
        (
            json!({"status":"none", "generation":8, "incarnation":"process"}),
            vec!["status", "generation", "incarnation"],
        ),
        (
            json!({"status":"ambiguous", "projectIds":["pj-a", "pj-b"], "generation":8, "incarnation":"process"}),
            vec!["status", "projectIds", "generation", "incarnation"],
        ),
    ] {
        for field in fields {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<ResolveRemoteReply>(missing).is_err(),
                "missing {field}"
            );
            for invalid in [Value::Null, json!(false)] {
                let mut wrong = value.clone();
                wrong[field] = invalid;
                assert!(
                    serde_json::from_value::<ResolveRemoteReply>(wrong).is_err(),
                    "invalid {field}"
                );
            }
        }
    }
    for ids in [json!("pj-a"), json!([null]), json!([1])] {
        assert!(serde_json::from_value::<ResolveRemoteStatus>(
            json!({"status":"ambiguous", "projectIds":ids})
        )
        .is_err());
    }
}

#[test]
fn resolve_remote_unknown_status_preserves_text() {
    for status in ["future-雪", "", "FOUND"] {
        let reply: Outcome<ResolveRemoteReply> = serde_json::from_value(json!({
            "result":{"status":status, "generation":8, "incarnation":"process", "future":true},
            "futureEnvelope":true
        }))
        .unwrap();
        let expected = Outcome::success(ResolveRemoteReply::new(
            ResolveRemoteStatus::other(status),
            8,
            "process".into(),
        ));
        assert_eq!(reply, expected);
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            json!({
                "result":{"status":status, "generation":8, "incarnation":"process"}
            })
        );
        round_trip(&reply);
    }
}
