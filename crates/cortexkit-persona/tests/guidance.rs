use std::num::{NonZeroU64, NonZeroUsize};

use cortexkit_persona::{Digest, FactsShape, GuidanceSet, RenderLimits, MAX_FILE_BYTES};
use serde_json::{json, Value};

fn shape() -> FactsShape {
    FactsShape::from_json_schema(&json!({
        "type": "object",
        "properties": {
            "enabled": {"type": "boolean"},
            "user": {"type": "object", "properties": {"name": {"type": "string"}}},
            "tags": {"type": "array", "items": {"type": "string"}},
            "count": {"type": "integer"},
            "ratio": {"type": "number"}
        }
    }))
    .unwrap()
}

#[test]
fn rendering_is_verbatim_and_stable() {
    let set = GuidanceSet::new([
        ("main", "Hello {{ user.name }}\r\n{% include 'tail' %}\n"),
        ("tail", "{% for tag in tags %}[{{ tag }}]{% endfor %}"),
    ])
    .unwrap();
    let facts = json!({"user": {"name": "{{ not_a_template }}\r\n<raw>"}, "tags": ["a", "b"]});
    let a = set.render("main", &facts, RenderLimits::default()).unwrap();
    let b = set.render("main", &facts, RenderLimits::default()).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.text, "Hello {{ not_a_template }}\r\n<raw>\n[a][b]");
    assert_eq!(a.output_digest, Digest::of(a.text.as_bytes()));
    assert_eq!(
        a.template_digests["main"],
        Digest::of(b"Hello {{ user.name }}\n{% include 'tail' %}\n")
    );
    assert_eq!(a.template_digests.len(), 2);
    let reordered = GuidanceSet::new([
        ("tail", "{% for tag in tags %}[{{ tag }}]{% endfor %}"),
        ("main", "Hello {{ user.name }}\n{% include 'tail' %}\n"),
    ])
    .unwrap();
    assert_eq!(set.digest(), reordered.digest());
    let changed_tail = GuidanceSet::new([
        ("main", "Hello {{ user.name }}\n{% include 'tail' %}\n"),
        ("tail", "changed"),
    ])
    .unwrap();
    assert_ne!(set.digest(), changed_tail.digest());
    assert_ne!(
        set.digest(),
        GuidanceSet::new([("main", "changed"), ("tail", "tail")])
            .unwrap()
            .digest()
    );
    assert_ne!(
        GuidanceSet::new([("a", "bc")]).unwrap().digest(),
        GuidanceSet::new([("ab", "c")]).unwrap().digest()
    );
}

#[test]
fn missing_facts_are_refused() {
    let set = GuidanceSet::new([("main", "{{ missing }}")]).unwrap();
    assert!(set
        .render("main", &json!({}), RenderLimits::default())
        .is_err());
    let nested = GuidanceSet::new([("main", "{{ user.name }}")]).unwrap();
    assert!(nested
        .render("main", &json!({"user": {}}), RenderLimits::default())
        .is_err());
}

#[test]
fn missing_entry_is_refused() {
    let set = GuidanceSet::new([("main", "literal")]).unwrap();
    assert!(set
        .render("absent", &json!({}), RenderLimits::default())
        .is_err());
}

#[test]
fn non_object_facts_are_refused() {
    let literal = GuidanceSet::new([("main", "literal")]).unwrap();
    let accepted: Vec<_> = [Value::Null, json!(1), json!("text"), json!([])]
        .into_iter()
        .filter(|v| literal.render("main", v, RenderLimits::default()).is_ok())
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted non-object facts: {accepted:?}"
    );
}

#[test]
fn explicit_defaults_work_but_typos_still_fail_check() {
    let set = GuidanceSet::new([(
        "main",
        "{% if optional is defined %}{{ optional }}{% else %}default{% endif %}",
    )])
    .unwrap();
    assert_eq!(
        set.render("main", &json!({}), RenderLimits::default())
            .unwrap()
            .text,
        "default"
    );
    assert!(set.check(&shape()).is_err());
    let declared_optional = FactsShape::from_json_schema(
        &json!({"type": "object", "properties": {"optional": {"type": "string"}}}),
    )
    .unwrap();
    set.check(&declared_optional).unwrap();
}

