//! Tool-enabled fork probes; all observations come from provider boundaries.

use super::*;
use cortexkit_role_llm_runner::{
    capabilities,
    fork_answer::{ForkFinish, FramingPlacement, ANSWER_NOW, TOOL_UNAVAILABLE},
};

const UNMARKED: &str = "conformance_fork_unmarked";
const UNLISTED: &str = "conformance_fork_unlisted";

/// One received tool call, including facts attested by its actual route bind.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ForkToolCallObservation {
    pub body: Value,
    pub head_session_route: bool,
    pub held_or_yield_opt_ins: bool,
}

impl ForkToolCallObservation {
    pub fn new(body: Value, head_session_route: bool, held_or_yield_opt_ins: bool) -> Self {
        Self {
            body,
            head_session_route,
            held_or_yield_opt_ins,
        }
    }
}

fn exact(value: &Value, needle: &str) -> usize {
    match value {
        Value::String(text) => usize::from(text == needle),
        Value::Array(values) => values.iter().map(|v| exact(v, needle)).sum(),
        Value::Object(values) => values.values().map(|v| exact(v, needle)).sum(),
        _ => 0,
    }
}

fn check_calls(
    name: &str,
    calls: &[ForkToolCallObservation],
    count: usize,
    fork: &str,
    pin: &str,
) -> Check {
    let mut keys = BTreeSet::new();
    if calls.len() != count {
        return Err(format!(
            "{name}: expected {count} tool dispatches, got {}",
            calls.len()
        ));
    }
    for call in calls {
        let key = call.body["call_key"].as_str().filter(|key| !key.is_empty());
        if !call.head_session_route
            || call.held_or_yield_opt_ins
            || call.body["fork_id"] != fork
            || call.body["schema_pin"] != pin
            || key.is_none_or(|key| !keys.insert(key))
        {
            return Err(format!("{name}: call lost fork identity, head route, unique key, frozen pin or no-opt-in transport"));
        }
    }
    Ok(())
}

fn check_framing(
    name: &str,
    bytes: &[u8],
    placement: FramingPlacement,
    framing: &str,
    question: &str,
) -> Check {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let messages = value
        .get("messages")
        .or_else(|| value.get("input"))
        .and_then(Value::as_array)
        .ok_or("model request has no conversation")?;
    let last = messages.last().ok_or("model request has no question")?;
    let valid = match placement {
        FramingPlacement::System => {
            messages.len() >= 2 && {
                let frame = &messages[messages.len() - 2];
                matches!(frame["role"].as_str(), Some("system" | "developer"))
                    && exact(frame, framing) == 1
                    && exact(last, question) == 1
            }
        }
        FramingPlacement::Prefix => exact(last, &format!("{framing}\n\n{question}")) == 1,
    };
    if !valid {
        return Err(format!(
            "{name}: framing did not use the declared placement immediately before the question"
        ));
    }
    Ok(())
}

fn check_cap(name: &str, requests: &[Vec<u8>], reply: &ForkAskReply) -> Check {
    if requests.len() != 8
        || reply.finish != ForkFinish::StepLimit
        || reply.usage.steps != 8
        || reply.usage.input_tokens != Some(136)
        || reply.usage.output_tokens != Some(88)
        || !reply.answer.contains("cap-answer")
    {
        return Err(format!(
            "{name}: expected eight requests, aggregate usage, text so far and step_limit"
        ));
    }
    for (i, bytes) in requests.iter().enumerate() {
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if exact(&value, ANSWER_NOW) != usize::from(i == 7) {
            return Err(format!(
                "{name}: fixed answer-now message must occur once, on request eight only"
            ));
        }
    }
    Ok(())
}

fn check_base(name: &str, base: &[u8], request: &[u8]) -> Check {
    raw::extends_base(base, request).map_err(|e| format!("{name}: {e}"))
}

async fn check_undeclared<R: RunnerRoute>(name: &str, route: &R, fork: &str) -> Check {
    for (member, value) in [("allowed_tools", json!([])), ("framing", json!("frame"))] {
        let mut request = params(fork, "undeclared", "question");
        request[member] = value;
        let error = expect_error(name, call(route, ops::SESSION_FORK_ASK, request).await?)?;
        expect_field(name, &error, errors::INVALID_PARAMS, member)?;
    }
    Ok(())
}

