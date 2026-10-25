//! The tests SPEC.md section 8 lists, one per rule, plus the trust rules for
//! supplied fragments and project keys. Each fails by name when its rule is
//! removed.

mod common;

use common::*;
use cortexkit_persona::{
    Approval, ApprovalSet, ApprovalState, Digest, Extension, FragmentOrigin, FragmentSources,
    LoadRequest, PersonaError, ProjectKey, RenderFailure, SourceRef, Tier, TrustClass,
};
use serde::{Deserialize, Serialize};

// 1. Unknown top-level keys are refused by name, unless registered.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Lifecycle {
    Seat,
    Hire,
}

#[test]
fn unknown_top_level_key_is_refused_unless_registered_as_an_extension() {
    let tiers = [base(&[("PERSONA.md", &full("lifecycle: seat\n", "Hi."))])];
    match load(&tiers) {
        Err(PersonaError::UnknownField { file, field }) => {
            assert_eq!(field, "lifecycle");
            assert_eq!(
                file,
                SourceRef::File {
                    tier: "base".into(),
                    path: "roles/writer/PERSONA.md".into()
                }
            );
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }

    let mut engine = engine();
    engine
        .register_extension(Extension::<Lifecycle>::new("lifecycle"))
        .unwrap();
    let role = engine
        .load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new()))
        .unwrap();
    assert_eq!(
        role.manifest().extension::<Lifecycle>("lifecycle"),
        Some(&Lifecycle::Seat)
    );

    // The registered schema is the validation.
    let bad = [base(&[(
        "PERSONA.md",
        &full("lifecycle: sometimes\n", "Hi."),
    )])];
    assert!(matches!(
        engine.load(LoadRequest::new(ROLE, &bad, &ApprovalSet::new())),
        Err(PersonaError::InvalidField { field, .. }) if field == "lifecycle"
    ));
}

// 2. Strict undefined.

#[test]
fn strict_undefined_missing_context_value_is_an_error_not_empty_text() {
    for (body, at) in [
        ("Hello {{ agent.nickname }}!", "agent.nickname"),
        ("{% if module.missing %}x{% endif %}", "module.missing"),
        ("{{ harness.capabilities.steer_native }}", "steer_native"),
    ] {
        let tiers = [base(&[("PERSONA.md", &full("", body))])];
        match render(&tiers) {
            Err(PersonaError::Render {
                failure: RenderFailure::UndefinedValue,
                snippet,
                origin,
                ..
            }) => {
                assert!(snippet.unwrap().contains(at), "{body}");
                assert!(matches!(origin, Some(SourceRef::File { .. })));
            }
            other => panic!("{body}: expected an undefined-value error, got {other:?}"),
        }
    }
}

#[test]
fn grant_maps_read_a_missing_name_as_false() {
    let body = "{{ tools.search }} {{ tools.shell }} {{ mcp.calendar }} {% if modules.memory %}m{% else %}none{% endif %}";
    let tiers = [base(&[("PERSONA.md", &full("", body))])];
    assert_eq!(text(&tiers), "true false false none");
}

// 3. Includes outside the declared fragments.

#[test]
fn include_outside_declared_fragments_is_refused() {
    let files = |body: &'static str| {
        [base(&[
            ("PERSONA.md", &full("fragments: [allowed]\n", body)),
            ("fragments/allowed.md", "A"),
            ("fragments/other.md", "B"),
        ])]
    };
    assert_eq!(text(&files("{% include \"allowed\" %}")), "A");
    match render(&files("{% include \"other\" %}")) {
        Err(PersonaError::UndeclaredReference {
            target, statement, ..
        }) => {
            assert_eq!(target, "other");
            assert_eq!(statement, "include");
        }
        other => panic!("expected UndeclaredReference, got {other:?}"),
    }
    assert!(matches!(
        render(&files("{% extends \"other\" %}")),
        Err(PersonaError::UndeclaredReference {
            statement: "extends",
            ..
        })
    ));
    assert!(matches!(
        render(&files("{% set n = \"allowed\" %}{% include n %}")),
        Err(PersonaError::NonLiteralReference { .. })
    ));
    assert!(matches!(
        render(&files("{% include \"allowed\" ignore missing %}")),
        Err(PersonaError::IgnoreMissing { .. })
    ));
}