#[test]
fn include_boundary_and_source_validation_are_refused() {
    let oversized = "x".repeat(MAX_FILE_BYTES as usize + 1);
    let cases = [
        ("missing include", "main", "{% include 'outside' %}"),
        (
            "hidden missing include",
            "main",
            "{% if false %}{% include 'outside' %}{% endif %}",
        ),
        ("dynamic include", "main", "{% include target %}"),
        (
            "ignore missing",
            "main",
            "{% include 'main' ignore missing %}",
        ),
        ("syntax", "main", "{{"),
        ("empty id", "", "literal"),
        ("separator", "../main", "literal"),
        ("uppercase", "Main", "literal"),
        ("leading hyphen", "-main", "literal"),
        (
            "long id",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "literal",
        ),
        ("size", "main", oversized.as_str()),
    ];
    let accepted: Vec<_> = cases
        .into_iter()
        .filter_map(|(label, id, source)| GuidanceSet::new([(id, source)]).is_ok().then_some(label))
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted invalid sources: {accepted:?}"
    );
}

#[test]
fn duplicate_ids_are_refused() {
    assert!(GuidanceSet::new([("main", "a"), ("main", "b")]).is_err());
}

#[test]
fn budgets_are_shared_with_persona_engine() {
    let set =
        GuidanceSet::new([("main", "{% for n in range(1000) %}{{ n }}{% endfor %}")]).unwrap();
    assert!(set
        .render(
            "main",
            &json!({}),
            RenderLimits::default().with_fuel(NonZeroU64::new(10).unwrap())
        )
        .is_err());
}

#[test]
fn include_depth_is_limited() {
    let recursive = GuidanceSet::new([("main", "{% include 'main' %}")]).unwrap();
    assert!(recursive
        .render(
            "main",
            &json!({}),
            RenderLimits::default().with_recursion(NonZeroUsize::new(10).unwrap())
        )
        .is_err());
}

#[test]
fn shape_check_passes_for_nested_paths_and_scalar_lists() {
    let set = GuidanceSet::new([("main", "{% if enabled %}{{ user.name }}{% endif %}{% for tag in tags %}{{ tag }}{% endfor %}{{ count }} {{ ratio }}")]).unwrap();
    set.check(&shape()).unwrap();
}

#[test]
fn shape_check_refuses_typos_in_all_templates_and_branches() {
    let cases = [
        "{% if enabled %}ok{% else %}{{ user.nmae }}{% endif %}",
        "{% if false %}{{ absent }}{% endif %}",
        "{{ user.name.typo }}",
        "{% for tag in tgas %}{{ tag }}{% endfor %}",
        "{% if user.nmae is defined %}ok{% endif %}",
    ];
    let accepted: Vec<_> = cases
        .into_iter()
        .filter(|text| {
            GuidanceSet::new([("main", *text)])
                .unwrap()
                .check(&shape())
                .is_ok()
        })
        .collect();
    assert!(accepted.is_empty(), "unchecked paths: {accepted:?}");
    let err = GuidanceSet::new([("main", cases[0])])
        .unwrap()
        .check(&shape())
        .unwrap_err();
    assert!(err.to_string().contains("user.nmae"), "{err}");
}

#[test]
fn unused_templates_are_checked() {
    assert!(
        GuidanceSet::new([("main", "ok"), ("unused", "{{ user.nmae }}")])
            .unwrap()
            .check(&shape())
            .is_err()
    );
}

#[test]
fn literal_items_and_local_aliases_are_checked() {
    GuidanceSet::new([("main", "{% set alias = user %}{{ alias['name'] }}{% with other=user %}{{ other.name }}{% endwith %}{{ tags[0] }}")]).unwrap().check(&shape()).unwrap();
}

#[test]
fn unprovable_paths_and_scalar_attributes_are_refused() {
    let cases = [
        "{{ user['nmae'] }}",
        "{% set alias = user %}{{ alias.nmae }}",
        "{% with alias=user %}{{ alias.nmae }}{% endwith %}",
        "{% for tag in tags %}{{ tag.name }}{% endfor %}",
        "{{ user[user.name] }}",
        "{{ user|attr('nmae') }}",
        "{{ tags|map(attribute='nmae')|list }}",
        "{{ tags|selectattr('nmae')|list }}",
        "{{ tags|rejectattr('nmae')|list }}",
        "{{ tags|groupby('nmae') }}",
        "{{ tags|sort(attribute='nmae') }}",
        "{% for tag in user.name %}{{ tag }}{% endfor %}",
        "{{ user.name[0:count] }} {{ absent[0:count] }}",
        "{% autoescape absent %}literal{% endautoescape %}",
        "{% filter replace(absent, 'a') %}literal{% endfilter %}",
        "{% set capture|replace(absent, 'a') %}literal{% endset %}",
    ];
    let accepted: Vec<_> = cases
        .into_iter()
        .filter(|text| {
            GuidanceSet::new([("main", *text)])
                .unwrap()
                .check(&shape())
                .is_ok()
        })
        .collect();
    assert!(accepted.is_empty(), "unproven paths accepted: {accepted:?}");
}

