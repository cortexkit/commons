//! The `call_identity` cases, `cortexkit-role-llm-runner` CONTRACT §9.16: a
//! display tool call and the live `tool_started` / `tool_finished` events
//! carry the call's `call_key` and the `schema_pin` of the definition it was
//! dispatched against.
//!
//! The cases run only on a runner whose `role.describe` declares the group;
//! on any other runner they are not applicable, never skipped. They compare
//! what the display surfaces say with two references the runner cannot
//! influence after the fact: the call's `session.read` attribution (§5),
//! which is where its `call_key` is defined, and the `schema_pin` the
//! scripted tool provider saw on the call when it arrived
//! ([`LlmRunnerSubject::dispatched_schema_pin`]).
//!
//! The module also publishes the tool definitions an adapter installs for
//! these cases ([`DISPATCHED_DEFINITION`], [`CHANGED_DEFINITION`]), with the
//! pins `tool-provider/v1` gives them.

use std::collections::BTreeMap;

use cortexkit_role_llm_runner::{
    capabilities as groups, live::LiveEvent, read::ToolCallAttribution, subscribe::SubscribeFrom,
};
use serde_json::Value;

use crate::{
    cases::{Case, Ending},
    display::display_walk,
    drive::read_all,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
    RunnerRoute, SCRIPTED_TOOL,
};

/// A tool definition as a `tool-provider/v1` catalog gives it, reduced to
/// what its `schema_pin` covers, and that pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolDefinitionFixture {
    /// The tool's name in the provider's catalog.
    pub tool: &'static str,
    /// The tool's input schema, as JSON text.
    pub input_schema: &'static str,
    /// The tool's `semantics` version.
    pub semantics: u64,
    /// The `schema_pin` `tool-provider/v1` computes for this definition:
    /// `tp1:<tool>:<schema_digest>:<semantics>`. A test of this crate checks
    /// it against the tool-provider crate's own pin function.
    pub schema_pin: &'static str,
}

impl ToolDefinitionFixture {
    /// The input schema as a JSON value.
    pub fn input_schema(&self) -> Value {
        serde_json::from_str(self.input_schema).expect("fixture schemas are JSON")
    }
}

/// The definition of [`SCRIPTED_TOOL`] an adapter's scripted tool provider
/// serves when a session starts. Its descriptions are not part of its pin.
pub const DISPATCHED_DEFINITION: ToolDefinitionFixture = ToolDefinitionFixture {
    tool: SCRIPTED_TOOL,
    input_schema: r#"{"type":"object","description":"Echoes its marker.","properties":{"marker":{"type":"string","description":"Returned as given."}},"required":["marker"]}"#,
    semantics: 1,
    schema_pin:
        "tp1:conformance_echo:634bdb1fe768fce0974f9c89e6f15f889470a8288277c6433e4711ab6a5b9335:1",
};

/// The definition [`LlmRunnerSubject::change_tool_definition`] installs in
/// its place: a schema with one more property and a new semantics version,
/// so its pin differs from [`DISPATCHED_DEFINITION`]'s.
pub const CHANGED_DEFINITION: ToolDefinitionFixture = ToolDefinitionFixture {
    tool: SCRIPTED_TOOL,
    input_schema: r#"{"type":"object","properties":{"marker":{"type":"string"},"times":{"type":"integer","minimum":1}},"required":["marker"]}"#,
    semantics: 2,
    schema_pin:
        "tp1:conformance_echo:221b29a5b49be9c915a07dd3d19a26c62b0f4a6a3dfb44daa7c694eef37e15b7:2",
};

type Check = Result<(), String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    if !describe.declares(groups::CALL_IDENTITY) {
        return Ok(Ending::Inapplicable(
            "runner does not declare call_identity".into(),
        ));
    }
    let missing: Vec<Capability> = [
        Capability::Queue,
        Capability::TranscriptReads,
        Capability::DispatchAttribution,
    ]
    .into_iter()
    .filter(|c| !case.declared.contains(c))
    .collect();
    if !missing.is_empty() {
        return Ok(Ending::Skipped {
            missing,
            reason: "the call_identity cases compare each call's display entries with its \
                     session.read attribution"
                .into(),
        });
    }
    let surfaces = Surfaces::of(case);
    if !surfaces.display && !surfaces.live {
        return Ok(Ending::Skipped {
            missing: vec![Capability::HistoryDisplay, Capability::LiveEvents],
            reason: "the adapter declares neither display history (history_display with \
                     host_history) nor live events (live_events with streaming), so there is no \
                     display tool-call entry to read"
                .into(),
        });
    }
    match name {
        "call_identity_key_matches_attribution" => {
            key_matches(case, surfaces).await.map(|()| Ending::Passed)
        }
        "call_identity_pin_frozen_at_dispatch" => pin_frozen(case, surfaces).await,
        "call_identity_denied_call_has_no_pin" => denied(case, surfaces).await,
        other => Err(format!("unknown call_identity case {other}")),
    }
}

