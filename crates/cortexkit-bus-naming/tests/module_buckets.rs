use cortexkit_bus_naming::{
    validate_module_bucket_name, validate_module_buckets, AccountNames, ModuleBucket,
    MODULE_BUCKETS,
};

#[test]
fn module_bucket_names_normalize_only_the_derived_name() {
    let names = AccountNames::derive("box_kv").unwrap();
    for (owner, name, expected) in [
        ("aft", "status", "CK_BOX_KV_AFT_STATUS"),
        (
            "prefrontal-core",
            "session-liveness",
            "CK_BOX_KV_PREFRONTAL_CORE_SESSION_LIVENESS",
        ),
        ("astrocyte", "read.marks", "CK_BOX_KV_ASTROCYTE_READ_MARKS"),
    ] {
        assert_eq!(names.module_bucket_name(owner, name).unwrap(), expected);
        validate_module_bucket_name(expected).unwrap();
    }
    for bad in [
        "",
        "ck_box_kv_aft_status",
        "CK_AFT",
        "KV_CK_BOX_AFT_STATUS",
        "CK_BOX_AFT_read-marks",
        "CK_BOX_AFT_READ.MARKS",
        "CK_BOX_AFT_*",
        "CK_BOX_AFT_>",
        "CK_BOX_AFT_É",
        "CK_BOX_AFT_ STATUS",
    ] {
        assert!(validate_module_bucket_name(bad).is_err(), "{bad}");
    }
    for bad in ["", "Upper", "*", ">", "a b", "a..b", ".a", "a."] {
        assert!(names.module_bucket_name("aft", bad).is_err(), "{bad}");
    }
    assert!(names.module_bucket_name("bad.module", "status").is_err());
}

#[test]
fn declared_module_buckets_pin_readers_value_caps_and_retention() {
    let expected = [
        ModuleBucket::new(
            "aft",
            "status",
            &["prefrontal-core"],
            2_048,
            1,
            Some(10),
            2_097_152,
        ),
        ModuleBucket::new(
            "astrocyte",
            "multiplier",
            &["striatum"],
            8_192,
            1,
            None,
            1_048_576,
        ),
        ModuleBucket::new(
            "astrocyte",
            "read-marks",
            &["thalamus-harness"],
            256,
            1,
            None,
            16_777_216,
        ),
        ModuleBucket::new(
            "entorhinal",
            "changes",
            &["prefrontal-core", "plexus", "magic-context"],
            512,
            1,
            None,
            33_554_432,
        ),
        ModuleBucket::new(
            "prefrontal-core",
            "session-liveness",
            &["entorhinal"],
            1_024,
            1,
            None,
            16_777_216,
        ),
    ];
    assert_eq!(MODULE_BUCKETS, expected);
    for account in ["box_kv", "box_second"] {
        let names = AccountNames::derive(account).unwrap();
        validate_module_buckets(&names, MODULE_BUCKETS).unwrap();
        for bucket in MODULE_BUCKETS {
            validate_module_bucket_name(
                &names.module_bucket_name(bucket.owner, bucket.name).unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn module_bucket_validation_refuses_collisions_invalid_identities_and_limits() {
    let names = AccountNames::derive("box_kv").unwrap();
    let bucket = MODULE_BUCKETS[0];
    let edits: [fn(&mut ModuleBucket); 9] = [
        |b| b.owner = "bad.module",
        |b| b.readers = &["bad.module"],
        |b| b.readers = &["reader", "reader"],
        |b| b.readers = &["aft"],
        |b| b.max_value_bytes = 0,
        |b| b.history = 0,
        |b| b.history = 65,
        |b| b.ttl = Some(0),
        |b| b.max_bytes = 2_047,
    ];
    for edit in edits {
        let mut bad = bucket;
        edit(&mut bad);
        assert!(validate_module_buckets(&names, &[bad]).is_err(), "{bad:?}");
    }
    assert!(validate_module_buckets(&names, &[bucket, bucket]).is_err());
    let mut dotted = bucket;
    dotted.name = "read.marks";
    let mut hyphenated = bucket;
    hyphenated.name = "read-marks";
    assert!(validate_module_buckets(&names, &[dotted, hyphenated]).is_err());
}
