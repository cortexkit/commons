//! `delivery_order` cases, run only when the subject declares
//! `delivery_order`: ordered delivery modes on `session.send`, plan-named
//! senders and plan templates.
//!
//! Every case reads what was delivered from `session.read` entries
//! (`delivered_as`) and from the markers a send's text carries, never from a
//! message body's schema.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{
        modes, template_digest, DeliveryOrder, Mode, PlanSender, Slot, Template, TemplateUse,
        FIELD, TEMPLATE_FIELD,
    },
    errors, ops,
    read::{HeadMeta, ReadMessage},
    retract::{RetractReply, RetractRequest},
    send::{states as send_states, DeliveredAs, SendReply},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all,
        wait_run_end,
    },
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, PlanSenderIdentity, Script, ScriptedPart, ScriptedTurn},
    tags::{params, require_declared, send_and_wait},
};

type Check = Result<(), String>;

/// How many polls a case waits before it decides a queued send did not start
/// a run.
const QUIET_POLLS: usize = 5;

const ALL_MODES: &[Mode] = &[
    Mode::ToolResult,
    Mode::TurnSystem,
    Mode::Steer,
    Mode::IdleStart,
];

/// A scenario requiring a delivery mode is inapplicable when the runner does not declare it.
pub(crate) async fn inapplicable<S>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Option<String>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let required: &[Mode] = match name {
        "delivery_order_ungranted_interrupt_refused"
        | "delivery_order_granted_interrupt_interrupts" => {
            let describe = case.checked_describe().await?;
            return Ok((!describe.declares(groups::INTERRUPT))
                .then(|| "role.describe does not declare interrupt".to_owned()));
        }
        "display_row_names_the_sender_never_in_read" => &[Mode::IdleStart],
        "delivery_order_declared_modes_respected" => &[],
        "delivery_order_first_possible_mode" => &[Mode::ToolResult, Mode::IdleStart],
        "delivery_order_retract_pending_tool_result" => &[Mode::ToolResult],
        "delivery_order_undeclared_mode_refused" => &[Mode::IdleStart],
        "delivery_order_never_starts_a_run"
        | "delivery_order_dedupe_supersedes_queued"
        | "delivery_order_expired_never_delivered"
        | "delivery_order_unnamed_sender_refused"
        | "delivery_order_steer_receipt_durable_across_restart"
        | "send_status_answers_as_a_resend_after_restart" => &[Mode::Steer],
        "delivery_order_template_renders_as_system" => {
            if case.subject.turn_system_supported() {
                &[Mode::TurnSystem]
            } else {
                &[Mode::Steer]
            }
        }
        _ => return Ok(None),
    };
    let describe = case.checked_describe().await?;
    if name == "delivery_order_declared_modes_respected" && describe.delivery_order_modes.is_none()
    {
        return Ok(Some(
            "role.describe does not advertise delivery_order_modes".into(),
        ));
    }
    Ok(required.iter().find_map(|mode| {
        (!mode.is_declared_by(&describe)).then(|| {
            format!(
                "role.describe does not declare the {} list mode",
                mode.as_str()
            )
        })
    }))
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let passed = |check: Check| check.map(|()| Ending::Passed);
    match name {
        "delivery_order_declared_modes_respected" => passed(declared_modes_respected(case).await),
        "delivery_order_never_starts_a_run" => passed(never_starts(case).await),
        "delivery_order_first_possible_mode" => passed(first_possible(case).await),
        "delivery_order_undeclared_mode_refused" => passed(undeclared_mode_refused(case).await),
        "delivery_order_retract_pending_tool_result" => {
            passed(retract_pending_tool_result(case).await)
        }
        "delivery_order_dedupe_supersedes_queued" => passed(dedupe(case).await),
        "delivery_order_expired_never_delivered" => passed(expiry(case).await),
        "delivery_order_members_without_list_refused" => passed(members_without_list(case).await),
        "delivery_order_list_refused_without_group" => passed(list_without_group(case).await),
        "delivery_order_unnamed_sender_refused" => passed(unnamed_sender(case).await),
        "delivery_order_template_renders_as_system" => passed(template_renders(case).await),
        "delivery_order_bad_slot_refused" => passed(bad_slot(case).await),
        "delivery_order_template_part_gone_refused" => passed(part_gone(case).await),
        "delivery_order_ungranted_interrupt_refused" => passed(ungranted_interrupt(case).await),
        "delivery_order_granted_interrupt_interrupts" => granted_interrupt(case).await,
        "delivery_order_named_sender_queue_string_admitted" => {
            passed(named_queue_string(case).await)
        }
        other => Err(format!("unknown delivery_order case {other}")),
    }
}

/// All non-empty ordered subsets: delivery lists have no duplicate modes.
fn mode_lists(remaining: &[Mode]) -> Vec<Vec<Mode>> {
    let mut lists = Vec::new();
    for (index, mode) in remaining.iter().enumerate() {
        lists.push(vec![*mode]);
        let mut rest = remaining.to_vec();
        rest.remove(index);
        for tail in mode_lists(&rest) {
            let mut list = vec![*mode];
            list.extend(tail);
            lists.push(list);
        }
    }
    lists
}