// 4. A missing qualified fragment.

#[test]
fn missing_qualified_fragment_is_refused_by_name() {
    let tiers = [base(&[(
        "PERSONA.md",
        &full(
            "fragments: [mc/memory-profile@1]\n",
            "{% include \"mc/memory-profile@1\" %}",
        ),
    )])];
    let role = load(&tiers).unwrap();
    match role.render(&context(), &FragmentSources::new()) {
        Err(e @ PersonaError::MissingFragment { .. }) => {
            assert!(e.to_string().contains("mc/memory-profile@1"), "{e}");
        }
        other => panic!("expected MissingFragment, got {other:?}"),
    }
    let supplied = FragmentSources::new()
        .with("mc/memory-profile@1", "profile")
        .unwrap();
    assert_eq!(role.render(&context(), &supplied).unwrap().text, "profile");
}

#[test]
fn missing_bare_fragment_is_refused_by_name() {
    let tiers = [base(&[("PERSONA.md", &full("fragments: [absent]\n", "x"))])];
    assert!(matches!(
        load(&tiers),
        Err(PersonaError::MissingFragment { fragment, .. }) if fragment == "absent"
    ));
}

// 5. Project-tier approval.

fn project(files: &[(&str, &str)]) -> Tier {
    tier("project", TrustClass::Repository, files)
}

fn ladder(project_persona: &str) -> [Tier; 2] {
    [
        base(&[
            (
                "PERSONA.md",
                &full("fragments: [note]\n", "Base {% include \"note\" %}."),
            ),
            ("fragments/note.md", "base note"),
        ]),
        project(&[
            ("PERSONA.md", project_persona),
            ("fragments/note.md", "project note"),
        ]),
    ]
}

const PROJECT: &str = "---\ndisplay_name: Project writer\ndisabled_blocks: []\n---\n";

fn approval(key: &ProjectKey, digest: Digest) -> Approval {
    Approval {
        module: "demo".into(),
        role: ROLE.into(),
        project_key: key.clone(),
        relative_path: "roles/writer".into(),
        layer_digest: digest,
    }
}

fn project_digest(tiers: &[Tier]) -> Digest {
    load(tiers).unwrap().tiers()[1].layer_digest.unwrap()
}

#[test]
fn unapproved_project_layer_is_skipped_whole_and_reported_unapproved() {
    let tiers = ladder(PROJECT);
    let key = ProjectKey::new("pj-1").unwrap();
    let out = load_approved(&tiers, &ApprovalSet::new(), &key)
        .unwrap()
        .render(&context(), &FragmentSources::new())
        .unwrap();
    // Neither its manifest field nor its fragment file applied.
    assert_eq!(out.text, "Base base note.");
    let t = &out.report.tiers[1];
    assert_eq!(t.name, "project");
    assert!(t.present && !t.used);
    assert_eq!(t.approval, ApprovalState::Unapproved);
    assert_eq!(t.layer_digest, Some(project_digest(&tiers)));
    let role = load_approved(&tiers, &ApprovalSet::new(), &key).unwrap();
    assert_eq!(role.manifest().display_name(), "Writer");
}

#[test]
fn approved_project_layer_applies_and_one_whose_bytes_change_is_reported_changed_and_skipped() {
    let key = ProjectKey::new("pj-1").unwrap();
    let tiers = ladder(PROJECT);
    let approvals: ApprovalSet = [approval(&key, project_digest(&tiers))]
        .into_iter()
        .collect();

    let role = load_approved(&tiers, &approvals, &key).unwrap();
    assert_eq!(role.manifest().display_name(), "Project writer");
    let out = role.render(&context(), &FragmentSources::new()).unwrap();
    assert_eq!(out.text, "Base project note.");
    assert_eq!(out.report.tiers[1].approval, ApprovalState::Approved);
    assert!(out.report.tiers[1].used);

    // The same path, edited after approval.
    let edited = ladder("---\ndisplay_name: Sneaky writer\n---\n");
    let role = load_approved(&edited, &approvals, &key).unwrap();
    assert_eq!(role.manifest().display_name(), "Writer");
    let out = role.render(&context(), &FragmentSources::new()).unwrap();
    assert_eq!(out.text, "Base base note.");
    assert_eq!(out.report.tiers[1].approval, ApprovalState::Changed);
    assert!(!out.report.tiers[1].used);
    assert_ne!(
        out.report.tiers[1].layer_digest,
        Some(project_digest(&tiers))
    );
}

