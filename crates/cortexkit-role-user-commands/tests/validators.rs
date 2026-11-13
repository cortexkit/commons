use cortexkit_role_user_commands::*;
use serde_json::{json, Map};

fn command() -> Command {
    Command::new(
        "change",
        "Change",
        "Changes local state.",
        SubjectKind::Computer,
        vec![],
        vec!["reversible".into()],
        Some(Run::new()),
    )
}

#[test]
fn read_with_run_is_rejected() {
    let mut read = command();
    read.effects = vec!["read".into()];
    assert_eq!(read.check().unwrap_err().field, "run");
    read.run = None;
    read.check().unwrap();
    assert!(!read.offers_run());
}

#[test]
fn command_limits_unique_sets_and_open_effect_fallback() {
    let good = command();
    good.check().unwrap();
    assert!(good.offers_run());
    for effects in [
        vec![],
        vec!["read", "reversible"],
        vec!["ends", "ends"],
        vec!["future"],
    ] {
        let mut bad = good.clone();
        bad.effects = effects.into_iter().map(str::to_owned).collect();
        assert_eq!(bad.check().unwrap_err().field, "effects");
    }
    let mut unknown = good.clone();
    unknown.effects.push("future".into());
    let decoded: Command = serde_json::from_value(serde_json::to_value(&unknown).unwrap()).unwrap();
    assert_eq!(decoded, unknown);
    assert!(!decoded.offers_run());
    for effect in EFFECTS.iter().filter(|e| **e != "read") {
        let mut c = good.clone();
        c.effects = vec![(*effect).into()];
        c.check().unwrap();
        c.run = None;
        assert_eq!(c.check().unwrap_err().field, "run");
    }
    let mut c = good.clone();
    c.id = "a".repeat(64);
    c.title = "é".repeat(40);
    c.summary = "é".repeat(240);
    c.check().unwrap();
    c.id.push('a');
    assert_eq!(c.check().unwrap_err().field, "id");
    c.id = "change".into();
    c.title.push('é');
    assert_eq!(c.check().unwrap_err().field, "title");
    c.title = "Change".into();
    c.summary.push('é');
    assert_eq!(c.check().unwrap_err().field, "summary");
    for id in ["", "A", "hyphen-id", "space id", "é"] {
        assert!(!validate_code(id));
    }
    assert!(validate_code("0_a"));
    let commands: Vec<_> = (0..64)
        .map(|i| {
            let mut c = good.clone();
            c.id = format!("cmd_{i}");
            c
        })
        .collect();
    validate_commands(&commands).unwrap();
    let mut too_many = commands.clone();
    too_many.push(good.clone());
    assert_eq!(validate_commands(&too_many).unwrap_err().field, "commands");
    let mut duplicates = commands;
    duplicates[1].id = duplicates[0].id.clone();
    assert_eq!(
        validate_commands(&duplicates).unwrap_err().field,
        "commands.1.id"
    );
    validate_commands(&[]).unwrap();
}

