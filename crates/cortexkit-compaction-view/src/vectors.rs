//! The supported operations listed in [`FUNCTIONS`] are available through one
//! JSON dispatcher, [`call`], with test vectors built on it. Not every public
//! Rust helper is a dispatcher operation.
//!
//! A runner that is not written in Rust can use the WebAssembly build, which
//! exposes the dispatcher through its pointer/length ABI. A runner implementing
//! these operations itself checks `vectors/cases.json`: for every [`Vector`],
//! calling its function with its input must give exactly its output. The vector
//! file is generated from this library and compared with its current outputs by
//! the crate's tests.

use cortexkit_role_compaction::part::{body_digest, Part, PartBody, Seal};
use cortexkit_role_compaction::read::{entry_digest, Entry, NoticeOrigin, Role};
use cortexkit_role_compaction::rules::ProviderRules;
use cortexkit_role_compaction::view::{Edit, View};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{clean, render, rules, tags};

/// One test vector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    /// What the case shows, in a few words.
    pub name: String,
    /// One of [`FUNCTIONS`].
    pub function: String,
    pub input: Value,
    /// What [`call`] answers for `input`: `{"ok": …}` or `{"refused": …}`.
    pub output: Value,
}

/// The functions [`call`] serves, with the input each takes.
pub const FUNCTIONS: &[(&str, &str)] = &[
    (
        "plan_view",
        "{entries, view, rules, signed_floor?}: the view-aware render plan with authorized newer-signature stripping",
    ),
    (
        "plan_view_at",
        "{version, entries, view, rules, signed_floor?}: the view-aware plan at a supported plan version",
    ),
    (
        "apply_with_rules",
        "{entries, view, rules, signed_floor?}: rendered messages with provider rules and newer-signature stripping",
    ),
    ("plan", "{entries, edits}: the render plan, or the refusal"),
    (
        "plan_at",
        "{version, entries, edits}: the render plan at an earlier plan version, or the refusal",
    ),
    (
        "apply",
        "{entries, view}: the rendered messages, or the refusal (checksum included)",
    ),
    (
        "edits_for_transcript",
        "{entries, edits}: the edits filtered by which messages remain; start inserts and unknown operations or anchors stay",
    ),
    (
        "check_carried",
        "{previous?, next}: whether each edit of the new view claims an honest `since`",
    ),
    (
        "first_change",
        "{before, after}: two render plans of the same transcript; where `after` first differs from `before`",
    ),
    (
        "cleared_ride",
        "{form, route_clears}: how a ride delivered in `form` renders on the session's current route",
    ),
    (
        "check_rules",
        "{entries, edits, rules, signed_floor?}: whether every signed part the view keeps stays valid",
    ),
    (
        "rules_defaults",
        "{model}: the rules a runner declares for a model",
    ),
    (
        "assign_tags",
        "{checkpoint, entries}: the entries with their parts numbered",
    ),
    (
        "tag_numbers_in",
        "{text}: the tag numbers written in the text that fit a u64; oversized tag notation is left out",
    ),
    (
        "clean_reply",
        "{text}: a cleaned {model_tag, text} candidate; the caller applies W4.6's empty-text policy",
    ),
    (
        "entry_digest",
        "{role, notice?, parts}: an entry's checksum",
    ),
    ("body_digest", "{seal?, body}: a part's checksum"),
    ("seal_view", "{version, edits, built_through?, new_signed?}: the view with its checksum"),
    (
        "plan_version",
        "{}: the version of what this library renders (`PLAN_VERSION`)",
    ),
];

/// Call one of the supported operations in [`FUNCTIONS`] with JSON input. Answers
/// `{"ok": value}`, `{"refused": refusal}` for a view the function refuses,
/// or `{"error": message}` for an unknown function or malformed input.
pub fn call(function: &str, input: Value) -> Value {
    match dispatch(function, input) {
        Ok(Outcome::Ok(value)) => json!({ "ok": value }),
        Ok(Outcome::Refused(value)) => json!({ "refused": value }),
        Err(message) => json!({ "error": message }),
    }
}

enum Outcome {
    Ok(Value),
    Refused(Value),
}