#[test]
fn approval_keyed_to_a_different_project_key_does_not_apply() {
    let tiers = ladder(PROJECT);
    let mine = ProjectKey::new("pj-mine").unwrap();
    let theirs = ProjectKey::new("pj-theirs").unwrap();
    let approvals: ApprovalSet = [approval(&theirs, project_digest(&tiers))]
        .into_iter()
        .collect();
    let role = load_approved(&tiers, &approvals, &mine).unwrap();
    assert_eq!(role.tiers()[1].approval, ApprovalState::Unapproved);
    assert!(!role.tiers()[1].used);
    assert_eq!(role.manifest().display_name(), "Writer");
    // And with no project key at all, nothing can match.
    let role = engine()
        .load(LoadRequest::new(ROLE, &tiers, &approvals))
        .unwrap();
    assert_eq!(role.tiers()[1].approval, ApprovalState::Unapproved);
}

#[test]
fn approval_for_another_module_role_or_path_does_not_apply() {
    let tiers = ladder(PROJECT);
    let key = ProjectKey::new("pj-1").unwrap();
    let good = approval(&key, project_digest(&tiers));
    for wrong in [
        Approval {
            module: "other".into(),
            ..good.clone()
        },
        Approval {
            role: "reader".into(),
            ..good.clone()
        },
        Approval {
            relative_path: "roles/other".into(),
            ..good.clone()
        },
    ] {
        let approvals: ApprovalSet = [wrong].into_iter().collect();
        let role = load_approved(&tiers, &approvals, &key).unwrap();
        assert_eq!(role.tiers()[1].approval, ApprovalState::Unapproved);
    }
}

// 6. Line endings.

#[test]
fn crlf_and_lf_sources_give_the_same_digests_and_bytes() {
    let lf_persona = full(
        "fragments: [note]\n",
        "One {{ agent.name }}.\nTwo.\n{% include \"note\" %}\n",
    );
    let crlf_persona = lf_persona.replace('\n', "\r\n");
    let cr_persona = lf_persona.replace('\n', "\r");
    let render_with = |persona: &str, note: &str| {
        render(&[base(&[
            ("PERSONA.md", persona),
            ("fragments/note.md", note),
        ])])
        .unwrap()
    };
    let lf = render_with(&lf_persona, "Note.\nEnd.\n");
    for (persona, note) in [
        (crlf_persona.as_str(), "Note.\r\nEnd.\r\n"),
        (cr_persona.as_str(), "Note.\rEnd.\r"),
    ] {
        let other = render_with(persona, note);
        assert_eq!(other.text, lf.text);
        assert_eq!(other.digest, lf.digest);
        assert_eq!(
            other.report.tiers[0].layer_digest,
            lf.report.tiers[0].layer_digest
        );
        assert_eq!(other.report.fragments, lf.report.fragments);
        assert_eq!(other.report.source_revision, lf.report.source_revision);
    }
    assert_eq!(lf.text, "One Ada.\nTwo.\nNote.\nEnd.");
}

#[test]
fn context_values_pass_verbatim_and_supplied_fragments_are_lf_normalised() {
    let tiers = [base(&[(
        "PERSONA.md",
        &full(
            "fragments: [mc/profile@1]\n",
            "{{ agent.name }}|{% include \"mc/profile@1\" %}",
        ),
    )])];
    let mut ctx = context();
    ctx.agent.name = Some("a\r\nb\rc".into());
    let role = load(&tiers).unwrap();
    let crlf = FragmentSources::new()
        .with("mc/profile@1", "x\r\ny")
        .unwrap();
    let lf = FragmentSources::new().with("mc/profile@1", "x\ny").unwrap();
    let a = role.render(&ctx, &crlf).unwrap();
    let b = role.render(&ctx, &lf).unwrap();
    assert_eq!(a.text, "a\r\nb\rc|x\ny");
    assert_eq!(a.digest, Digest::of(a.text.as_bytes()));
    assert_eq!(a.report, b.report);
}

