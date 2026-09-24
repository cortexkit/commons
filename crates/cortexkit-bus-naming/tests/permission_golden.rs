use cortexkit_bus_naming::{
    generate_permission_golden, validate_permission_file, AccountNames, GrantError, Principal,
    PINNED_GOLDEN_FIXTURE,
};

const CHECKED_IN_GOLDEN: &str = include_str!("grants/permission_golden.txt");

#[test]
fn permission_golden_is_unconditional_and_host_independent() {
    let document = generate_permission_golden(PINNED_GOLDEN_FIXTURE).unwrap();
    assert_eq!(document.render(), CHECKED_IN_GOLDEN);

    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();
    validate_permission_file(CHECKED_IN_GOLDEN, &account).unwrap();
}

#[test]
fn generator_refuses_invalid_account_before_emitting_permissions() {
    let mut fixture = PINNED_GOLDEN_FIXTURE;
    fixture.account = "box_a-b";
    let error = generate_permission_golden(fixture).expect_err("invalid account must not emit");
    assert!(matches!(error, GrantError::Naming(_)));
    assert!(error.to_string().contains("box_a-b"));
}

#[test]
fn permission_model_is_allow_only_and_expands_all_streams() {
    let document = generate_permission_golden(PINNED_GOLDEN_FIXTURE).unwrap();
    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();
    let rendered = document.render();

    assert!(rendered.lines().any(|line| line.starts_with("allow ")));
    assert!(!rendered.lines().any(|line| line.starts_with("deny ")));
    // async-nats 0.50 starts a KV watch by publishing to the bare CREATE subject;
    // allowing only the suffixed form causes the server to reject that exact publish.
    assert!(rendered.contains(
        "allow participant publish $JS.API.CONSUMER.CREATE.KV_CK_BOX_GOLDENFIXTURE_CENSUS\n"
    ));
    for stream in account.streams().all() {
        assert!(rendered
            .split_whitespace()
            .any(|field| { field.split('.').any(|token| token == stream) }));
    }

    let account_subject_prefix = format!("ck.{}.", account.account());
    for entry in document
        .allows()
        .iter()
        .filter(|entry| entry.principal == Principal::Bus)
    {
        if entry.subject.starts_with(&account_subject_prefix) {
            assert_eq!(
                entry.subject,
                account.sentinel_ping(),
                "the bus management identity must not carry workload rights"
            );
        }
    }
}

#[test]
fn forbidden_permission_shapes_fail_the_build_validator() {
    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();

    let with_partial_wildcard = format!(
        "{CHECKED_IN_GOLDEN}allow bus publish $JS.API.STREAM.INFO.CK_BOX_GOLDENFIXTURE_*\n"
    );
    let partial_error = validate_permission_file(&with_partial_wildcard, &account)
        .expect_err("partial-token wildcard must fail");
    assert!(partial_error.to_string().contains("whole subject token"));

    let with_deny =
        format!("{CHECKED_IN_GOLDEN}deny participant publish ck.box_goldenfixture.sentinel.ping\n");
    let deny_error =
        validate_permission_file(&with_deny, &account).expect_err("deny entry must fail");
    assert!(deny_error
        .to_string()
        .contains("deny entries are forbidden"));
}

#[test]
fn golden_names_bound_and_refused_fixture_identities() {
    for literal in [
        "box_goldenfixture",
        "ckbus",
        "agent_gold_a",
        "agent_gold_b",
        "room_gold_bound",
        "room_gold_unbound",
    ] {
        assert!(CHECKED_IN_GOLDEN.contains(literal), "missing {literal}");
    }
    assert!(CHECKED_IN_GOLDEN.contains("workload-publish"));
    assert!(CHECKED_IN_GOLDEN.contains("unbound-room"));
}

/// NATS subject matching: `*` matches one token, a final `>` one or more.
fn subject_matches(pattern: &str, subject: &str) -> bool {
    let pattern = pattern.split('.').collect::<Vec<_>>();
    let subject = subject.split('.').collect::<Vec<_>>();
    for (index, token) in pattern.iter().enumerate() {
        match *token {
            ">" => return subject.len() > index,
            "*" if index < subject.len() => {}
            literal if subject.get(index) == Some(&literal) => {}
            _ => return false,
        }
    }
    pattern.len() == subject.len()
}

fn allowed(
    allows: &[cortexkit_bus_naming::AllowEntry],
    principal: Principal,
    operation: cortexkit_bus_naming::Operation,
    subject: &str,
) -> bool {
    allows.iter().any(|entry| {
        entry.principal == principal
            && entry.operation == operation
            && subject_matches(&entry.subject, subject)
    })
}

