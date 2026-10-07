//! Retention cases use real elapsed time and real routes. An expired read
//! alone cannot prove deletion: tombstones hide content before it is erased.

use std::{collections::BTreeSet, path::Path, time::Duration};

use cortexkit_role_harness::{CrashDriver, DriverHandle, KillPoint, Trigger};
use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, Retention, RoleDescribe},
    errors::{self, ExpiredDetail},
    ops, points,
    read::{HeadMeta, ReadPage, ReadRequest},
    run::RunState,
    send::SendReply,
};
use futures_util::future::join;
use serde_json::json;

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, holding, read,
        read_all, run_result, send_params, wait_run_end, Mint,
    },
    route::{Reply, RunnerRoute, SubscribeOutcome},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

const MARGIN_MS: u64 = 300;
const STATUS: &str = "run.status";

fn wait_duration(seconds: u64, deletion_ms: u64) -> Result<Duration, String> {
    let ms = seconds
        .checked_mul(1000)
        .and_then(|n| n.checked_add(deletion_ms))
        .and_then(|n| n.checked_add(MARGIN_MS))
        .ok_or("retention wait overflows milliseconds")?;
    Ok(Duration::from_millis(ms))
}

async fn describe<R: RunnerRoute>(route: &R) -> Result<RoleDescribe, String> {
    let raw = expect_ok(
        "role.describe",
        call(route, ops::ROLE_DESCRIBE, json!({})).await?,
    )?;
    check_describe(&raw).map_err(|e| format!("role.describe: {e:?}"))
}

fn limits(describe: &RoleDescribe) -> Result<Retention, String> {
    if !describe.declares(capabilities::RETENTION) {
        return Err("the subject declares retention but role.describe does not".into());
    }
    describe
        .retention
        .ok_or("role.describe has no retention limits".into())
}

fn expiry(last_activity: u64, seconds: u64) -> Result<u64, String> {
    seconds
        .checked_mul(1000)
        .and_then(|n| last_activity.checked_add(n))
        .ok_or("expiry timestamp overflows milliseconds".into())
}

fn check_expired(method: &str, reply: Reply, expected: u64) -> Result<(), String> {
    let error = expect_error(method, reply)?;
    expect_code(method, &error, errors::EXPIRED)?;
    let detail: ExpiredDetail = decode(method, error.detail.ok_or("expired has no detail")?)?;
    if detail.expired_at_ms != expected {
        return Err(format!(
            "{method} expired_at_ms {}, expected {expected} (last durable activity + retention)",
            detail.expired_at_ms
        ));
    }
    Ok(())
}

async fn expired_reads<R: RunnerRoute>(
    route: &R,
    declared: &BTreeSet<Capability>,
    run_id: &str,
    expected: u64,
) -> Result<(), String> {
    let mut requests = vec![
        (ops::SESSION_READ, json!({})),
        (ops::SESSION_READ, json!({"from_ordinal":0})),
        (ops::SESSION_HEAD, json!({})),
        (ops::RUN_RESULT, json!({"run_id":run_id})),
    ];
    if declared.contains(&Capability::ModelView) {
        requests.push((ops::SESSION_READ, json!({"view":"model"})));
    }
    for (method, params) in requests {
        check_expired(method, call(route, method, params).await?, expected)?;
    }
    Ok(())
}

async fn deletion<S: LlmRunnerSubject>(
    subject: &S,
    handle: &DriverHandle<S>,
    session: &str,
) -> Result<(), String> {
    if subject
        .retention_deletion_finished(handle.inner(), session)
        .await
        .map_err(|e| format!("inspecting retention deletion: {e}"))?
    {
        Ok(())
    } else {
        Err("runner-held deletion has not finished within delete_within_ms".into())
    }
}

struct Seed<R> {
    session: Session<R>,
    run_id: String,
    lineage: String,
    last_activity: u64,
    markers: Vec<String>,
}

