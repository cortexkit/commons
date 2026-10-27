//! The `rich_ui_stamp` cases, `cortexkit-role-llm-runner` CONTRACT §9.17: a
//! run whose frozen plan's fetched system-text answers declare a rich UI
//! stamp serves that stamp, verbatim and fixed per run, on every assistant
//! display row it writes, on its `run_started`, as `session.head`'s
//! `last_run_rich_ui` while it is the last run, and on its assistant
//! `session.read` entries when the read opts in with `include_rich_ui`.
//!
//! The cases run only on a runner whose `role.describe` declares the group;
//! on any other runner they are not applicable, never skipped. They build
//! each plan through [`LlmRunnerSubject::rich_ui_plan_fields`], so the stamp
//! the runner must serve is one the suite chose and the runner fetched, and
//! compare every surface with it. An adapter without that hook makes the
//! cases not applicable, by name.

use std::collections::BTreeSet;

use cortexkit_role_llm_runner::{
    capabilities as groups, errors,
    live::LiveEvent,
    ops,
    read::{ReadMessage, ReadRequest},
    rich_ui::RichUiStamp,
    send::SendReply,
    subscribe::SubscribeFrom,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    display::display_walk,
    drive::{
        call, decode, expect_code, expect_error, expect_ok, head, read, read_all, wait_run_end,
    },
    route::Reply,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// The dialect of the stamp the cases' stamped answers declare.
pub const FIXTURE_DIALECT: &str = "1.0";
/// The catalog digest of the stamp the cases' stamped answers declare.
pub const FIXTURE_CATALOG_DIGEST: &str =
    "2b8cf2a58e69aee8016627e9edd23b5931751dc8aa9a715eb291fa07195f585c";
/// The dialect of the second stamp the conflict case's plan declares.
pub const CONFLICTING_DIALECT: &str = "1.1";
/// The catalog digest of the second stamp the conflict case's plan declares.
pub const CONFLICTING_CATALOG_DIGEST: &str =
    "2f44cf8bda20f171efd332acc40ee15e0ffae13a8caae3b4b11f364582dc1465";

/// The stamp the cases' stamped system-text answers declare.
pub fn fixture_stamp() -> RichUiStamp {
    RichUiStamp::new(FIXTURE_DIALECT, FIXTURE_CATALOG_DIGEST).expect("the fixture stamp is valid")
}

/// The stamp that conflicts with [`fixture_stamp`] in the conflict case.
pub fn conflicting_stamp() -> RichUiStamp {
    RichUiStamp::new(CONFLICTING_DIALECT, CONFLICTING_CATALOG_DIGEST)
        .expect("the conflicting stamp is valid")
}

const NO_GROUP: &str = "runner does not declare rich_ui_stamp";
const NO_PLAN_FIXTURE: &str = "the adapter cannot build a plan whose system-text answers declare \
     a rich UI stamp (rich_ui_plan_fields)";
const NO_REPLAN_FIXTURE: &str =
    "the adapter cannot replan a session's rich UI stamp (replan_rich_ui)";

type Check = Result<(), String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case
        .checked_describe()
        .await?
        .declares(groups::RICH_UI_STAMP)
    {
        return Ok(Ending::Inapplicable(NO_GROUP.into()));
    }
    let missing: Vec<Capability> = [Capability::Queue, Capability::TranscriptReads]
        .into_iter()
        .filter(|c| !case.declared.contains(c))
        .collect();
    if !missing.is_empty() {
        return Ok(Ending::Skipped {
            missing,
            reason: "the rich_ui_stamp cases start a session with a send and read its head and \
                     entries"
                .into(),
        });
    }
    let surfaces = Surfaces::of(case);
    match name {
        "rich_ui_stamp_served_on_stamped_run" => served(case, surfaces).await,
        "rich_ui_stamp_absent_on_unstamped_run" => unstamped(case, surfaces).await,
        "rich_ui_stamp_read_needs_opt_in" => opt_in(case).await,
        "rich_ui_stamp_conflict_refused" => conflict(case).await,
        "rich_ui_stamp_head_follows_last_run" => follows(case, surfaces).await,
        other => Err(format!("unknown rich_ui_stamp case {other}")),
    }
}

