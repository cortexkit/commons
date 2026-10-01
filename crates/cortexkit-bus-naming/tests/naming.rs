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
        names.event_subject("module_a", "event_a", 1).unwrap(),
        names.event_publish_grant("module_a").unwrap(),
        names.event_binding(),
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

#[test]
fn event_names_are_one_subject_token_without_hyphens() {
    use cortexkit_bus_naming::validate_token;
    let names = AccountNames::derive("box_events").unwrap();
    for bad in ["pull-request", "a.b"] {
        let error =
            validate_token(TokenKind::EventName, bad).expect_err("event name must be refused");
        assert_eq!(error.kind(), TokenKind::EventName);
        assert_eq!(error.token(), bad);
        assert!(error.to_string().contains(bad));

        let error = names
            .event_subject("github", bad, 1)
            .expect_err("event subject must refuse the same name");
        assert_eq!(error.kind(), TokenKind::EventName);
        assert_eq!(error.token(), bad);
    }
    for bad in ["", "*", ">", "Pull", "_lead", "a b"] {
        assert_eq!(
            validate_token(TokenKind::EventName, bad)
                .expect_err("event name must be refused")
                .kind(),
            TokenKind::EventName,
            "{bad:?}"
        );
    }
    validate_token(TokenKind::EventName, "pull_request_review").unwrap();
    assert_eq!(
        names
            .event_subject("github", "pull_request_review", 1)
            .unwrap(),
        "ck.box_events.event.github.pull_request_review.v1"
    );
}

#[test]
fn event_versions_are_v_then_a_positive_integer() {
    use cortexkit_bus_naming::validate_event_version;
    assert_eq!(validate_event_version("v1").unwrap(), 1);
    assert_eq!(validate_event_version("v42").unwrap(), 42);
    for bad in ["v0", "v01", "v", "1", "V1", "v1a", "v-1", "v99999999999"] {
        let error = validate_event_version(bad).expect_err("version must be refused");
        assert_eq!(error.kind(), TokenKind::EventVersion, "{bad:?}");
        assert_eq!(error.token(), bad);
    }
    let names = AccountNames::derive("box_events").unwrap();
    let error = names
        .event_subject("github", "push", 0)
        .expect_err("version 0 must be refused");
    assert_eq!(error.kind(), TokenKind::EventVersion);
}

#[test]
fn the_event_stream_is_the_sixth_stream_and_holds_module_durables() {
    let names = AccountNames::derive("box_events").unwrap();
    let streams = names.streams();
    assert_eq!(streams.event, "CK_BOX_EVENTS_EVENT");
    assert_eq!(streams.all().len(), 6);
    assert_eq!(streams.all()[5], streams.event);
    assert!(!streams.agent_streams().contains(&streams.event.as_str()));

    assert_eq!(names.event_binding(), "ck.box_events.event.>");
    assert_eq!(
        names.event_publish_grant("prefrontal-core").unwrap(),
        "ck.box_events.event.prefrontal-core.>"
    );
    assert_eq!(
        AccountNames::module_consumer_name("basal").unwrap(),
        "m_basal"
    );
    for bad in ["", "a.b", "*", ">"] {
        assert_eq!(
            AccountNames::module_consumer_name(bad)
                .expect_err("module durable must refuse a non-token")
                .kind(),
            TokenKind::ModuleId
        );
        assert!(names.event_publish_grant(bad).is_err());
    }
}

#[test]
fn root_credential_ids_follow_the_ceremony_grammar() {
    use cortexkit_bus_naming::{root_credential_id, RootCredentialKind};
    use std::num::NonZeroU32;
    let one = NonZeroU32::new(1);
    // The ids the operator ceremony mints (ck-bus spec, credentials table).
    assert_eq!(
        root_credential_id(RootCredentialKind::Signing, "ck-bus-account", one).unwrap(),
        "signing:ck-bus-account:1"
    );
    assert_eq!(
        root_credential_id(RootCredentialKind::Signing, "ck-bus-operator-signer", one).unwrap(),
        "signing:ck-bus-operator-signer:1"
    );
    assert_eq!(
        root_credential_id(RootCredentialKind::Signing, "ck-bus-operator-root", one).unwrap(),
        "signing:ck-bus-operator-root:1"
    );
    assert_eq!(
        root_credential_id(RootCredentialKind::Signing, "msgsig", None).unwrap(),
        "signing:msgsig"
    );
    assert_eq!(
        root_credential_id(RootCredentialKind::Kem, "fed-seal", NonZeroU32::new(2)).unwrap(),
        "kem:fed-seal:2"
    );
    // A provider can never add a segment, widen with a wildcard, or change case.
    for provider in [
        "ck-bus-account:2",
        "ck-bus.account",
        "*",
        "Ck-bus",
        "",
        " msgsig",
    ] {
        let error = root_credential_id(RootCredentialKind::Signing, provider, one)
            .expect_err("provider outside the lexicon must refuse");
        assert_eq!(error.kind(), cortexkit_bus_naming::TokenKind::RootProvider);
    }
}

#[test]
fn census_key_is_the_module_id_and_refuses_anything_but_one_token() {
    let names = AccountNames::derive("box_census").unwrap();
    assert_eq!(
        AccountNames::census_key("prefrontal-core").unwrap(),
        "prefrontal-core"
    );
    assert_eq!(
        names.census_subject("prefrontal-core").unwrap(),
        "$KV.CK_BOX_CENSUS_CENSUS.prefrontal-core"
    );
    for bad in ["", "a.b", "*", ">", "Prefrontal", "mod ule", "-lead"] {
        let error = AccountNames::census_key(bad).expect_err("census key must refuse a non-token");
        assert_eq!(error.kind(), TokenKind::ModuleId, "{bad:?}");
        names
            .census_subject(bad)
            .expect_err("census subject must refuse the same tokens");
    }
    validate_tenancy_name(
        &names,
        &names.census_subject("prefrontal-core").unwrap(),
        NamingRule::Exempt(NamingExemption::KeyValuePrefix),
    )
    .unwrap();
}