async fn declared_modes_respected<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let lists = mode_lists(ALL_MODES);
    let turns = lists
        .iter()
        .map(|_| ScriptedTurn::new(vec![case.mint.text("listed-mode-answer").0]))
        .collect();
    let (session, route) = ordered_session(case, "declared-modes", turns, true).await?;
    // The plan sender has the idle_start grant and a template, permitting every list shape.
    for list in lists
        .iter()
        .filter(|list| !list.iter().any(|mode| mode.is_declared_by(&describe)))
    {
        let before = snapshot(&session.route).await?;
        let order = DeliveryOrder::new(list.clone()).with_template(template_use(json!(50)));
        let error = expect_error(
            &format!("the all-undeclared list {list:?}"),
            ordered(
                case,
                &session,
                &route,
                "",
                &case.mint.next("refused"),
                &order,
            )
            .await?,
        )
        .map_err(|error| format!("expected delivery_unsupported for {list:?}: {error}"))?;
        expect_code(
            "an all-undeclared list",
            &error,
            errors::DELIVERY_UNSUPPORTED,
        )?;
        if error.detail.as_ref().and_then(|detail| detail.get(FIELD)) != Some(&json!(list)) {
            return Err(format!(
                "delivery_unsupported did not echo {list:?} in detail.delivery"
            ));
        }
        if !same(&before, &snapshot(&session.route).await?) {
            return Err(format!("refusing {list:?} wrote to the transcript"));
        }
    }
    for list in lists
        .into_iter()
        .filter(|list| list.iter().any(|mode| mode.is_declared_by(&describe)))
    {
        let order = DeliveryOrder::new(list.clone()).with_template(template_use(json!(50)));
        expect_ok(
            &format!("a list with a listed mode {list:?}"),
            ordered(
                case,
                &session,
                &route,
                "",
                &case.mint.next("accepted"),
                &order,
            )
            .await?,
        )?;
    }
    Ok(())
}

/// The template the template cases declare in the plan, in the plan
/// sender's entry of `senders`.
fn template() -> Template {
    let mut slots = std::collections::BTreeMap::new();
    slots.insert("pct".to_owned(), Slot::Percentage);
    Template::new("ceiling_note", "Context is {pct}% full.", slots)
}

fn template_use(pct: Value) -> TemplateUse {
    let mut slots = Map::new();
    slots.insert("pct".into(), pct);
    TemplateUse::new("ceiling_note", template_digest(&template()), slots)
}

fn sender<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<PlanSenderIdentity, String> {
    case.subject
        .plan_sender()
        .ok_or_else(|| "the subject names no plan sender, which delivery_order requires".into())
}

/// A session whose first send's plan names the subject's plan sender (with
/// the case's template, and the `idle_start` grant when `grant`), after one
/// finished run. Each turn of `turns` answers one later run.
async fn ordered_session<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: Vec<ScriptedTurn>,
    grant: bool,
) -> Result<(Session<S::Route>, S::Route), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    sender_session(case, label, turns, grant, false).await
}

/// [`ordered_session`], whose plan sender also holds the `interrupt` grant
/// when `interrupt`.
pub(crate) async fn sender_session<S>(
    case: &Case<'_, S>,
    label: &str,
    turns: Vec<ScriptedTurn>,
    grant: bool,
    interrupt: bool,
) -> Result<(Session<S::Route>, S::Route), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut all = vec![ScriptedTurn::new(vec![case.mint.text("opening-answer").0])];
    all.extend(turns);
    let mut session = case.open(label, Script { turns: all }).await?;
    require_declared(&session.route, groups::DELIVERY_ORDER, "delivery_order").await?;
    let identity = sender(case)?;
    let mut entry = PlanSender::new(&identity.module).with_templates(vec![template()]);
    if grant {
        entry = entry.with_idle_start();
    }
    if interrupt {
        entry = entry.with_interrupt();
    }
    let mut members = Map::new();
    members.insert("senders".into(), json!([entry]));
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or("the subject builds no plan carrying senders, which delivery_order requires")?;
    let opening = case.mint.next("opening");
    send_and_wait(case, &mut session, Some(&fields), &opening, &Map::new()).await?;
    let route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &identity.stamp)
        .await
        .map_err(|e| format!("opening {} as the plan sender: {e}", session.name))?;
    Ok((session, route))
}

/// A `delivery_order` send of `prompt` on `route`, with `order`'s members.
/// A template send carries no `prompt` member, so `prompt` is dropped when
/// `order` names a template.
async fn ordered<S, R>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    route: &R,
    prompt: &str,
    send_id: &str,
    order: &DeliveryOrder,
) -> Result<Reply, String>
where
    S: LlmRunnerSubject,
    R: RunnerRoute,
{
    let mut params = params(
        case,
        session,
        None,
        prompt,
        send_id,
        &order.to_send_members(),
    );
    if order.template.is_some() {
        params
            .as_object_mut()
            .expect("params are an object")
            .remove("prompt");
    }
    call(route, ops::SESSION_SEND, params).await
}

/// What the transcript holds: the head's position and every message.
async fn snapshot<R: RunnerRoute>(route: &R) -> Result<(HeadMeta, Vec<ReadMessage>), String> {
    Ok((head(route).await?, read_all(route).await?.1))
}

