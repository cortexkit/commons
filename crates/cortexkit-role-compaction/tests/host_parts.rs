use cortexkit_role_compaction::{
    part::{body_digest, Part, PartBody},
    provides_host_parts,
    read::{entry_digest, Entry, HostPart, Role},
    COMPACTION_HOST_PARTS_PROVIDES, COMPACTION_PROVIDES,
};
use serde_json::{json, Value};

fn entry(role: Role) -> Entry {
    Entry::new(
        1,
        "message-a",
        role,
        1_700_000_000_000,
        vec![
            Part::new(
                "prompt",
                PartBody::Text {
                    text: "An invented prompt.".into(),
                },
            ),
            Part::new(
                "interrupt",
                PartBody::Text {
                    text: "[Request interrupted by user]\n".into(),
                },
            ),
        ],
    )
}

#[test]
fn host_parts_round_trip_user_and_assistant_entries() {
    for role in [Role::User, Role::Assistant] {
        for code in [None, Some("interrupted"), Some("future_host_note_2")] {
            let fact = match code {
                Some(code) => HostPart::new("interrupt").with_host_code(code),
                None => HostPart::new("interrupt"),
            };
            let marked = entry(role.clone())
                .with_host_parts(vec![fact.clone()], true)
                .unwrap();
            let wire = serde_json::to_value(&marked).unwrap();
            assert_eq!(wire["host_parts"][0]["part"], "interrupt");
            assert_eq!(
                wire["host_parts"][0].get("host_code"),
                code.map(Value::from).as_ref()
            );
            assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), marked);
            let llm_fact = cortexkit_role_llm_runner::read::HostPart::new("interrupt");
            let llm_fact = match code {
                Some(code) => llm_fact.with_host_code(code),
                None => llm_fact,
            };
            assert_eq!(
                serde_json::to_vec(&fact).unwrap(),
                serde_json::to_vec(&llm_fact).unwrap()
            );
        }
    }
}

#[test]
fn host_parts_empty_is_omitted_but_null_is_refused() {
    let legacy = serde_json::to_value(entry(Role::User)).unwrap();
    assert!(legacy.get("host_parts").is_none());
    let mut empty = legacy.clone();
    empty["host_parts"] = json!([]);
    let decoded: Entry = serde_json::from_value(empty).unwrap();
    assert!(decoded.host_parts.is_empty());
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let mut null = legacy;
    null["host_parts"] = Value::Null;
    assert!(serde_json::from_value::<Entry>(null).is_err());
}

#[test]
fn host_parts_decode_and_memory_checks_reject_invalid_facts() {
    for facts in [
        json!([{"part": "missing"}]),
        json!([{"part": "interrupt"}, {"part": "interrupt"}]),
        json!([{"part": "interrupt"}, {"part": "prompt"}]),
    ] {
        let mut wire = serde_json::to_value(entry(Role::User)).unwrap();
        wire["host_parts"] = facts.clone();
        assert!(serde_json::from_value::<Entry>(wire).is_err(), "{facts}");
        let mut built = entry(Role::User);
        built.host_parts = serde_json::from_value(facts).unwrap();
        assert!(built.check_host_parts().is_err());
        assert!(entry(Role::User)
            .with_host_parts(built.host_parts, true)
            .is_err());
    }
    for role in [Role::System, Role::Unknown("future_role".into())] {
        let mut wire = serde_json::to_value(entry(role.clone())).unwrap();
        wire["host_parts"] = json!([{"part": "interrupt"}]);
        assert!(serde_json::from_value::<Entry>(wire).is_err());
        assert!(entry(role)
            .with_host_parts(vec![HostPart::new("interrupt")], true)
            .is_err());
    }
    let all_host = entry(Role::User)
        .with_host_parts(
            vec![HostPart::new("prompt"), HostPart::new("interrupt")],
            true,
        )
        .unwrap();
    assert!(all_host.check_host_parts().is_ok());
}

