//! The engine's other rules from SPEC.md: required fields, fragment and body
//! resolution across tiers, explicit parents, extension merge rules, the
//! include closure, the sealed renderer and the default ladder.

mod common;

use std::sync::Arc;

use common::*;
use cortexkit_persona::{
    ApprovalSet, ApprovalState, DefaultLadder, EmbeddedTier, Extension, FragmentSources,
    LoadRequest, MergeRule, PersonaError, RenderContext, RenderFailure, SourceRef, Tier,
    TrustClass,
};
use serde::{Deserialize, Serialize};

#[test]
fn first_applied_layer_must_supply_the_required_fields() {
    let tiers = [base(&[(
        "PERSONA.md",
        &partial("name: writer\nversion: 1\n", "x"),
    )])];
    assert!(matches!(
        load(&tiers),
        Err(PersonaError::MissingField { field, .. }) if field == "display_name"
    ));

    let mut engine = engine();
    engine
        .register_extension(Extension::<String>::new("lifecycle").required())
        .unwrap();
    let tiers = [
        base(&[("PERSONA.md", &full("", "x"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("lifecycle: seat\n", ""))],
        ),
    ];
    assert!(matches!(
        engine.load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new())),
        Err(PersonaError::MissingField { field, .. }) if field == "lifecycle"
    ));
}

#[test]
fn a_later_layer_may_be_partial_but_a_present_name_must_match_the_role() {
    let tiers = [
        base(&[("PERSONA.md", &full("", "x"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("name: reader\n", ""))],
        ),
    ];
    assert!(matches!(
        load(&tiers),
        Err(PersonaError::InvalidField { field, .. }) if field == "name"
    ));
}

#[test]
fn each_fragment_comes_from_the_most_specific_tier_and_never_concatenates() {
    let tiers = [
        base(&[
            (
                "PERSONA.md",
                &full(
                    "fragments: [a, b]\n",
                    "{% include \"a\" %}{% include \"b\" %}",
                ),
            ),
            ("fragments/a.md", "base-a "),
            ("fragments/b.md", "base-b"),
        ]),
        tier(
            "user",
            TrustClass::Operator,
            &[
                ("PERSONA.md", &partial("", "")),
                ("fragments/a.md", "user-a "),
            ],
        ),
    ];
    let out = render(&tiers).unwrap();
    assert_eq!(out.text, "user-a base-b");
    assert_eq!(out.report.entry_tier, "base");
}

#[test]
fn a_body_extends_the_previous_non_empty_body_and_the_last_is_the_entry() {
    let tiers = [
        base(&[(
            "PERSONA.md",
            &full(
                "",
                "<{% block a %}base-a{% endblock %}|{% block b %}base-b{% endblock %}>",
            ),
        )]),
        tier(
            "user",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial("", "{% block a %}user-a{% endblock %}"),
            )],
        ),
        // An empty body leaves the previous one standing.
        tier(
            "empty",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("", "\n  \n"))],
        ),
        tier(
            "workspace",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial("", "{% block b %}ws-b {{ super() }}{% endblock %}"),
            )],
        ),
    ];
    let out = render(&tiers).unwrap();
    assert_eq!(out.text, "<user-a|ws-b base-b>");
    assert_eq!(out.report.entry_tier, "workspace");
}

#[test]
fn an_inheriting_body_with_text_outside_blocks_is_refused() {
    let tiers = [
        base(&[("PERSONA.md", &full("", "{% block a %}A{% endblock %}"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("", "Replacement text"))],
        ),
    ];
    assert!(matches!(
        render(&tiers),
        Err(PersonaError::ContentOutsideBlocks { origin, .. })
            if matches!(&origin, SourceRef::File { tier, .. } if tier == "user")
    ));
}

#[test]
fn an_explicit_extends_names_a_lower_parent_of_the_same_role() {
    let tiers = [
        base(&[("PERSONA.md", &full("", "<{% block a %}base{% endblock %}>"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial("", "{% block a %}user{% endblock %}"),
            )],
        ),
        tier(
            "workspace",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial(
                    "extends: {role: writer, tier: base}\n",
                    "{% block a %}ws+{{ super() }}{% endblock %}",
                ),
            )],
        ),
    ];
    // The user body is skipped: the workspace body names base as its parent.
    assert_eq!(text(&tiers), "<ws+base>");

    let upward = [
        base(&[(
            "PERSONA.md",
            &full("extends: {role: writer, tier: user}\n", "x"),
        )]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("", "y"))],
        ),
    ];
    assert!(matches!(
        load(&upward),
        Err(PersonaError::ExtendsUnavailable { tier, .. }) if tier == "user"
    ));
}