fn same(before: &(HeadMeta, Vec<ReadMessage>), after: &(HeadMeta, Vec<ReadMessage>)) -> bool {
    (before.0.last_ordinal, &before.0.last_run_state, &before.1)
        == (after.0.last_ordinal, &after.0.last_run_state, &after.1)
}

/// The entries holding `marker`.
fn holding<'m>(messages: &'m [ReadMessage], marker: &str) -> Vec<&'m ReadMessage> {
    messages
        .iter()
        .filter(|m| m.message.to_string().contains(marker))
        .collect()
}

/// `marker` was delivered exactly once, in `mode`.
pub(crate) fn delivered_once(
    messages: &[ReadMessage],
    marker: &str,
    mode: &str,
) -> Result<(), String> {
    match holding(messages, marker).as_slice() {
        [entry] => match &entry.delivered_as {
            Some(delivered) if delivered.mode == mode => Ok(()),
            other => Err(format!(
                "the entry holding {marker} records delivery {other:?}, not {mode}"
            )),
        },
        other => Err(format!(
            "{} entries hold {marker}, expected it delivered exactly once",
            other.len()
        )),
    }
}

fn never_delivered(messages: &[ReadMessage], marker: &str, what: &str) -> Check {
    match holding(messages, marker).len() {
        0 => Ok(()),
        n => Err(format!("{what} was delivered: {n} entries hold {marker}")),
    }
}

/// Re-send `send_id` with the same request and expect the terminal `state`.
async fn resend_state<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    route: &S::Route,
    prompt: &str,
    send_id: &str,
    order: &DeliveryOrder,
    state: &str,
) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let reply: SendReply = decode(
        "a re-send",
        expect_ok(
            "a re-send of a queued send",
            ordered(case, session, route, prompt, send_id, order).await?,
        )?,
    )?;
    if reply.state == state {
        Ok(())
    } else {
        Err(format!(
            "a re-send of {send_id} answered state {}, not {state}",
            reply.state
        ))
    }
}

async fn never_starts<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, _) = ordered_session(
        case,
        "order-never-starts",
        vec![ScriptedTurn::new(vec![case.mint.text("later-answer").0])],
        false,
    )
    .await?;
    let before = snapshot(&session.route).await?;
    let marker = case.mint.next("queued-steer");
    let order = DeliveryOrder::new(vec![Mode::ToolResult, Mode::TurnSystem, Mode::Steer]);
    let send_id = case.mint.next("send");
    let reply: SendReply = decode(
        "a list without idle_start",
        expect_ok(
            "a list without idle_start on an idle session",
            ordered(case, &session, &session.route, &marker, &send_id, &order).await?,
        )?,
    )?;
    if reply.state != send_states::PENDING {
        return Err(format!(
            "a list without idle_start on an idle session answered {}, not pending: it must \
             wait, queued, and never start a run",
            reply.state
        ));
    }
    for _ in 0..QUIET_POLLS {
        case.subject.pause().await;
        if !same(&before, &snapshot(&session.route).await?) {
            return Err(
                "a list without idle_start changed an idle session: it must wait, queued, and \
                 never start a run"
                    .into(),
            );
        }
    }
    // The owner's next turn is a request the session makes anyway: the
    // queued send rides it as a steer.
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    delivered_once(&messages, &marker, modes::STEER)
}