#[test]
fn host_parts_codes_check_shape_not_vocabulary() {
    for code in ["a".to_owned(), "0_".into(), "a".repeat(64)] {
        let fact = HostPart::new("interrupt").with_host_code(&code);
        assert!(fact.check().is_ok());
        let wire = serde_json::to_value(&fact).unwrap();
        assert_eq!(serde_json::from_value::<HostPart>(wire).unwrap(), fact);
    }
    for code in [
        "".to_owned(),
        "a".repeat(65),
        "Bad".into(),
        "a-b".into(),
        "é".into(),
    ] {
        let fact = HostPart::new("interrupt").with_host_code(&code);
        assert!(fact.check().is_err(), "{code:?}");
        let mut wire = serde_json::to_value(entry(Role::User)).unwrap();
        wire["host_parts"] = json!([{"part": "interrupt", "host_code": code}]);
        assert!(serde_json::from_value::<Entry>(wire).is_err());
        assert!(entry(Role::User).with_host_parts(vec![fact], true).is_err());
    }
    for code in [Value::Null, json!(true), json!(3), json!([])] {
        let wire = json!({"part": "interrupt", "host_code": code});
        assert!(serde_json::from_value::<HostPart>(wire).is_err());
    }
}

#[test]
fn host_parts_provider_gate_clears_previously_marked_entries() {
    assert_eq!(COMPACTION_HOST_PARTS_PROVIDES, "compaction-host-parts/v1");
    let marked = entry(Role::User)
        .with_host_parts(vec![HostPart::new("interrupt")], true)
        .unwrap();
    for (capabilities, declared) in [
        (vec![], false),
        (vec![COMPACTION_PROVIDES.into()], false),
        (vec!["compaction-host-parts/v2".into()], false),
        (vec!["compaction-host-parts/v1-extra".into()], false),
        (
            vec![
                COMPACTION_PROVIDES.into(),
                COMPACTION_HOST_PARTS_PROVIDES.into(),
            ],
            true,
        ),
    ] {
        assert_eq!(provides_host_parts(&capabilities), declared);
        let projected = marked
            .clone()
            .with_host_parts(
                marked.host_parts.clone(),
                provides_host_parts(&capabilities),
            )
            .unwrap();
        assert_eq!(projected.host_parts.is_empty(), !declared);
        assert_eq!(
            serde_json::to_value(projected)
                .unwrap()
                .get("host_parts")
                .is_some(),
            declared
        );
    }
}

#[test]
fn host_parts_leave_entry_and_part_digest_bytes_unchanged() {
    let plain = entry(Role::User);
    let marked = plain
        .clone()
        .with_host_parts(
            vec![HostPart::new("interrupt").with_host_code("interrupted")],
            true,
        )
        .unwrap();
    assert_ne!(
        serde_json::to_vec(&marked).unwrap(),
        serde_json::to_vec(&plain).unwrap()
    );
    assert_eq!(marked.parts, plain.parts);
    assert_eq!(
        marked.digest.as_bytes().unwrap(),
        plain.digest.as_bytes().unwrap()
    );
    assert_eq!(
        entry_digest(marked.role.clone(), marked.notice.as_ref(), &marked.parts)
            .as_bytes()
            .unwrap(),
        plain.digest.as_bytes().unwrap()
    );
    for (marked, plain) in marked.parts.iter().zip(&plain.parts) {
        assert_eq!(
            marked.digest.as_bytes().unwrap(),
            plain.digest.as_bytes().unwrap()
        );
        assert_eq!(
            body_digest(marked.seal.clone(), &marked.body)
                .as_bytes()
                .unwrap(),
            plain.digest.as_bytes().unwrap()
        );
        assert_eq!(
            serde_json::to_vec(marked).unwrap(),
            serde_json::to_vec(plain).unwrap()
        );
    }
    let mut compact = marked.clone();
    compact.parts = compact.parts.iter().map(Part::compact).collect();
    assert_eq!(compact.host_parts, marked.host_parts);
    assert_eq!(
        compact.digest.as_bytes().unwrap(),
        plain.digest.as_bytes().unwrap()
    );
}
