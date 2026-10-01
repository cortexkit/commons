//! Requests the cases share: sending, reading and waiting for a run, each
//! decoded with the role crate's types.

use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};

use cortexkit_role_llm_runner::{
    errors, ops,
    read::{HeadMeta, ReadMessage, ReadPage, ReadRequest},
    run::{RunResult, RunResultRequest, RunState},
    send::SendReply,
};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

use crate::{
    route::{Reply, RunnerRoute},
    subject::{Capability, LlmRunnerSubject, ScriptedPart, ScriptedToolCall},
    SCRIPTED_TOOL,
};

/// How many times the suite asks whether a run has ended before it gives
/// up, pausing between asks.
pub const MAX_POLLS: usize = 400;

/// How many pages a full read follows before it gives up.
const MAX_PAGES: usize = 10_000;

/// Mints names and markers that are unique within one run, so a session
/// name, a `send_id` or a marker string is never used twice.
#[derive(Default)]
pub struct Mint(AtomicU64);

impl Mint {
    pub fn next(&self, what: &str) -> String {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        format!("cf-{what}-{n}")
    }

    /// A text part holding a fresh marker. Returns the part and its marker.
    pub fn text(&self, what: &str) -> (ScriptedPart, String) {
        let marker = self.next(what);
        (ScriptedPart::Text(marker.clone()), marker)
    }

    /// A call to the scripted tool with fresh, unique arguments and result.
    pub fn call(&self, tool_call_id: &str) -> ScriptedToolCall {
        let marker = self.next("call");
        ScriptedToolCall {
            tool_call_id: tool_call_id.to_owned(),
            tool: SCRIPTED_TOOL.to_owned(),
            arguments: json!({ "marker": format!("{marker}-args") }),
            result: json!({ "marker": format!("{marker}-result") }),
            hold: false,
        }
    }
}