async fn undeclared_mode_refused<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("undeclared-steer-held-call");
    held.hold = true;
    let (session, _) = ordered_session(
        case,
        "order-undeclared-steer",
        vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-undeclared-steer").0]),
            ScriptedTurn::new(vec![case.mint.text("after-idle-start").0]),
        ],
        false,
    )
    .await?;
    let run_prompt = case.mint.next("undeclared-steer-held-run");
    expect_ok(
        "the owner's held turn",
        call(
            &session.route,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                &run_prompt,
                &case.mint.next("send"),
                &Map::new(),
            ),
        )
        .await?,
    )?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let held_state = async {
        let running = head(&session.route)
            .await?
            .last_run_state
            .ok_or("the held turn has no run state")?;
        if running.state != cortexkit_role_llm_runner::run::states::ACTIVE {
            return Err(format!("the held turn is {}, not active", running.state));
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(error) = held_state {
        let _ = case.subject.release_tool_call(&held.arguments).await;
        return Err(error);
    }

    let refusal = async {
        let before = snapshot(&session.route).await?;
        let invalid_prompt = case.mint.next("undeclared-steer-only");
        let shared_send_id = case.mint.next("send");
        let invalid_order = DeliveryOrder::new(vec![Mode::Steer]);
        let error = expect_error(
            "a list containing only undeclared steer",
            ordered(
                case,
                &session,
                &session.route,
                &invalid_prompt,
                &shared_send_id,
                &invalid_order,
            )
            .await?,
        )
        .map_err(|error| format!("expected delivery_unsupported: {error}"))?;
        expect_code(
            "a list containing only undeclared steer",
            &error,
            errors::DELIVERY_UNSUPPORTED,
        )?;
        let expected = json!([modes::STEER]);
        if error.detail.as_ref().and_then(|detail| detail.get(FIELD)) != Some(&expected) {
            return Err(format!(
                "delivery_unsupported detail.delivery was {:?}, not the sent list {expected}",
                error.detail
            ));
        }
        if !same(&before, &snapshot(&session.route).await?) {
            return Err("the undeclared-mode list was refused but wrote nothing".into());
        }

        let marker = case.mint.next("declared-idle-start");
        let valid_id = shared_send_id;
        let valid_order = DeliveryOrder::new(vec![Mode::Steer, Mode::IdleStart]);
        let reply: SendReply = decode(
            "a list with declared idle_start",
            expect_ok(
                "a list with declared idle_start",
                ordered(
                    case,
                    &session,
                    &session.route,
                    &marker,
                    &valid_id,
                    &valid_order,
                )
                .await?,
            )?,
        )?;
        if reply.state != send_states::PENDING {
            return Err(format!(
                "the list with declared idle_start answered state {:?}, not pending while held",
                reply.state
            ));
        }
        let (_, messages) = read_all(&session.route).await?;
        never_delivered(&messages, &marker, "the queued mixed-mode send")?;
        Ok::<_, String>((marker, valid_id, valid_order))
    }
    .await;
    let released = case
        .subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"));
    let (marker, valid_id, valid_order) = refusal?;
    released?;
    let mut final_reply = None;
    for _ in 0..crate::drive::MAX_POLLS {
        let reply: SendReply = decode(
            "re-sending the mixed-mode list",
            expect_ok(
                "re-sending the mixed-mode list",
                ordered(
                    case,
                    &session,
                    &session.route,
                    &marker,
                    &valid_id,
                    &valid_order,
                )
                .await?,
            )?,
        )?;
        if reply.state != send_states::PENDING {
            final_reply = Some(reply);
            break;
        }
        case.subject.pause().await;
    }
    let final_reply =
        final_reply.ok_or("the mixed-mode list stayed pending after the run ended")?;
    let receipt = final_reply
        .delivered
        .as_ref()
        .ok_or("the mixed-mode list was accepted but has no delivered receipt")?;
    if receipt.r#as() != DeliveredAs::Turn {
        return Err(format!(
            "the mixed-mode list delivered as {:?}, not idle_start's turn",
            receipt.r#as
        ));
    }
    let (_, messages) = read_all(&session.route).await?;
    delivered_once(&messages, &marker, modes::IDLE_START)
}

async fn first_possible<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("order-held");
    held.hold = true;
    let (session, _) = ordered_session(
        case,
        "order-first-possible",
        vec![
            ScriptedTurn::new(vec![case.mint.text("idle-start-answer").0]),
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-held").0]),
        ],
        false,
    )
    .await?;
    // On an idle session nothing but idle_start is possible.
    let started = case.mint.next("idle-start");
    let order = DeliveryOrder::new(vec![Mode::ToolResult, Mode::Steer, Mode::IdleStart]);
    let before = head(&session.route).await?.last_run_state.map(|l| l.run_id);
    let reply: SendReply = decode(
        "an idle_start send",
        expect_ok(
            "a list ending in idle_start on an idle session",
            ordered(
                case,
                &session,
                &session.route,
                &started,
                &case.mint.next("send"),
                &order,
            )
            .await?,
        )?,
    )?;
    crate::drive::wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    delivered_once(&messages, &started, modes::IDLE_START)?;
    // While a tool call is held, a list naming tool_result first goes out
    // with that call's result, not as a steer at the step.
    let prompt = case.mint.next("held-turn");
    expect_ok(
        "the owner's turn with a held call",
        call(
            &session.route,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                &prompt,
                &case.mint.next("send"),
                &Map::new(),
            ),
        )
        .await?,
    )?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let rides = case.mint.next("rides-result");
    // Steer is possible at that boundary too; the list's order picks
    // tool_result.
    let order = DeliveryOrder::new(vec![Mode::ToolResult, Mode::Steer, Mode::IdleStart]);
    expect_ok(
        "a tool_result send during a held call",
        ordered(
            case,
            &session,
            &session.route,
            &rides,
            &case.mint.next("send"),
            &order,
        )
        .await?,
    )?;
    let running = head(&session.route).await?.last_run_state.map(|l| l.run_id);
    case.subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"))?;
    for _ in 0..crate::drive::MAX_POLLS {
        let last = head(&session.route).await?.last_run_state;
        if last.as_ref().is_some_and(|l| {
            Some(&l.run_id) == running.as_ref()
                && cortexkit_role_llm_runner::run::RunState::parse(&l.state).is_terminal()
        }) {
            break;
        }
        case.subject.pause().await;
    }
    let (_, messages) = read_all(&session.route).await?;
    match holding(&messages, &rides).as_slice() {
        [entry] => match &entry.delivered_as {
            Some(d) if d.mode == modes::TOOL_RESULT && d.call_key.is_some() => Ok(()),
            other => Err(format!(
                "the send naming tool_result first records delivery {other:?}: expected \
                 tool_result with the call_key of the result it went out with"
            )),
        },
        other => Err(format!(
            "{} entries hold the tool_result send, expected one",
            other.len()
        )),
    }
}

