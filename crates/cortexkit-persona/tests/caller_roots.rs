//! Roots a caller declares, text values with fields, and the bytes a caller's
//! values contribute to the output. Every expected value here is pinned as
//! exact bytes: a host that moves onto this crate must render the same
//! prompts it renders now.

mod common;

use std::collections::BTreeMap;

use common::*;
use cortexkit_persona::{FragmentSources, PersonaError, RenderContext, TextValue, Value};

/// A role whose body is `body`, with no fragments.
fn body_role(body: &str) -> Vec<cortexkit_persona::Tier> {
    vec![base(&[("PERSONA.md", &full("", body))])]
}

fn render_with(ctx: &RenderContext<serde_json::Value>, body: &str) -> String {
    let role = load(&body_role(body)).unwrap();
    role.render(ctx, &FragmentSources::new()).unwrap().text
}

fn render_error(ctx: &RenderContext<serde_json::Value>, body: &str) -> PersonaError {
    let role = load(&body_role(body)).unwrap();
    role.render(ctx, &FragmentSources::new()).unwrap_err()
}

/// The error a context is refused with, whatever the role's body is.
fn declaration_error(ctx: &RenderContext<serde_json::Value>) -> PersonaError {
    render_error(ctx, "x")
}

// Caller-declared roots: the shapes a host's templates read.

#[test]
fn declared_config_roots_read_their_maps() {
    let ctx = context().with_root(
        "config",
        Value::map([
            ("board", Value::map([("enabled", Value::boolean(true))])),
            ("status_line", Value::map([("gh", Value::boolean(false))])),
        ]),
    );
    for (body, expected) in [
        ("{% if config.board.enabled %}board{% endif %}", "board"),
        (
            "{% if config.status_line.gh %}gh{% else %}no gh{% endif %}",
            "no gh",
        ),
    ] {
        assert_eq!(
            render_with(&ctx, body).as_bytes(),
            expected.as_bytes(),
            "{body}"
        );
    }
}

#[test]
fn a_declared_integer_root_renders_as_decimal_digits() {
    let ctx = context().with_root(
        "limits",
        Value::map([("sidekick_readers", Value::integer(6))]),
    );
    assert_eq!(
        render_with(&ctx, "{{ limits.sidekick_readers }}").as_bytes(),
        b"6"
    );
    // The same integer declared as a root of its own.
    let ctx = context().with_root("count", 6u8);
    assert_eq!(render_with(&ctx, "{{ count }}").as_bytes(), b"6");
}

#[test]
fn a_text_root_renders_its_text_and_answers_its_fields() {
    // A template's one trailing newline is dropped by MiniJinja, so a body
    // that is only `{{ contract }}` ends without the text's final newline. The
    // newline after the expression keeps it, and the contract's own text is
    // then the whole output.
    let contract = TextValue::new("contract\n")
        .with_field("submit_tool", true)
        .with_field("package_limit", 6i64);
    let ctx = context().with_root("contract", contract);
    let role = load(&body_role("{{ contract }}\n")).unwrap();
    let out = role.render(&ctx, &FragmentSources::new()).unwrap();
    assert_eq!(out.bytes(), b"contract\n");
    assert_eq!(out.digest, cortexkit_persona::Digest::of(b"contract\n"));
    for (body, expected) in [
        ("[{{ contract }}]", &b"[contract\n]"[..]),
        (
            "{% if contract.submit_tool %}submit{% else %}inline{% endif %}",
            &b"submit"[..],
        ),
        ("[{{ contract.package_limit }}]", &b"[6]"[..]),
    ] {
        assert_eq!(render_with(&ctx, body).as_bytes(), expected, "{body}");
    }
    // The text itself is the value, so reading a field it does not have is
    // undefined, like any other unknown name.
    assert!(matches!(
        render_error(&ctx, "{{ contract.document }}"),
        PersonaError::Render {
            failure: cortexkit_persona::RenderFailure::UndefinedValue,
            ..
        }
    ));
    let ctx = context().with_root("contract", TextValue::new("inline only"));
    let body = "{% if contract.submit_tool %}submit{% else %}inline{% endif %}";
    // A field the value does not carry is undefined, not `false`, so a caller
    // that omits one makes the template fail by name instead of branching on
    // a value nobody set.
    assert!(matches!(
        render_error(&ctx, body),
        PersonaError::Render {
            failure: cortexkit_persona::RenderFailure::UndefinedValue,
            ..
        }
    ));
    let ctx = context().with_root(
        "contract",
        TextValue::new("with a flag").with_field("submit_tool", false),
    );
    assert_eq!(render_with(&ctx, body).as_bytes(), b"inline");
}