/// Which display surfaces the adapter declares, and so which the cases read.
#[derive(Clone, Copy)]
struct Surfaces {
    display: bool,
    live: bool,
}

impl Surfaces {
    fn of<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Self {
        let has = |c| case.declared.contains(&c);
        Self {
            display: has(Capability::HistoryDisplay) && has(Capability::HostHistory),
            live: has(Capability::LiveEvents) && has(Capability::Streaming),
        }
    }
}

/// One display tool-call entry: where it was read, the call it names, and
/// the two members this group adds.
#[derive(Debug)]
struct Shown {
    surface: &'static str,
    mid: String,
    tool_call_id: String,
    call_key: Option<String>,
    schema_pin: Option<String>,
}

/// Every display tool-call entry the session serves on the surfaces the
/// adapter declares: each `tool_call` part of display history, and each
/// replayed `tool_started` and `tool_finished` event.
async fn shown<R: RunnerRoute>(route: &R, surfaces: Surfaces) -> Result<Vec<Shown>, String> {
    let mut out = Vec::new();
    if surfaces.display {
        for entry in display_walk(route, None).await? {
            for call in entry.row.parts.iter().filter_map(|part| match part {
                cortexkit_role_llm_runner::display::DisplayPart::ToolCall(call) => Some(call),
                _ => None,
            }) {
                out.push(Shown {
                    surface: "the display history tool_call",
                    mid: entry.mid.clone(),
                    tool_call_id: call.tool_call_id.clone(),
                    call_key: call.call_key.clone(),
                    schema_pin: call.schema_pin.clone(),
                });
            }
        }
    }
    if surfaces.live {
        for frame in crate::live::stream(route, SubscribeFrom::Start).await? {
            let shown = match frame.event {
                Some(LiveEvent::ToolStarted(e)) => Shown {
                    surface: "the live tool_started event",
                    mid: e.mid,
                    tool_call_id: e.tool_call_id,
                    call_key: e.call_key,
                    schema_pin: e.schema_pin,
                },
                Some(LiveEvent::ToolFinished(e)) => Shown {
                    surface: "the live tool_finished event",
                    mid: e.mid,
                    tool_call_id: e.tool_call_id,
                    call_key: e.call_key,
                    schema_pin: e.schema_pin,
                },
                _ => continue,
            };
            out.push(shown);
        }
    }
    Ok(out)
}

/// Each call's `session.read` attribution, by the `mid` of the message
/// carrying it and the model's id.
async fn attribution<R: RunnerRoute>(
    route: &R,
) -> Result<BTreeMap<(String, String), ToolCallAttribution>, String> {
    let (_, messages) = read_all(route).await?;
    let mut out = BTreeMap::new();
    for message in messages {
        for call in message.tool_calls {
            out.insert((message.mid.clone(), call.tool_call_id.clone()), call);
        }
    }
    Ok(out)
}

/// The entries naming `call`, which must be on every declared surface: a
/// display history `tool_call` part, and with live events a `tool_finished`
/// (and a `tool_started` when `started` is set).
fn entries_of<'a>(
    all: &'a [Shown],
    call: &ScriptedToolCall,
    surfaces: Surfaces,
    started: bool,
) -> Result<Vec<&'a Shown>, String> {
    let found: Vec<&Shown> = all
        .iter()
        .filter(|s| s.tool_call_id == call.tool_call_id)
        .collect();
    let mut wanted = Vec::new();
    if surfaces.display {
        wanted.push("the display history tool_call");
    }
    if surfaces.live {
        wanted.push("the live tool_finished event");
        if started {
            wanted.push("the live tool_started event");
        }
    }
    for surface in wanted {
        if !found.iter().any(|s| s.surface == surface) {
            return Err(format!(
                "{surface} of call {} is missing, so its call identity cannot be read",
                call.tool_call_id
            ));
        }
    }
    Ok(found)
}

/// The entry's `call_key` is the one `session.read` attributes to the same
/// call, absent exactly when that is absent.
fn same_key(
    shown: &Shown,
    attributed: &BTreeMap<(String, String), ToolCallAttribution>,
) -> Result<(), String> {
    let Some(attribution) = attributed.get(&(shown.mid.clone(), shown.tool_call_id.clone())) else {
        return Err(format!(
            "{} names call {} of message {}, but session.read attributes no such call to that \
             message",
            shown.surface, shown.tool_call_id, shown.mid
        ));
    };
    if shown.call_key != attribution.call_key {
        return Err(format!(
            "{} of call {} in message {} carries call_key {:?}, but session.read attributes \
             call_key {:?} to that call: a display entry carries the same key §5 exposes, and \
             leaves it out only while §5 does",
            shown.surface, shown.tool_call_id, shown.mid, shown.call_key, attribution.call_key
        ));
    }
    Ok(())
}