async fn retract_pending_tool_result<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("retract-held-tool-result");
    held.hold = true;
    let (session, _) = ordered_session(
        case,
        "order-retract-pending-tool-result",
        vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-retract").0]),
        ],
        false,
    )
    .await?;
    require_declared(&session.route, groups::RETRACT, "retract").await?;

    let run_prompt = case.mint.next("retract-held-run");
    let run_reply: SendReply = decode(
        "the owner's held turn",
        expect_ok(
            "the owner's held turn",
            call(
                &session.route,
                ops::SESSION_SEND,
                params(
                    case,
                    &session,
                    None,
                    &run_prompt,
                    &case.mint.next("send"),
                    &Map::new(),
                ),
            )
            .await?,
        )?,
    )?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let held_state = async {
        let running = head(&session.route)
            .await?
            .last_run_state
            .ok_or("the held turn has no run state")?;
        if running.state != cortexkit_role_llm_runner::run::states::ACTIVE {
            return Err(format!("the held turn is {}, not active", running.state));
        }
        Ok::<_, String>(())
    }
    .await;
    if let Err(error) = held_state {
        let _ = case.subject.release_tool_call(&held.arguments).await;
        return Err(error);
    }

    let marker = case.mint.next("retract-before-tool-result");
    let send_id = case.mint.next("send");
    let order = DeliveryOrder::new(vec![Mode::ToolResult]);
    let outcome = async {
        let pending: SendReply = decode(
            "the waiting tool_result send",
            expect_ok(
                "the waiting tool_result send",
                ordered(case, &session, &session.route, &marker, &send_id, &order).await?,
            )?,
        )?;
        if pending.state != send_states::PENDING {
            return Err(format!(
                "the waiting tool_result send answered state {:?}, not pending",
                pending.state
            ));
        }
        let submission_id = pending
            .submission_id
            .filter(|submission_id| !submission_id.is_empty())
            .ok_or("the pending tool_result send omitted a non-empty submission_id")?;
        let retract = || {
            serde_json::to_value(RetractRequest::new(submission_id.clone()))
                .expect("a retract request serializes")
        };
        let first: RetractReply = decode(
            "session.retract",
            expect_ok(
                "session.retract",
                call(&session.route, ops::SESSION_RETRACT, retract()).await?,
            )?,
        )?;
        if first != RetractReply::Retracted {
            return Err(format!("session.retract answered {first:?}, not retracted"));
        }
        let second: RetractReply = decode(
            "session.retract retry",
            expect_ok(
                "session.retract retry",
                call(&session.route, ops::SESSION_RETRACT, retract()).await?,
            )?,
        )?;
        if second != RetractReply::NotPending {
            return Err(format!(
                "the second session.retract answered {second:?}, not not_pending"
            ));
        }
        Ok::<_, String>(())
    }
    .await;
    let released = case
        .subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"));
    outcome?;
    released?;
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        None,
        &run_reply,
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    never_delivered(&messages, &marker, "retracted tool_result send")
}

async fn dedupe<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, _) = ordered_session(
        case,
        "order-dedupe",
        vec![ScriptedTurn::new(vec![case.mint.text("dedupe-answer").0])],
        false,
    )
    .await?;
    let key = case.mint.next("key");
    let order = DeliveryOrder::new(vec![Mode::Steer]).with_dedupe_key(&key);
    let (first, second) = (case.mint.next("replaced"), case.mint.next("replacing"));
    let first_id = case.mint.next("send");
    let route = &session.route;
    expect_ok(
        "the first keyed send",
        ordered(case, &session, route, &first, &first_id, &order).await?,
    )?;
    expect_ok(
        "the second keyed send",
        ordered(
            case,
            &session,
            route,
            &second,
            &case.mint.next("send"),
            &order,
        )
        .await?,
    )?;
    resend_state(
        case,
        &session,
        route,
        &first,
        &first_id,
        &order,
        send_states::SUPERSEDED,
    )
    .await?;
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    never_delivered(&messages, &first, "a superseded send")?;
    delivered_once(&messages, &second, modes::STEER)
}

async fn expiry<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, _) = ordered_session(
        case,
        "order-expiry",
        vec![ScriptedTurn::new(vec![case.mint.text("expiry-answer").0])],
        false,
    )
    .await?;
    // A time long past, so the send is dropped at its first boundary.
    let order = DeliveryOrder::new(vec![Mode::Steer]).with_expires_at_ms(1);
    let marker = case.mint.next("expired");
    let send_id = case.mint.next("send");
    expect_ok(
        "an expiring send",
        ordered(case, &session, &session.route, &marker, &send_id, &order).await?,
    )?;
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    never_delivered(&messages, &marker, "a send past its expires_at_ms")?;
    resend_state(
        case,
        &session,
        &session.route,
        &marker,
        &send_id,
        &order,
        send_states::EXPIRED,
    )
    .await
}

/// `members` added to a send of `prompt` into `session`, beside the
/// subject's fields and the role's. An empty `prompt` is left out, as a
/// template send leaves it out.
async fn send_with<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    prompt: &str,
    members: &Map<String, Value>,
) -> Result<Reply, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut params = params(
        case,
        session,
        None,
        prompt,
        &case.mint.next("send"),
        members,
    );
    if prompt.is_empty() {
        params
            .as_object_mut()
            .expect("params are an object")
            .remove("prompt");
    }
    call(&session.route, ops::SESSION_SEND, params).await
}

