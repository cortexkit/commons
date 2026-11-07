//! Opt-in custody checks over real call and late-result routes.

use async_trait::async_trait;
use cortexkit_role_harness::{HarnessError, KillReport};
use cortexkit_role_tool_provider::{
    catalog::{session_capabilities, CatalogAnswer},
    late_results::{kinds, LateEntry, LateResultsReply},
    reply_custody::{self, ExecutionMode},
};
use futures_util::future::join;
use serde_json::{json, Value};
use std::path::Path;

use crate::{single_terminal, CallSpec, Exchange, ObservedFrame, ToolProviderSubject, ToolRoute};

/// The lifecycle a provider's real-route fixture must exercise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplyCustodyScenario {
    Dependencies,
    Immediate,
    Held,
    RefusalAfterDispatch,
    RouteClose,
    Restart,
    FreshKey,
    PureDirect,
    PureRouteClose,
    PureHeld,
    FrozenCatalog,
    RetentionAck,
}

impl ReplyCustodyScenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dependencies => "reply_custody_dependencies",
            Self::Immediate => "reply_custody_immediate",
            Self::Held => "reply_custody_held",
            Self::RefusalAfterDispatch => "reply_custody_refusal_after_dispatch",
            Self::RouteClose => "reply_custody_route_close",
            Self::Restart => "reply_custody_restart",
            Self::FreshKey => "reply_custody_fresh_key",
            Self::PureDirect => "reply_custody_pure_direct",
            Self::PureRouteClose => "reply_custody_pure_route_close",
            Self::PureHeld => "reply_custody_pure_held",
            Self::FrozenCatalog => "reply_custody_frozen_catalog",
            Self::RetentionAck => "reply_custody_retention_ack",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [
            Self::Dependencies,
            Self::Immediate,
            Self::Held,
            Self::RefusalAfterDispatch,
            Self::RouteClose,
            Self::Restart,
            Self::FreshKey,
            Self::PureDirect,
            Self::PureRouteClose,
            Self::PureHeld,
            Self::FrozenCatalog,
            Self::RetentionAck,
        ]
        .into_iter()
        .find(|scenario| scenario.name() == name)
    }
}

/// Lifecycle controls, not an implementation of the tool or late-result ops.
/// All outcomes are read independently over the fixture's real routes.
#[async_trait]
pub trait ReplyCustodyControl<R: ToolRoute>: Send + Sync {
    /// Wait until the keyed call is dispatched, then let it finish. For held
    /// scenarios, wait for the held first reply before releasing the work.
    /// RouteClose and PureRouteClose close only the call route (not its scope)
    /// before release. A Pure call may stop, but must retain its outcome when
    /// no direct terminal reply was written to the route.
    /// Restart kills the real provider after DispatchStarted and restarts on
    /// the same state root without re-dispatch. Pin the observed outcome, or
    /// set expected_reduced_outcome to unknown for an indeterminate fixture.
    /// FrozenCatalog changes the live mode to pure after dispatch, before
    /// closing the call route. Return a fresh route as the same custodian,
    /// declaring late_results and reply_custody, after the work settles.
    async fn finish(&self, call_key: &str) -> Result<R, HarnessError>;

    /// Observe actual dispatches/effects independently of response bodies.
    async fn dispatch_count(&self) -> Result<u64, HarnessError>;

    /// The Restart fixture's kill at DispatchStarted. A missing report or a
    /// simulated kill fails that case; other scenarios need no report.
    fn restart_kill_report(&self) -> Option<KillReport> {
        None
    }
}

/// One fresh provider instance and live routes. Keep its handle alive in
/// `control`; dropping the fixture must stop that instance. The call route's
/// bind declares reply_custody, late_results and call_key. Held probes also
/// declare tool-result/v1 and held-reply/v1. The reader uses the principal
/// that raised the call (its carrier), which is also the result's custodian.
/// Expected values must be pinned, not obtained by calling the provider.
#[non_exhaustive]
pub struct ReplyCustodyFixture<R: ToolRoute> {
    pub call_route: R,
    pub catalog_arguments: Value,
    pub call: CallSpec,
    pub custodian: String,
    pub expected_result: Value,
    pub expected_reduced_outcome: String,
    pub control: Box<dyn ReplyCustodyControl<R>>,
}