/// Which surfaces besides `session.read` and `session.head` the adapter
/// declares, and so which the cases read.
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

/// Open a session on `script` and send its first send with the adapter's
/// rich UI plan for `stamps`. `None` when the adapter has no such plan.
async fn first_send<S>(
    case: &Case<'_, S>,
    label: &str,
    script: Script,
    stamps: &[Option<RichUiStamp>],
) -> Result<Option<(Session<S::Route>, Reply)>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case.open(label, script).await?;
    let fields = case
        .subject
        .rich_ui_plan_fields(&session.name, stamps)
        .await
        .map_err(|e| format!("building the rich UI plan of {}: {e}", session.name))?;
    let Some(mut fields) = fields else {
        return Ok(None);
    };
    let send_id = case.mint.next("rich-ui-send");
    let prompt = case.mint.next("rich-ui-prompt");
    fields.insert("prompt".into(), json!(prompt));
    fields.insert("send_id".into(), json!(send_id));
    let reply = call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?;
    session.first = Some((send_id, prompt));
    Ok(Some((session, reply)))
}

/// A session whose first run ran under the adapter's rich UI plan for
/// `stamps`, with that run's id. `None` when the adapter has no such plan.
async fn first_run<S>(
    case: &Case<'_, S>,
    label: &str,
    script: Script,
    stamps: &[Option<RichUiStamp>],
) -> Result<Option<(Session<S::Route>, String)>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Some((session, reply)) = first_send(case, label, script, stamps).await? else {
        return Ok(None);
    };
    let reply: SendReply = decode("session.send", expect_ok("session.send", reply)?)?;
    let end = wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    Ok(Some((session, end.run_id)))
}

/// A tool call and a closing text: a run that writes two assistant
/// messages, a tool result and the user's prompt.
fn two_step_turns<S: LlmRunnerSubject>(case: &Case<'_, S>, label: &str) -> Vec<ScriptedTurn> {
    vec![
        ScriptedTurn::new(vec![ScriptedPart::ToolCall(
            case.mint.call(&format!("{label}-call")),
        )]),
        ScriptedTurn::new(vec![case.mint.text(&format!("{label}-closing")).0]),
    ]
}

/// One row or entry a surface served: where, for which message, who wrote
/// it, and the stamp it carries.
struct Shown {
    surface: &'static str,
    mid: String,
    role: Option<String>,
    rich_ui: Option<RichUiStamp>,
}

/// Every entry of the current lineage, read forward with `include_rich_ui`.
async fn opt_in_entries<R: RunnerRoute>(route: &R) -> Result<Vec<ReadMessage>, String> {
    let mut entries = Vec::new();
    let mut from = head(route).await?.first_ordinal.unwrap_or(0);
    for _ in 0..crate::drive::MAX_PAGES {
        let page = read(route, &ReadRequest::range(from).with_include_rich_ui(true)).await?;
        entries.extend(page.messages.iter().cloned());
        match page.next_from_ordinal {
            Some(next) if next > from && !page.messages.is_empty() => from = next,
            Some(next) => {
                return Err(format!(
                    "a range read with include_rich_ui from {from} answered next_from_ordinal \
                     {next} without moving forward"
                ))
            }
            None => return Ok(entries),
        }
    }
    Err("the transcript did not end within the page limit".into())
}