/// Two sibling calls, dispatched and finished: every display entry of each
/// carries the key `session.read` attributes to that call, and a dispatched
/// call has one.
async fn key_matches<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let calls = [
        case.mint.call("identity-first"),
        case.mint.call("identity-second"),
    ];
    let mut session = case
        .open(
            "call-identity",
            Script {
                turns: vec![
                    ScriptedTurn::new(calls.iter().cloned().map(ScriptedPart::ToolCall).collect()),
                    ScriptedTurn::new(vec![case.mint.text("identity-closing").0]),
                ],
            },
        )
        .await?;
    case.run_prompt(&mut session, &case.mint.next("identity-prompt"))
        .await?;
    let attributed = attribution(&session.route).await?;
    let all = shown(&session.route, surfaces).await?;
    for call in &calls {
        for entry in entries_of(&all, call, surfaces, true)? {
            if entry.call_key.is_none() {
                return Err(format!(
                    "{} of call {}, which was dispatched, carries no call_key: a runner declaring \
                     call_identity names the key of every prepared or dispatched call",
                    entry.surface, call.tool_call_id
                ));
            }
            same_key(entry, &attributed)?;
        }
    }
    Ok(())
}

/// The pin on every display entry of a dispatched call is the one the tool
/// provider received with it, and stays that pin after the catalog gives
/// the tool another definition.
async fn pin_frozen<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let call = case.mint.call("pinned");
    let mut session = case
        .open(
            "call-identity-pin",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(call.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("pinned-closing").0]),
                    ScriptedTurn::new(vec![case.mint.text("pinned-after-change").0]),
                ],
            },
        )
        .await?;
    case.run_prompt(&mut session, &case.mint.next("pinned-prompt"))
        .await?;
    let dispatched = case
        .subject
        .dispatched_schema_pin(&call.arguments)
        .await
        .map_err(|e| {
            format!(
                "the adapter must report the schema_pin its tool provider received with each \
                 call on a runner declaring call_identity: {e}"
            )
        })?
        .ok_or_else(|| {
            format!(
                "the tool provider received call {} without a schema_pin, so no display entry \
                 can be checked against the definition it was dispatched against",
                call.tool_call_id
            )
        })?;
    let check = |all: &[Shown], when: &str| -> Check {
        for entry in entries_of(all, &call, surfaces, true)? {
            if entry.schema_pin.as_deref() != Some(dispatched.as_str()) {
                return Err(format!(
                    "{when}, {} of call {} carries schema_pin {:?}, but the call was dispatched \
                     against {dispatched:?}: the pin is the one recorded at dispatch, never one \
                     recomputed from the catalog",
                    entry.surface, call.tool_call_id, entry.schema_pin
                ));
            }
        }
        Ok(())
    };
    check(&shown(&session.route, surfaces).await?, "after the run")?;
    let changed = case
        .subject
        .change_tool_definition(&session.name)
        .await
        .map_err(|e| format!("changing the scripted tool's definition: {e}"))?;
    let Some(changed) = changed else {
        return Ok(Ending::PassedWithInapplicableRule(
            "the adapter cannot change the scripted tool's definition, so the pin was not \
             checked across a catalog change"
                .into(),
        ));
    };
    if changed == dispatched {
        return Err(format!(
            "after change_tool_definition the catalog still gives the tool pin {changed:?}, the \
             one the call was dispatched against: the fixture must change the definition"
        ));
    }
    // A run after the change, so a runner that reads its catalog only when
    // it runs has done so before the entries are read again.
    case.run_prompt(&mut session, &case.mint.next("pinned-after-prompt"))
        .await?;
    check(
        &shown(&session.route, surfaces).await?,
        "after the catalog changed the tool's definition",
    )?;
    Ok(Ending::Passed)
}

/// A call the runner refused before dispatch carries no `schema_pin`, and
/// its `call_key` only as `session.read` attributes it.
async fn denied<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let call = case.mint.call("denied");
    let mut session = case
        .open(
            "call-identity-denied",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(call.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("denied-closing").0]),
                ],
            },
        )
        .await?;
    let arranged = case
        .subject
        .deny_tool_call(&session.name, &call.arguments)
        .await
        .map_err(|e| format!("arranging the denial: {e}"))?;
    if !arranged {
        return Ok(Ending::Inapplicable(
            "the adapter cannot make the runner deny a call before dispatch".into(),
        ));
    }
    case.run_prompt(&mut session, &case.mint.next("denied-prompt"))
        .await?;
    if case.subject.tool_invocations(&call.arguments).await != 0 {
        return Err(format!(
            "the denied call {} reached the tool provider: the denial fixture must refuse it \
             before dispatch",
            call.tool_call_id
        ));
    }
    let attributed = attribution(&session.route).await?;
    let all = shown(&session.route, surfaces).await?;
    for entry in entries_of(&all, &call, surfaces, false)? {
        if let Some(pin) = &entry.schema_pin {
            return Err(format!(
                "{} of the denied call {} carries schema_pin {pin:?}: a call never dispatched \
                 was dispatched against no definition, so it has no pin",
                entry.surface, call.tool_call_id
            ));
        }
        same_key(entry, &attributed)?;
    }
    Ok(Ending::Passed)
}