impl<R: ToolRoute> ReplyCustodyFixture<R> {
    pub fn new(
        call_route: R,
        catalog_arguments: Value,
        call: CallSpec,
        custodian: String,
        expected_result: Value,
        expected_reduced_outcome: String,
        control: Box<dyn ReplyCustodyControl<R>>,
    ) -> Self {
        Self {
            call_route,
            catalog_arguments,
            call,
            custodian,
            expected_result,
            expected_reduced_outcome,
            control,
        }
    }
}

fn response(exchange: Exchange) -> Result<Value, String> {
    match single_terminal(&exchange).map_err(|e| e.to_string())? {
        ObservedFrame::Response(value) => Ok(value.clone()),
        other => Err(format!("expected response, got {other:?}")),
    }
}

async fn read<R: ToolRoute>(route: &R) -> Result<LateResultsReply, String> {
    let mut since = None;
    let mut entries = Vec::new();
    loop {
        let mut page: LateResultsReply = serde_json::from_value(response(
            route
                .request(json!({"name":"late_results", "arguments":{"since":since}}))
                .await
                .map_err(|e| e.message)?,
        )?)
        .map_err(|e| e.to_string())?;
        entries.append(&mut page.entries);
        if !page.more {
            page.entries = entries;
            return Ok(page);
        }
        if since.as_ref() == Some(&page.cursor) {
            return Err("late-results pagination did not advance its cursor".into());
        }
        since = Some(page.cursor);
    }
}

fn terminal<'a>(entries: &'a [LateEntry], key: &str) -> Result<&'a LateEntry, String> {
    let matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry.call_key == key && entry.kind == kinds::RESULT)
        .collect();
    if matches.len() != 1 {
        return Err(format!(
            "expected one result for {key}, found {}",
            matches.len()
        ));
    }
    Ok(matches[0])
}

fn check_outcome<R: ToolRoute>(
    fixture: &ReplyCustodyFixture<R>,
    entry: &LateEntry,
    restart: bool,
) -> Result<(), String> {
    if entry.custodian != fixture.custodian {
        return Err("entry belongs to another custodian".into());
    }
    if restart && fixture.expected_reduced_outcome == "unknown" {
        if entry.outcome.as_deref() != Some("unknown") || entry.result.is_some() {
            return Err("indeterminate restart must record unknown".into());
        }
    } else if entry.reduced {
        if entry.outcome.as_deref() != Some(&fixture.expected_reduced_outcome) {
            return Err("reduced custody outcome differs from the call".into());
        }
    } else if entry.result.as_ref() != Some(&fixture.expected_result) {
        return Err("custody result differs from the call".into());
    }
    Ok(())
}

pub(crate) async fn run<S: ToolProviderSubject>(
    subject: &S,
    root: &Path,
    scenario: ReplyCustodyScenario,
    key: &str,
) -> Result<(), String>
where
    S::Route: ToolRoute,
{
    let fixture = subject
        .reply_custody_fixture(root, scenario)
        .await
        .map_err(|e| e.to_string())?;
    check(scenario, key, fixture)
        .await
        .map_err(|error| format!("{}: {error}", scenario.name()))
}