/// A read entry's role: its `role`, or the message's own `role` member.
fn entry_role(entry: &ReadMessage) -> Option<String> {
    entry.role.clone().or_else(|| {
        entry
            .message
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// The opt-in read entries and, where declared, the display rows of the
/// session.
async fn shown<R: RunnerRoute>(route: &R, surfaces: Surfaces) -> Result<Vec<Shown>, String> {
    let mut out: Vec<Shown> = opt_in_entries(route)
        .await?
        .into_iter()
        .map(|entry| Shown {
            surface: "the session.read entry read with include_rich_ui",
            role: entry_role(&entry),
            mid: entry.mid,
            rich_ui: entry.rich_ui,
        })
        .collect();
    if surfaces.display {
        out.extend(
            display_walk(route, None)
                .await?
                .into_iter()
                .map(|entry| Shown {
                    surface: "the display history row",
                    mid: entry.mid,
                    role: Some(entry.row.role),
                    rich_ui: entry.row.rich_ui,
                }),
        );
    }
    Ok(out)
}

/// Every row in `all` carries the stamp `want` gives its message when it is
/// an assistant message, and no stamp otherwise. Each surface must show at
/// least one assistant message, so the check is never vacuous.
fn judge(all: &[Shown], want: &dyn Fn(&str) -> Option<RichUiStamp>) -> Check {
    let mut assistants: BTreeSet<&str> = BTreeSet::new();
    for shown in all {
        let Some(role) = shown.role.as_deref() else {
            continue;
        };
        if role != "assistant" {
            if let Some(got) = &shown.rich_ui {
                return Err(format!(
                    "{} of {role} message {} carries rich_ui {got:?}: the stamp is only ever on \
                     an assistant message a stamped run wrote, never on a user, steered, tool, \
                     imported or adopted row",
                    shown.surface, shown.mid
                ));
            }
            continue;
        }
        assistants.insert(shown.surface);
        let expected = want(&shown.mid);
        match (&shown.rich_ui, &expected) {
            (None, None) => {}
            (Some(got), Some(expected)) if got == expected => {}
            (None, Some(expected)) => {
                return Err(format!(
                    "{} of assistant message {} carries no rich_ui, but the run that wrote it \
                     froze a plan carrying {expected:?}: every assistant row of a stamped run \
                     carries the stamp",
                    shown.surface, shown.mid
                ))
            }
            (Some(got), None) => {
                return Err(format!(
                    "{} of assistant message {} carries rich_ui {got:?}, but the run that wrote \
                     it froze a plan carrying no stamp: absent means unstamped, and a runner \
                     never stamps a run its plan does not stamp",
                    shown.surface, shown.mid
                ))
            }
            (Some(got), Some(expected)) => {
                return Err(format!(
                    "{} of assistant message {} carries rich_ui {got:?}, but the run that wrote \
                     it froze a plan carrying {expected:?}: the runner copies the fetched stamp \
                     verbatim",
                    shown.surface, shown.mid
                ))
            }
        }
    }
    for surface in all
        .iter()
        .map(|shown| shown.surface)
        .collect::<BTreeSet<_>>()
    {
        if !assistants.contains(surface) {
            return Err(format!(
                "{surface} shows no assistant message with a known role, so no stamp can be \
                 checked there"
            ));
        }
    }
    Ok(())
}

/// The head names `run_id` as the last run and carries `want` as its stamp.
async fn judge_head<R: RunnerRoute>(route: &R, run_id: &str, want: Option<&RichUiStamp>) -> Check {
    let head = head(route).await?;
    let named = head
        .last_run_state
        .as_ref()
        .map(|last| last.run_id.as_str());
    if named != Some(run_id) {
        return Err(format!(
            "session.head names run {named:?} as the last run, but the run the suite waited for \
             is {run_id}"
        ));
    }
    match (&head.last_run_rich_ui, want) {
        (None, None) => Ok(()),
        (Some(got), Some(want)) if got == want => Ok(()),
        (None, Some(want)) => Err(format!(
            "session.head omits last_run_rich_ui while last_run_state names run {run_id}, whose \
             frozen plan carries {want:?}"
        )),
        (Some(got), None) => Err(format!(
            "session.head carries last_run_rich_ui {got:?} while last_run_state names run \
             {run_id}, whose frozen plan carries no stamp: the member describes the last run \
             and nothing earlier"
        )),
        (Some(got), Some(want)) => Err(format!(
            "session.head carries last_run_rich_ui {got:?} for run {run_id}, whose frozen plan \
             carries {want:?}"
        )),
    }
}

/// The `run_started` of `run_id` carries `want`, where live events are
/// served.
async fn judge_run_started<R: RunnerRoute>(
    route: &R,
    surfaces: Surfaces,
    run_id: &str,
    want: Option<&RichUiStamp>,
) -> Check {
    if !surfaces.live {
        return Ok(());
    }
    let frames = crate::live::stream(route, SubscribeFrom::Start).await?;
    let started = frames
        .into_iter()
        .find_map(|frame| match frame.event {
            Some(LiveEvent::RunStarted(started)) if started.run_id == run_id => Some(started),
            _ => None,
        })
        .ok_or_else(|| format!("the live stream replays no run_started for run {run_id}"))?;
    if started.rich_ui.as_ref() != want {
        return Err(format!(
            "run_started of run {run_id} carries rich_ui {:?}, but the run's frozen plan carries \
             {want:?}",
            started.rich_ui
        ));
    }
    Ok(())
}

/// One stamp, declared by two answers beside an unstamped one: every
/// surface carries it on the run's assistant messages and on nothing else.
async fn served<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let stamp = fixture_stamp();
    let script = Script {
        turns: two_step_turns(case, "rich-ui-served"),
    };
    let plan = [Some(stamp.clone()), None, Some(stamp.clone())];
    let Some((session, run_id)) = first_run(case, "rich-ui-served", script, &plan).await? else {
        return Ok(Ending::Inapplicable(NO_PLAN_FIXTURE.into()));
    };
    judge_head(&session.route, &run_id, Some(&stamp)).await?;
    judge_run_started(&session.route, surfaces, &run_id, Some(&stamp)).await?;
    judge(&shown(&session.route, surfaces).await?, &|_| {
        Some(stamp.clone())
    })?;
    Ok(Ending::Passed)
}

/// A plan whose only system-text answer declares no stamp: nothing anywhere
/// carries a stamp.
async fn unstamped<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let script = Script {
        turns: two_step_turns(case, "rich-ui-unstamped"),
    };
    let Some((session, run_id)) = first_run(case, "rich-ui-unstamped", script, &[None]).await?
    else {
        return Ok(Ending::Inapplicable(NO_PLAN_FIXTURE.into()));
    };
    judge_head(&session.route, &run_id, None).await?;
    judge_run_started(&session.route, surfaces, &run_id, None).await?;
    judge(&shown(&session.route, surfaces).await?, &|_| None)?;
    Ok(Ending::Passed)
}

/// Without `include_rich_ui` no entry carries the stamp, and every entry is
/// the opt-in entry without it.
async fn opt_in<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let stamp = fixture_stamp();
    let script = Script {
        turns: vec![ScriptedTurn::new(vec![case.mint.text("rich-ui-opt-in").0])],
    };
    let Some((session, _)) = first_run(case, "rich-ui-opt-in", script, &[Some(stamp)]).await?
    else {
        return Ok(Ending::Inapplicable(NO_PLAN_FIXTURE.into()));
    };
    let (_, plain) = read_all(&session.route).await?;
    if let Some(entry) = plain.iter().find(|entry| entry.rich_ui.is_some()) {
        return Err(format!(
            "session.read without include_rich_ui answered entry {} with rich_ui {:?}: the stamp \
             rides on read entries only when the read opts in, so a read without it keeps its \
             bytes",
            entry.mid, entry.rich_ui
        ));
    }
    let opted = opt_in_entries(&session.route).await?;
    if opted.len() != plain.len() {
        return Err(format!(
            "session.read answered {} entries with include_rich_ui and {} without it",
            opted.len(),
            plain.len()
        ));
    }
    for (opted, plain) in opted.iter().zip(&plain) {
        let mut without = opted.clone();
        without.rich_ui = None;
        if &without != plain {
            return Err(format!(
                "session.read with include_rich_ui answered entry {} differently from the same \
                 read without it, beyond the rich_ui member: {opted:?} against {plain:?}",
                plain.mid
            ));
        }
    }
    Ok(Ending::Passed)
}