impl<S> Case<'_, S>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    async fn retention_limits(&self) -> Result<(RoleDescribe, Retention), String> {
        let session = self.open("retention-describe", Script::default()).await?;
        let described = describe(&session.route).await?;
        let limits = limits(&described)?;
        Ok((described, limits))
    }

    async fn retained_send(
        &self,
        session: &mut Session<S::Route>,
        seconds: Option<u64>,
        title: Option<&str>,
    ) -> Result<(String, HeadMeta), String> {
        let prompt = self.mint.next("retention-prompt");
        let id = self.mint.next("send");
        let mut params = self.params(session, &prompt, &id, None);
        if let Some(seconds) = seconds {
            params["retention"] = json!(seconds);
        }
        if let Some(title) = title {
            params["title"] = json!(title);
        }
        let reply: SendReply = decode(
            "session.send",
            expect_ok(
                "session.send",
                call(&session.route, ops::SESSION_SEND, params).await?,
            )?,
        )?;
        if session.first.is_none() {
            session.first = Some((id, prompt.clone()));
        }
        wait_run_end(self.subject, &session.route, self.declared, None, &reply).await?;
        Ok((prompt, head(&session.route).await?))
    }

    async fn retention_seed(
        &self,
        seconds: Option<u64>,
        content: bool,
    ) -> Result<Seed<S::Route>, String> {
        let mut markers = Vec::new();
        let turns = if content {
            let mut tool = self.mint.call("retention-call");
            let marker = self.mint.next("retention-tool-result");
            tool.result = json!({"marker":marker});
            markers.push(marker);
            vec![
                ScriptedTurn {
                    parts: vec![ScriptedPart::ToolCall(tool)],
                },
                ScriptedTurn {
                    parts: vec![self.mint.text("retention-answer").0],
                },
            ]
        } else {
            (0..4)
                .map(|_| ScriptedTurn {
                    parts: vec![self.mint.text("retention-answer").0],
                })
                .collect()
        };
        let mut session = self.open("retention", Script { turns }).await?;
        let title = self.mint.next("retention-title");
        let (prompt, meta) = self
            .retained_send(&mut session, seconds, content.then_some(title.as_str()))
            .await?;
        let (_, messages) = read_all(&session.route).await?;
        if holding(&messages, &prompt) != 1 || (content && holding(&messages, &markers[0]) != 1) {
            return Err(
                "retention seed did not durably record its prompt and scripted tool result".into(),
            );
        }
        markers.push(prompt);
        if content {
            markers.push(title);
        }
        Ok(Seed {
            session,
            markers,
            run_id: meta
                .last_run_state
                .ok_or("seed has no terminal run")?
                .run_id,
            lineage: meta.lineage_id.ok_or("seed has no lineage")?,
            last_activity: meta.updated_at.ok_or("seed has no last activity")?,
        })
    }

    async fn wait_expired(
        &self,
        seed: &Seed<S::Route>,
        seconds: u64,
        limits: Retention,
    ) -> Result<(), String> {
        // Check immediately after expiry as well as after the deletion bound:
        // delete_within_ms is not an extra period of readable retention.
        tokio::time::sleep(wait_duration(seconds, 0)?).await;
        let expected = expiry(seed.last_activity, seconds)?;
        expired_reads(&seed.session.route, self.declared, &seed.run_id, expected).await?;
        tokio::time::sleep(Duration::from_millis(limits.delete_within_ms)).await;
        expired_reads(&seed.session.route, self.declared, &seed.run_id, expected).await?;
        deletion(self.subject, self.handle, &seed.session.name).await
    }

    async fn retention_refused(
        &self,
        seed: Option<&Seed<S::Route>>,
        seconds: u64,
        max: Option<u64>,
    ) -> Result<(), String> {
        let fresh;
        let session = match seed {
            Some(seed) => &seed.session,
            None => {
                fresh = self.open("retention-refusal", Script::default()).await?;
                &fresh
            }
        };
        let before = if self.declared.contains(&Capability::TranscriptReads) {
            Some(head(&session.route).await?)
        } else {
            None
        };
        let mut params = self.params(
            session,
            &self.mint.next("prompt"),
            &self.mint.next("send"),
            None,
        );
        params["retention"] = json!(seconds);
        let error = expect_error(
            "session.send retention",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?;
        expect_field(
            "session.send retention",
            &error,
            errors::INVALID_PARAMS,
            "retention",
        )?;
        if let Some(max) = max {
            if error
                .detail
                .as_ref()
                .and_then(|d| d["max_seconds"].as_u64())
                != Some(max)
            {
                return Err(format!(
                    "above-max refusal must carry max_seconds {max}: {:?}",
                    error.detail
                ));
            }
        }
        if let Some(before) = before {
            if head(&session.route).await? != before {
                return Err("a refused retention send wrote activity".into());
            }
        }
        Ok(())
    }

    pub(crate) async fn retention_case(&self, name: &str) -> Result<Ending, String> {
        if name == "retention_without_group_refused" {
            self.retention_refused(None, 1, None).await?;
            return Ok(Ending::Passed);
        }
        let (described, cap) = self.retention_limits().await?;
        let short = cap.max_seconds.min(2);
        match name {
            "retention_zero_refused" => self.retention_refused(None, 0, None).await?,
            "retention_above_max_refused" => {
                let Some(above) = cap.max_seconds.checked_add(1) else {
                    return Ok(Ending::Inapplicable(
                        "max_seconds is u64::MAX, so no u64 exceeds it".into(),
                    ));
                };
                self.retention_refused(None, above, Some(cap.max_seconds))
                    .await?;
            }
            "retention_lengthen_refused" => {
                if cap.max_seconds == 1 {
                    return Ok(Ending::Inapplicable(
                        "max_seconds is 1; every longer value is already above-max".into(),
                    ));
                }
                let seed = self.retention_seed(Some(1), false).await?;
                self.retention_refused(Some(&seed), 2, None).await?;
            }
            "retention_late_opt_in_refused" => {
                let seed = self.retention_seed(None, false).await?;
                self.retention_refused(Some(&seed), short, None).await?;
            }
            "retention_shorten_continues_lineage" | "retention_equal_noop" => {
                let initial = if name == "retention_equal_noop" {
                    short
                } else {
                    cap.max_seconds.min(4)
                };
                if name == "retention_shorten_continues_lineage" && initial == 1 {
                    return Ok(Ending::Inapplicable(
                        "max_seconds is 1, so no positive shorter retention exists".into(),
                    ));
                }
                let seconds = if name == "retention_equal_noop" {
                    initial
                } else {
                    1
                };
                let mut seed = self.retention_seed(Some(initial), false).await?;
                if name == "retention_shorten_continues_lineage" {
                    tokio::time::sleep(Duration::from_millis(1300)).await;
                }
                // The idle lineage is still inside its OLD retention. A new send
                // must preserve it, even if the new policy is shorter.
                let (_, meta) = self
                    .retained_send(&mut seed.session, Some(seconds), None)
                    .await?;
                if meta.lineage_id.as_deref() != Some(&seed.lineage) {
                    return Err("retention change replaced an unexpired lineage".into());
                }
                let (_, messages) = read_all(&seed.session.route).await?;
                if holding(&messages, &seed.markers[0]) != 1 {
                    return Err("retention change lost earlier messages".into());
                }
                seed.run_id = meta
                    .last_run_state
                    .ok_or("changed session has no run")?
                    .run_id;
                seed.last_activity = meta.updated_at.ok_or("changed session has no activity")?;
                if name == "retention_equal_noop" {
                    let (_, meta) = self.retained_send(&mut seed.session, None, None).await?;
                    seed.run_id = meta
                        .last_run_state
                        .ok_or("absent retention has no run")?
                        .run_id;
                    seed.last_activity =
                        meta.updated_at.ok_or("absent retention has no activity")?;
                }
                self.wait_expired(&seed, seconds, cap).await?;
            }
            "retention_honoured" => {
                let mut seed = self.retention_seed(Some(short), false).await?;
                self.wait_expired(&seed, short, cap).await?;
                // Empty success and expired are checked side by side.
                let unwritten = self
                    .open("retention-never-written", Script::default())
                    .await?;
                if read(&unwritten.route, &ReadRequest::tail()).await? != ReadPage::no_lineage()
                    || head(&unwritten.route).await? != HeadMeta::no_lineage()
                {
                    return Err("never-written session did not keep its empty success".into());
                }
                // Fresh admission fields and no retention: the new lineage must
                // be forever, not inherit the expired one's policy.
                seed.session.first = None;
                let (_, meta) = self.retained_send(&mut seed.session, None, None).await?;
                if meta.lineage_id.as_deref() == Some(&seed.lineage) {
                    return Err("fresh send reused the expired lineage".into());
                }
                let (_, messages) = read_all(&seed.session.route).await?;
                if seed.markers.iter().any(|m| holding(&messages, m) != 0) {
                    return Err("fresh lineage replayed expired content".into());
                }
                let error = expect_error(
                    "old lineage read",
                    call(
                        &seed.session.route,
                        ops::SESSION_READ,
                        json!({"lineage_id":seed.lineage}),
                    )
                    .await?,
                )?;
                expect_code("old lineage read", &error, errors::LINEAGE_CHANGED)?;
                tokio::time::sleep(wait_duration(short, cap.delete_within_ms)?).await;
                if head(&seed.session.route).await?.lineage_id != meta.lineage_id {
                    return Err("fresh lineage inherited old retention".into());
                }
            }
            "retention_run_status_expired" => {
                if !described.majors.iter().any(|m| m.serves(STATUS)) {
                    return Ok(Ending::Inapplicable(
                        "the runner does not advertise run.status".into(),
                    ));
                }
                let seed = self.retention_seed(Some(short), false).await?;
                self.wait_expired(&seed, short, cap).await?;
                check_expired(
                    STATUS,
                    call(&seed.session.route, STATUS, json!({"run_id":seed.run_id})).await?,
                    expiry(seed.last_activity, short)?,
                )?;
            }
            "retention_no_content_served" => {
                let seed = self.retention_seed(Some(short), true).await?;
                self.wait_expired(&seed, short, cap).await?;
                self.no_content(&seed, &described).await?;
            }
            "retention_active_run_never_expires" => self.retention_held(short, cap).await?,
            _ => return Err(format!("unknown retention case {name}")),
        }
        Ok(Ending::Passed)
    }

    async fn no_content(
        &self,
        seed: &Seed<S::Route>,
        described: &RoleDescribe,
    ) -> Result<(), String> {
        let route = &seed.session.route;
        let mut probes = vec![
            (ops::ROLE_DESCRIBE.to_owned(), json!({})),
            (ops::SESSION_READ.to_owned(), json!({})),
            (
                ops::SESSION_READ.to_owned(),
                json!({"from_ordinal":0,"include_originals":true}),
            ),
            (ops::SESSION_HEAD.to_owned(), json!({})),
            (ops::RUN_RESULT.to_owned(), json!({"run_id":seed.run_id})),
            (ops::SESSION_BASELINE.to_owned(), json!({})),
        ];
        if self.declared.contains(&Capability::ModelView) {
            probes.push((ops::SESSION_READ.into(), json!({"view":"model"})));
        }
        let methods: BTreeSet<&str> = described
            .majors
            .iter()
            .flat_map(|m| m.ops.iter().map(String::as_str))
            .collect();
        let role_ops: BTreeSet<&str> = capabilities::GROUPS
            .iter()
            .flat_map(|(_, ops)| ops.iter().copied())
            .chain([ops::ROLE_DESCRIBE, ops::SESSION_BASELINE])
            .collect();
        for method in methods {
            if method == STATUS {
                probes.push((method.into(), json!({"run_id":seed.run_id})));
                continue;
            }
            if role_ops.contains(method) {
                continue;
            }
            if let Some(variants) = self
                .subject
                .retention_probe_params(&seed.session.name, method)
                .map_err(|e| e.to_string())?
            {
                if variants.is_empty() {
                    return Err(format!("{method} has no retention probe variants"));
                }
                probes.extend(
                    variants
                        .into_iter()
                        .map(|params| (method.to_owned(), params)),
                );
            }
        }
        for (method, params) in probes {
            let reply = call(route, &method, params).await?;
            let body = match reply {
                Reply::Response(body) => body.to_string(),
                Reply::Error(error) => {
                    format!("{} {:?} {}", error.code, error.detail, error.message)
                }
            };
            if let Some(marker) = seed
                .markers
                .iter()
                .find(|marker| body.contains(marker.as_str()))
            {
                return Err(format!(
                    "expired content marker {marker} is still served by {method}"
                ));
            }
        }
        if self.declared.contains(&Capability::Streaming) {
            let reply = route
                .subscribe(json!({"from":"start"}))
                .await
                .map_err(|e| e.message)?;
            let body = match reply {
                SubscribeOutcome::Events(events) => format!("{events:?}"),
                SubscribeOutcome::Refused(error) => format!("{error:?}"),
            };
            if let Some(marker) = seed
                .markers
                .iter()
                .find(|marker| body.contains(marker.as_str()))
            {
                return Err(format!(
                    "expired content marker {marker} is still served by session.subscribe"
                ));
            }
        }
        Ok(())
    }

    async fn retention_held(&self, seconds: u64, cap: Retention) -> Result<(), String> {
        let mut held = self.mint.call("retention-held-call");
        held.hold = true;
        let session = self
            .open(
                "retention-held",
                Script {
                    turns: vec![
                        ScriptedTurn {
                            parts: vec![ScriptedPart::ToolCall(held.clone())],
                        },
                        ScriptedTurn {
                            parts: vec![self.mint.text("held-final").0],
                        },
                    ],
                },
            )
            .await?;
        let mut params = self.params(
            &session,
            &self.mint.next("prompt"),
            &self.mint.next("send"),
            None,
        );
        params["retention"] = json!(seconds);
        let probe = async {
            let checked = async {
                self.subject
                    .await_tool_call(&held.arguments)
                    .await
                    .map_err(|e| e.to_string())?;
                let before = head(&session.route).await?;
                let last = before
                    .last_run_state
                    .as_ref()
                    .ok_or("held session has no run")?;
                if RunState::parse(&last.state).is_terminal() {
                    return Err("held run is already terminal".into());
                }
                tokio::time::sleep(wait_duration(seconds, cap.delete_within_ms)?).await;
                let after = head(&session.route).await?;
                let run = run_result(&session.route, &last.run_id).await?;
                if after.lineage_id != before.lineage_id || run.run_state().is_terminal() {
                    return Err("held non-terminal run expired or became terminal".into());
                }
                read_all(&session.route).await?;
                Ok::<_, String>(())
            }
            .await;
            // Always release, including on a failed probe; otherwise a real
            // synchronous send could keep the entire suite waiting forever.
            let released = self
                .subject
                .release_tool_call(&held.arguments)
                .await
                .map_err(|e| e.to_string());
            checked.and(released)
        };
        let (sent, checked) = join(call(&session.route, ops::SESSION_SEND, params), probe).await;
        checked?;
        let reply: SendReply = decode("held send", expect_ok("held send", sent?)?)?;
        let end = wait_run_end(self.subject, &session.route, self.declared, None, &reply).await?;
        let meta = head(&session.route).await?;
        let seed = Seed {
            session,
            run_id: end.run_id,
            lineage: meta.lineage_id.ok_or("held session has no lineage")?,
            last_activity: meta.updated_at.ok_or("held session has no terminal time")?,
            markers: vec![],
        };
        self.wait_expired(&seed, seconds, cap).await
    }
}