#[test]
fn every_expected_refusal_is_actually_refused_by_the_allows() {
    let document = generate_permission_golden(PINNED_GOLDEN_FIXTURE).unwrap();
    for refusal in document.refused() {
        assert!(
            !allowed(
                document.allows(),
                refusal.principal,
                refusal.operation,
                &refusal.subject
            ),
            "{} may {} {} ({}), which the golden says is refused",
            refusal.principal.as_str(),
            refusal.operation.as_str(),
            refusal.subject,
            refusal.reason
        );
    }
}

#[test]
fn participants_read_any_agent_durable_but_publish_no_workload() {
    use cortexkit_bus_naming::{participant_permissions, Operation};
    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();
    let allows = participant_permissions(&account, "ckhost", &["room_gold_bound"]).unwrap();

    for agent in ["agent_gold_a", "agent_gold_b", "agent_never_seen"] {
        let consumer = AccountNames::consumer_name(agent).unwrap();
        for stream in account.streams().agent_streams() {
            for subject in [
                format!("$JS.API.CONSUMER.MSG.NEXT.{stream}.{consumer}"),
                format!("$JS.API.CONSUMER.INFO.{stream}.{consumer}"),
                format!("$JS.ACK.{stream}.{consumer}.1.2.3.4.5"),
            ] {
                assert!(
                    allowed(
                        &allows,
                        Principal::Participant,
                        Operation::Publish,
                        &subject
                    ),
                    "participant cannot {subject}"
                );
            }
        }
    }

    // The only account subjects a participant may publish are the dead-letter
    // record and its bound room's posts.
    let account_prefix = format!("ck.{}.", account.account());
    let published = allows
        .iter()
        .filter(|entry| entry.operation == Operation::Publish)
        .filter(|entry| entry.subject.starts_with(&account_prefix))
        .map(|entry| entry.subject.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        published,
        vec![
            account.effect_dead(),
            account.room_post("room_gold_bound").unwrap()
        ]
    );
}

#[test]
fn the_delivery_authority_adds_exactly_the_three_agent_stream_bindings() {
    use cortexkit_bus_naming::{delivery_authority_permissions, participant_permissions};
    use std::collections::BTreeSet;
    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();
    let participant = participant_permissions(&account, "ckcore", &["room_gold_bound"])
        .unwrap()
        .into_iter()
        .map(|entry| (entry.operation, entry.subject))
        .collect::<BTreeSet<_>>();
    let authority =
        delivery_authority_permissions(&account, "ckcore", &["room_gold_bound"]).unwrap();
    assert!(authority
        .iter()
        .all(|entry| entry.principal == Principal::DeliveryAuthority));
    let authority = authority
        .into_iter()
        .map(|entry| (entry.operation, entry.subject))
        .collect::<BTreeSet<_>>();

    assert!(participant.is_subset(&authority));
    let extra = authority
        .difference(&participant)
        .map(|(operation, subject)| (operation.as_str(), subject.as_str()))
        .collect::<Vec<_>>();
    let (wake, peer, effect) = (
        account.wake_binding(),
        account.peer_binding(),
        account.effect_binding(),
    );
    assert_eq!(
        extra,
        vec![
            ("publish", effect.as_str()),
            ("publish", peer.as_str()),
            ("publish", wake.as_str()),
        ]
    );
}

#[test]
fn agent_streams_are_wake_peer_and_effect_only() {
    let account = AccountNames::derive(PINNED_GOLDEN_FIXTURE.account).unwrap();
    let streams = account.streams();
    assert_eq!(
        streams.agent_streams(),
        [
            streams.wake.as_str(),
            streams.peer.as_str(),
            streams.effect.as_str()
        ]
    );

    // The bus lists and purges agent durables on exactly those streams.
    let bus = cortexkit_bus_naming::bus_permissions(&account, "ckbus").unwrap();
    for stream in streams.all() {
        for verb in ["CONSUMER.NAMES", "STREAM.PURGE"] {
            let subject = format!("$JS.API.{verb}.{stream}");
            let granted = bus.iter().any(|entry| entry.subject == subject);
            assert_eq!(
                granted,
                streams.agent_streams().contains(&stream),
                "{subject}"
            );
        }
    }
}

#[test]
fn system_user_can_look_up_account_claims_and_read_only_its_own_replies() {
    use cortexkit_bus_naming::{system_permissions, Operation};
    let allows = system_permissions("cksys").unwrap();
    let has = |operation: Operation, subject: &str| {
        allows
            .iter()
            .any(|entry| entry.operation == operation && entry.subject == subject)
    };
    assert!(has(Operation::Publish, "$SYS.REQ.ACCOUNT.*.CLAIMS.LOOKUP"));
    assert!(has(Operation::Publish, "$SYS.REQ.CLAIMS.LIST"));
    assert!(has(Operation::Subscribe, "_INBOX.cksys.>"));
    // Replies are scoped to the system user's own inbox, never every inbox.
    assert!(!has(Operation::Subscribe, "_INBOX.>"));
    assert!(system_permissions("cksys.>").is_err());
}