#[test]
fn argument_kinds_defaults_and_exact_limits() {
    let mut bounded = Arg::new("a".repeat(64), ArgKind::Text, "é".repeat(40));
    bounded.check().unwrap();
    bounded.key.push('a');
    assert_eq!(bounded.check().unwrap_err().field, "key");
    bounded.key = "key".into();
    bounded.label.push('é');
    assert_eq!(bounded.check().unwrap_err().field, "label");
    let null_default: Arg =
        serde_json::from_value(json!({"key":"x", "kind":"text", "label":"X", "default":null}))
            .unwrap();
    assert_eq!(null_default.check().unwrap_err().field, "default");
    let mut choice = Arg::new("choice", ArgKind::Choice, "Choice");
    assert_eq!(choice.check().unwrap_err().field, "options");
    choice.options = Some(vec![ChoiceOption::new("one", "One")]);
    choice.default = Some(json!("one"));
    choice.check().unwrap();
    assert!(!choice.accepts(&json!("two")));
    choice.default = Some(json!("two"));
    assert_eq!(choice.check().unwrap_err().field, "default");
    choice.default = None;
    choice.options = Some(
        (0..32)
            .map(|i| ChoiceOption::new(format!("value{i}"), "é".repeat(40)))
            .collect(),
    );
    choice.check().unwrap();
    choice
        .options
        .as_mut()
        .unwrap()
        .push(ChoiceOption::new("last", "Last"));
    assert_eq!(choice.check().unwrap_err().field, "options");
    choice.options = Some(vec![
        ChoiceOption::new("one", "One"),
        ChoiceOption::new("one", "Other"),
    ]);
    assert!(choice.check().unwrap_err().field.ends_with("value"));
    for options in [
        vec![],
        vec![ChoiceOption::new("", "One")],
        vec![ChoiceOption::new("a".repeat(65), "One")],
        vec![ChoiceOption::new("one", "")],
        vec![ChoiceOption::new("one", "a".repeat(41))],
    ] {
        choice.options = Some(options);
        assert!(choice.check().is_err());
    }
    let cases = [
        (
            ArgKind::Text,
            json!("é".repeat(2000)),
            json!("é".repeat(2001)),
        ),
        (ArgKind::Number, json!(1.5), json!("1.5")),
        (ArgKind::Toggle, json!(false), json!(0)),
        (
            ArgKind::Agent,
            json!("a".repeat(128)),
            json!("a".repeat(129)),
        ),
        (
            ArgKind::Session,
            json!({"runner":"r", "harness":"h", "session":"s"}),
            json!("s"),
        ),
    ];
    for (kind, good, bad) in cases {
        let mut arg = Arg::new("arg", kind, "Arg");
        arg.default = Some(good.clone());
        arg.check().unwrap();
        assert!(arg.accepts(&good));
        assert!(!arg.accepts(&bad));
        assert!(!arg.accepts(&json!(null)));
        arg.default = Some(bad);
        assert_eq!(arg.check().unwrap_err().field, "default");
        arg.default = None;
        arg.options = Some(vec![]);
        assert_eq!(arg.check().unwrap_err().field, "options");
    }
    let mut c = command();
    c.args = (0..8)
        .map(|i| Arg::new(format!("arg{i}"), ArgKind::Toggle, "Arg"))
        .collect();
    c.check().unwrap();
    c.args.push(Arg::new("ninth", ArgKind::Text, "Ninth"));
    assert_eq!(c.check().unwrap_err().field, "args");
    c.args.pop();
    c.args[1].key = c.args[0].key.clone();
    assert_eq!(c.check().unwrap_err().field, "args.1.key");
    let mut arg = Arg::new("mode", ArgKind::Toggle, "Mode");
    arg.default = Some(json!(true));
    c.args = vec![arg];
    assert_eq!(
        c.resolve_args(&Map::new()).unwrap(),
        json!({"mode":true}).as_object().unwrap().clone()
    );
    assert_eq!(
        c.resolve_args(json!({"mode":false}).as_object().unwrap())
            .unwrap()["mode"],
        false
    );
    assert_eq!(
        c.resolve_args(json!({"other":true}).as_object().unwrap())
            .unwrap_err()
            .field,
        "args.other"
    );
    c.args[0].default = None;
    assert_eq!(c.resolve_args(&Map::new()).unwrap_err().field, "args.mode");
}

#[test]
fn failed_fact_state_is_accepted() {
    let body = json!({"label":"Errors", "state":"failed"});
    validate_fact(&body).unwrap();
    let fact: Fact = serde_json::from_value(body).unwrap();
    fact.check().unwrap();
    validate_fact_value(&json!({"state":"failed"})).unwrap();
}

#[test]
fn fact_boundaries_finiteness_and_unknowns() {
    let mut fact = Fact::new("é".repeat(64), FactValue::number(1.5));
    fact.data.unit = Some("ratio".into());
    fact.check().unwrap();
    fact.label.push('é');
    assert_eq!(fact.check().unwrap_err().code, FACT_RANGE);
    fact.label = "Count".into();
    fact.data.unit = None;
    fact.data.noun = Some("é".repeat(24));
    fact.check().unwrap();
    fact.data.noun.as_mut().unwrap().push('é');
    assert_eq!(fact.check().unwrap_err().code, FACT_RANGE);
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            FactValue::number(value).check().unwrap_err().code,
            FACT_RANGE
        );
        let mut building = FactValue::state("building");
        building.progress = Some(value);
        assert_eq!(building.check().unwrap_err().code, FACT_RANGE);
    }
    for progress in [0.0, 1.0] {
        let mut building = FactValue::state("building");
        building.progress = Some(progress);
        building.check().unwrap();
    }
    for (field, value) in [("unit", "future"), ("state", "future"), ("tone", "future")] {
        let body = if field == "state" {
            json!({"state":value})
        } else {
            json!({"value":1.0, (field):value})
        };
        let decoded: FactValue = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), body);
        assert_eq!(decoded.check().unwrap_err().code, FACT_MALFORMED);
    }
    for body in [
        json!({"label":"Items", "value":null}),
        json!({"label":"Items", "value":"1"}),
        json!({"label":"Items", "value":1, "approx":"true"}),
        json!({"label":"Items", "state":"building", "progress":null}),
    ] {
        assert_eq!(validate_fact(&body).unwrap_err().code, FACT_MALFORMED);
    }
}