/// Two different stamps in one plan: refused `plan_stale`, naming both,
/// with nothing written.
async fn conflict<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (first, second) = (fixture_stamp(), conflicting_stamp());
    let script = Script {
        turns: vec![ScriptedTurn::new(vec![
            case.mint.text("rich-ui-conflict").0,
        ])],
    };
    let plan = [Some(first.clone()), Some(second.clone())];
    let Some((session, reply)) = first_send(case, "rich-ui-conflict", script, &plan).await? else {
        return Ok(Ending::Inapplicable(NO_PLAN_FIXTURE.into()));
    };
    if let Reply::Response(_) = &reply {
        return Err(
            "a plan whose fetched system-text answers declare two different rich UI stamps was \
             admitted: a run carries one stamp, so the send is refused plan_stale at admission"
                .into(),
        );
    }
    let refusal = expect_error("a send with two different rich UI stamps", reply)?;
    expect_code(
        "a send with two different rich UI stamps",
        &refusal,
        errors::PLAN_STALE,
    )?;
    let differences = refusal
        .detail
        .as_ref()
        .and_then(|detail| detail.get("differences"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for stamp in [&first, &second] {
        let named = differences.iter().any(|difference| {
            difference.get("rich_ui") == Some(&serde_json::to_value(stamp).unwrap())
        });
        if !named {
            return Err(format!(
                "the plan_stale refusal names no difference carrying rich_ui {stamp:?}: \
                 detail.differences lists every stamped answer, got {:?}",
                refusal.detail
            ));
        }
    }
    let head = head(&session.route).await?;
    if head.lineage_id.is_some() || head.last_run_state.is_some() {
        return Err(format!(
            "after refusing the plan the session has lineage {:?} and last run {:?}: a refusal \
             at admission writes nothing",
            head.lineage_id, head.last_run_state
        ));
    }
    Ok(Ending::Passed)
}

/// A stamped run, then a replan to an unstamped plan and a second run: the
/// head describes the second run and carries no stamp, and the first run's
/// assistant messages keep theirs.
async fn follows<S>(case: &Case<'_, S>, surfaces: Surfaces) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let stamp = fixture_stamp();
    let script = Script {
        turns: vec![
            ScriptedTurn::new(vec![case.mint.text("rich-ui-stamped-run").0]),
            ScriptedTurn::new(vec![case.mint.text("rich-ui-unstamped-run").0]),
        ],
    };
    let Some((mut session, first_run_id)) =
        first_run(case, "rich-ui-follows", script, &[Some(stamp.clone())]).await?
    else {
        return Ok(Ending::Inapplicable(NO_PLAN_FIXTURE.into()));
    };
    judge_head(&session.route, &first_run_id, Some(&stamp)).await?;
    let stamped_mids: BTreeSet<String> = opt_in_entries(&session.route)
        .await?
        .into_iter()
        .map(|entry| entry.mid)
        .collect();
    let replanned = case
        .subject
        .replan_rich_ui(&session.name, None)
        .await
        .map_err(|e| format!("replanning {} without a stamp: {e}", session.name))?;
    if !replanned {
        return Ok(Ending::Inapplicable(NO_REPLAN_FIXTURE.into()));
    }
    let (_, end) = case
        .run_prompt(&mut session, &case.mint.next("rich-ui-after-replan"))
        .await?;
    if end.run_id == first_run_id {
        return Err(format!(
            "the send after the replan names run {first_run_id}, the stamped run, instead of a \
             new run"
        ));
    }
    judge_head(&session.route, &end.run_id, None).await?;
    judge_run_started(&session.route, surfaces, &end.run_id, None).await?;
    judge_run_started(&session.route, surfaces, &first_run_id, Some(&stamp)).await?;
    judge(&shown(&session.route, surfaces).await?, &|mid| {
        stamped_mids.contains(mid).then(|| stamp.clone())
    })?;
    Ok(Ending::Passed)
}