async fn check<R: ToolRoute>(
    scenario: ReplyCustodyScenario,
    key: &str,
    fixture: ReplyCustodyFixture<R>,
) -> Result<(), String> {
    let catalog: CatalogAnswer = serde_json::from_value(response(
        fixture
            .call_route
            .request(json!({
                "name":"tool.catalog", "arguments":fixture.catalog_arguments
            }))
            .await
            .map_err(|e| e.message)?,
    )?)
    .map_err(|e| e.to_string())?;
    if !catalog.declares(reply_custody::CAPABILITY) {
        return Err("fixture catalog does not declare reply_custody".into());
    }
    reply_custody::check_dependencies(
        true,
        catalog.declares(session_capabilities::LATE_RESULTS),
        catalog.declares(session_capabilities::CALL_KEY),
    )
    .map_err(|e| e.message)?;
    if scenario == ReplyCustodyScenario::Dependencies {
        return Ok(());
    }
    let tool = catalog
        .tools
        .iter()
        .find(|tool| tool.name == fixture.call.0)
        .ok_or("probe tool missing from frozen catalog")?;
    let pure = matches!(
        scenario,
        ReplyCustodyScenario::PureDirect
            | ReplyCustodyScenario::PureHeld
            | ReplyCustodyScenario::PureRouteClose
    );
    if reply_custody::is_pure(tool) != pure {
        return Err("probe mode disagrees with frozen catalog".into());
    }
    let body = |key: &str| json!({"name":fixture.call.0,"arguments":fixture.call.1,"call_key":key});
    let (direct, reader) = join(
        fixture.call_route.request(body(key)),
        fixture.control.finish(key),
    )
    .await;
    let reader = reader.map_err(|e| e.to_string())?;
    if scenario == ReplyCustodyScenario::Restart {
        let kill = fixture
            .control
            .restart_kill_report()
            .ok_or("restart fixture supplied no kill report")?;
        if kill.point.as_str() != cortexkit_role_tool_provider::points::DISPATCH_STARTED
            || !kill.mechanism.is_real_process_kill()
        {
            return Err("restart fixture did not kill a real process at DispatchStarted".into());
        }
    }
    if !matches!(
        scenario,
        ReplyCustodyScenario::RouteClose
            | ReplyCustodyScenario::PureRouteClose
            | ReplyCustodyScenario::Restart
            | ReplyCustodyScenario::FrozenCatalog
    ) {
        let exchange = direct.map_err(|e| e.message)?;
        let frame = single_terminal(&exchange).map_err(|e| e.to_string())?;
        match scenario {
            ReplyCustodyScenario::RefusalAfterDispatch => match frame {
                ObservedFrame::Error(error)
                    if serde_json::to_value(error).map_err(|e| e.to_string())?
                        == fixture.expected_result => {}
                _ => return Err("expected the pinned post-dispatch refusal".into()),
            },
            ReplyCustodyScenario::Held | ReplyCustodyScenario::PureHeld => match frame {
                ObservedFrame::Response(value) if value.get("held").is_some() => (),
                _ => return Err("probe did not return a held first reply".into()),
            },
            _ => {
                if response(exchange)? != fixture.expected_result {
                    return Err("direct reply differs from the pinned outcome".into());
                }
            }
        }
    }
    let first = read(&reader).await?;
    if scenario == ReplyCustodyScenario::PureDirect {
        // Omission is permitted, but an entry that is kept must be truthful.
        if first.entries.iter().any(|entry| entry.call_key == key) {
            check_outcome(&fixture, terminal(&first.entries, key)?, false)?;
        }
    } else {
        check_outcome(
            &fixture,
            terminal(&first.entries, key)?,
            scenario == ReplyCustodyScenario::Restart,
        )?;
    }
    if scenario == ReplyCustodyScenario::FrozenCatalog {
        let live: CatalogAnswer = serde_json::from_value(response(
            reader
                .request(json!({
                    "name":"tool.catalog", "arguments":fixture.catalog_arguments
                }))
                .await
                .map_err(|e| e.message)?,
        )?)
        .map_err(|e| e.to_string())?;
        if !live.tools.iter().any(|tool| {
            tool.name == fixture.call.0 && tool.execution_mode == Some(ExecutionMode::Pure)
        }) {
            return Err("fixture did not change the live catalog to pure".into());
        }
    }
    if scenario == ReplyCustodyScenario::RetentionAck {
        let repeated = read(&reader).await?;
        if terminal(&repeated.entries, key)? != terminal(&first.entries, key)? {
            return Err("unacknowledged result changed or disappeared after a read".into());
        }
        let ack = response(
            reader
                .request(json!({"name":"late_results.ack",
            "arguments":{"through":first.cursor}}))
                .await
                .map_err(|e| e.message)?,
        )?;
        if ack != json!({}) {
            return Err("ack did not return an empty response".into());
        }
        if read(&reader)
            .await?
            .entries
            .iter()
            .any(|entry| entry.call_key == key)
        {
            return Err("acknowledged entry is still returned".into());
        }
    }
    let expected_dispatches = if scenario == ReplyCustodyScenario::FreshKey {
        let fresh = format!("{key}/fresh");
        if response(reader.request(body(&fresh)).await.map_err(|e| e.message)?)?
            != fixture.expected_result
        {
            return Err("fresh-key call did not return its own outcome".into());
        }
        let both = read(&reader).await?;
        check_outcome(&fixture, terminal(&both.entries, key)?, false)?;
        check_outcome(&fixture, terminal(&both.entries, &fresh)?, false)?;
        2
    } else {
        1
    };
    if fixture
        .control
        .dispatch_count()
        .await
        .map_err(|e| e.to_string())?
        != expected_dispatches
    {
        return Err("provider skipped a fresh call or re-ran a started key".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RouteFailure, CASES};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        Dependencies,
        MissingKey,
        MissingEntry,
        CancelOnClose,
        Rerun,
        GuessedRestart,
        ObservedRestart,
        SimulatedRestart,
        ReuseKey,
        PureWrongEntry,
        LiveCatalog,
        ReadDeletes,
        AckIgnored,
    }

    struct State {
        scenario: ReplyCustodyScenario,
        fault: Fault,
        entries: Vec<LateEntry>,
        dispatches: u64,
        live_pure: bool,
        done: bool,
    }
    #[derive(Clone)]
    struct Route(Arc<Mutex<State>>);
    struct Control(Arc<Mutex<State>>);

    fn result() -> Value {
        json!({"answer":"done"})
    }
    fn refusal() -> Value {
        json!({"code":"tool_unavailable","message":"unavailable after dispatch"})
    }
    fn reply(value: Value) -> Result<Exchange, RouteFailure> {
        Ok(Exchange {
            frames: vec![ObservedFrame::Response(value)],
        })
    }
    fn entry(key: &str, state: &State) -> LateEntry {
        let mut entry = LateEntry::new(
            "result",
            "owner",
            "scope",
            1,
            "carrier",
            key,
            format!("event/{key}"),
            100,
        );
        if state.scenario == ReplyCustodyScenario::Restart && state.fault != Fault::ObservedRestart
        {
            entry.reduced = true;
            entry.outcome = Some(
                if state.fault == Fault::GuessedRestart {
                    "ok"
                } else {
                    "unknown"
                }
                .into(),
            );
        } else {
            entry.result = Some(
                if state.scenario == ReplyCustodyScenario::RefusalAfterDispatch {
                    refusal()
                } else if state.fault == Fault::PureWrongEntry {
                    json!({"answer":"wrong"})
                } else {
                    result()
                },
            );
        }
        entry
    }
    fn record(state: &mut State, key: &str) {
        if state.fault == Fault::MissingEntry
            || state.fault == Fault::CancelOnClose
            || state.fault == Fault::LiveCatalog
        {
            return;
        }
        if state.scenario == ReplyCustodyScenario::PureDirect && state.fault == Fault::None {
            return;
        }
        state.entries.push(entry(key, state));
    }
    impl Route {
        fn request_now(&self, body: Value) -> Option<Result<Exchange, RouteFailure>> {
            let mut s = self.0.lock().unwrap();
            Some(match body["name"].as_str().unwrap() {
                "tool.catalog" => {
                    let pure = s.live_pure
                        || matches!(
                            s.scenario,
                            ReplyCustodyScenario::PureDirect
                                | ReplyCustodyScenario::PureHeld
                                | ReplyCustodyScenario::PureRouteClose
                        );
                    let mut caps =
                        json!({"reply_custody":true,"late_results":true,"call_key":true});
                    if s.fault == Fault::Dependencies {
                        caps["late_results"] = json!(false);
                    }
                    if s.fault == Fault::MissingKey {
                        caps["call_key"] = json!(false);
                    }
                    reply(
                        json!({"generation":"g","catalog_digest":"digest", "capabilities":caps,
                        "tools":[{"name":"work","schema_digest":"digest","semantics":1,
                            "execution_mode":if pure {"pure"} else {"mutating"},
                            "input_schema":{"type":"object"}}]}),
                    )
                }
                "late_results" => {
                    let entries = s.entries.clone();
                    if s.fault == Fault::ReadDeletes {
                        s.entries.clear();
                    }
                    reply(
                        json!({"entries":entries,"cursor":{"provider_incarnation":"p","seq":entries.len()},"more":false}),
                    )
                }
                "late_results.ack" => {
                    if s.fault != Fault::AckIgnored {
                        s.entries.clear();
                    }
                    reply(json!({}))
                }
                _ => {
                    let key = body["call_key"].as_str().unwrap();
                    if s.fault == Fault::ReuseKey && s.dispatches > 0 {
                        return Some(reply(result()));
                    }
                    s.dispatches += 1;
                    let held = matches!(
                        s.scenario,
                        ReplyCustodyScenario::Held | ReplyCustodyScenario::PureHeld
                    );
                    let waiting = matches!(
                        s.scenario,
                        ReplyCustodyScenario::RouteClose
                            | ReplyCustodyScenario::PureRouteClose
                            | ReplyCustodyScenario::Restart
                            | ReplyCustodyScenario::FrozenCatalog
                    );
                    if held {
                        return Some(reply(json!({"content":[],"held":{"reason":"running"}})));
                    }
                    if waiting {
                        return None;
                    }
                    record(&mut s, key);
                    if s.scenario == ReplyCustodyScenario::RefusalAfterDispatch {
                        Ok(Exchange {
                            frames: vec![ObservedFrame::Error(
                                serde_json::from_value(refusal()).unwrap(),
                            )],
                        })
                    } else {
                        reply(result())
                    }
                }
            })
        }
    }
    #[async_trait]
    impl ToolRoute for Route {
        async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
            match self.request_now(body) {
                Some(reply) => reply,
                None => self.wait_for_close().await,
            }
        }
        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }
    impl Route {
        async fn wait_for_close(&self) -> Result<Exchange, RouteFailure> {
            loop {
                if self.0.lock().unwrap().done {
                    return Err(RouteFailure::new("route closed"));
                }
                tokio::task::yield_now().await;
            }
        }
    }
    #[async_trait]
    impl ReplyCustodyControl<Route> for Control {
        async fn finish(&self, key: &str) -> Result<Route, HarnessError> {
            loop {
                if self.0.lock().unwrap().dispatches != 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            let mut s = self.0.lock().unwrap();
            if matches!(
                s.scenario,
                ReplyCustodyScenario::Held
                    | ReplyCustodyScenario::PureHeld
                    | ReplyCustodyScenario::RouteClose
                    | ReplyCustodyScenario::PureRouteClose
                    | ReplyCustodyScenario::Restart
                    | ReplyCustodyScenario::FrozenCatalog
            ) {
                if s.scenario == ReplyCustodyScenario::FrozenCatalog {
                    s.live_pure = true;
                }
                if s.fault == Fault::Rerun {
                    s.dispatches += 1;
                }
                record(&mut s, key);
            }
            s.done = true;
            Ok(Route(self.0.clone()))
        }
        async fn dispatch_count(&self) -> Result<u64, HarnessError> {
            Ok(self.0.lock().unwrap().dispatches)
        }
        fn restart_kill_report(&self) -> Option<KillReport> {
            use cortexkit_role_harness::{KillMechanism, KillPoint};
            Some(KillReport {
                point: KillPoint::new(cortexkit_role_tool_provider::points::DISPATCH_STARTED),
                mechanism: if self.0.lock().unwrap().fault == Fault::SimulatedRestart {
                    KillMechanism::FaultHook
                } else {
                    KillMechanism::ProcessSignal
                },
            })
        }
    }

    async fn probe(scenario: ReplyCustodyScenario, fault: Fault) -> Result<(), String> {
        let state = Arc::new(Mutex::new(State {
            scenario,
            fault,
            entries: vec![],
            dispatches: 0,
            live_pure: false,
            done: false,
        }));
        let fixture = ReplyCustodyFixture::new(
            Route(state.clone()),
            json!({}),
            ("work".into(), json!({})),
            "carrier".into(),
            if scenario == ReplyCustodyScenario::RefusalAfterDispatch {
                refusal()
            } else {
                result()
            },
            if scenario == ReplyCustodyScenario::Restart && fault != Fault::ObservedRestart {
                "unknown"
            } else {
                "ok"
            }
            .into(),
            Box::new(Control(state)),
        );
        check(scenario, "call/1", fixture)
            .await
            .map_err(|error| format!("{}: {error}", scenario.name()))
    }

    async fn rejects(scenario: ReplyCustodyScenario, fault: Fault) {
        probe(scenario, Fault::None).await.unwrap();
        let error = probe(scenario, fault).await.unwrap_err();
        assert!(error.starts_with(scenario.name()), "{error}");
        println!("rejected {error}");
    }

    #[tokio::test]
    async fn reply_custody_dependencies_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::Dependencies, Fault::Dependencies).await;
        rejects(ReplyCustodyScenario::Dependencies, Fault::MissingKey).await;
    }
    #[tokio::test]
    async fn reply_custody_immediate_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::Immediate, Fault::MissingEntry).await;
    }
    #[tokio::test]
    async fn reply_custody_held_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::Held, Fault::MissingEntry).await;
    }
    #[tokio::test]
    async fn reply_custody_refusal_after_dispatch_rejects_faulty_provider() {
        rejects(
            ReplyCustodyScenario::RefusalAfterDispatch,
            Fault::MissingEntry,
        )
        .await;
    }
    #[tokio::test]
    async fn reply_custody_route_close_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::RouteClose, Fault::CancelOnClose).await;
    }
    #[tokio::test]
    async fn reply_custody_restart_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::Restart, Fault::Rerun).await;
        rejects(ReplyCustodyScenario::Restart, Fault::MissingEntry).await;
        rejects(ReplyCustodyScenario::Restart, Fault::GuessedRestart).await;
        rejects(ReplyCustodyScenario::Restart, Fault::SimulatedRestart).await;
        probe(ReplyCustodyScenario::Restart, Fault::ObservedRestart)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn reply_custody_fresh_key_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::FreshKey, Fault::ReuseKey).await;
    }
    #[tokio::test]
    async fn reply_custody_pure_direct_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::PureDirect, Fault::PureWrongEntry).await;
    }
    #[tokio::test]
    async fn reply_custody_pure_route_close_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::PureRouteClose, Fault::MissingEntry).await;
    }
    #[tokio::test]
    async fn reply_custody_pure_held_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::PureHeld, Fault::MissingEntry).await;
    }
    #[tokio::test]
    async fn reply_custody_frozen_catalog_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::FrozenCatalog, Fault::LiveCatalog).await;
    }
    #[tokio::test]
    async fn reply_custody_retention_ack_rejects_faulty_provider() {
        rejects(ReplyCustodyScenario::RetentionAck, Fault::ReadDeletes).await;
        rejects(ReplyCustodyScenario::RetentionAck, Fault::AckIgnored).await;
    }
    #[test]
    fn reply_custody_cases_require_the_opt_in_declaration() {
        let cases: Vec<_> = CASES
            .iter()
            .filter(|case| case.name.starts_with("reply_custody_"))
            .collect();
        assert_eq!(cases.len(), 12);
        for case in cases {
            assert!(
                case.requires.contains(&crate::Capability::ReplyCustody),
                "{}",
                case.name
            );
            assert!(ReplyCustodyScenario::from_name(case.name).is_some());
        }
    }
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    use crate::RouteFailure;

    struct Pages;
    #[async_trait]
    impl ToolRoute for Pages {
        async fn request(&self, body: Value) -> Result<Exchange, RouteFailure> {
            let first = body["arguments"]["since"].is_null();
            let entry = LateEntry::new(
                "result",
                "owner",
                "scope",
                1,
                "carrier",
                if first { "other" } else { "target" },
                "event",
                100,
            );
            Ok(Exchange {
                frames: vec![ObservedFrame::Response(json!({
                    "entries":[entry], "cursor":{"provider_incarnation":"p", "seq":if first {1} else {2}},
                    "more":first
                }))],
            })
        }
        async fn request_then_cancel(&self, body: Value) -> Result<Exchange, RouteFailure> {
            self.request(body).await
        }
    }

    #[tokio::test]
    async fn reply_custody_reads_all_late_result_pages() {
        let log = read(&Pages).await.unwrap();
        assert_eq!(log.entries.len(), 2);
        assert_eq!(terminal(&log.entries, "target").unwrap().event_id, "event");
        assert_eq!(log.cursor.seq, 2);
        assert!(!log.more);
    }
}