#[test]
fn preview_table_and_figure_limits() {
    let fact = Fact::new("Items", FactValue::number(1.0));
    let mut preview = Preview::new(" ".repeat(128), true);
    preview.facts = Some(vec![fact.clone(); 16]);
    preview.effect = Some("é".repeat(120));
    preview.check().unwrap();
    preview.facts.as_mut().unwrap().push(fact.clone());
    assert_eq!(preview.check().unwrap_err().field, "facts");
    preview.facts = None;
    preview.effect.as_mut().unwrap().push('é');
    assert_eq!(preview.check().unwrap_err().field, "effect");
    for effect in ["", "a\nb", "a\rb", "a\u{2028}b"] {
        preview.effect = Some(effect.into());
        assert_eq!(preview.check().unwrap_err().field, "effect");
    }
    preview.effect = None;
    preview.available = false;
    assert_eq!(preview.check().unwrap_err().field, "reason");
    preview.reason = Some("example_reason".into());
    preview.check().unwrap();
    let decoded: Preview =
        serde_json::from_value(json!({"version":"v", "available":true, "table":null})).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        json!({"version":"v", "available":true})
    );
    let row = TableRow::new("é".repeat(64), vec![FactValue::number(1.0); 6]);
    let mut table = Table::new(vec!["é".repeat(40); 6], vec![row; 32]);
    table.check().unwrap();
    table.rows.push(table.rows[0].clone());
    assert_eq!(table.check().unwrap_err().field, "rows");
    table.rows.pop();
    table.rows[0].cells.pop();
    assert_eq!(table.check().unwrap_err().field, "rows.0.cells");
    table.rows[0].cells.push(FactValue::state("future"));
    assert_eq!(table.check().unwrap_err().code, FACT_MALFORMED);
    table.rows.clear();
    table.columns.push("Seventh".into());
    assert_eq!(table.check().unwrap_err().field, "columns");
    table.columns.clear();
    assert_eq!(table.check().unwrap_err().field, "columns");
    for label in ["".to_owned(), "é".repeat(41)] {
        assert_eq!(
            Table::new(vec![label], vec![]).check().unwrap_err().field,
            "columns.0"
        );
    }
    for label in ["".to_owned(), "é".repeat(65)] {
        let table = Table::new(
            vec!["Column".into()],
            vec![TableRow::new(label, vec![FactValue::number(1.0)])],
        );
        assert_eq!(table.check().unwrap_err().field, "rows.0.label");
    }
    let mut figure = Figure::new("Figure", 1.5, "ratio");
    figure.was = Some(1.2);
    figure.check().unwrap();
    figure.value = f64::NAN;
    assert_eq!(figure.check().unwrap_err().field, "value");
    figure.value = 1.0;
    figure.was = Some(f64::INFINITY);
    assert_eq!(figure.check().unwrap_err().field, "was");
    let mut reply = RunReply::new("example_outcome");
    reply.facts = Some(vec![fact; 17]);
    assert_eq!(reply.check().unwrap_err().field, "facts");
}

