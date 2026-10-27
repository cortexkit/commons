//! Hand-written Draft 2020-12 schemas: the reused role types have no schema derive.
//! Semantic and catalog-dependent checks remain in validators/the serving module.

use serde_json::{json, Value};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}

fn document(mut schema: Value) -> Value {
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema
}

fn text() -> Value {
    json!({"type":"string"})
}
fn nonempty() -> Value {
    json!({"type":"string","minLength":1})
}
fn cost() -> Value {
    json!({"type":"number","minimum":0})
}

fn usage() -> Value {
    let count = json!({"type":"integer","minimum":0,"maximum":u64::MAX});
    object(
        json!({"input_tokens":count,"cached_input_tokens":count,"cache_write_tokens":count,
        "output_tokens":count,"reasoning_tokens":count}),
        &[],
    )
}

/// An item image. The `{handle}` reference form is listed only when
/// `references` is set: a provider's tool schema offers it only in a release
/// that resolves references.
fn image(references: bool) -> Value {
    let mut forms = vec![
        json!({"type":"string"}),
        object(
            json!({"content_type":text(),"base64":text()}),
            &["content_type", "base64"],
        ),
    ];
    if references {
        forms.push(object(
            json!({"handle":{"type":"string","pattern":"^[A-Za-z0-9._:-]{1,128}$"}}),
            &["handle"],
        ));
    }
    json!({"oneOf":forms})
}

fn images(references: bool) -> Value {
    json!({"type":"array","items":image(references)})
}

fn question() -> Value {
    let instructions = json!({"anyOf":[nonempty(),{"type":"object"},{"type":"array"}]});
    let mut schema = object(
        json!({
            "type":{"enum":["noul","choice","score"]}, "instructions":instructions,
            "criteria":{"anyOf":[{"type":"object"},{"type":"array"}]}
        }),
        &["type", "instructions"],
    );
    schema["allOf"] = json!([
        {"if":{"properties":{"type":{"const":"noul"}}},"then":{"properties":{"criteria":
            object(json!({"true":text(),"false":text()}), &["true","false"])}}},
        {"if":{"properties":{"type":{"const":"choice"}}},"then":{"required":["criteria"],
            "properties":{"criteria":{"type":"object","minProperties":2,"maxProperties":255}}}},
        {"if":{"properties":{"type":{"const":"score"}}},"then":{"required":["criteria"],
            "properties":{"criteria":{"type":"array","minItems":2,"maxItems":10}}}}
    ]);
    schema
}

pub(crate) fn classify_input(references: bool) -> Value {
    document(object(
        json!({"model":nonempty(), "max_cost_usd":cost(),
            "questions":{"type":"object","minProperties":1,
                "propertyNames":{"pattern":"^[a-z0-9_-]{1,64}$"},"additionalProperties":question()},
            "item":object(json!({"state":{"type":["string","object","array"]},"images":images(references)}), &["state"])
        }),
        &["model", "questions", "item", "max_cost_usd"],
    ))
}

fn field() -> Value {
    let mut branches = Vec::new();
    for kind in cortexkit_role_extractor::field::types::ALL {
        let mut properties = json!({"type":{"const":kind},"instructions":nonempty(),
            "null_threshold":{"type":"number","exclusiveMinimum":0,"maximum":1}});
        let mut required = vec!["type"];
        if ["number", "duration"].contains(kind) {
            properties["integer"] = json!({"type":"boolean"});
            properties["unit"] = if *kind == "duration" {
                json!({"enum":["seconds","minutes","hours","days","weeks","months","years"]})
            } else {
                nonempty()
            };
        }
        if ["number", "duration", "list"].contains(kind) {
            properties["decimals"] = json!({"type":"integer","minimum":0,"maximum":6});
        }
        if *kind == "list" {
            properties["items"] = json!({"enum":["text","number","date"]});
            required.push("items");
        }
        if ["estimate", "forecast"].contains(kind) {
            let bound = if *kind == "estimate" {
                json!({"type":["number","string"]})
            } else {
                json!({"type":"number"})
            };
            properties["min"] = bound.clone();
            properties["max"] = bound;
            properties["step"] = if *kind == "estimate" {
                json!({"anyOf":[{"type":"number","exclusiveMinimum":0},{"const":"month"}]})
            } else {
                json!({"type":"number","exclusiveMinimum":0})
            };
            properties["scale"] = json!({"enum":["linear","log"]});
            required.extend(["min", "max"]);
        }
        if *kind == "forecast" {
            properties["horizon"] = json!({"type":"integer","minimum":1,"maximum":64});
            properties["interval"] =
                json!({"enum":["step","minute","hour","day","week","month","quarter","year"]});
            required.extend(["horizon", "interval"]);
        }
        branches.push(object(properties, &required));
    }
    json!({"oneOf":branches})
}

pub(crate) fn extract_input(references: bool) -> Value {
    let mut item = object(
        json!({"state":text(),"images":images(references),
        "reference_date":{"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"}}),
        &["state"],
    );
    item["properties"]["images"]["minItems"] = json!(1);
    item["anyOf"] = json!([{"properties":{"state":{"minLength":1}}},{"required":["images"]}]);
    document(object(
        json!({"model":nonempty(),"max_cost_usd":cost(),"item":item,
            "fields":{"type":"object","minProperties":1,
                "propertyNames":{"pattern":"^[A-Za-z_][A-Za-z0-9_-]{0,63}$"},"additionalProperties":field()},
            "options":object(json!({"quantiles":{"type":"boolean"},"state_text":{"type":"boolean"}}), &[])
        }),
        &["model", "fields", "item", "max_cost_usd"],
    ))
}

pub(crate) fn role_result(extract: bool) -> Value {
    // Answer types intentionally decode open, including future type names and members.
    let mut answer = json!({"type":"object","properties":{"type":text()},"required":["type"]});
    if !extract {
        answer["properties"]["noul"] = json!({"type":"number"});
        answer["properties"]["choice"] = text();
        answer["properties"]["score"] = json!({"type":"number"});
        answer["properties"]["legend"] = json!({"type":"object"});
        answer["properties"]["probabilities"] =
            json!({"type":"object","additionalProperties":{"type":"number"}});
        answer["properties"]["confidence"] = json!({"type":"number"});
    }
    let mut properties = json!({"answers":{"type":"object","additionalProperties":answer},"usage":usage(),"cost_usd":cost()});
    if extract {
        properties["truncated"] = json!({"type":"boolean"});
        properties["state_text"] = text();
    }
    document(object(properties, &["answers", "usage", "cost_usd"]))
}

pub(crate) fn complete_input() -> Value {
    let rating = json!({"type":"integer","minimum":0,"maximum":100});
    document(object(
        json!({
            "demand":object(json!({"iq":rating,"eq":rating}), &["iq"]),
            "prompt":nonempty(),"system":text(),"output_schema":{"type":"object"},
            "max_output_tokens":{"type":"integer","minimum":1,"maximum":u64::MAX},"max_cost_usd":cost()
        }),
        &["demand", "prompt", "max_output_tokens", "max_cost_usd"],
    ))
}

pub(crate) fn complete_result() -> Value {
    let mut schema = object(
        json!({"text":text(),"json":{},"usage":usage(),"cost_usd":cost(),
            "model":object(json!({"provider":nonempty(),"model":nonempty(),"variant":text()}), &["provider","model"])
        }),
        &["model", "usage", "cost_usd"],
    );
    schema["oneOf"] = json!([{"required":["text"],"not":{"required":["json"]}},
        {"required":["json"],"not":{"required":["text"]}}]);
    document(schema)
}