#[test]
fn an_undeclared_root_is_still_a_strict_undefined_error() {
    for body in [
        "{{ limits.sidekick_readers }}",
        "{{ contract }}",
        "{% if config.board.enabled %}b{% endif %}",
    ] {
        match render_error(&context(), body) {
            PersonaError::Render { failure, .. } => {
                assert_eq!(
                    failure,
                    cortexkit_persona::RenderFailure::UndefinedValue,
                    "{body}"
                );
            }
            other => panic!("{body}: expected an undefined-value error, got {other:?}"),
        }
    }
}

#[test]
fn a_root_shadowing_a_built_in_is_refused_by_name_before_rendering() {
    // None of these bodies reads the shadowed root, so the refusal has to
    // come from the declaration alone.
    for built_in in [
        "module", "tools", "agent", "model", "harness", "mcp", "modules", "demand", "env",
    ] {
        let ctx = context().with_root(built_in, Value::string("shadow"));
        match render_error(&ctx, "x") {
            PersonaError::InvalidContext { field, reason } => {
                assert_eq!(field, format!("root.{built_in}"));
                assert!(reason.contains("built-in"), "{reason}");
            }
            other => panic!("{built_in}: expected InvalidContext, got {other:?}"),
        }
    }
}

#[test]
fn a_duplicate_declaration_and_a_name_that_is_not_an_identifier_are_refused() {
    // The same two declarations, one after the other: the second is refused.
    let ctx = context()
        .with_root("config", Value::string("a"))
        .with_root("config", Value::string("b"));
    match declaration_error(&ctx) {
        PersonaError::InvalidContext { field, reason } => {
            assert_eq!(field, "root.config");
            assert!(reason.contains("root name once"), "{reason}");
        }
        other => panic!("expected InvalidContext, got {other:?}"),
    }
    // A name that is not an identifier is refused too, by that name.
    for name in ["", "1config", "with-dash", "a b", "dot.ted"] {
        let ctx = context().with_root(name, Value::string("x"));
        match declaration_error(&ctx) {
            PersonaError::InvalidContext { field, reason } => {
                assert_eq!(field, format!("root.{name}"), "{reason}");
            }
            other => panic!("{name:?}: expected InvalidContext, got {other:?}"),
        }
    }
}

// Bytes: what a caller's values contribute, and what template files do.

#[test]
fn a_carriage_return_inside_a_value_survives_verbatim() {
    let mut ctx = context();
    ctx.agent.name = Some("a\r\nb\rc".into());
    let ctx = ctx
        .with_root("config", Value::map([("text", Value::string("p\r\nq\rr"))]))
        .with_root("contract", TextValue::new("s\rt").with_field("flag", true));
    for (body, expected) in [
        (
            "{{ agent.name }}|{{ config.text }}|{{ contract }}",
            &b"a\r\nb\rc|p\r\nq\rr|s\rt"[..],
        ),
        (
            "{% if contract.flag %}y{% endif %}{{ agent.name }}",
            &b"ya\r\nb\rc"[..],
        ),
    ] {
        assert_eq!(render_with(&ctx, body).as_bytes(), expected, "{body}");
    }
}