#[test]
fn request_identity_and_admission_are_separate_from_caller_claims() {
    let principal = Principal::Reserved {
        module_id: "occipital".into(),
    };
    check_admission(&principal, false, false).unwrap();
    check_admission(&principal, true, true).unwrap();
    assert_eq!(
        check_admission(&principal, false, true).unwrap_err().code,
        CALLER_NOT_ADMITTED
    );
    for principal in [
        Principal::Direct,
        Principal::Reserved {
            module_id: "example-module".into(),
        },
    ] {
        assert_eq!(
            check_admission(&principal, true, false).unwrap_err().code,
            CALLER_NOT_ADMITTED
        );
    }
    let c = command();
    let mut caller = Caller::new("future_app", "future_admission");
    caller.peer_build = Some("future_build".into());
    caller.device = Some("aAbB09".into());
    caller.check().unwrap();
    caller.device = Some("not-hex".into());
    assert_eq!(caller.check().unwrap_err().field, "device");
    caller.device = None;
    let preview = ModulePreviewRequest::new("change", Subject::computer(), Map::new(), caller);
    preview.check(&c).unwrap();
    let mut run = ModuleRunRequest::new(preview, "v1", "A_-9".repeat(16));
    run.check(&c).unwrap();
    run.request_id.push('x');
    assert_eq!(run.check(&c).unwrap_err().field, "request_id");
    for id in ["", "has space", "bad/char", "é"] {
        assert!(!validate_request_id(id));
    }
    for version in ["", "é", "new\nline", "\u{007f}"] {
        assert!(!validate_version(version));
    }
    assert!(validate_version(&"x".repeat(128)));
    assert!(!validate_version(&"x".repeat(129)));
    run.request_id = "r1".into();
    run.preview_version = "x".repeat(129);
    assert_eq!(run.check(&c).unwrap_err().field, "preview_version");
    run.preview_version = "v1".into();
    run.subject = Subject::agent("example-agent");
    assert_eq!(run.check(&c).unwrap_err().field, "subject.kind");
    run.subject = Subject::computer();
    run.id = "other".into();
    assert_eq!(run.check(&c).unwrap_err().field, "id");
    let session = SessionRef::new("r".repeat(256), "h".repeat(256), "s".repeat(256));
    session.check().unwrap();
    let mut too_long = session.clone();
    too_long.session.push('s');
    assert_eq!(too_long.check().unwrap_err().field, "session");
    let subject = Subject::session(session);
    subject.check().unwrap();
    assert_eq!(subject.kind(), SubjectKind::Session);
    Subject::agent("a".repeat(128)).check().unwrap();
    assert!(Subject::agent("").check().is_err());
    assert!(Subject::agent("a".repeat(129)).check().is_err());
    ListRequest::new(subject).check().unwrap();
    let app = AppPreviewRequest::new("example-module", "change", Subject::computer(), Map::new());
    app.check(&c).unwrap();
    AppRunRequest::new(app, "v1", "r1").check(&c).unwrap();
    assert_eq!(
        serde_json::to_value(DescribeRequest::new()).unwrap(),
        json!({})
    );
}

#[test]
fn replies_refusals_strict_shapes_and_fixed_jcs_digest() {
    let c = command();
    let reply = DescribeReply::new(vec![c.clone()]).unwrap();
    reply.check().unwrap();
    let list = ListReply::new(vec![ListedCommand::new("example-module", c.clone())]);
    list.check().unwrap();
    let mut duplicates = list.clone();
    duplicates.commands.push(duplicates.commands[0].clone());
    assert!(duplicates.check().is_err());
    let mut reply = RunReply::new(outcomes::APPLIED);
    for kind in [OpenKind::Ask, OpenKind::Artifact, OpenKind::WorkItem] {
        reply.open = Some(OpenItem::new(kind, "example-item"));
        reply.check().unwrap();
    }
    reply.open.as_mut().unwrap().id.clear();
    assert_eq!(reply.check().unwrap_err().field, "open.id");
    reply.open = None;
    reply.outcome = "Bad code".into();
    assert_eq!(reply.check().unwrap_err().field, "outcome");
    assert_eq!(
        commands_digest(&[]).unwrap(),
        "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
    );
    let mut request =
        json!({"module":"example-module", "id":"change", "subject":{"kind":"computer"}, "args":{}});
    request["caller"] = json!({"app":"desktop", "admission":"operator_app"});
    assert!(serde_json::from_value::<AppPreviewRequest>(request).is_err());
    assert!(serde_json::from_value::<DescribeRequest>(json!({"unknown":1})).is_err());
    assert!(serde_json::from_value::<Subject>(json!({"kind":"computer", "agent_id":"a"})).is_err());
    assert!(serde_json::from_value::<Run>(json!({"job":true})).is_err());
    let error = ValidationError::new("args.mode");
    let wire = serde_json::to_value(error.refusal()).unwrap();
    assert_eq!(wire["code"], INVALID_PARAMS);
    assert_eq!(wire["detail"], json!({"field":"args.mode"}));
    let error: ErrorBody =
        serde_json::from_value(json!({"code":UNKNOWN_COMMAND, "message":"Unknown", "future":true}))
            .unwrap();
    assert_eq!(error.code, UNKNOWN_COMMAND);
    assert_eq!(PROVIDES, "user-commands/v1");
    assert_eq!(MODULE_OPS.len(), 3);
    assert_eq!(APP_OPS.len(), 3);
}