async fn members_without_list<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, _) = ordered_session(case, "order-members", Vec::new(), false).await?;
    let before = snapshot(&session.route).await?;
    let template = DeliveryOrder::new(vec![Mode::TurnSystem, Mode::Steer])
        .with_template(template_use(json!(82)))
        .to_members();
    let keyed = DeliveryOrder::new(vec![Mode::Steer])
        .with_dedupe_key(case.mint.next("key"))
        .to_members();
    let expiring = DeliveryOrder::new(vec![Mode::Steer])
        .with_expires_at_ms(1)
        .to_members();
    let mut steer = Map::new();
    steer.insert("delivery".into(), json!("steer"));
    let with = |mut members: Map<String, Value>, extra: &Map<String, Value>| {
        members.extend(extra.clone());
        members
    };
    // Without a list an older runner would ignore each of these members, so
    // a runner of the group refuses them by name.
    for (what, prompt, members, field) in [
        (
            "a template send with a string delivery",
            String::new(),
            with(template, &steer),
            TEMPLATE_FIELD,
        ),
        (
            "a dedupe_key with no delivery",
            case.mint.next("keyed"),
            keyed,
            cortexkit_role_llm_runner::delivery_order::DEDUPE_KEY_FIELD,
        ),
        (
            "expires_at_ms with a string delivery",
            case.mint.next("expiring"),
            with(expiring, &steer),
            cortexkit_role_llm_runner::delivery_order::EXPIRES_AT_FIELD,
        ),
    ] {
        let error = expect_error(what, send_with(case, &session, &prompt, &members).await?)?;
        expect_field(what, &error, errors::INVALID_PARAMS, field)?;
        if !same(&before, &snapshot(&session.route).await?) {
            return Err(format!("{what} was refused but changed the transcript"));
        }
    }
    Ok(())
}

async fn list_without_group<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "order-without-group",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("plain-answer").0])],
            },
        )
        .await?;
    let opening = case.mint.next("opening");
    send_and_wait(case, &mut session, None, &opening, &Map::new()).await?;
    let before = snapshot(&session.route).await?;
    let what = "a list in delivery on a runner without delivery_order";
    let members = DeliveryOrder::new(vec![Mode::Steer, Mode::IdleStart]).to_send_members();
    let error = expect_error(
        what,
        send_with(case, &session, &case.mint.next("listed"), &members).await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, FIELD)?;
    for _ in 0..QUIET_POLLS {
        if !same(&before, &snapshot(&session.route).await?) {
            return Err(format!(
                "{what} was refused but changed the transcript: the list must never go out as \
                 a legacy mode"
            ));
        }
        case.subject.pause().await;
    }
    Ok(())
}

async fn unnamed_sender<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, named) = ordered_session(
        case,
        "order-senders",
        vec![ScriptedTurn::new(vec![case.mint.text("senders-answer").0])],
        false,
    )
    .await?;
    let identity = sender(case)?;
    let stranger = case
        .subject
        .session_route(
            case.handle.inner(),
            &session.name,
            &case.subject.stranger_stamp(),
        )
        .await
        .map_err(|e| format!("opening {} as a stranger: {e}", session.name))?;
    let order = DeliveryOrder::new(vec![Mode::Steer]);
    let refused = case.mint.next("unnamed");
    for (what, extra) in [
        ("a send from a module the plan does not name", Map::new()),
        (
            "a send from an unnamed module naming the plan sender in a request field",
            [("sender".to_owned(), json!(identity.module))]
                .into_iter()
                .collect(),
        ),
    ] {
        let mut members = order.to_members();
        members.extend(extra);
        let error = expect_error(
            what,
            call(
                &stranger,
                ops::SESSION_SEND,
                params(
                    case,
                    &session,
                    None,
                    &refused,
                    &case.mint.next("send"),
                    &members,
                ),
            )
            .await?,
        )?;
        expect_code(what, &error, errors::SCOPE_OWNER_MISMATCH)?;
    }
    let what = "a plan sender without the idle_start grant listing idle_start";
    let error = expect_error(
        what,
        ordered(
            case,
            &session,
            &named,
            &refused,
            &case.mint.next("send"),
            &DeliveryOrder::new(vec![Mode::Steer, Mode::IdleStart]),
        )
        .await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, FIELD)?;
    let accepted = case.mint.next("named");
    expect_ok(
        "the plan sender's steer",
        ordered(
            case,
            &session,
            &named,
            &accepted,
            &case.mint.next("send"),
            &order,
        )
        .await?,
    )?;
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    never_delivered(&messages, &refused, "a refused sender's send")?;
    delivered_once(&messages, &accepted, modes::STEER)
}