#[test]
fn schema_annotations_are_accepted() {
    let mut schema = json!({"type": "object", "properties": {"name": {"type": "string", "minLength": 1, "description": "A name"}, "tags": {"type": "array", "items": {"type": "string"}, "uniqueItems": true, "x-sorted": true}}});
    for key in [
        "required",
        "additionalProperties",
        "$schema",
        "$id",
        "title",
        "description",
        "x-extra",
    ] {
        schema
            .as_object_mut()
            .unwrap()
            .insert(key.into(), Value::Null);
    }
    FactsShape::from_json_schema(&schema).unwrap();
}

#[test]
fn unsupported_schema_keywords_are_refused_by_name_at_every_depth() {
    let mut accepted = Vec::new();
    for keyword in [
        "$ref",
        "$defs",
        "oneOf",
        "anyOf",
        "allOf",
        "patternProperties",
        "if",
        "then",
        "else",
        "not",
        "enum",
        "format",
        "prefixItems",
        "unknown",
    ] {
        for nested in [false, true] {
            let mut node = json!({"type": "string"});
            node.as_object_mut()
                .unwrap()
                .insert(keyword.into(), Value::Null);
            let schema = if nested {
                json!({"type": "object", "properties": {"name": node}})
            } else {
                node
            };
            match FactsShape::from_json_schema(&schema) {
                Ok(_) => accepted.push(format!("{keyword} nested={nested}")),
                Err(e) => assert!(e.to_string().contains(keyword), "{e}"),
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "accepted unsupported keywords: {accepted:?}"
    );
}

#[test]
fn malformed_schemas_are_refused() {
    let cases = [
        Value::Null,
        json!(true),
        json!({}),
        json!({"type": ["string", "null"]}),
        json!({"type": "null"}),
        json!({"type": "object", "properties": []}),
        json!({"type": "array"}),
        json!({"type": "array", "items": {"type": "object"}}),
        json!({"type": "array", "items": {"type": "array", "items": {"type": "string"}}}),
        json!({"type": "string", "properties": {}}),
        json!({"type": "object", "items": {"type": "string"}}),
    ];
    let accepted: Vec<_> = cases
        .into_iter()
        .filter(|schema| FactsShape::from_json_schema(schema).is_ok())
        .collect();
    assert!(
        accepted.is_empty(),
        "accepted malformed schemas: {accepted:?}"
    );
}

#[test]
fn check_requires_an_object_shape() {
    let scalar = FactsShape::from_json_schema(&json!({"type": "string"})).unwrap();
    assert!(GuidanceSet::new([("main", "literal")])
        .unwrap()
        .check(&scalar)
        .is_err());
    assert!(GuidanceSet::new(Vec::<(&str, &str)>::new())
        .unwrap()
        .check(&scalar)
        .is_err());
}

#[test]
fn conditional_alias_changes_cannot_hide_paths() {
    for text in [
        "{% if enabled %}{% set user=tags %}{% endif %}{{ user.name }}",
        "{% set user=tags %}{% include 'tail' %}",
        "{% for user in tags %}{% include 'tail' %}{% endfor %}",
    ] {
        let set = GuidanceSet::new([("main", text), ("tail", "{{ user.name }}")]).unwrap();
        assert!(set.check(&shape()).is_err(), "{text}");
    }
}

#[test]
fn conditional_local_alias_changes_cannot_hide_paths() {
    let set = GuidanceSet::new([(
        "main",
        "{% set alias=user %}{% if enabled %}{% set alias=tags %}{% endif %}{{ alias.name }}",
    )])
    .unwrap();
    assert!(set.check(&shape()).is_err());
}

#[test]
fn aliases_follow_runtime_capture_and_with_scopes() {
    let text = "{% autoescape false %}{% set alias=user %}{% endautoescape %}{{ alias.name }}{% filter upper %}{% set other=user %}{% endfilter %}{{ other.name }}{% set captured %}{% set third=user %}{% endset %}{{ third.name }}{% with one=user, two=one %}{{ two.name }}{% endwith %}";
    let set = GuidanceSet::new([("main", text)]).unwrap();
    assert_eq!(
        set.render(
            "main",
            &json!({"user": {"name": "a"}}),
            RenderLimits::default()
        )
        .unwrap()
        .text,
        "aaaa"
    );
    set.check(&shape()).unwrap();
}