/// The marker string inside a scripted call's result.
pub fn result_marker(call: &ScriptedToolCall) -> String {
    call.result["marker"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

pub async fn call<R: RunnerRoute>(route: &R, method: &str, params: Value) -> Result<Reply, String> {
    route
        .request(method, params)
        .await
        .map_err(|e| format!("{method}: the route failed: {}", e.message))
}

pub fn expect_ok(what: &str, reply: Reply) -> Result<Value, String> {
    match reply {
        Reply::Response(body) => Ok(body),
        Reply::Error(error) => Err(format!(
            "{what} was refused {} {:?}: {}",
            error.code, error.detail, error.message
        )),
    }
}

pub fn expect_error(what: &str, reply: Reply) -> Result<ErrorBody, String> {
    match reply {
        Reply::Error(error) => Ok(error),
        Reply::Response(body) => Err(format!("{what} was answered {body}, not refused")),
    }
}

pub fn decode<T: serde::de::DeserializeOwned>(what: &str, body: Value) -> Result<T, String> {
    serde_json::from_value(body.clone())
        .map_err(|e| format!("{what} answer {body} does not decode: {e}"))
}

pub fn expect_code(what: &str, error: &ErrorBody, code: &str) -> Result<(), String> {
    if error.code == code {
        Ok(())
    } else {
        Err(format!(
            "{what} was refused {} {:?}, not {code}",
            error.code, error.detail
        ))
    }
}

/// Check that a refusal has the error code `code` and names `field` as its
/// `detail.field`, as `invalid_params` and `send_id_reuse` refusals do.
pub fn expect_field(what: &str, error: &ErrorBody, code: &str, field: &str) -> Result<(), String> {
    if error.code == code
        && errors::refused_field(&error.code, error.detail.as_ref()) == Some(field)
    {
        Ok(())
    } else {
        Err(format!(
            "{what} was refused {} {:?}, not {code} {{field: \"{field}\"}}",
            error.code, error.detail
        ))
    }
}

pub fn request_params(request: &ReadRequest) -> Value {
    serde_json::to_value(request).expect("a read request serializes")
}

pub async fn read<R: RunnerRoute>(route: &R, request: &ReadRequest) -> Result<ReadPage, String> {
    let body = expect_ok(
        "session.read",
        call(route, ops::SESSION_READ, request_params(request)).await?,
    )?;
    decode("session.read", body)
}

pub async fn read_refusal<R: RunnerRoute>(
    route: &R,
    what: &str,
    request: &ReadRequest,
) -> Result<ErrorBody, String> {
    expect_error(
        what,
        call(route, ops::SESSION_READ, request_params(request)).await?,
    )
}

/// `session.head`, checked for consistency with its lineage.
pub async fn head<R: RunnerRoute>(route: &R) -> Result<HeadMeta, String> {
    let body = expect_ok(
        "session.head",
        call(route, ops::SESSION_HEAD, json!({})).await?,
    )?;
    let head: HeadMeta = decode("session.head", body.clone())?;
    if !head.lineage_consistent() {
        return Err(format!(
            "session.head answered {body}, inconsistent with its lineage"
        ));
    }
    Ok(head)
}

/// Every message of the transcript, read forward from ordinal 0 by
/// following `next_from_ordinal`, with every page read.
pub async fn read_all<R: RunnerRoute>(
    route: &R,
) -> Result<(Vec<ReadPage>, Vec<ReadMessage>), String> {
    let mut pages = Vec::new();
    let mut messages: Vec<ReadMessage> = Vec::new();
    let mut from = 0;
    for _ in 0..MAX_PAGES {
        let page = read(route, &ReadRequest::range(from)).await?;
        messages.extend(page.messages.iter().cloned());
        let next = page.next_from_ordinal;
        let empty = page.messages.is_empty();
        pages.push(page);
        match next {
            None => return Ok((pages, messages)),
            Some(_) if empty => {
                return Err(format!(
                    "a range read from {from} answered no message but next_from_ordinal {next:?}"
                ))
            }
            Some(next) if next <= from => {
                return Err(format!(
                    "a range read from {from} answered next_from_ordinal {next}, which does not move forward"
                ))
            }
            Some(next) => from = next,
        }
    }
    Err(format!(
        "the transcript did not end within {MAX_PAGES} pages"
    ))
}

/// The ordinals and mids of `messages` are each used once.
pub fn check_unique(what: &str, messages: &[ReadMessage]) -> Result<(), String> {
    let mut ordinals = BTreeSet::new();
    let mut mids = BTreeSet::new();
    for message in messages {
        if !ordinals.insert(message.ordinal) {
            return Err(format!("{what}: ordinal {} is used twice", message.ordinal));
        }
        if !mids.insert(message.mid.as_str()) {
            return Err(format!("{what}: mid {} is used twice", message.mid));
        }
    }
    Ok(())
}

/// How many messages hold `marker` somewhere in their body.
pub fn holding(messages: &[ReadMessage], marker: &str) -> usize {
    messages
        .iter()
        .filter(|message| message.message.to_string().contains(marker))
        .count()
}

pub async fn run_result<R: RunnerRoute>(route: &R, run_id: &str) -> Result<RunResult, String> {
    let params = serde_json::to_value(RunResultRequest::new(run_id)).expect("serializes");
    let body = expect_ok("run.result", call(route, ops::RUN_RESULT, params).await?)?;
    let result: RunResult = decode("run.result", body.clone())?;
    if !result.final_message_consistent() {
        return Err(format!(
            "run.result answered {body}, whose final_message disagrees with its state"
        ));
    }
    Ok(result)
}

/// The `session.send` params: the subject's fields for the session, then
/// the role's.
pub fn send_params<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
    first: bool,
    prompt: &str,
    send_id: &str,
    delivery: Option<&str>,
) -> Value {
    let mut params: Map<String, Value> = subject.send_fields(session, first);
    params.insert("prompt".into(), json!(prompt));
    params.insert("send_id".into(), json!(send_id));
    if let Some(delivery) = delivery {
        params.insert("delivery".into(), json!(delivery));
    }
    Value::Object(params)
}

/// The run the suite waited for, once it has a terminal state.
#[derive(Clone, Debug)]
pub struct RunEnd {
    pub run_id: String,
}

/// Wait until the run a send started has a terminal state. `reply` is the
/// send's reply; when it names no run, the run is the first one whose id is
/// not `before`. Observed through `session.head` when `transcript_reads` is
/// declared, otherwise through `run.result` and the run attribution on
/// `session.read` (both in `run_ops`).
pub async fn wait_run_end<S: LlmRunnerSubject>(
    subject: &S,
    route: &S::Route,
    declared: &BTreeSet<Capability>,
    before: Option<&str>,
    reply: &SendReply,
) -> Result<RunEnd, String>
where
    S::Route: RunnerRoute,
{
    let ours = |run_id: &str| match &reply.run_id {
        Some(id) => id == run_id,
        None => Some(run_id) != before,
    };
    for _ in 0..MAX_POLLS {
        if declared.contains(&Capability::TranscriptReads) {
            let head = head(route).await?;
            if let Some(last) = head.last_run_state {
                if ours(&last.run_id) && RunState::parse(&last.state).is_terminal() {
                    return Ok(RunEnd {
                        run_id: last.run_id,
                    });
                }
            }
        } else if declared.contains(&Capability::RunOps) {
            let run_id = match &reply.run_id {
                Some(id) => Some(id.clone()),
                None => read(route, &ReadRequest::tail())
                    .await?
                    .messages
                    .iter()
                    .rev()
                    .filter_map(|m| m.run.as_ref())
                    .map(|run| run.run_id.clone())
                    .find(|id| ours(id)),
            };
            if let Some(run_id) = run_id {
                let result = run_result(route, &run_id).await?;
                if result.run_state().is_terminal() {
                    return Ok(RunEnd { run_id });
                }
            }
        } else {
            return Err(
                "the suite cannot observe a run without transcript_reads or run_ops".into(),
            );
        }
        subject.pause().await;
    }
    Err(format!("the run did not end within {MAX_POLLS} polls"))
}

/// Whether `key` is a well-formed `call_key`: 1 to 256 bytes, every byte
/// printable ASCII from 0x21 to 0x7E (the bounds the role takes from
/// `tool-provider/v1`).
pub fn call_key_well_formed(key: &str) -> bool {
    (1..=256).contains(&key.len()) && key.bytes().all(|b| (0x21..=0x7E).contains(&b))
}