#[test]
fn an_explicit_extends_may_name_another_roles_layer_and_reports_it() {
    let mut bundles = EmbeddedTier::new();
    bundles
        .insert(
            "frame",
            "PERSONA.md",
            "---\nname: frame\nversion: 1\ndisplay_name: Frame\n---\n[{% block body %}{% endblock %}]",
        )
        .unwrap();
    bundles
        .insert(
            ROLE,
            "PERSONA.md",
            full("", "{% block body %}framed{% endblock %}"),
        )
        .unwrap();
    let base_tier = Tier::embedded("base", TrustClass::Operator, bundles).unwrap();
    let user = tier(
        "user",
        TrustClass::Operator,
        &[(
            "PERSONA.md",
            &partial(
                "extends: {role: frame, tier: base}\n",
                "{% block body %}user {{ agent.name }}{% endblock %}",
            ),
        )],
    );
    let out = render(&[base_tier, user]).unwrap();
    assert_eq!(out.text, "[user Ada]");
    assert_eq!(out.report.parents.len(), 1);
    assert_eq!(out.report.parents[0].role, "frame");
    assert_eq!(out.report.parents[0].approval, ApprovalState::NotRequired);
    assert!(out.report.parents[0].layer_digest.is_some());
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Flow {
    package: String,
    version: u32,
}

#[test]
fn an_extension_merges_by_its_registered_rule() {
    let mut engine = engine();
    engine
        .register_extension(Extension::<Vec<String>>::new("flows_remove"))
        .unwrap();
    // Merge by package id, after removing the layer's `flows_remove` ids.
    engine
        .register_extension(
            Extension::<Vec<Flow>>::new("flows").merge(MergeRule::Custom(Arc::new(|input| {
                let mut flows = input.inherited.unwrap_or_default();
                if let Some(remove) = input.layer.get::<Vec<String>>("flows_remove") {
                    flows.retain(|f| !remove.contains(&f.package));
                }
                for flow in input.present.unwrap_or_default() {
                    match flows.iter_mut().find(|f| f.package == flow.package) {
                        Some(existing) => *existing = flow,
                        None => flows.push(flow),
                    }
                }
                Ok(Some(flows))
            }))),
        )
        .unwrap();
    let tiers = [
        base(&[(
            "PERSONA.md",
            &full(
                "flows: [{package: a, version: 1}, {package: b, version: 1}]\n",
                "x",
            ),
        )]),
        tier(
            "user",
            TrustClass::Operator,
            &[(
                "PERSONA.md",
                &partial(
                    "flows_remove: [a]\nflows: [{package: b, version: 2}, {package: c, version: 1}]\n",
                    "",
                ),
            )],
        ),
    ];
    let role = engine
        .load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new()))
        .unwrap();
    let flows = role.manifest().extension::<Vec<Flow>>("flows").unwrap();
    let got: Vec<(&str, u32)> = flows
        .iter()
        .map(|f| (f.package.as_str(), f.version))
        .collect();
    assert_eq!(got, [("b", 2), ("c", 1)]);
    // The composed value is part of the manifest JSON.
    assert_eq!(role.manifest().to_json()["flows"][1]["package"], "c");
}

#[test]
fn an_extension_cannot_shadow_a_core_field_or_register_twice() {
    let mut engine = engine();
    assert!(matches!(
        engine.register_extension(Extension::<String>::new("fragments")),
        Err(PersonaError::ExtensionConflict { .. })
    ));
    engine
        .register_extension(Extension::<String>::new("routing"))
        .unwrap();
    assert!(matches!(
        engine.register_extension(Extension::<u8>::new("routing")),
        Err(PersonaError::ExtensionConflict { .. })
    ));
    assert!(matches!(
        engine.register_extension(Extension::<u8>::new("Bad-Key")),
        Err(PersonaError::InvalidId { .. })
    ));
}

#[test]
fn source_revision_counts_only_fragments_in_the_include_closure() {
    let tiers = [base(&[(
        "PERSONA.md",
        &full(
            "fragments: [mc/used@1, mc/unused@1]\n",
            "{% if false %}{% include \"mc/used@1\" %}{% endif %}",
        ),
    )])];
    let role = load(&tiers).unwrap();
    let render_with = |used: &str, unused: &str| {
        let s = FragmentSources::new()
            .with("mc/used@1", used)
            .unwrap()
            .with("mc/unused@1", unused)
            .unwrap();
        role.render(&context(), &s).unwrap().report
    };
    let a = render_with("u1", "x1");
    // A fragment outside the closure changes nothing.
    assert_eq!(a.source_revision, render_with("u1", "x2").source_revision);
    // One inside it counts, even behind a branch that is not taken.
    assert_ne!(a.source_revision, render_with("u2", "x1").source_revision);
    assert!(a.fragments[0].included && !a.fragments[1].included);
}