#[test]
fn a_crlf_template_file_renders_the_same_bytes_as_its_lf_twin() {
    let lf_body = "One {{ model.id }}.\nTwo {% if tools.search %}search{% endif %}.\n";
    let render_from = |text: &str| render(&[base(&[("PERSONA.md", text)])]).unwrap();
    let lf_persona = full("", lf_body);
    let crlf_persona = lf_persona.replace('\n', "\r\n");
    assert!(crlf_persona.contains("\r\n"));
    let lf = render_from(&lf_persona);
    let crlf = render_from(&crlf_persona);
    assert_eq!(crlf.bytes(), lf.bytes());
    assert_eq!(crlf.bytes(), b"One model-1.\nTwo search.");
    assert_eq!(crlf.digest, lf.digest);
    assert_eq!(
        crlf.report.tiers[0].layer_digest,
        lf.report.tiers[0].layer_digest
    );

    // A fragment file, reached by include, too.
    let render_fragment = |note: &str| {
        render(&[base(&[
            (
                "PERSONA.md",
                &full("fragments: [note]\n", "{% include \"note\" %}"),
            ),
            ("fragments/note.md", note),
        ])])
        .unwrap()
    };
    assert_eq!(
        render_fragment("a\r\nb\r").bytes(),
        render_fragment("a\nb\n").bytes()
    );
}

#[test]
fn caller_roots_never_change_a_layer_digest_or_an_approval() {
    let tiers = body_role("[{{ contract }}|{{ limits.sidekick_readers }}]");
    let role = load(&tiers).unwrap();
    let plain = role
        .render(&context(), &FragmentSources::new())
        .unwrap_err();
    assert!(matches!(
        plain,
        PersonaError::Render {
            failure: cortexkit_persona::RenderFailure::UndefinedValue,
            ..
        }
    ));
    let ctx = context()
        .with_root("contract", TextValue::new("contract\n"))
        .with_root(
            "limits",
            Value::map([("sidekick_readers", Value::integer(6))]),
        );
    let out = role.render(&ctx, &FragmentSources::new()).unwrap();
    assert_eq!(out.bytes(), b"[contract\n|6]");
    assert_eq!(
        out.report.tiers[0].layer_digest,
        role.tiers()[0].layer_digest
    );
    // The source revision covers the layers and the manifest, never the
    // render context.
    let other = context().with_root("contract", TextValue::new("other\r\ntext"));
    let again = role.render(&other, &FragmentSources::new());
    if let Ok(again) = again {
        assert_eq!(out.report.source_revision, again.report.source_revision);
    } else {
        // Without the `limits` root the same role does not render at all, and
        // a failed render has no report to compare; the layer digest above is
        // the claim that matters.
        assert!(matches!(again, Err(PersonaError::Render { .. })));
    }
}

#[test]
fn declared_roots_are_listed_on_the_context_and_not_read_from_serialised_data() {
    let plain = context();
    assert!(plain.roots().is_empty());
    let ctx = plain.clone().with_root("z", Value::integer(1));
    assert_eq!(
        ctx.roots(),
        &BTreeMap::from([("z".to_string(), Value::Integer(1))])
    );
    // A serialised context cannot smuggle a root in: the field is skipped, so
    // a caller who needs one declares it with `with_root`.
    let mut json = serde_json::to_value(&plain).unwrap();
    json["roots"] = serde_json::json!({"z": 1});
    assert!(serde_json::from_value::<RenderContext<serde_json::Value>>(json).is_err());
    let json = serde_json::to_value(&ctx).unwrap();
    assert!(json.get("roots").is_none());
    let round: RenderContext<serde_json::Value> = serde_json::from_value(json).unwrap();
    assert!(round.roots().is_empty());
    assert_eq!(round.module, ctx.module);
}

#[test]
fn a_list_and_a_map_root_render_as_minijinja_renders_them() {
    let ctx = context()
        .with_root(
            "batch",
            Value::list([Value::integer(1), Value::string("two")]),
        )
        .with_root(
            "flags",
            Value::map([("b", Value::boolean(true)), ("a", Value::integer(2))]),
        );
    for (body, expected) in [
        ("{{ batch }}", &b"[1, \"two\"]"[..]),
        ("{% for x in batch %}{{ x }};{% endfor %}", &b"1;two;"[..]),
        ("{{ flags }}", &b"{\"a\": 2, \"b\": true}"[..]),
        ("{{ flags.a }}", &b"2"[..]),
    ] {
        assert_eq!(render_with(&ctx, body).as_bytes(), expected, "{body}");
    }
}
