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
    send::{states as send_states, SendReply},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all},
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, PlanSenderIdentity, Script, ScriptedPart, ScriptedTurn},
    tags::{params, require_declared, send_and_wait},
};

type Check = Result<(), String>;

/// How many polls a case waits before it decides a queued send did not start
/// a run.
const QUIET_POLLS: usize = 5;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let passed = |check: Check| check.map(|()| Ending::Passed);
    match name {
        "delivery_order_never_starts_a_run" => passed(never_starts(case).await),
        "delivery_order_first_possible_mode" => passed(first_possible(case).await),
        "delivery_order_dedupe_supersedes_queued" => passed(dedupe(case).await),
        "delivery_order_expired_never_delivered" => passed(expiry(case).await),
        "delivery_order_members_without_list_refused" => passed(members_without_list(case).await),
        "delivery_order_list_refused_without_group" => passed(list_without_group(case).await),
        "delivery_order_unnamed_sender_refused" => passed(unnamed_sender(case).await),
        "delivery_order_template_renders_as_system" => passed(template_renders(case).await),
        "delivery_order_bad_slot_refused" => passed(bad_slot(case).await),
        other => Err(format!("unknown delivery_order case {other}")),
    }
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
    let mut all = vec![ScriptedTurn::new(vec![case.mint.text("opening-answer").0])];
    all.extend(turns);
    let mut session = case.open(label, Script { turns: all }).await?;
    require_declared(&session.route, groups::DELIVERY_ORDER, "delivery_order").await?;
    let identity = sender(case)?;
    let mut entry = PlanSender::new(&identity.module).with_templates(vec![template()]);
    if grant {
        entry = entry.with_idle_start();
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
    call(
        route,
        ops::SESSION_SEND,
        params(
            case,
            session,
            None,
            prompt,
            send_id,
            &order.to_send_members(),
        ),
    )
    .await
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
fn delivered_once(messages: &[ReadMessage], marker: &str, mode: &str) -> Result<(), String> {
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
/// subject's fields and the role's.
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
    call(
        &session.route,
        ops::SESSION_SEND,
        params(
            case,
            session,
            None,
            prompt,
            &case.mint.next("send"),
            members,
        ),
    )
    .await
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
