use cortexkit_bus_naming::{
    validate_tenancy_name, AccountNames, NamingExemption, NamingRule, TokenKind,
    CLOSED_NAMING_EXEMPTIONS,
};

#[test]
fn account_derivation_refuses_hyphen_without_normalizing() {
    let error = AccountNames::derive("box_a-b").expect_err("hyphen must be refused");
    assert_eq!(error.kind(), TokenKind::Account);
    assert_eq!(error.token(), "box_a-b");
    assert!(error.to_string().contains("box_a-b"));

    let accepted = AccountNames::derive("box_a_b").expect("underscore is in the lexicon");
    assert_eq!(accepted.account_upper(), "BOX_A_B");
}

#[test]
fn accepted_accounts_have_distinct_stream_and_bucket_sets() {
    let left = AccountNames::derive("box_a_b").unwrap();
    let right = AccountNames::derive("box_a_c").unwrap();

    assert_ne!(left.streams().all(), right.streams().all());
    assert_ne!(left.buckets().all(), right.buckets().all());
}

#[test]
fn token_refusal_names_the_call_site_token() {
    let names = AccountNames::derive("box_fixture").unwrap();
    let error = names
        .peer_delivery("agent.good", "session_ok")
        .expect_err("dot must be refused at interpolation");
    assert_eq!(error.kind(), TokenKind::AgentId);
    assert_eq!(error.token(), "agent.good");
    assert!(error.to_string().contains("agent.good"));
}

#[test]
fn every_governed_name_obeys_its_rule_and_exemptions_are_closed() {
    assert_eq!(
        CLOSED_NAMING_EXEMPTIONS,
        [
            NamingExemption::InboxPrefix,
            NamingExemption::JetStreamPrefix,
            NamingExemption::KeyValuePrefix,
            NamingExemption::SystemPrefix,
            NamingExemption::DurableConsumer,
        ]
    );

    let names = AccountNames::derive("box_rules").unwrap();
    let subjects = [
        names.room_post("room_a").unwrap(),
        names.room_subscription("room_a").unwrap(),
        names.room_binding(),
        names.wake_fire("agent_a").unwrap(),
        names.wake_binding(),
        names.peer_delivery("agent_a", "session_a").unwrap(),
        names.peer_filter("agent_a").unwrap(),
        names.peer_subscription("agent_a").unwrap(),
        names.peer_binding(),
        names.effect_intent("agent_a", "session_a").unwrap(),
        names.effect_filter("agent_a").unwrap(),
        names.effect_publish_grant("agent_a").unwrap(),
        names.effect_binding(),
        names.effect_dead(),
        names.sentinel_ping(),
    ];
    for subject in subjects {
        validate_tenancy_name(&names, &subject, NamingRule::CkSubject).unwrap();
    }
    for stream in names.streams().all() {
        validate_tenancy_name(&names, stream, NamingRule::StreamName).unwrap();
    }
    for bucket in names.buckets().all() {
        validate_tenancy_name(&names, bucket, NamingRule::BucketName).unwrap();
    }

    let exempt = [
        ("_INBOX.UFIXTURE.>", NamingExemption::InboxPrefix),
        ("$JS.API.STREAM.INFO.X", NamingExemption::JetStreamPrefix),
        ("$KV.X.>", NamingExemption::KeyValuePrefix),
        ("$SYS.REQ.CLAIMS.UPDATE", NamingExemption::SystemPrefix),
        ("c_agent_a", NamingExemption::DurableConsumer),
    ];
    for (name, exemption) in exempt {
        validate_tenancy_name(&names, name, NamingRule::Exempt(exemption)).unwrap();
    }
}