async fn template_renders<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, named) = ordered_session(
        case,
        "order-template",
        vec![ScriptedTurn::new(vec![case.mint.text("template-answer").0])],
        false,
    )
    .await?;
    let order = DeliveryOrder::new(vec![Mode::TurnSystem, Mode::Steer])
        .with_template(template_use(json!(82)));
    // A template send carries no prompt member: one carrying an empty one
    // has two texts, and is refused before anything is written.
    let before = snapshot(&session.route).await?;
    let what = "a template send carrying an empty prompt";
    let error = expect_error(
        what,
        call(
            &named,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                "",
                &case.mint.next("send"),
                &order.to_send_members(),
            ),
        )
        .await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, TEMPLATE_FIELD)?;
    if !same(&before, &snapshot(&session.route).await?) {
        return Err(format!("{what} was refused but changed the transcript"));
    }
    expect_ok(
        "a template send with valid slots",
        ordered(case, &session, &named, "", &case.mint.next("send"), &order).await?,
    )?;
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    // One text whatever mode carries it: system where the route supports a
    // turn-scoped system message, the same text at user authority otherwise.
    let mode = if case.subject.turn_system_supported() {
        modes::TURN_SYSTEM
    } else {
        modes::STEER
    };
    delivered_once(&messages, "Context is 82% full.", mode)
}

/// On a session whose plan renders tags, a template send whose `part_tag`
/// slot names a part the session does not show is refused
/// `template_part_gone` naming the slot, writing nothing; one naming a part
/// it shows is accepted.
async fn part_gone<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "order-part-gone",
            Script {
                turns: vec![ScriptedTurn::new(vec![
                    case.mint.text("part-gone-answer").0,
                ])],
            },
        )
        .await?;
    require_declared(&session.route, groups::DELIVERY_ORDER, "delivery_order").await?;
    let identity = sender(case)?;
    let mut slots = std::collections::BTreeMap::new();
    slots.insert("part".to_owned(), Slot::PartTag);
    let template = Template::new("drop_hint", "Consider dropping {part}.", slots);
    let entry = PlanSender::new(&identity.module).with_templates(vec![template.clone()]);
    let mut members = Map::new();
    members.insert("senders".into(), json!([entry]));
    members.insert(
        cortexkit_role_llm_runner::tags::PLAN_RENDER_TAGS.into(),
        json!(true),
    );
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or(
            "the subject builds no plan carrying senders and render_tags, which this case requires",
        )?;
    let opening = case.mint.next("part-gone-opening");
    send_and_wait(case, &mut session, Some(&fields), &opening, &Map::new()).await?;
    let named = case
        .subject
        .session_route(case.handle.inner(), &session.name, &identity.stamp)
        .await
        .map_err(|e| format!("opening {} as the plan sender: {e}", session.name))?;
    let (_, messages) = read_all(&session.route).await?;
    let shown = messages
        .iter()
        .find_map(|m| {
            m.tags
                .iter()
                .find(|row| row.tag.is_some())
                .map(|row| json!({ "mid": m.mid, "part": row.part }))
        })
        .ok_or("the session shows no numbered part to name in a part_tag slot")?;
    let template_send = |part: Value| {
        let mut slots = Map::new();
        slots.insert("part".into(), part);
        DeliveryOrder::new(vec![Mode::TurnSystem, Mode::Steer]).with_template(TemplateUse::new(
            "drop_hint",
            template_digest(&template),
            slots,
        ))
    };
    let before = snapshot(&session.route).await?;
    let gone = json!({ "mid": case.mint.next("no-such-message"), "part": "0" });
    let what = "a template send whose part_tag slot names a part the session does not show";
    let error = expect_error(
        what,
        ordered(
            case,
            &session,
            &named,
            "",
            &case.mint.next("send"),
            &template_send(gone),
        )
        .await?,
    )?;
    expect_code(what, &error, errors::TEMPLATE_PART_GONE)?;
    let detail: errors::TemplatePartGoneDetail =
        serde_json::from_value(error.detail.clone().unwrap_or(Value::Null))
            .map_err(|e| format!("{what}: the refusal's detail does not name the slot: {e}"))?;
    if detail.slot != "part" {
        return Err(format!(
            "{what}: the refusal names slot {:?}, not part",
            detail.slot
        ));
    }
    if !same(&before, &snapshot(&session.route).await?) {
        return Err(format!("{what} was refused but changed the transcript"));
    }
    let what = "a template send whose part_tag slot names a part the session shows";
    expect_ok(
        what,
        ordered(
            case,
            &session,
            &named,
            "",
            &case.mint.next("send"),
            &template_send(shown),
        )
        .await?,
    )?;
    Ok(())
}

async fn bad_slot<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (mut session, named) = ordered_session(
        case,
        "order-bad-slot",
        vec![ScriptedTurn::new(vec![case.mint.text("bad-slot-answer").0])],
        false,
    )
    .await?;
    let before = snapshot(&session.route).await?;
    let order = DeliveryOrder::new(vec![Mode::TurnSystem, Mode::Steer])
        .with_template(template_use(json!(101)));
    let what = "a template send whose percentage slot is 101";
    let error = expect_error(
        what,
        ordered(case, &session, &named, "", &case.mint.next("send"), &order).await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, TEMPLATE_FIELD)?;
    if !same(&before, &snapshot(&session.route).await?) {
        return Err(format!("{what} was refused but changed the transcript"));
    }
    let prompt = case.mint.next("owner-turn");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let (_, messages) = read_all(&session.route).await?;
    never_delivered(&messages, "Context is 101% full.", what)?;
    if messages.iter().any(|m| {
        m.delivered_as
            .as_ref()
            .is_some_and(|d| d.mode == modes::TURN_SYSTEM)
    }) {
        return Err(format!("after {what}, an entry went out as turn_system"));
    }
    Ok(())
}

