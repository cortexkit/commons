mod wire_contract {
    use cortexkit_doctor::*;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    const PLAN_DIGEST: &str =
        "sha256:79429517c39da15d7a860a4f5ecf33fe9202a1feb0038664c9436662c5b86253";
    const FIXED: &str = r#"{"schema":"ck.doctor-fix/v1","result":"fixed"}"#;

    fn check_doc() -> Value {
        json!({
            "schema": "ck.doctor-check/v1",
            "findings": [{
                "id": "task-queue_backlog",
                "severity": "warning",
                "summary": "La file est pleine.",
                "evidence": [{"name": "n_jobs", "kind": "count", "value": 10}],
                "fix": {"class": "safe", "needs_stop": true, "plan": {
                    "steps": ["Élaguer la file.", "Rouvrir."],
                    "digest": PLAN_DIGEST,
                    "timeout_ms": 1234
                }}
            }]
        })
    }

    fn check_error(doc: &Value, error: DecodeError) {
        assert_eq!(CheckOutput::decode(doc.to_string().as_bytes()), Err(error));
    }

    fn fix_error(doc: &Value, error: DecodeError) {
        assert_eq!(FixOutput::decode(doc.to_string().as_bytes()), Err(error));
    }

    fn bad_digest() -> String {
        format!("sha256:{}", "1".repeat(64))
    }

    #[test]
    fn framing_precedes_shape_and_keeps_only_rfc_8259_whitespace() {
        let fixed = FixOutput::fixed();
        for suffix in [b"\n".as_slice(), b"\r\n\t "] {
            let bytes = [FIXED.as_bytes(), suffix].concat();
            assert_eq!(FixOutput::decode(&bytes), Ok(fixed.clone()));
        }
        for byte in u8::MIN..=u8::MAX {
            let bytes = [FIXED.as_bytes(), &[byte]].concat();
            if !b" \t\r\n".contains(&byte) {
                assert_eq!(
                    FixOutput::decode(&bytes),
                    Err(DecodeError::ExtraOutput),
                    "suffix {byte:02x}"
                );
            }
        }
        for invalid in [
            b"".as_slice(),
            b"\t{",
            b"{\"x\":trux}",
            b"\"\\z\"",
            b"\"\xff\"",
        ] {
            assert_eq!(FixOutput::decode(invalid), Err(DecodeError::NotJson));
        }
        for wrong_shape in [
            b"null".as_slice(),
            b"12",
            b"1e999",
            b"[\"ck.doctor-fix/v1\",\"fixed\"]",
        ] {
            assert_eq!(FixOutput::decode(wrong_shape), Err(DecodeError::Shape));
            let bytes = [wrong_shape, b"x"].concat();
            assert_eq!(FixOutput::decode(&bytes), Err(DecodeError::ExtraOutput));
        }
        for (error, code) in [
            (DecodeError::TooLarge, "too_large"),
            (DecodeError::NotJson, "not_json"),
            (DecodeError::ExtraOutput, "extra_output"),
            (DecodeError::Shape, "shape"),
            (DecodeError::PlanDigest, "plan_digest"),
            (DecodeError::TemplateId, "template_id"),
            (DecodeError::TemplatesDigest, "templates_digest"),
        ] {
            assert_eq!(error.as_str(), code);
            assert_eq!(error.to_string(), code);
        }
        assert_eq!(
            FixOutput::decode(&vec![b'x'; MAX_OUTPUT_BYTES + 1]),
            Err(DecodeError::TooLarge)
        );
    }

    #[test]
    fn check_binds_the_timeout_and_all_shape_checks_run_before_digests() {
        let initial = check_doc();
        let out = CheckOutput::decode(initial.to_string().as_bytes()).unwrap();
        let finding = &out.findings()[0];
        let plan = finding.fix().plan().unwrap();
        assert_eq!(plan.digest(), PLAN_DIGEST);
        assert_eq!(plan.timeout_ms(), 1234);
        assert_eq!(
            plan_digest(finding.id(), plan.steps(), true, FixClass::Safe, 1234).unwrap(),
            PLAN_DIGEST
        );
        assert_eq!(serde_json::to_value(out).unwrap(), initial);

        let mut doc = initial.clone();
        doc["findings"][0]["fix"]["plan"]["timeout_ms"] = json!(1235);
        check_error(&doc, DecodeError::PlanDigest);
        doc["findings"][0]["fix"]["plan"]["timeout_ms"] = json!(999);
        check_error(&doc, DecodeError::Shape);
        let mut doc = initial.clone();
        doc["findings"][0]["fix"]["plan"]["digest"] = json!("sha256:xyz");
        check_error(&doc, DecodeError::Shape);
        doc["findings"][0]["fix"]["plan"]["digest"] = json!(bad_digest());
        check_error(&doc, DecodeError::PlanDigest);
        doc["findings"]
            .as_array_mut()
            .unwrap()
            .push(json!({"unknown": 1}));
        check_error(&doc, DecodeError::Shape);

        let mut doc = initial.clone();
        doc["findings"][0]["fix"]["plan"]["digest"] = json!(bad_digest());
        doc["findings"][0]["summary"] = json!("x".repeat(201));
        check_error(&doc, DecodeError::Shape);
        let invalid = initial.to_string().replace(
            r#""timeout_ms":1234"#,
            r#""timeout_ms":1234,"timeout_ms":1234"#,
        );
        assert_ne!(invalid, initial.to_string());
        assert_eq!(
            CheckOutput::decode(invalid.as_bytes()),
            Err(DecodeError::Shape)
        );
        for token in ["10.0", "1e3", "-0", "null"] {
            let invalid = initial
                .to_string()
                .replace(r#""timeout_ms":1234"#, &format!(r#""timeout_ms":{token}"#));
            assert_eq!(
                CheckOutput::decode(invalid.as_bytes()),
                Err(DecodeError::Shape),
                "timeout {token}"
            );
            let invalid = initial
                .to_string()
                .replace(r#""value":10"#, &format!(r#""value":{token}"#));
            assert_eq!(
                CheckOutput::decode(invalid.as_bytes()),
                Err(DecodeError::Shape),
                "count {token}"
            );
        }
    }

    #[test]
    fn check_shape_enforces_ids_evidence_and_summary() {
        let initial = check_doc();
        let mut duplicate = initial.clone();
        duplicate["findings"]
            .as_array_mut()
            .unwrap()
            .push(initial["findings"][0].clone());
        check_error(&duplicate, DecodeError::Shape);
        let mut duplicate = initial.clone();
        duplicate["findings"][0]["evidence"]
            .as_array_mut()
            .unwrap()
            .push(initial["findings"][0]["evidence"][0].clone());
        check_error(&duplicate, DecodeError::Shape);
        for (key, invalid) in [
            ("id", json!("01456789abcdef0123456789abcdef0123456789a")),
            ("id", json!("sk-example")),
            ("severity", json!({"warning": null})),
            ("summary", json!("")),
            ("summary", json!("line\nline")),
            ("summary", json!("control\u{85}")),
            ("summary", json!("é".repeat(201))),
            ("evidence", Value::Null),
            (
                "evidence",
                json!([{"name": "code", "kind": "count", "value": "text"}]),
            ),
            (
                "evidence",
                json!([{"name": "ckh_secret", "kind": "count", "value": 1}]),
            ),
            (
                "evidence",
                json!([{"name": "path", "kind": "path", "value": ["data", "fake/file"]}]),
            ),
            (
                "evidence",
                json!([{"name": "path", "kind": "path", "value": {"category": "data", "path": "fake/file", "surprise": null}}]),
            ),
            (
                "evidence",
                json!([{"name": "n_jobs", "kind": "count", "value": 9007199254740992_u64}]),
            ),
            (
                "evidence",
                json!((1..=17)
                    .map(|n| json!({"name": format!("n_{n}"), "kind": "count", "value": n}))
                    .collect::<Vec<_>>()),
            ),
            ("fix", json!({"class": "safe", "needs_stop": true})),
            (
                "fix",
                json!({"class": "none", "needs_stop": false, "plan": null}),
            ),
            (
                "fix",
                json!({"class": "none", "needs_stop": false, "plan": initial["findings"][0]["fix"]["plan"].clone()}),
            ),
        ] {
            let mut doc = initial.clone();
            doc["findings"][0][key] = invalid;
            check_error(&doc, DecodeError::Shape);
        }
        let mut boundary = initial;
        boundary["findings"][0]["summary"] = json!("é".repeat(200));
        assert!(CheckOutput::decode(boundary.to_string().as_bytes()).is_ok());
    }

    #[test]
    fn fix_requires_valid_codes_and_optional_evidence_is_never_null() {
        let duplicate = r#"{"schema":"ck.doctor-fix/v1","result":"fixed","\u0072esult":"fixed"}"#;
        assert_eq!(
            FixOutput::decode(duplicate.as_bytes()),
            Err(DecodeError::Shape)
        );
        assert_eq!(
            FixOutput::decode(format!("{duplicate}x").as_bytes()),
            Err(DecodeError::ExtraOutput)
        );
        let duplicate_path = r#"{"schema":"ck.doctor-fix/v1","result":"fixed","evidence":[{"name":"path","kind":"path","value":{"category":"data","path":"a","path":"b"}}]}"#;
        assert_eq!(
            FixOutput::decode(duplicate_path.as_bytes()),
            Err(DecodeError::Shape)
        );
        for bad in [
            json!({"result": "fixed", "code": "module_running"}),
            json!({"result": "fixed", "code": null}),
            json!({"result": "refused"}),
            json!({"result": "failed", "code": "sk-secret"}),
            json!({"result": "failed", "code": "a".repeat(32)}),
            json!({"result": {"fixed": null}}),
            json!({"result": "fixed", "evidence": null}),
            json!({"result": "fixed", "extra": true}),
            json!({"result": "fixed", "evidence": [{"name": "ratio", "kind": "ratio", "value": null}]}),
            json!({"result": "fixed", "evidence": (1..=17).map(|n| json!({"name": format!("n_{n}"), "kind": "count", "value": n})).collect::<Vec<_>>()}),
        ] {
            let mut doc = bad;
            doc["schema"] = json!("ck.doctor-fix/v1");
            fix_error(&doc, DecodeError::Shape);
        }
        let evidence = Evidence::new(
            "path",
            EvidenceValue::Path {
                category: PathCategory::Data,
                path: "fake/path".into(),
            },
        )
        .unwrap();
        let output = FixOutput::refused("module_running")
            .unwrap()
            .with_evidence(vec![evidence])
            .unwrap();
        let bytes = serde_json::to_vec(&output).unwrap();
        assert_eq!(FixOutput::decode(&bytes), Ok(output));
        let output = FixOutput::failed("check_failed").unwrap();
        assert_eq!(output.result(), FixResult::Failed);
        assert_eq!(
            FixOutput::decode(&serde_json::to_vec(&output).unwrap()),
            Ok(output)
        );
    }

    fn manifest(templates: Vec<Template>) -> Manifest {
        Manifest::new(
            "demo".parse().unwrap(),
            "https://github.com/ExampleOwner/demo.repo",
            vec![LogSource::new("logs/*.log").unwrap()],
            templates,
        )
        .unwrap()
    }

    fn manifest_doc() -> Value {
        let mut templates = vec![
            Template::new(
                "demo::nested",
                "check",
                vec![
                    TemplateField::new("n_bytes").unwrap(),
                    TemplateField::new("session_id").unwrap(),
                ],
            )
            .unwrap(),
            Template::new("demo", "fix", vec![]).unwrap(),
        ];
        templates.sort_by(|a, b| a.id().cmp(b.id()));
        serde_json::to_value(manifest(templates)).unwrap()
    }

    fn manifest_error(doc: &Value, error: DecodeError) {
        assert_eq!(Manifest::decode(doc.to_string().as_bytes()), Err(error));
    }

    fn digest_value(templates: &Value) -> String {
        format!(
            "sha256:{:x}",
            Sha256::digest(serde_jcs::to_vec(templates).unwrap())
        )
    }

    #[test]
    fn manifest_checks_metadata_grammars_and_secrets() {
        let initial = manifest_doc();
        for glob in [
            "/a", "../a", "a/../b", "a/./b", "a//b", "a/", "**/a", "a**", "a\\b", "",
        ] {
            let mut doc = initial.clone();
            doc["logs"][0]["glob"] = json!(glob);
            manifest_error(&doc, DecodeError::Shape);
        }
        let mut doc = initial.clone();
        doc["logs"][0]["dir"] = json!("state");
        manifest_error(&doc, DecodeError::Shape);
        for module in [
            "line\nline",
            &"a".repeat(32),
            "sk-secret",
            "-demo",
            "demo_state",
        ] {
            let mut doc = initial.clone();
            doc["module_id"] = json!(module);
            manifest_error(&doc, DecodeError::Shape);
        }
        let mut doc = initial.clone();
        doc["logs"] = json!([]);
        manifest_error(&doc, DecodeError::Shape);
        let mut doc = initial;
        doc["templates"][0]["target"] = json!("x\u{85}");
        manifest_error(&doc, DecodeError::Shape);
    }

    #[test]
    fn manifest_distinguishes_invalid_repository_url_from_shape() {
        let initial = manifest_doc();
        for url in [
            "http://github.com/owner/demo",
            "https://example.com/owner/demo",
        ] {
            let mut doc = initial.clone();
            doc["repository"] = json!(url);
            manifest_error(&doc, DecodeError::Repository);
        }
        for repo in [
            "owner/..",
            "owner/demo.git",
            "owner/path/demo",
            "owner/demo#ref",
            "owner/demo?key=foo",
            "sk-secret/demo",
            "owner/sk-secret",
            &format!("{}/demo", "a".repeat(32)),
            &format!("owner/{}", "a".repeat(32)),
        ] {
            let mut doc = initial.clone();
            doc["repository"] = json!(format!("https://github.com/{repo}"));
            manifest_error(&doc, DecodeError::Repository);
        }
        let mut doc = initial.clone();
        doc["repository"] = json!("not a URL");
        doc["logs"] = json!([]);
        manifest_error(&doc, DecodeError::Shape);
        let mut doc = initial;
        doc["repository"] = json!(null);
        manifest_error(&doc, DecodeError::Shape);
        doc.as_object_mut().unwrap().remove("repository");
        manifest_error(&doc, DecodeError::Shape);
    }

    #[test]
    fn manifest_stage_order_separates_shape_ids_and_templates_digest() {
        let initial = manifest_doc();
        let decoded = Manifest::decode(initial.to_string().as_bytes()).unwrap();
        assert_eq!(template_id("demo::nested", "check"), "b50820cc1e54");
        let bytes = decoded.to_canonical_bytes().unwrap();
        assert_eq!(
            bytes,
            [serde_jcs::to_vec(&initial).unwrap(), b"\n".to_vec()].concat()
        );
        assert_eq!(Manifest::decode(&bytes), Ok(decoded));
        let template = initial["templates"]
            .as_array()
            .unwrap()
            .iter()
            .position(|t| t["text"] == "check")
            .unwrap();

        let mut doc = initial.clone();
        doc["templates_digest"] = json!(bad_digest());
        manifest_error(&doc, DecodeError::TemplatesDigest);
        doc["templates"][template]["fields"][1]["kind"] = json!("code");
        manifest_error(&doc, DecodeError::Shape);
        doc["templates_digest"] = json!(digest_value(&doc["templates"]));
        manifest_error(&doc, DecodeError::Shape);
        doc["templates"][template]["fields"][1] = json!({"name": "a".repeat(32), "kind": "ref"});
        doc["templates_digest"] = json!(digest_value(&doc["templates"]));
        manifest_error(&doc, DecodeError::Shape);
        doc["templates"][template]["fields"][1] = doc["templates"][template]["fields"][0].clone();
        manifest_error(&doc, DecodeError::Shape);

        let mut doc = initial.clone();
        doc["templates"][0]["id"] = json!("00000000000A");
        manifest_error(&doc, DecodeError::Shape);
        doc["templates"][0]["id"] = json!("000000000000");
        manifest_error(&doc, DecodeError::TemplateId);
        doc["templates_digest"] = json!(digest_value(&doc["templates"]));
        manifest_error(&doc, DecodeError::TemplateId);
        let mut doc = initial.clone();
        doc["templates"][0]["id"] = doc["templates"][1]["id"].clone();
        manifest_error(&doc, DecodeError::Shape);
        let mut doc = initial;
        doc["templates"].as_array_mut().unwrap().reverse();
        manifest_error(&doc, DecodeError::Shape);
    }

    #[test]
    fn manifest_limit_includes_its_final_newline() {
        fn templates(extra_bytes: usize) -> Vec<Template> {
            let mut templates: Vec<_> = (0..1024)
                .map(|index| {
                    let length = 1 + extra_bytes.saturating_sub(index * 1023).min(1023);
                    Template::new(format!("demo::{index:04}"), "x".repeat(length), vec![]).unwrap()
                })
                .collect();
            templates.sort_by(|a, b| a.id().cmp(b.id()));
            templates
        }
        let base = manifest(templates(0)).to_canonical_bytes().unwrap().len();
        let exact = manifest(templates(MAX_OUTPUT_BYTES - base));
        let mut bytes = exact.to_canonical_bytes().unwrap();
        assert_eq!(bytes.len(), 1_048_576);
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert_eq!(Manifest::decode(&bytes), Ok(exact));
        bytes.push(b' ');
        assert_eq!(Manifest::decode(&bytes), Err(DecodeError::TooLarge));
        let oversized = manifest(templates(MAX_OUTPUT_BYTES + 1 - base));
        assert_eq!(oversized.to_canonical_bytes(), Err(DecodeError::TooLarge));
    }

    #[test]
    fn field_kind_rules_prefer_prefixes_and_ignore_unlisted_names() {
        use EvidenceKind::*;
        for (name, expected) in [
            ("is_available", Bool),
            ("has_file", Bool),
            ("n_bytes", Count),
            ("ok", Bool),
            ("elapsed_ms", DurationMs),
            ("duration_ms", DurationMs),
            ("timeout_ms", DurationMs),
            ("bytes", Bytes),
            ("count", Count),
            ("ratio", Ratio),
            ("version", Version),
            ("path", Path),
            ("code", Code),
            ("reason", Code),
            ("state", Code),
            ("cause", Code),
            ("outcome", Code),
            ("level", Code),
            ("kind", Code),
            ("session", Ref),
            ("agent", Ref),
            ("project", Ref),
            ("scope_ref", Ref),
            ("last_at_ms", TimestampMs),
            ("login_ms", DurationMs),
            ("read_bytes", Bytes),
            ("job_count", Count),
            ("read_ratio", Ratio),
            ("error_code", Code),
            ("job_reason", Code),
            ("job_state", Code),
            ("cli_version", Version),
            ("home_path", Path),
            ("home_dir", Path),
            ("log_file", Path),
            ("session_id", Ref),
            ("query_ref", Ref),
        ] {
            assert_eq!(field_kind(name), Some(expected), "{name}");
        }
        for name in [
            "extra",
            "self.session_id",
            "UPPER_path",
            "ckh_session_id",
            "sk-file",
            &format!("{}_file", "a".repeat(32)),
        ] {
            assert_eq!(field_kind(name), None, "{name}");
        }
    }

    #[test]
    fn secret_pass_orders_fleet_before_pem_email_and_tokens() {
        let text = concat!(
            "Authorization: Bearer sample | bEAreR\tsample | eyJone.two.three | ckh_demo | sk-demo | ghp_demo | github_pat_demo\n",
            "https://owner:sample@example.invalid/path?password=sample&other=yes\n",
            "-----BEGIN RSA PRIVATE KEY-----\nbody\n-----END EC PRIVATE KEY-----\n",
            "survives canary.person@example.invalid sha256:0123456789abcdef0123456789abcdef ",
            "0123456789abcdef0123456789abcdefREST_0123ABC_Z codes taxes-2025.pdf b50820cc1e54",
        );
        assert_eq!(redact_secrets(text), concat!(
            "Authorization: [redacted] | bEAreR [redacted] | [redacted] | [redacted] | [redacted] | [redacted] | [redacted]\n",
            "https://[redacted]@example.invalid/path?password=[redacted]&other=yes\n",
            "[redacted]\n",
            "survives [redacted] sha256:[redacted] [redacted] codes taxes-2025.pdf b50820cc1e54",
        ));
        assert_eq!(
            redact_secrets("before -----BEGIN PRIVATE KEY-----\nno END line"),
            "before [redacted]"
        );
        assert_eq!(redact_secrets("Zxc1".repeat(10).as_str()), "[redacted]");
        assert_eq!(redact_secrets("Zxc1".repeat(9).as_str()), "Zxc1".repeat(9));
        assert_eq!(redact_secrets("task-queue_backlog"), "task-queue_backlog");
    }

    #[test]
    fn constructors_preserve_validated_values_and_optional_members() {
        let id = FindingId::new("task-queue_backlog").unwrap();
        assert!(FindingId::new("sk-example").is_err());
        assert!(ModuleId::new("a".repeat(32)).is_err());
        let fix = Fix::safe(
            &id,
            true,
            vec!["Élaguer la file.".into(), "Rouvrir.".into()],
            1234,
        )
        .unwrap();
        assert_eq!(fix.plan().unwrap().digest(), PLAN_DIGEST);
        let wrong_id = FindingId::new("different").unwrap();
        assert_eq!(
            Finding::new(
                wrong_id,
                Severity::Info,
                "Wrong binding.",
                vec![],
                fix.clone()
            ),
            Err(DecodeError::PlanDigest)
        );
        let evidence = vec![
            Evidence::new("count", EvidenceValue::Count((1 << 53) - 1)).unwrap(),
            Evidence::new("bytes", EvidenceValue::Bytes(2)).unwrap(),
            Evidence::new("duration_ms", EvidenceValue::DurationMs(0.5)).unwrap(),
            Evidence::new("ratio", EvidenceValue::Ratio(-0.25)).unwrap(),
            Evidence::new("bool", EvidenceValue::Bool(true)).unwrap(),
            Evidence::new(
                "code",
                EvidenceValue::Code("0123456789abcdef0123456789abcdef".into()),
            )
            .unwrap(),
            Evidence::new(
                "version",
                EvidenceValue::Version("sk-CanaryKey0123456789abcdef".into()),
            )
            .unwrap(),
            Evidence::new("timestamp_ms", EvidenceValue::TimestampMs(2)).unwrap(),
            Evidence::new(
                "path",
                EvidenceValue::Path {
                    category: PathCategory::Other,
                    path: "fake/file".into(),
                },
            )
            .unwrap(),
            Evidence::new("session", EvidenceValue::Ref("opaque-ref".into())).unwrap(),
        ];
        assert_eq!(
            Evidence::new("ratio", EvidenceValue::Ratio(f64::INFINITY)),
            Err(DecodeError::Shape)
        );
        let finding =
            Finding::new(id, Severity::Warning, "Finding summary.", evidence, fix).unwrap();
        assert_eq!(
            CheckOutput::new(vec![finding.clone(), finding.clone()]),
            Err(DecodeError::Shape)
        );
        let check = CheckOutput::new(vec![finding]).unwrap();
        assert_eq!(
            CheckOutput::decode(&serde_json::to_vec(&check).unwrap()),
            Ok(check)
        );
    }
}
