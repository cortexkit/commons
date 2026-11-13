use std::{collections::BTreeSet, fs, path::Path};

use cortexkit_role_user_commands::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

fn vector(name: &str) -> Value {
    serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-vectors/user-commands-v1")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}

fn roundtrip<T: DeserializeOwned + Serialize>(body: &Value) -> T {
    let typed: T = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), *body);
    typed
}

fn flush() -> Command {
    serde_json::from_value(vector("declarations.json")["example-context"][0].clone()).unwrap()
}

#[test]
fn every_json_vector_is_exercised_and_roundtrips() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/user-commands-v1");
    let actual: BTreeSet<_> = fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".json"))
        .collect();
    let expected: BTreeSet<_> = [
        "declarations.json",
        "describe.json",
        "describe-project.json",
        "describe-workspace.json",
        "list.json",
        "requests.json",
        "previews.json",
        "run-replies.json",
        "facts.json",
        "invalid.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(actual, expected);
    for name in actual {
        let value = vector(&name);
        let again: Value = serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(again, value, "{name}");
    }
}

#[test]
fn declaration_describe_and_merged_list_vectors() {
    let declarations = vector("declarations.json");
    for commands in declarations.as_object().unwrap().values() {
        let commands: Vec<Command> = roundtrip(commands);
        validate_commands(&commands).unwrap();
        for command in commands {
            assert_eq!(command.offers_run(), command.id == "flush");
        }
    }
    let mut describe: DescribeReply = roundtrip(&vector("describe.json"));
    describe.check().unwrap();
    assert_eq!(
        describe.commands,
        serde_json::from_value::<Vec<Command>>(declarations["example-context"].clone()).unwrap()
    );
    assert_eq!(
        describe.digest,
        "624e0f9a94469b9e398a26d8ee43f9bcc540cee3de0d33ae42a1da48640b1a53"
    );
    describe.digest = "0".repeat(64);
    assert_eq!(describe.check().unwrap_err().field, "digest");
    let list: ListReply = roundtrip(&vector("list.json"));
    list.check().unwrap();
    assert_eq!(list.commands.len(), 3);
    for entry in list.commands {
        assert!(declarations[&entry.module]
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(entry.command).unwrap()));
    }
}

#[test]
fn project_and_workspace_describe_and_run_vectors() {
    let requests = vector("requests.json");
    for (describe_name, request_name, kind) in [
        ("describe-project.json", "project_run", "project"),
        ("describe-workspace.json", "workspace_run", "workspace"),
    ] {
        let body = vector(describe_name);
        assert_eq!(body["commands"][0]["subject"], kind);
        assert_eq!(requests[request_name]["subject"]["kind"], kind);
        let describe: DescribeReply = roundtrip(&body);
        describe.check().unwrap();
        let request: ModuleRunRequest = roundtrip(&requests[request_name]);
        request.check(&describe.commands[0]).unwrap();
    }
}

#[test]
fn raw_and_typed_digests_agree_for_command_vectors() {
    use sha2::{Digest, Sha256};

    let declarations = vector("declarations.json");
    let mut arrays: Vec<Value> = declarations
        .as_object()
        .unwrap()
        .values()
        .cloned()
        .collect();
    for name in [
        "describe.json",
        "describe-project.json",
        "describe-workspace.json",
    ] {
        let body = vector(name);
        let tolerant = DescribeReply::decode_tolerant(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(tolerant.unreadable.is_empty());
        assert_eq!(tolerant.digest, body["digest"]);
        arrays.push(body["commands"].clone());
    }
    for entry in vector("list.json")["commands"].as_array().unwrap() {
        arrays.push(serde_json::json!([entry["command"]]));
    }
    for case in vector("invalid.json").as_array().unwrap() {
        if case["kind"] == "command" {
            arrays.push(serde_json::json!([case["body"]]));
        }
    }
    for raw in arrays {
        let commands: Vec<Command> = serde_json::from_value(raw.clone()).unwrap();
        let raw_digest = format!("{:x}", Sha256::digest(serde_jcs::to_vec(&raw).unwrap()));
        assert_eq!(raw_digest, commands_digest(&commands).unwrap());
    }
}

#[test]
fn preview_and_run_reply_vectors() {
    for case in vector("previews.json").as_array().unwrap() {
        let preview: Preview = roundtrip(&case["body"]);
        preview
            .check()
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
    }
    for case in vector("run-replies.json").as_array().unwrap() {
        let reply: RunReply = roundtrip(&case["body"]);
        reply
            .check()
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
    }
}

#[test]
fn module_and_app_request_vectors() {
    let bodies = vector("requests.json");
    let command = flush();
    let _: DescribeRequest = roundtrip(&bodies["describe"]);
    roundtrip::<ListRequest>(&bodies["list"]).check().unwrap();
    roundtrip::<AppPreviewRequest>(&bodies["app_preview"])
        .check(&command)
        .unwrap();
    roundtrip::<ModulePreviewRequest>(&bodies["module_preview"])
        .check(&command)
        .unwrap();
    roundtrip::<AppRunRequest>(&bodies["app_run"])
        .check(&command)
        .unwrap();
    roundtrip::<ModuleRunRequest>(&bodies["module_run"])
        .check(&command)
        .unwrap();
}

#[test]
fn fact_vectors_classify_each_error_and_roundtrip_decodable_bodies() {
    let mut classes = BTreeSet::new();
    for case in vector("facts.json").as_array().unwrap() {
        let body = &case["body"];
        let result = validate_fact(body);
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}: {result:?}",
            case["name"]
        );
        if let Err(error) = result {
            assert_eq!(error.code, case["code"], "{}", case["name"]);
            assert_eq!(error.field, case["field"], "{}", case["name"]);
            classes.insert(error.code);
        }
        if let Ok(fact) = serde_json::from_value::<Fact>(body.clone()) {
            assert_eq!(serde_json::to_value(&fact).unwrap(), *body);
            assert_eq!(fact.check().is_ok(), case["valid"].as_bool().unwrap());
        } else {
            assert!(!case["valid"].as_bool().unwrap());
        }
        let mut cell = body.clone();
        cell.as_object_mut().unwrap().remove("label");
        if case["field"] != "label" {
            assert_eq!(
                validate_fact_value(&cell).is_ok(),
                case["valid"].as_bool().unwrap()
            );
        }
    }
    assert_eq!(
        classes,
        [FACT_MALFORMED, FACT_CONFLICT, FACT_RANGE]
            .map(str::to_owned)
            .into()
    );
}

#[test]
fn invalid_body_vectors_name_the_refused_field() {
    for case in vector("invalid.json").as_array().unwrap() {
        let error = match case["kind"].as_str().unwrap() {
            "command" => roundtrip::<Command>(&case["body"]).check().unwrap_err(),
            "preview" => roundtrip::<Preview>(&case["body"]).check().unwrap_err(),
            "run" => roundtrip::<ModuleRunRequest>(&case["body"])
                .check(&flush())
                .unwrap_err(),
            kind => panic!("unexercised vector kind: {kind}"),
        };
        assert_eq!(error.code, INVALID_PARAMS);
        assert_eq!(error.field, case["field"]);
        let refusal = error.refusal();
        assert_eq!(refusal.code, INVALID_PARAMS);
        assert_eq!(refusal.detail.unwrap()["field"], case["field"]);
    }
}