/// The retention crash scenario cuts a background expiry rather than a send.
/// It gets its own root and uses the same one-kill ledger as other points.
pub(crate) async fn crash<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let root = work_dir.join("crash-RetentionTombstoned");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let name = mint.next("retention-crash");
    subject
        .install_script(
            &name,
            Script {
                turns: vec![ScriptedTurn {
                    parts: vec![mint.text("retention-crash-text").0],
                }],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let route = subject
        .session_route(handle.inner(), &name, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    let cap = limits(&describe(&route).await?)?;
    let seconds = cap.max_seconds.min(2);
    let mut params = send_params(
        subject,
        &name,
        true,
        &mint.next("prompt"),
        &mint.next("send"),
        None,
    );
    params["retention"] = json!(seconds);
    let reply: SendReply = decode(
        "crash send",
        expect_ok("crash send", call(&route, ops::SESSION_SEND, params).await?)?,
    )?;
    let end = wait_run_end(subject, &route, declared, None, &reply).await?;
    let last_activity = head(&route)
        .await?
        .updated_at
        .ok_or("crash session has no activity")?;
    let wait = wait_duration(seconds, cap.delete_within_ms)?;
    let trigger: Trigger<'_> = Box::pin(async {
        tokio::time::sleep(wait).await;
        // A read may drive lazy expiry; on a background sweeper it simply
        // observes the kill already performed by the armed harness.
        let _ = route.request(ops::SESSION_HEAD, json!({})).await;
    });
    driver
        .kill_at(
            handle,
            &KillPoint::new(points::RETENTION_TOMBSTONED),
            trigger,
        )
        .await
        .map_err(|e| format!("kill at RetentionTombstoned: {e}"))?;
    drop(route);
    let handle = driver.restart(&root).await.map_err(|e| e.to_string())?;
    let route = subject
        .session_route(handle.inner(), &name, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    expired_reads(
        &route,
        declared,
        &end.run_id,
        expiry(last_activity, seconds)?,
    )
    .await?;
    tokio::time::sleep(wait_duration(0, cap.delete_within_ms)?).await;
    deletion(subject, &handle, &name).await?;
    let kill = driver
        .ledger()
        .kills()
        .last()
        .ok_or("retention crash made no kill")?;
    if !kill.mechanism.is_real_process_kill() {
        return Err(
            "RetentionTombstoned requires a real process kill between tombstone and delete".into(),
        );
    }
    Ok(())
}