#[test]
fn source_revision_changes_with_the_composed_manifest_and_layer_digests() {
    let rev =
        |front: &str, body: &str| render(&[base(&[("PERSONA.md", &full(front, body))])]).unwrap();
    let a = rev("", "x");
    assert_ne!(
        a.report.source_revision,
        rev("", "y").report.source_revision
    );
    let b = rev("", "x");
    assert_eq!(a.report.source_revision, b.report.source_revision);
    let c = render(&[
        base(&[("PERSONA.md", &full("", "x"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", &partial("version: 2\n", ""))],
        ),
    ])
    .unwrap();
    assert_eq!(c.text, a.text);
    assert_ne!(c.report.source_revision, a.report.source_revision);
}

#[test]
fn an_unknown_disabled_block_is_refused() {
    let tiers = [base(&[(
        "PERSONA.md",
        &full("disabled_blocks: [nope]\n", "{% block a %}A{% endblock %}"),
    )])];
    assert!(matches!(
        render(&tiers),
        Err(PersonaError::UnknownDisabledBlock { block, .. }) if block == "nope"
    ));
}

#[test]
fn disabled_blocks_reach_blocks_of_a_fragment_the_root_body_extends() {
    let tiers = [base(&[
        (
            "PERSONA.md",
            &full(
                "fragments: [frame]\ndisabled_blocks: [footer]\n",
                "{% extends \"frame\" %}{% block main %}M{% endblock %}",
            ),
        ),
        (
            "fragments/frame.md",
            "<{% block main %}{% endblock %}{% block footer %}F{% endblock %}>",
        ),
    ])];
    assert_eq!(text(&tiers), "<M>");
}

#[test]
fn the_disabled_blocks_chain_follows_extends_through_fragments_but_not_includes() {
    let files = |disabled: &str| {
        [base(&[
            (
                "PERSONA.md",
                &full(
                    &format!("fragments: [outer, inner, side]\ndisabled_blocks: [{disabled}]\n"),
                    "{% extends \"outer\" %}{% block main %}M{% endblock %}",
                ),
            ),
            // `outer` extends `inner`, two steps from the root body.
            (
                "fragments/outer.md",
                "{% extends \"inner\" %}{% block main %}{% endblock %}",
            ),
            (
                "fragments/inner.md",
                "<{% block main %}{% endblock %}|{% block deep %}D{% endblock %}|{% include \"side\" %}>",
            ),
            ("fragments/side.md", "{% block aside %}S{% endblock %}"),
        ])]
    };
    // A block two `extends` steps down is in the chain.
    assert_eq!(text(&files("deep")), "<M||S>");
    // A block of an included fragment is not.
    assert!(matches!(
        render(&files("aside")),
        Err(PersonaError::UnknownDisabledBlock { block, .. }) if block == "aside"
    ));
}

#[test]
fn rendering_touches_no_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let layer = dir.path().join("roles/writer");
    std::fs::create_dir_all(layer.join("fragments")).unwrap();
    std::fs::write(
        layer.join("PERSONA.md"),
        full("fragments: [n]\n", "{% include \"n\" %}"),
    )
    .unwrap();
    std::fs::write(layer.join("fragments/n.md"), "on disk").unwrap();
    let tiers = [Tier::directory("base", TrustClass::Operator, dir.path(), "roles").unwrap()];
    let role = load(&tiers).unwrap();
    drop(dir);
    assert_eq!(
        role.render(&context(), &FragmentSources::new())
            .unwrap()
            .text,
        "on disk"
    );
}

#[test]
fn fuel_and_recursion_limits_stop_runaway_templates() {
    let loops = [base(&[(
        "PERSONA.md",
        &full(
            "",
            "{% for i in range(2000) %}{% for j in range(2000) %}{% endfor %}{% endfor %}",
        ),
    )])];
    assert!(matches!(
        render(&loops),
        Err(PersonaError::Render {
            failure: RenderFailure::OutOfFuel,
            ..
        })
    ));
    let recursive = [base(&[
        (
            "PERSONA.md",
            &full("fragments: [r]\n", "{% include \"r\" %}"),
        ),
        ("fragments/r.md", "x{% include \"r\" %}"),
    ])];
    assert!(matches!(
        render(&recursive),
        Err(PersonaError::Render {
            failure: RenderFailure::RecursionLimit,
            ..
        })
    ));
}

#[test]
fn an_error_inside_a_fragment_names_the_fragment_file() {
    let tiers = [base(&[
        (
            "PERSONA.md",
            &full("fragments: [inner]\n", "a {% include \"inner\" %}"),
        ),
        ("fragments/inner.md", "line one\n{{ agent.nickname }}"),
    ])];
    match render(&tiers) {
        Err(PersonaError::Render {
            origin: Some(SourceRef::File { path, .. }),
            failure: RenderFailure::UndefinedValue,
            line: Some(2),
            snippet: Some(snippet),
            ..
        }) => {
            assert_eq!(path, "roles/writer/fragments/inner.md");
            assert_eq!(snippet, "{{ agent.nickname }}");
        }
        other => panic!("expected an undefined value in the fragment, got {other:?}"),
    }
}

#[test]
fn the_render_context_is_closed_and_validated() {
    let mut json = serde_json::to_value(context()).unwrap();
    json["extra"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RenderContext<serde_json::Value>>(json).is_err());
    let mut json = serde_json::to_value(context()).unwrap();
    json["agent"]["nickname"] = serde_json::json!("x");
    assert!(serde_json::from_value::<RenderContext<serde_json::Value>>(json).is_err());
    let mut json = serde_json::to_value(context()).unwrap();
    json["env"]["os"] = serde_json::json!("plan9");
    assert!(serde_json::from_value::<RenderContext<serde_json::Value>>(json).is_err());

    let role = load(&[base(&[("PERSONA.md", &full("", "x"))])]).unwrap();
    let mut ctx = context();
    ctx.demand.iq = 101;
    assert!(matches!(
        role.render(&ctx, &FragmentSources::new()),
        Err(PersonaError::InvalidContext { field, .. }) if field == "demand.iq"
    ));
    let mut ctx = context();
    ctx.env.timezone = "Europe Istanbul".into();
    assert!(matches!(
        role.render(&ctx, &FragmentSources::new()),
        Err(PersonaError::InvalidContext { field, .. }) if field == "env.timezone"
    ));
}

#[test]
fn the_default_ladder_uses_the_config_directory_and_trust_classes() {
    let config = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let ladder = DefaultLadder::with_config_home("demo", EmbeddedTier::new(), config.path())
        .unwrap()
        .workspace("ws-1")
        .unwrap()
        .project(project.path());
    let tiers = ladder.tiers();
    let summary: Vec<(&str, TrustClass, String)> = tiers
        .iter()
        .map(|t| (t.name(), t.trust(), t.relative_path("writer")))
        .collect();
    assert_eq!(
        summary,
        [
            ("base", TrustClass::Operator, "roles/writer".to_string()),
            (
                "user",
                TrustClass::Operator,
                "cortexkit/demo/roles/writer".to_string()
            ),
            (
                "workspace",
                TrustClass::Operator,
                "cortexkit/demo/workspaces/ws-1/roles/writer".to_string()
            ),
            (
                "project",
                TrustClass::Repository,
                ".cortexkit/demo/roles/writer".to_string()
            ),
        ]
    );
    assert_eq!(tiers[1].root(), Some(config.path()));
    assert_eq!(tiers[3].root(), Some(project.path()));
    assert!(
        DefaultLadder::with_config_home("demo", EmbeddedTier::new(), config.path())
            .unwrap()
            .workspace("../escape")
            .is_err()
    );

    // The user tier is read from <config>/cortexkit/<module>/roles/<role>/.
    let user_dir = config.path().join("cortexkit/demo/roles/writer");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("PERSONA.md"), full("", "from user")).unwrap();
    assert_eq!(text(&tiers), "from user");
}

#[test]
fn config_home_is_absolute_or_refused() {
    match cortexkit_persona::config_home() {
        Ok(path) => assert!(path.is_absolute()),
        Err(PersonaError::ConfigHomeNotAbsolute { resolved }) => {
            assert!(!std::path::Path::new(&resolved).is_absolute())
        }
        Err(other) => panic!("unexpected error {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_repository_tier_refuses_symbolic_links() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("secret.txt");
    std::fs::write(&outside, "secret").unwrap();
    let layer = dir.path().join("p/roles/writer");
    std::fs::create_dir_all(&layer).unwrap();
    std::os::unix::fs::symlink(&outside, layer.join("PERSONA.md")).unwrap();
    let repo = [Tier::directory(
        "project",
        TrustClass::Repository,
        dir.path().join("p"),
        "roles",
    )
    .unwrap()];
    assert!(matches!(
        load(&repo),
        Err(PersonaError::LinkInRepositoryTier {
            component: SourceRef::File { path, .. }
        }) if path == "roles/writer/PERSONA.md"
    ));
}

#[test]
fn the_error_type_stays_small_enough_to_return_by_value() {
    // Clippy's `result_large_err` flags error types over 128 bytes in every
    // consumer function that returns one.
    assert!(std::mem::size_of::<PersonaError>() <= 128);
}

#[test]
fn errors_name_the_file_at_fault() {
    let tiers = [
        base(&[("PERSONA.md", &full("", "x"))]),
        tier(
            "user",
            TrustClass::Operator,
            &[("PERSONA.md", "no front matter")],
        ),
    ];
    let err = load(&tiers).unwrap_err();
    assert_eq!(
        err.to_string(),
        "roles/writer/PERSONA.md (tier `user`): must start with a `---` front matter line"
    );
}