/// The `delivery` member of a send carrying the string `mode`.
fn string_delivery(mode: &str) -> Map<String, Value> {
    [(FIELD.to_owned(), json!(mode))].into_iter().collect()
}

/// A plan sender whose entry grants `idle_start` but not `interrupt` sends
/// the `interrupt` string: refused `invalid_params` naming `delivery`,
/// writing nothing.
async fn ungranted_interrupt<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, named) = ordered_session(
        case,
        "order-ungranted-interrupt",
        vec![ScriptedTurn::new(vec![
            case.mint.text("ungranted-answer").0,
        ])],
        true,
    )
    .await?;
    let before = snapshot(&session.route).await?;
    let marker = case.mint.next("ungranted-interrupt");
    let what = "a plan sender without the interrupt grant sending interrupt";
    let error = expect_error(
        what,
        call(
            &named,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                &marker,
                &case.mint.next("send"),
                &string_delivery(groups::INTERRUPT),
            ),
        )
        .await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, FIELD)?;
    if !same(&before, &snapshot(&session.route).await?) {
        return Err(format!("{what} was refused but changed the transcript"));
    }
    Ok(())
}

/// While the owner's run waits on a held model step, a plan sender granted
/// `interrupt` sends the `interrupt` string: it is admitted, the run it
/// interrupts ends `cancelled`, and its message starts a run that ends.
async fn granted_interrupt<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, named) = sender_session(
        case,
        "order-granted-interrupt",
        vec![
            ScriptedTurn::new(vec![case.mint.text("held-answer").0]),
            ScriptedTurn::new(vec![case.mint.text("interrupt-answer").0]),
        ],
        true,
        true,
    )
    .await?;
    if let Err(e) = case.subject.hold_next_model_step(&session.name).await {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot hold a model step, so there is no running turn to interrupt: {e}"
        )));
    }
    expect_ok(
        "the owner's send whose model step is held",
        call(
            &session.route,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                &case.mint.next("held-turn"),
                &case.mint.next("send"),
                &Map::new(),
            ),
        )
        .await?,
    )?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| format!("waiting for the held model step: {e}"))?;
    let held_run = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id)
        .ok_or("the held turn has no run state")?;
    let marker = case.mint.next("granted-interrupt");
    let reply = call(
        &named,
        ops::SESSION_SEND,
        params(
            case,
            &session,
            None,
            &marker,
            &case.mint.next("send"),
            &string_delivery(groups::INTERRUPT),
        ),
    )
    .await;
    // The interrupt aborts the held stream, so releasing it may find
    // nothing left to release; the release only keeps a runner that did
    // not interrupt from waiting forever.
    let _ = case.subject.release_model_step(&session.name).await;
    let what = "a plan sender granted interrupt sending interrupt";
    let reply: SendReply = decode(
        what,
        expect_ok(what, reply?).map_err(|e| {
            format!("{e}: a granted interrupt is admitted and behaves as the owner's")
        })?,
    )?;
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        Some(&held_run),
        &reply,
    )
    .await?;
    let interrupted = crate::drive::run_result(&session.route, &held_run).await?;
    if interrupted.state != cortexkit_role_llm_runner::run::states::CANCELLED {
        return Err(format!(
            "the run {held_run} that {what} interrupted ended {}, not cancelled",
            interrupted.state
        ));
    }
    let (_, messages) = read_all(&session.route).await?;
    match holding(&messages, &marker).as_slice() {
        [entry]
            if entry
                .run
                .as_ref()
                .is_some_and(|run| run.run_id == end.run_id) =>
        {
            Ok(Ending::Passed)
        }
        other => Err(format!(
            "{} entries hold the interrupting message, attributed to {:?}: it is written once, in \
             the run it started ({})",
            other.len(),
            other.iter().map(|m| &m.run).collect::<Vec<_>>(),
            end.run_id
        )),
    }
}

/// A plan sender granted `idle_start` sends the string `queue`, which stands
/// for `[idle_start]`: it is admitted, and its message starts a run.
async fn named_queue_string<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, named) = ordered_session(
        case,
        "order-named-queue",
        vec![ScriptedTurn::new(vec![
            case.mint.text("named-queue-answer").0,
        ])],
        true,
    )
    .await?;
    let before = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id);
    let marker = case.mint.next("named-queue");
    let what = "a plan sender granted idle_start sending the string queue";
    let reply: SendReply = decode(
        what,
        expect_ok(
            what,
            call(
                &named,
                ops::SESSION_SEND,
                params(
                    case,
                    &session,
                    None,
                    &marker,
                    &case.mint.next("send"),
                    &string_delivery(groups::QUEUE),
                ),
            )
            .await?,
        )
        .map_err(|e| {
            format!("{e}: a named sender's string queue stands for [idle_start] and is admitted on its idle_start grant")
        })?,
    )?;
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    match holding(&messages, &marker).len() {
        1 => Ok(()),
        n => Err(format!(
            "{n} entries hold the message of {what}: it is written exactly once"
        )),
    }
}