async fn check_invalid<R: RunnerRoute>(name: &str, route: &R, fork: &str) -> Check {
    for (member, value) in [
        ("allowed_tools", Value::Null),
        ("allowed_tools", json!("read")),
        ("allowed_tools", json!([""])),
        ("allowed_tools", json!(["read", "read"])),
        ("allowed_tools", json!([1])),
        (
            "allowed_tools",
            json!((0..33).map(|i| i.to_string()).collect::<Vec<_>>()),
        ),
        ("framing", Value::Null),
        ("framing", json!(1)),
        ("framing", json!("")),
        ("framing", json!("é".repeat(2049))),
    ] {
        let mut request = params(fork, "invalid", "question");
        request[member] = value;
        let error = expect_error(name, call(route, ops::SESSION_FORK_ASK, request).await?)?;
        expect_field(name, &error, errors::INVALID_PARAMS, member)?;
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    name: &str,
) -> Check<Ending> {
    let describe = case.checked_describe().await?;
    let declared = describe.declares(capabilities::FORK_TOOLS);
    if name == "fork_tools_undeclared" {
        if declared {
            return Ok(Ending::Inapplicable("runner declares fork_tools".into()));
        }
        let session = seed(case, name, vec![], None).await?;
        let fork = prepare(&session.route).await?;
        check_undeclared(name, &session.route, &fork).await?;
        return Ok(Ending::Passed);
    }
    if !declared {
        return Ok(Ending::Inapplicable(
            "runner does not declare fork_tools".into(),
        ));
    }
    if name == "fork_tools_routed_call_refused" && !describe.declares(capabilities::TOOL_ROUTES) {
        return Ok(Ending::Inapplicable(
            "runner does not declare tool_routes".into(),
        ));
    }
    if name == "fork_tools_invalid" {
        let session = seed(case, name, vec![], None).await?;
        let fork = prepare(&session.route).await?;
        let before = requests(case, &session.name).await?.len();
        check_invalid(name, &session.route, &fork).await?;
        if requests(case, &session.name).await?.len() != before {
            return Err(format!("{name}: invalid request reached the model"));
        }
        return Ok(Ending::Passed);
    }
    let mut tool = case.mint.call("fork-read");
    let result_marker = tool.result["marker"]
        .as_str()
        .ok_or("scripted result needs marker")?
        .to_owned();
    let turns = match name {
        "fork_tools_allowed_call" => vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
            text("done"),
        ],
        "fork_tools_routed_call_refused" => {
            tool.arguments["machine"] = json!("remote");
            vec![
                ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
                text("continued"),
            ]
        }
        "fork_tools_denied_calls" => {
            tool.tool = UNMARKED.into();
            let mut unlisted = case.mint.call("fork-unlisted");
            unlisted.tool = UNLISTED.into();
            vec![
                ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
                ScriptedTurn::new(vec![ScriptedPart::ToolCall(unlisted)]),
                text("continued"),
            ]
        }
        "fork_tools_step_limit" => (0..8)
            .map(|i| {
                let tool = case.mint.call(&format!("fork-step-{i}"));
                let mut parts = Vec::new();
                if i == 7 {
                    parts.push(ScriptedPart::Text("cap-answer".into()));
                }
                parts.push(ScriptedPart::ToolCall(tool));
                ScriptedTurn::new(parts).with_usage(ScriptedUsage::new(17, 3, 0, 11))
            })
            .collect(),
        "fork_tools_framing" | "fork_tools_identity" => vec![text("answer")],
        _ => return Err(format!("unknown fork tools check {name}")),
    };
    let mut all = vec![text("head answer")];
    all.extend(turns);
    let session = case.open(name, Script { turns: all }).await?;
    let Some(provider) = case
        .subject
        .configure_fork_tools(&session.name, name == "fork_tools_routed_call_refused")
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Ending::Inapplicable(
            "adapter has no fork_tools catalog fixture".into(),
        ));
    };
    let session = start(case, session, None).await?;
    let base = base_request(case, &session.name).await?;
    let fork = prepare(&session.route).await?;
    let framing = "quick-question-frame";
    let question = "quick-question-body";
    let mut request = params(&fork, "ask", question);
    request["allowed_tools"] = json!([crate::SCRIPTED_TOOL, UNMARKED]);
    request["framing"] = json!(framing);
    let first = expect_ok(
        name,
        call(&session.route, ops::SESSION_FORK_ASK, request.clone()).await?,
    )?;
    let reply: ForkAskReply = decode(name, first.clone())?;
    let branch: Vec<Vec<u8>> = requests(case, &session.name)
        .await?
        .into_iter()
        .filter(|r| r.fork_id.as_deref() == Some(&fork))
        .map(|r| r.body)
        .collect();
    if branch.is_empty() {
        return Err(format!("{name}: no branch model requests"));
    }
    for bytes in &branch {
        check_base(name, &base, bytes)?;
    }
    match name {
        "fork_tools_allowed_call"
        | "fork_tools_denied_calls"
        | "fork_tools_step_limit"
        | "fork_tools_routed_call_refused" => {
            let calls = case
                .subject
                .fork_tools_calls(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let pin = case
                .subject
                .fork_tools_schema_pin()
                .ok_or("fork_tools fixture omitted its default schema pin")?;
            let count = match name {
                "fork_tools_allowed_call" => 1,
                "fork_tools_step_limit" => 7,
                _ => 0,
            };
            check_calls(name, &calls, count, &fork, &pin)?;
            if name == "fork_tools_allowed_call"
                && (branch.len() != 2
                    || !contains(&branch[1], &result_marker)
                    || reply.finish != ForkFinish::EndTurn)
            {
                return Err(format!(
                    "{name}: tool result did not join only the continued branch"
                ));
            }
            if matches!(
                name,
                "fork_tools_denied_calls" | "fork_tools_routed_call_refused"
            ) {
                let expected_requests = if name == "fork_tools_denied_calls" {
                    3
                } else {
                    2
                };
                if branch.len() != expected_requests
                    || reply.answer != "continued"
                    || reply.finish != ForkFinish::EndTurn
                {
                    return Err(format!(
                        "{name}: denied tools ended the fork instead of continuing"
                    ));
                }
                for bytes in &branch[1..] {
                    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
                    if exact(&value, TOOL_UNAVAILABLE) == 0 {
                        return Err(format!("{name}: missing fixed refusal tool result"));
                    }
                }
            }
            if name == "fork_tools_step_limit" {
                check_cap(name, &branch, &reply)?;
            }
            if name != "fork_tools_allowed_call" {
                let results = case
                    .subject
                    .fork_tools_results(&session.name, &fork)
                    .await
                    .map_err(|e| e.to_string())?;
                let fixed_count = results
                    .iter()
                    .filter(|text| text.as_str() == TOOL_UNAVAILABLE)
                    .count();
                let expected = if matches!(
                    name,
                    "fork_tools_step_limit" | "fork_tools_routed_call_refused"
                ) {
                    1
                } else {
                    2
                };
                if fixed_count != expected {
                    return Err(format!("{name}: missing fixed branch-only tool results"));
                }
            }
        }
        "fork_tools_framing" => {
            let placement = describe
                .fork_tools
                .as_ref()
                .and_then(|map| map.get(&provider))
                .ok_or("fixture provider has no declared framing placement")?;
            check_framing(name, &branch[0], *placement, framing, question)?;
        }
        "fork_tools_identity" => {
            check_identity(name, &session.route, &request, &first).await?;
            let after = requests(case, &session.name)
                .await?
                .into_iter()
                .filter(|r| r.fork_id.as_deref() == Some(&fork))
                .count();
            if after != branch.len() {
                return Err(format!(
                    "{name}: replay or identity refusal reached the model"
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(Ending::Passed)
}

async fn check_identity<R: RunnerRoute>(
    name: &str,
    route: &R,
    request: &Value,
    first: &Value,
) -> Check {
    if expect_ok(
        name,
        call(route, ops::SESSION_FORK_ASK, request.clone()).await?,
    )? != *first
    {
        return Err(format!("{name}: identical retry changed stored answer"));
    }
    for member in ["allowed_tools", "framing"] {
        let mut changed = request.clone();
        changed.as_object_mut().unwrap().remove(member);
        let error = expect_error(name, call(route, ops::SESSION_FORK_ASK, changed).await?)?;
        expect_code(name, &error, errors::ASK_ID_REUSE)?;
        let mut changed = request.clone();
        changed[member] = if member == "allowed_tools" {
            json!([])
        } else {
            json!("changed frame")
        };
        let error = expect_error(name, call(route, ops::SESSION_FORK_ASK, changed).await?)?;
        expect_code(name, &error, errors::ASK_ID_REUSE)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::{Reply, RouteFailure, SubscribeOutcome};
    use async_trait::async_trait;
    use cortexkit_role_llm_runner::usage::RunUsage;
    use subc_protocol::ErrorBody;

    /// A deliberately small runner with independent defects in the contracts
    /// observed by these checks, not an oracle built from the wire helpers.
    struct Runner {
        defect: &'static str,
        identity: Value,
        answer: Value,
    }
    impl Runner {
        fn call(&self) -> ForkToolCallObservation {
            let mut call = ForkToolCallObservation::new(
                json!({"name":"read","arguments":{},"fork_id":"fork:1","call_key":"unique:1","schema_pin":"frozen"}),
                true,
                false,
            );
            match self.defect {
                "marker" => {
                    call.body.as_object_mut().unwrap().remove("fork_id");
                }
                "route" => call.head_session_route = false,
                "opt-ins" => call.held_or_yield_opt_ins = true,
                "pin" => call.body["schema_pin"] = json!("changed"),
                _ => {}
            }
            call
        }
        fn requests(&self) -> Vec<Vec<u8>> {
            let count = if self.defect == "ninth" { 9 } else { 8 };
            (0..count).map(|i| {
                let content = if i == 7 && self.defect != "no-instruction" { ANSWER_NOW } else { "tool result" };
                let tools = if self.defect == "hide-tools" && i == 7 { json!([]) } else { json!([{"name":"read"}]) };
                serde_json::to_vec(&json!({"tools":tools,"messages":[{"role":"user","content":"head"},{"role":"user","content":content}]})).unwrap()
            }).collect()
        }
    }
    #[async_trait]
    impl RunnerRoute for Runner {
        async fn request(&self, _method: &str, params: Value) -> Result<Reply, RouteFailure> {
            if matches!(self.defect, "undeclared" | "invalid") {
                let field = if params.get("allowed_tools").is_some() {
                    "allowed_tools"
                } else {
                    "framing"
                };
                return Ok(Reply::Error(
                    ErrorBody::new("invalid_params", "unknown member")
                        .with_detail(json!({"field":field})),
                ));
            }
            if params == self.identity || self.defect == "ignore-identity" {
                Ok(Reply::Response(self.answer.clone()))
            } else {
                Ok(Reply::Error(ErrorBody::new(
                    "ask_id_reuse",
                    "changed identity",
                )))
            }
        }
        async fn subscribe(&self, _params: Value) -> Result<SubscribeOutcome, RouteFailure> {
            Ok(SubscribeOutcome::Events(vec![]))
        }
    }
    fn runner(defect: &'static str) -> Runner {
        Runner {
            defect,
            identity: json!({"fork_id":"fork:1","ask_id":"ask","question":"q","allowed_tools":["read"],"framing":"frame"}),
            answer: json!({"answer":"stored","finish":"end_turn","usage":{"steps":1,"unreported_steps":0,"by_model":[]}}),
        }
    }

    pub(crate) async fn case_result(name: &str, faulty: bool) -> Check {
        let result = match name {
            "fork_tools_allowed_call" => check_calls(
                name,
                &[runner(if faulty { "marker" } else { "" }).call()],
                1,
                "fork:1",
                "frozen",
            ),
            "fork_tools_denied_calls" | "fork_tools_routed_call_refused" => check_calls(
                name,
                &if faulty {
                    vec![runner("").call()]
                } else {
                    vec![]
                },
                0,
                "fork:1",
                "frozen",
            ),
            "fork_tools_step_limit" => {
                let mut usage = RunUsage::new();
                usage.steps = 8;
                usage.input_tokens = Some(136);
                usage.output_tokens = Some(88);
                let reply = ForkAskReply::new("cap-answer", ForkFinish::StepLimit, usage);
                check_cap(
                    name,
                    &runner(if faulty { "ninth" } else { "" }).requests(),
                    &reply,
                )
            }
            "fork_tools_framing" => {
                let bytes = serde_json::to_vec(&json!({"messages":[{"role":"system","content":"frame"},{"role":"user","content":"q"}]})).unwrap();
                check_framing(
                    name,
                    &bytes,
                    if faulty {
                        FramingPlacement::Prefix
                    } else {
                        FramingPlacement::System
                    },
                    "frame",
                    "q",
                )
            }
            "fork_tools_identity" => {
                let runner = runner(if faulty { "ignore-identity" } else { "" });
                check_identity(name, &runner, &runner.identity, &runner.answer).await
            }
            "fork_tools_undeclared" => {
                check_undeclared(
                    name,
                    &runner(if faulty {
                        "ignore-identity"
                    } else {
                        "undeclared"
                    }),
                    "fork:1",
                )
                .await
            }
            "fork_tools_invalid" => {
                check_invalid(
                    name,
                    &runner(if faulty { "ignore-identity" } else { "invalid" }),
                    "fork:1",
                )
                .await
            }
            _ => Err(format!("{name}: no boundary fixture")),
        };
        result.map_err(|error| format!("{name}: {error}"))
    }

    #[test]
    fn fork_tools_bad_call_implementations_fail_named_checks() {
        let name = "fork_tools_allowed_call";
        check_calls(name, &[runner("").call()], 1, "fork:1", "frozen").unwrap();
        for defect in ["marker", "route", "opt-ins", "pin"] {
            assert!(
                check_calls(name, &[runner(defect).call()], 1, "fork:1", "frozen")
                    .unwrap_err()
                    .starts_with(name)
            );
        }
        let name = "fork_tools_denied_calls";
        check_calls(name, &[], 0, "fork:1", "frozen").unwrap();
        assert!(
            check_calls(name, &[runner("").call()], 0, "fork:1", "frozen")
                .unwrap_err()
                .starts_with(name)
        );
    }

    #[test]
    fn fork_tools_bad_step_caps_and_cache_changes_fail_by_name() {
        let name = "fork_tools_step_limit";
        let mut usage = RunUsage::new();
        usage.steps = 8;
        usage.input_tokens = Some(136);
        usage.output_tokens = Some(88);
        let reply = ForkAskReply::new("cap-answer", ForkFinish::StepLimit, usage);
        check_cap(name, &runner("").requests(), &reply).unwrap();
        for defect in ["ninth", "no-instruction"] {
            assert!(check_cap(name, &runner(defect).requests(), &reply)
                .unwrap_err()
                .starts_with(name));
        }
        let base = serde_json::to_vec(
            &json!({"tools":[{"name":"read"}],"messages":[{"role":"user","content":"head"}]}),
        )
        .unwrap();
        for bytes in runner("").requests() {
            check_base(name, &base, &bytes).unwrap();
        }
        let failed = runner("hide-tools")
            .requests()
            .into_iter()
            .filter_map(|bytes| check_base(name, &base, &bytes).err())
            .any(|error| error.starts_with(name));
        assert!(
            failed,
            "{name}: cache-changing implementation escaped the prefix check"
        );
    }

    #[test]
    fn fork_tools_both_placements_reject_bad_implementations_by_name() {
        let name = "fork_tools_framing";
        let system = serde_json::to_vec(&json!({"messages":[{"role":"system","content":"frame"},{"role":"user","content":"q"}]})).unwrap();
        let prefix =
            serde_json::to_vec(&json!({"messages":[{"role":"user","content":"frame\n\nq"}]}))
                .unwrap();
        check_framing(name, &system, FramingPlacement::System, "frame", "q").unwrap();
        check_framing(name, &prefix, FramingPlacement::Prefix, "frame", "q").unwrap();
        for (bytes, placement) in [
            (&system, FramingPlacement::Prefix),
            (&prefix, FramingPlacement::System),
        ] {
            assert!(check_framing(name, bytes, placement, "frame", "q")
                .unwrap_err()
                .starts_with(name));
        }
    }

    #[tokio::test]
    async fn fork_tools_identity_rejects_runner_ignoring_new_identity_members_by_name() {
        let name = "fork_tools_identity";
        let good = runner("");
        check_identity(name, &good, &good.identity, &good.answer)
            .await
            .unwrap();
        let bad = runner("ignore-identity");
        assert!(check_identity(name, &bad, &bad.identity, &bad.answer)
            .await
            .unwrap_err()
            .contains(name));
    }

    #[tokio::test]
    async fn fork_tools_undeclared_rejects_runner_accepting_unknown_members_by_name() {
        let name = "fork_tools_undeclared";
        check_undeclared(name, &runner("undeclared"), "fork:1")
            .await
            .unwrap();
        assert!(check_undeclared(name, &runner("ignore-identity"), "fork:1")
            .await
            .unwrap_err()
            .contains(name));
    }

    #[tokio::test]
    async fn fork_tools_invalid_rejects_runner_accepting_bad_shapes_by_name() {
        let name = "fork_tools_invalid";
        check_invalid(name, &runner("invalid"), "fork:1")
            .await
            .unwrap();
        assert!(check_invalid(name, &runner("ignore-identity"), "fork:1")
            .await
            .unwrap_err()
            .contains(name));
    }
}
