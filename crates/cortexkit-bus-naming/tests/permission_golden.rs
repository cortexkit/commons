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
    assert!(CHECKED_IN_GOLDEN.contains("foreign-identity"));
    assert!(CHECKED_IN_GOLDEN.contains("unbound-room"));
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