fn dispatch(function: &str, input: Value) -> Result<Outcome, String> {
    Ok(match function {
        "plan_view" | "apply_with_rules" => {
            let input: ViewRules = parse(input)?;
            let floor = input.signed_floor.as_deref();
            if function == "plan_view" {
                refusable(render::plan_view(
                    &input.entries,
                    &input.view,
                    &input.rules,
                    floor,
                ))
            } else {
                refusable(render::apply_with_rules(
                    &input.entries,
                    &input.view,
                    &input.rules,
                    floor,
                ))
            }
        }
        "plan_view_at" => {
            let input: VersionedViewRules = parse(input)?;
            match render::plan_view_at(
                input.version,
                &input.input.entries,
                &input.input.view,
                &input.input.rules,
                input.input.signed_floor.as_deref(),
            ) {
                Some(result) => refusable(result),
                None => return Err(format!("unsupported plan version {}", input.version)),
            }
        }
        "plan" => {
            let input: EntriesEdits = parse(input)?;
            refusable(render::plan(&input.entries, &input.edits))
        }
        "plan_at" => {
            let input: VersionedEdits = parse(input)?;
            match render::plan_at(input.version, &input.entries, &input.edits) {
                Some(result) => refusable(result),
                None => return Err(format!("unsupported plan version {}", input.version)),
            }
        }
        "apply" => {
            let input: EntriesView = parse(input)?;
            refusable(render::apply(&input.entries, &input.view))
        }
        "edits_for_transcript" => {
            let input: EntriesEdits = parse(input)?;
            ok(render::edits_for_transcript(&input.entries, &input.edits))
        }
        "check_carried" => {
            let input: Carried = parse(input)?;
            refusable(render::check_carried(input.previous.as_ref(), &input.next))
        }
        "first_change" => {
            let input: Plans = parse(input)?;
            ok(render::first_change(&input.before, &input.after))
        }
        "cleared_ride" => {
            let input: RideRoute = parse(input)?;
            ok(render::cleared_ride(input.form, input.route_clears))
        }
        "check_rules" => {
            let input: RulesInput = parse(input)?;
            refusable(rules::check(
                &input.entries,
                &input.edits,
                &input.rules,
                input.signed_floor.as_deref(),
            ))
        }
        "rules_defaults" => {
            let input: Model = parse(input)?;
            ok(rules::defaults(&input.model))
        }
        "assign_tags" => {
            let mut input: Numbering = parse(input)?;
            tags::assign_tags(input.checkpoint, &mut input.entries);
            ok(input.entries)
        }
        "tag_numbers_in" => {
            let input: Text = parse(input)?;
            ok(tags::tag_numbers_in(&input.text))
        }
        "clean_reply" => {
            let input: Text = parse(input)?;
            ok(clean::clean_reply(&input.text))
        }
        "entry_digest" => {
            let input: EntryParts = parse(input)?;
            ok(entry_digest(
                input.role,
                input.notice.as_ref(),
                &input.parts,
            ))
        }
        "body_digest" => {
            let input: Body = parse(input)?;
            ok(body_digest(input.seal, &input.body))
        }
        "seal_view" => {
            let input: Sealing = parse(input)?;
            ok(View::seal_with_policy(
                input.version,
                input.edits,
                input.built_through,
                input.new_signed,
            ))
        }
        "plan_version" => ok(crate::PLAN_VERSION),
        other => return Err(format!("unknown function {other}")),
    })
}

fn parse<T: DeserializeOwned>(input: Value) -> Result<T, String> {
    serde_json::from_value(input).map_err(|error| format!("malformed input: {error}"))
}

fn ok<T: Serialize>(value: T) -> Outcome {
    Outcome::Ok(serde_json::to_value(value).expect("library types serialize"))
}

fn refusable<T: Serialize, E: Serialize>(result: Result<T, E>) -> Outcome {
    match result {
        Ok(value) => ok(value),
        Err(refusal) => {
            Outcome::Refused(serde_json::to_value(refusal).expect("refusals serialize"))
        }
    }
}

#[derive(Deserialize)]
struct EntriesEdits {
    entries: Vec<Entry>,
    edits: Vec<Edit>,
}

#[derive(Deserialize)]
struct VersionedEdits {
    version: u32,
    entries: Vec<Entry>,
    edits: Vec<Edit>,
}

#[derive(Deserialize)]
struct EntriesView {
    entries: Vec<Entry>,
    view: View,
}

#[derive(Deserialize)]
struct ViewRules {
    entries: Vec<Entry>,
    view: View,
    rules: ProviderRules,
    #[serde(default)]
    signed_floor: Option<String>,
}

#[derive(Deserialize)]
struct VersionedViewRules {
    version: u32,
    #[serde(flatten)]
    input: ViewRules,
}

#[derive(Deserialize)]
struct Carried {
    #[serde(default)]
    previous: Option<View>,
    next: View,
}

#[derive(Deserialize)]
struct Plans {
    before: render::RenderPlan,
    after: render::RenderPlan,
}

#[derive(Deserialize)]
struct RulesInput {
    entries: Vec<Entry>,
    edits: Vec<Edit>,
    rules: ProviderRules,
    #[serde(default)]
    signed_floor: Option<String>,
}

#[derive(Deserialize)]
struct Model {
    model: String,
}

#[derive(Deserialize)]
struct RideRoute {
    form: cortexkit_role_compaction::part::RideForm,
    route_clears: bool,
}

#[derive(Deserialize)]
struct Numbering {
    checkpoint: u64,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

#[derive(Deserialize)]
struct EntryParts {
    role: Role,
    #[serde(default)]
    notice: Option<NoticeOrigin>,
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    seal: Seal,
    #[serde(flatten)]
    body: PartBody,
}

#[derive(Deserialize)]
struct Sealing {
    version: u64,
    edits: Vec<Edit>,
    #[serde(default)]
    built_through: Option<String>,
    #[serde(default)]
    new_signed: Option<String>,
}