// 7. Determinism.

#[test]
fn same_inputs_render_byte_identical_twice_and_the_report_is_identical() {
    let make = || {
        [
            base(&[
                (
                    "PERSONA.md",
                    &full(
                        "fragments: [note, mc/p@2]\n",
                        "{% for k in tools %}{{ k }} {% endfor %}{% include \"note\" %} {% include \"mc/p@2\" %}",
                    ),
                ),
                ("fragments/note.md", "n"),
            ]),
            tier(
                "user",
                TrustClass::Operator,
                &[("PERSONA.md", &partial("version: 3\n", ""))],
            ),
        ]
    };
    let supplied = FragmentSources::new().with("mc/p@2", "p").unwrap();
    let a = load(&make())
        .unwrap()
        .render(&context(), &supplied)
        .unwrap();
    let b = load(&make())
        .unwrap()
        .render(&context(), &supplied)
        .unwrap();
    assert_eq!(a.bytes(), b.bytes());
    assert_eq!(a.digest, b.digest);
    assert_eq!(
        serde_json::to_vec(&a.report).unwrap(),
        serde_json::to_vec(&b.report).unwrap()
    );
    assert_eq!(a.text, "search n p");
}

// 8. Merging.

#[test]
fn disabled_blocks_union_across_layers_and_other_fields_replace() {
    let body =
        "[{% block a %}A{% endblock %}|{% block b %}B{% endblock %}|{% block c %}C{% endblock %}]";
    let tiers = [
        base(&[
            (
                "PERSONA.md",
                &full("disabled_blocks: [a]\nfragments: [x]\n", body),
            ),
            ("fragments/x.md", ""),
        ]),
        tier(
            "user",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial(
                    "version: 7\ndisplay_name: Renamed\ndisabled_blocks: [b]\nfragments: []\n",
                    "",
                ),
            )],
        ),
    ];
    let role = load(&tiers).unwrap();
    assert_eq!(role.manifest().version(), 7);
    assert_eq!(role.manifest().display_name(), "Renamed");
    assert!(role.manifest().fragments().is_empty());
    assert_eq!(
        role.manifest().disabled_blocks().iter().collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        role.render(&context(), &FragmentSources::new())
            .unwrap()
            .text,
        "[||C]"
    );
}

// Qualified ids and supplied fragments.

#[test]
fn repository_bundle_file_named_like_a_qualified_id_never_resolves_as_a_fragment() {
    let dir = tempfile::tempdir().unwrap();
    let layer = dir.path().join(".cortexkit/demo/roles/writer");
    std::fs::create_dir_all(layer.join("fragments/mc")).unwrap();
    std::fs::write(
        layer.join("PERSONA.md"),
        full(
            "fragments: [mc/memory-profile@1]\n",
            "[{% include \"mc/memory-profile@1\" %}]",
        ),
    )
    .unwrap();
    // Every spelling a repository could try for the qualified id.
    std::fs::write(layer.join("fragments/mc/memory-profile@1.md"), "FROM REPO").unwrap();
    std::fs::write(layer.join("fragments/memory-profile@1.md"), "FROM REPO").unwrap();
    std::fs::write(layer.join("fragments/mc-memory-profile@1.md"), "FROM REPO").unwrap();
    let tiers = [Tier::directory(
        "project",
        TrustClass::Repository,
        dir.path(),
        ".cortexkit/demo/roles",
    )
    .unwrap()];
    let key = ProjectKey::new("pj-1").unwrap();
    // Unapproved, the repository layer never applies, with or without a key.
    let no_key = load(&tiers).unwrap_err();
    assert!(matches!(no_key, PersonaError::NoLayers { .. }));
    let unapproved = engine()
        .load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new()).project_key(&key))
        .unwrap_err();
    assert!(matches!(unapproved, PersonaError::NoLayers { .. }));

    // Approve the layer so the repository text is in play.
    let probe = Tier::directory(
        "project",
        TrustClass::Operator,
        dir.path(),
        ".cortexkit/demo/roles",
    )
    .unwrap();
    let layer_digest = load(&[probe]).unwrap().tiers()[0].layer_digest.unwrap();
    let approvals: ApprovalSet = [approval(&key, layer_digest)]
        .into_iter()
        .map(|a| Approval {
            relative_path: ".cortexkit/demo/roles/writer".into(),
            ..a
        })
        .collect();
    let role = load_approved(&tiers, &approvals, &key).unwrap();
    assert_eq!(role.tiers()[0].approval, ApprovalState::Approved);
    // The stray files are not part of the layer at all: its digest is that of
    // the same PERSONA.md alone.
    let persona = std::fs::read_to_string(layer.join("PERSONA.md")).unwrap();
    let alone = load(&[base(&[("PERSONA.md", &persona)])]).unwrap().tiers()[0]
        .layer_digest
        .unwrap();
    assert_eq!(role.tiers()[0].layer_digest, Some(alone));
    assert!(matches!(
        role.render(&context(), &FragmentSources::new()),
        Err(PersonaError::MissingFragment { fragment, .. }) if fragment == "mc/memory-profile@1"
    ));
    let supplied = FragmentSources::new()
        .with("mc/memory-profile@1", "FROM MODULE")
        .unwrap();
    let out = role.render(&context(), &supplied).unwrap();
    assert_eq!(out.text, "[FROM MODULE]");
    assert_eq!(out.report.fragments[0].origin, FragmentOrigin::Supplied);
}

#[test]
fn supplied_fragment_digest_appears_in_the_report() {
    let tiers = [base(&[
        (
            "PERSONA.md",
            &full(
                "fragments: [note, mc/memory-profile@1]\n",
                "{% include \"note\" %} {% include \"mc/memory-profile@1\" %}",
            ),
        ),
        ("fragments/note.md", "n"),
    ])];
    let profile = "Remember the user's citation style.";
    let supplied = FragmentSources::new()
        .with("mc/memory-profile@1", profile)
        .unwrap();
    let out = load(&tiers).unwrap().render(&context(), &supplied).unwrap();
    let f = out
        .report
        .fragments
        .iter()
        .find(|f| f.id == "mc/memory-profile@1")
        .expect("listed");
    assert_eq!(f.origin, FragmentOrigin::Supplied);
    assert_eq!(f.digest, Digest::of(profile.as_bytes()));
    assert!(f.included);
    let note = &out.report.fragments[0];
    assert_eq!(note.id, "note");
    assert_eq!(
        note.origin,
        FragmentOrigin::Bundle {
            tier: "base".into()
        }
    );
    assert_eq!(note.digest, Digest::of(b"n"));
}

#[test]
fn a_supplied_source_must_use_a_qualified_id() {
    assert!(matches!(
        FragmentSources::new().with("note", "x"),
        Err(PersonaError::SuppliedFragmentNotQualified { .. })
    ));
}

// 9. Repeated front matter keys.

#[test]
fn a_repeated_front_matter_key_is_refused_by_name_at_any_depth() {
    let mut engine = engine();
    engine
        .register_extension(Extension::<serde_json::Value>::new("limits"))
        .unwrap();
    engine
        .register_extension(Extension::<serde_json::Value>::new("flows"))
        .unwrap();
    for (front, key) in [
        // The reader sees version 1; a last-wins parser would apply 9.
        ("version: 9\n", "version"),
        (
            "limits:\n  max_words: 100\n  max_words: 900\n",
            "limits.max_words",
        ),
        ("flows:\n  - {package: a, package: b}\n", "flows[0].package"),
    ] {
        let persona = format!("---\nname: writer\nversion: 1\ndisplay_name: Writer\n{front}---\nx");
        let tiers = [base(&[("PERSONA.md", &persona)])];
        match engine.load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new())) {
            Err(PersonaError::DuplicateKey { key: found, file }) => {
                assert_eq!(found, key);
                assert_eq!(
                    file,
                    SourceRef::File {
                        tier: "base".into(),
                        path: "roles/writer/PERSONA.md".into()
                    }
                );
            }
            other => panic!("{front:?}: expected DuplicateKey {key}, got {other:?}"),
        }
    }
}
