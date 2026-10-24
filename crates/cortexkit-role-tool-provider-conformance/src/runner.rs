//! The cases, run against a live provider.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
};

use cortexkit_role_harness::{
    CrashDriver, DriverHandle, KillPoint, RealKillVerdict, RouteStamp, Trigger,
};
use cortexkit_role_tool_provider::{
    call::{SchemaPin, CALL_KEY_FIELD, SCHEMA_PIN_FIELD},
    catalog::{
        check_flat_schema, check_modes, check_presets, check_reply, is_schema_digest,
        params_for_capabilities, schema_digest, session_capabilities, system_text_digest,
        CatalogAnswer,
    },
    check_capability_tag,
    describe::{check_describe, select_tool_capabilities},
    errors,
    late_results::{kinds, LateResultsReply, CURSOR_INCARNATION_CHANGED},
    ops, points,
    result::{validate, ResultFormat, ToolResult, MAX_RESULT_BYTES},
    withdraw::{WithdrawAnswer, TARGET_CALL_KEY_FIELD},
    CallMode, CATALOG_CAPABILITY_FILTER, HELD_REPLY_V1, PROVIDES, TOOL_RESULT_V1_CAPABILITY,
};
use futures_util::future::join;
use serde_json::{json, Value};
use subc_protocol::ErrorBody;

use crate::{
    report::{CaseOutcome, CaseReport, SuiteReport, SuiteVerdict, CASES},
    route::{single_terminal, Exchange, ObservedFrame, ToolRoute},
    subject::{Capability, ScopedPrincipals, ToolProviderSubject},
};

/// The shared vectors, compiled in so every provider's CI checks the same
/// bytes the wire crate's own tests check.
///
/// The bytes come from the wire crate's public `vectors` module. Its
/// `test-vectors` feature is enabled by this crate's dependency, so consumers
/// need neither vendored copies nor a repository checkout.
const CALL_KEY_VECTORS: &str = cortexkit_role_tool_provider::vectors::CALL_KEY_JSON;
const SCHEMA_PIN_VECTORS: &str = cortexkit_role_tool_provider::vectors::SCHEMA_PIN_JSON;

/// A tool name no provider serves, for the refusal case.
const UNSERVED_TOOL: &str = "conformance.not-a-served-tool";

/// A preset no provider defines, for the unknown-preset case.
const UNDEFINED_PRESET: &str = "cortexkit-conformance-undefined-preset";

/// The `tool.catalog` request member naming the plan item's preset.
const PRESET_FIELD: &str = "preset";

/// The run could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// The work directory is not empty. Each run needs a fresh one, because
    /// call keys must never be reused against the same state root.
    WorkDirNotEmpty(PathBuf),
    Io(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkDirNotEmpty(path) => write!(f, "{} is not empty", path.display()),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SetupError {}

type CaseResult = Result<(), String>;

/// Mints call keys that are unique within one run.
struct KeyMint(u64);

impl KeyMint {
    fn next(&mut self, case: &str) -> String {
        self.0 += 1;
        format!("conformance/{case}/{}", self.0)
    }
}

/// Run every case against `subject`, with state roots and marker files under
/// `work_dir`, which must be empty or absent.
pub async fn run_suite<S>(subject: &S, work_dir: &Path) -> Result<SuiteReport, SetupError>
where
    S: ToolProviderSubject,
    S::Route: ToolRoute,
{
    if work_dir
        .read_dir()
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
    {
        return Err(SetupError::WorkDirNotEmpty(work_dir.to_owned()));
    }
    for dir in ["main", "markers"] {
        std::fs::create_dir_all(work_dir.join(dir))
            .map_err(|e| SetupError::Io(format!("creating {dir}: {e}")))?;
    }

    let mut capabilities = subject.capabilities();
    capabilities.remove(&Capability::CatalogCapabilityFilter);
    // A provider may serve every known mode on every tool. Without a
    // restricted probe there is no refusal to exercise, so report a skip.
    if subject.call_mode_probe().is_none() {
        capabilities.remove(&Capability::CallModes);
    }
    let mut driver = CrashDriver::new(subject);
    let mut mint = KeyMint(0);
    let main = driver
        .spawn(&work_dir.join("main"))
        .await
        .map_err(|e| format!("spawning the provider failed: {e}"));

    // The live role.describe answer, not a subject flag, decides whether the
    // catalog-filter case is required.
    let advertises_catalog_filter = match &main {
        Ok(handle) => {
            let case = Case {
                subject,
                driver: &driver,
                handle,
                work_dir,
                capabilities: &capabilities,
            };
            match case.plain_route().await {
                Ok(route) => case
                    .describe(&route)
                    .await
                    .ok()
                    .and_then(|answer| answer["capabilities"].as_array().cloned())
                    .is_some_and(|declared| {
                        declared.iter().any(|capability| {
                            capability.as_str() == Some(CATALOG_CAPABILITY_FILTER)
                        })
                    }),
                Err(_) => false,
            }
        }
        Err(_) => false,
    };
    if advertises_catalog_filter {
        capabilities.insert(Capability::CatalogCapabilityFilter);
    }

    // Catalog declarations, not a subject flag, decide whether result checks
    // apply. A catalog/setup error must fail the case rather than hide as a skip.
    let has_tool_result_v1 = match &main {
        Err(_) => true,
        Ok(handle) => {
            let case = Case {
                subject,
                driver: &driver,
                handle,
                work_dir,
                capabilities: &capabilities,
            };
            match case.plain_route().await {
                Err(_) => true,
                Ok(route) => match case.catalog(&route).await {
                    Err(_) => true,
                    Ok(catalog) => catalog
                        .tools
                        .iter()
                        .any(|tool| tool.result_format == Some(ResultFormat::ToolResultV1)),
                },
            }
        }
    };
    capabilities.remove(&Capability::ToolResultV1);
    if has_tool_result_v1 {
        capabilities.insert(Capability::ToolResultV1);
    }

    capabilities.remove(&Capability::UserCommands);
    if let Ok(handle) = &main {
        let case = Case {
            subject,
            driver: &driver,
            handle,
            work_dir,
            capabilities: &capabilities,
        };
        let listed = match case.plain_route().await {
            Ok(route) => case
                .catalog(&route)
                .await
                .map(|answer| !answer.user_commands.is_empty())
                .unwrap_or(true),
            Err(_) => true,
        };
        if listed {
            capabilities.insert(Capability::UserCommands);
        }
    }

    // The provider's own preset-less catalog answer, not a subject flag,
    // decides whether the served-presets case applies. A catalog error runs
    // the case, so it fails instead of hiding as a skip.
    capabilities.remove(&Capability::Presets);
    if let Ok(handle) = &main {
        let case = Case {
            subject,
            driver: &driver,
            handle,
            work_dir,
            capabilities: &capabilities,
        };
        let listed = match case.plain_route().await {
            Ok(route) => case
                .preset_less_catalog(&route)
                .await
                .map(|answer| !answer.presets.is_empty())
                .unwrap_or(true),
            Err(_) => true,
        };
        if listed {
            capabilities.insert(Capability::Presets);
        }
    }

    let mut cases = Vec::with_capacity(CASES.len());
    for spec in CASES {
        let missing: Vec<Capability> = spec
            .requires
            .iter()
            .filter(|required| !capabilities.contains(required))
            .copied()
            .collect();
        let outcome = if spec.name == "system_text_default_served"
            && !capabilities.contains(&Capability::SystemTextDefault)
        {
            CaseOutcome::Inapplicable {
                reason: "provider does not declare system-text-default/v1".into(),
            }
        } else if !missing.is_empty() {
            CaseOutcome::Skipped { missing }
        } else {
            let result = match spec.name {
                "crash_after_prepared_not_started" => {
                    crash_case(subject, &mut driver, work_dir, &mut mint, points::PREPARED).await
                }
                "crash_after_authorized_not_started" => {
                    crash_case(
                        subject,
                        &mut driver,
                        work_dir,
                        &mut mint,
                        points::AUTHORIZED,
                    )
                    .await
                }
                name => match &main {
                    Err(error) => Err(error.clone()),
                    Ok(handle) => {
                        let case = Case {
                            subject,
                            driver: &driver,
                            handle,
                            work_dir,
                            capabilities: &capabilities,
                        };
                        case.run(name, &mut mint).await
                    }
                },
            };
            match result {
                Ok(()) => CaseOutcome::Passed,
                Err(reason) => CaseOutcome::Failed { reason },
            }
        };
        cases.push(CaseReport {
            case: spec.name,
            requires: spec.requires.to_vec(),
            outcome,
        });
    }
    drop(main);

    let kills = driver.ledger().kills().to_vec();
    let mut failures: Vec<String> = cases
        .iter()
        .filter_map(|case| match &case.outcome {
            CaseOutcome::Failed { reason } => Some(format!("{}: {reason}", case.case)),
            _ => None,
        })
        .collect();
    if driver.ledger().verdict() == RealKillVerdict::OnlySimulated {
        let used: Vec<String> = kills
            .iter()
            .map(|kill| format!("{} by {}", kill.point, kill.mechanism))
            .collect();
        failures.push(format!(
            "no kill in this run ended a real process ({}), so the run cannot tell whether \
             the provider keeps something only in memory",
            used.join(", ")
        ));
    }
    let skipped: BTreeSet<Capability> = cases
        .iter()
        .filter_map(|case| match &case.outcome {
            CaseOutcome::Skipped { missing } => Some(missing.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    let verdict = if !failures.is_empty() {
        SuiteVerdict::Failed { reasons: failures }
    } else if !skipped.is_empty() {
        SuiteVerdict::ConformingForDeclaredCapabilities {
            skipped: skipped.into_iter().collect(),
        }
    } else {
        SuiteVerdict::Passed
    };
    Ok(SuiteReport {
        cases,
        kills,
        verdict,
    })
}

/// The context of a case that runs on the shared module.
struct Case<'a, S: ToolProviderSubject> {
    subject: &'a S,
    driver: &'a CrashDriver<'a, S>,
    handle: &'a DriverHandle<S>,
    work_dir: &'a Path,
    capabilities: &'a BTreeSet<Capability>,
}

impl<S> Case<'_, S>
where
    S: ToolProviderSubject,
    S::Route: ToolRoute,
{
    async fn run(&self, name: &str, mint: &mut KeyMint) -> CaseResult {
        match name {
            name if name.starts_with("user_commands_") => {
                self.user_commands(name, mint.next(name)).await
            }
            "role_describe_shape" => self.role_describe_shape().await,
            "role_describe_cacheable" => self.role_describe_cacheable().await,
            "catalog_schemas_flat" => self.catalog_schemas_flat().await,
            "catalog_reply_valid" => self.catalog_reply_valid().await,
            "catalog_modes_valid" => self.catalog_modes_valid().await,
            "catalog_emits_events_boolean" => self.catalog_emits_events_boolean().await,
            "call_mode_matches_catalog" => self.call_mode_matches_catalog().await,
            "catalog_schema_digest_stable" => self.catalog_schema_digest_stable().await,
            "catalog_digest_only" => self.catalog_digest_only().await,
            "catalog_unknown_preset_refused" => self.catalog_unknown_preset_refused().await,
            "catalog_listed_presets_served" => self.catalog_listed_presets_served().await,
            "catalog_capability_filter_matches_describe" => {
                self.catalog_capability_filter_matches_describe().await
            }
            "system_text_digests_match_text" => self.system_text_digests_match_text().await,
            "system_text_preflight_digest_stable" => {
                self.system_text_preflight_digest_stable().await
            }
            "system_text_preset_optional_served" => self.system_text_preset_optional_served().await,
            "system_text_default_served" => self.system_text_preset_optional_served().await,
            "catalog_disabled_tool_absent" => self.catalog_disabled_tool_absent().await,
            "call_disabled_tool_refused_by_name" => self.call_disabled_tool_refused().await,
            "terminal_frame_on_success" => self.terminal_frame_on_success().await,
            "tool_result_v1_valid" => self.tool_result_v1_valid().await,
            "tool_result_held_reply_route_gating" => self.held_reply_route_gating(mint).await,
            "terminal_frame_on_refusal" => self.terminal_frame_on_refusal().await,
            "terminal_frame_on_cancel" => self.terminal_frame_on_cancel().await,
            "call_key_malformed_refused" => self.call_key_malformed_refused().await,
            "call_key_well_formed_accepted" => self.call_key_well_formed_accepted().await,
            "schema_pin_current_accepted" => self.schema_pin_current_accepted().await,
            "schema_pin_malformed_refused" => self.schema_pin_malformed_refused().await,
            "withdraw_unknown_call" => self.withdraw_unknown_call(mint.next(name)).await,
            "withdraw_answer_identical_on_repeat_and_owner" => {
                self.withdraw_identical(mint.next(name)).await
            }
            "withdraw_owner_as_carrier_needs_no_carrier" => {
                self.withdraw_owner_as_carrier(mint.next(name)).await
            }
            "withdraw_owner_without_carrier_refused" => {
                self.withdraw_owner_without_carrier(mint.next(name)).await
            }
            "withdraw_carrier_naming_other_carrier_refused" => {
                self.withdraw_carrier_naming_other(mint.next(name)).await
            }
            "withdraw_scope_mismatch_refused" => {
                self.withdraw_scope_mismatch(mint.next(name)).await
            }
            "withdraw_top_level_call_key_refused" => {
                let own_key = mint.next(name);
                self.withdraw_top_level_call_key(mint.next(name), own_key)
                    .await
            }
            "withdraw_malformed_target_key_refused" => self.withdraw_malformed_target().await,
            "withdraw_not_permitted_is_route_error" => {
                self.withdraw_not_permitted(mint.next(name)).await
            }
            "late_results_cursor_round_trip" => self.late_results_round_trip(mint.next(name)).await,
            "late_results_ack" => self.late_results_ack(mint.next(name)).await,
            "late_results_incarnation_change_refused" => {
                self.late_results_incarnation_change().await
            }
            other => Err(format!("the runner has no case named {other}")),
        }
    }

    async fn route(&self, stamp: &RouteStamp) -> Result<S::Route, String> {
        self.driver
            .route(self.handle, stamp)
            .await
            .map_err(|e| format!("opening a route as {}: {e}", stamp.principal))
    }

    async fn plain_route(&self) -> Result<S::Route, String> {
        self.route(&self.subject.plain_stamp()).await
    }

    async fn consumer_route(&self, consumer_capabilities: &[String]) -> Result<S::Route, String> {
        let stamp = self.subject.plain_stamp();
        self.subject
            .route_with_consumer_capabilities(self.handle.inner(), &stamp, consumer_capabilities)
            .await
            .map_err(|e| format!("opening a route with consumer capabilities: {e}"))
    }

    fn principals(&self) -> Result<ScopedPrincipals, String> {
        principals(self.subject)
    }

    async fn describe(&self, route: &S::Route) -> Result<Value, String> {
        expect_response(
            "role.describe",
            request(route, op_body(ops::ROLE_DESCRIBE, json!({}))).await?,
        )
    }

    async fn catalog_for(
        &self,
        route: &S::Route,
        arguments: Value,
    ) -> Result<CatalogAnswer, String> {
        let body = expect_response(
            "tool.catalog",
            request(route, op_body(ops::TOOL_CATALOG, arguments)).await?,
        )?;
        serde_json::from_value(body)
            .map_err(|e| format!("tool.catalog answer does not decode: {e}"))
    }

    fn remove_unadvertised_catalog_filter(&self, arguments: &mut Value) {
        if !self
            .capabilities
            .contains(&Capability::CatalogCapabilityFilter)
        {
            if let Some(params) = arguments.get_mut("params").and_then(Value::as_object_mut) {
                params.remove("capabilities");
            }
        }
    }

    async fn catalog_with(
        &self,
        route: &S::Route,
        digest_only: bool,
    ) -> Result<CatalogAnswer, String> {
        let mut arguments = self.subject.catalog_arguments();
        if digest_only {
            arguments["digest_only"] = json!(true);
        }
        self.remove_unadvertised_catalog_filter(&mut arguments);
        self.catalog_for(route, arguments).await
    }

    async fn catalog(&self, route: &S::Route) -> Result<CatalogAnswer, String> {
        self.catalog_with(route, false).await
    }

    async fn tool_result_v1_valid(&self) -> CaseResult {
        let legacy_route = self.plain_route().await?;
        let catalog = self.catalog(&legacy_route).await?;
        let calls = self.subject.tool_result_v1_calls();
        let tools: Vec<_> = catalog
            .tools
            .iter()
            .filter(|tool| tool.result_format == Some(ResultFormat::ToolResultV1))
            .collect();
        if tools.is_empty() {
            return Err("tool-result/v1 catalog declaration disappeared during the run".into());
        }
        let capabilities = vec![TOOL_RESULT_V1_CAPABILITY.to_owned()];
        let tool_result_v1_route = self.consumer_route(&capabilities).await?;
        let opted_in_catalog = self.catalog(&tool_result_v1_route).await?;
        for tool in tools {
            if !opted_in_catalog.tools.iter().any(|listed| {
                listed.name == tool.name && listed.result_format == Some(ResultFormat::ToolResultV1)
            }) {
                return Err(format!(
                    "{}: catalog did not list result_format tool-result/v1 on the declared route",
                    tool.name
                ));
            }
            let probes: Vec<_> = calls
                .iter()
                .filter(|(name, _)| name == &tool.name)
                .collect();
            if probes.is_empty() {
                return Err(format!(
                    "{}: no tool_result_v1_calls probe for declared tool-result/v1 tool",
                    tool.name
                ));
            }
            for (name, arguments) in probes {
                let call = call_body(name, arguments.clone(), None);
                let legacy_body =
                    expect_response(name, request(&legacy_route, call.clone()).await?)?;
                if serde_json::from_value::<ToolResult>(legacy_body).is_ok() {
                    return Err(format!(
                        "{name}: route without consumer declaration returned a body decodable as tool-result/v1"
                    ));
                }

                let body = expect_response(name, request(&tool_result_v1_route, call).await?)?;
                let bytes = serde_json::to_vec(&body)
                    .map_err(|e| format!("{name}: reply serialization: {e}"))?;
                if bytes.len() >= MAX_RESULT_BYTES {
                    return Err(format!(
                        "{name}: reply: serialized reply must be under 3 MiB"
                    ));
                }
                let result: ToolResult = serde_json::from_value(body)
                    .map_err(|e| format!("{name}: tool-result/v1 reply does not decode: {e}"))?;
                validate(&result).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        Ok(())
    }

    async fn held_reply_route_gating(&self, mint: &mut KeyMint) -> CaseResult {
        let principals = self.principals()?;
        let stamp = principals.stamp(&principals.carrier);
        let owner = self.owner_route().await?;
        // Expected decisions are independent of the provider's gating helper.
        for (declarations, expected_held) in [
            (None, false),
            (Some(vec![]), false),
            (Some(vec![TOOL_RESULT_V1_CAPABILITY.to_owned()]), false),
            (Some(vec![HELD_REPLY_V1.to_owned()]), false),
            (
                Some(vec![
                    TOOL_RESULT_V1_CAPABILITY.to_owned(),
                    HELD_REPLY_V1.to_owned(),
                ]),
                true,
            ),
        ] {
            let route = match declarations {
                None => self.route(&stamp).await?,
                Some(ref caps) => self
                    .subject
                    .route_with_consumer_capabilities(self.handle.inner(), &stamp, caps)
                    .await
                    .map_err(|e| format!("opening held-reply route: {e}"))?,
            };
            let key = mint.next("held-reply");
            let (name, arguments) = self
                .subject
                .held_reply_call(&self.marker(&key))
                .ok_or("no held_reply_call fixture supplied")?;
            let catalog = self.catalog(&route).await?;
            if !catalog.declares(session_capabilities::LATE_RESULTS) {
                return Err("held replies require the late_results catalog capability".into());
            }
            if !catalog.tools.iter().any(|tool| {
                tool.name == name && tool.result_format == Some(ResultFormat::ToolResultV1)
            }) {
                return Err(format!(
                    "{name}: held_reply_call tool must declare tool-result/v1"
                ));
            }
            let body = expect_response(
                &name,
                request(&route, call_body(&name, arguments, Some(&key))).await?,
            )?;
            if body.get("held").is_some() != expected_held {
                return Err(format!(
                    "{name}: held marker disagrees with route declarations {declarations:?}"
                ));
            }
            if expected_held
                || declarations
                    .as_ref()
                    .is_some_and(|caps| caps.iter().any(|cap| cap == TOOL_RESULT_V1_CAPABILITY))
            {
                if serde_json::to_vec(&body).map_err(|e| e.to_string())?.len() >= MAX_RESULT_BYTES {
                    return Err(format!(
                        "{name}: reply: serialized reply must be under 3 MiB"
                    ));
                }
                let result: ToolResult = serde_json::from_value(body).map_err(|e| {
                    format!("{name}: tool-result/v1 held reply does not decode: {e}")
                })?;
                validate(&result).map_err(|e| format!("{name}: {e}"))?;
            }
            self.subject
                .await_held(&key)
                .await
                .map_err(|e| e.to_string())?;
            if self.marker(&key).exists() {
                return Err(format!("{name}: held call ran before approval"));
            }
            self.subject
                .approve(&key)
                .await
                .map_err(|e| e.to_string())?;
            self.subject.settle().await;
            let late = pull(&owner, Value::Null).await?;
            if !late.entries.iter().any(|entry| {
                entry.call_key == key
                    && entry.custodian == principals.scope.owner
                    && entry.scope_ref == principals.scope.scope_ref
                    && entry.scope_epoch == principals.scope.scope_epoch
                    && entry.kind == kinds::RESULT
            }) {
                return Err(format!(
                    "{name}: no result late answer for approved held call {key}"
                ));
            }
        }
        Ok(())
    }

    async fn role_describe_shape(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let answer = self.describe(&route).await?;
        let describe =
            check_describe(&answer).map_err(|problems| format!("refused: {problems:?}"))?;
        let describe = describe
            .major(PROVIDES)
            .ok_or("check_describe accepted an answer without the v1 major")?;
        if self.capabilities.contains(&Capability::HeldCalls) && !describe.holds_calls() {
            return Err(format!(
                "the subject declares held_calls but role.describe does not list {}",
                ops::TOOL_WITHDRAW
            ));
        }
        if self.capabilities.contains(&Capability::UserCommands)
            && !describe.serves(ops::USER_COMMAND_INVOKE)
        {
            return Err(
                "catalog lists commands but role.describe omits user_command.invoke".into(),
            );
        }
        if self.capabilities.contains(&Capability::LateResults) {
            for op in [ops::LATE_RESULTS, ops::LATE_RESULTS_ACK] {
                if !describe.serves(op) {
                    return Err(format!(
                        "the subject declares late_results but role.describe does not list {op}"
                    ));
                }
            }
        }
        Ok(())
    }

    async fn role_describe_cacheable(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let first = self.describe(&route).await?;
        let second = self.describe(&route).await?;
        if first != second {
            return Err(format!("two answers differ: {first} then {second}"));
        }
        Ok(())
    }

    async fn catalog_schemas_flat(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        let (quick, _) = self.subject.quick_call();
        if !catalog.tools.iter().any(|tool| tool.name == quick) {
            return Err(format!(
                "the quick call's tool {quick} is not in the catalog"
            ));
        }
        for tool in &catalog.tools {
            if [
                ops::ROLE_DESCRIBE,
                ops::TOOL_CATALOG,
                ops::TOOL_WITHDRAW,
                ops::LATE_RESULTS,
                ops::LATE_RESULTS_ACK,
            ]
            .contains(&tool.name.as_str())
            {
                return Err(format!("role op {} is listed as a model tool", tool.name));
            }
            if tool.schema_digest.is_empty() {
                return Err(format!("{} has an empty schema_digest", tool.name));
            }
            check_flat_schema(&tool.input_schema)
                .map_err(|problem| format!("{} schema is not flat: {problem:?}", tool.name))?;
            for tag in &tool.capabilities {
                check_capability_tag(tag).map_err(|problem| {
                    format!("{} carries capability tag {tag:?}: {problem}", tool.name)
                })?;
            }
        }
        Ok(())
    }

    async fn catalog_reply_valid(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        for tool in &catalog.tools {
            check_reply(tool).map_err(|problem| format!("invalid_answer: {problem}"))?;
        }
        Ok(())
    }

    async fn catalog_modes_valid(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        for tool in &catalog.tools {
            check_modes(tool).map_err(|problem| format!("invalid_answer: {problem}"))?;
        }
        Ok(())
    }

    /// Every tool that carries `emits_events` declares a JSON boolean.
    ///
    /// This reads the raw answer rather than the decoded `CatalogTool`,
    /// because decoding turns an explicit `null` into absence, and a provider
    /// that writes `null` has declared nothing. The case checks only the
    /// declaration's shape: whether a tool declaring `false` really never
    /// makes the provider emit an event cannot be observed from outside the
    /// provider, so a pass says nothing about that promise.
    async fn catalog_emits_events_boolean(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let mut arguments = self.subject.catalog_arguments();
        self.remove_unadvertised_catalog_filter(&mut arguments);
        let body = expect_response(
            "tool.catalog",
            request(&route, op_body(ops::TOOL_CATALOG, arguments)).await?,
        )?;
        // An absent list is no tools, as the decoder treats it.
        let tools = match body.get("tools") {
            None => return Ok(()),
            Some(tools) => tools
                .as_array()
                .ok_or("tool.catalog answer's tools is not a list")?,
        };
        for tool in tools {
            let Some(declared) = tool.get("emits_events") else {
                continue;
            };
            if !declared.is_boolean() {
                let name = tool["name"].as_str().unwrap_or("<unnamed>");
                return Err(format!(
                    "invalid_answer: {name} emits_events: must be a boolean, got {declared}"
                ));
            }
        }
        Ok(())
    }

    async fn call_mode_matches_catalog(&self) -> CaseResult {
        let probe = self.subject.call_mode_probe().ok_or(
            "call_modes declared without a restricted tool probe; omit the capability to skip",
        )?;
        if matches!(probe.unlisted_mode, CallMode::Unknown(_)) {
            return Err("the probe must name a known unlisted mode".into());
        }
        let catalog = self.catalog(&self.plain_route().await?).await?;
        let (name, arguments) = probe.call;
        let tool = catalog
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| format!("the mode probe's tool {name} is not in the catalog"))?;
        check_modes(tool).map_err(|problem| format!("invalid_answer: {problem}"))?;
        let default = [CallMode::Classic];
        let modes = tool.modes.as_deref().unwrap_or(&default);
        if modes.contains(&probe.unlisted_mode) {
            return Err(format!(
                "the mode probe's tool {name} lists its supposed unlisted mode"
            ));
        }
        let served_mode = &modes[0];
        let stamp = self.subject.plain_stamp();
        let unlisted_route = self
            .subject
            .route_with_mode(self.handle.inner(), &stamp, &probe.unlisted_mode)
            .await
            .map_err(|e| format!("opening an unlisted-mode route: {e}"))?;
        // The same call body goes over both routes: the bind, not an argument,
        // must decide which mode the provider enforces.
        let body = call_body(&name, arguments, None);
        expect_error(
            "an unlisted-mode call",
            request(&unlisted_route, body.clone()).await?,
        )?;
        let served_route = self
            .subject
            .route_with_mode(self.handle.inner(), &stamp, served_mode)
            .await
            .map_err(|e| format!("opening a served-mode route: {e}"))?;
        let exchange = request(&served_route, body).await?;
        match single_terminal(&exchange).map_err(|p| p.to_string())? {
            ObservedFrame::Error(error) => Err(format!("the listed-mode call failed: {error:?}")),
            _ => Ok(()),
        }
    }

    async fn catalog_schema_digest_stable(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let pins = |catalog: &CatalogAnswer| -> BTreeMap<String, (String, u64)> {
            catalog
                .tools
                .iter()
                .map(|tool| {
                    (
                        tool.name.clone(),
                        (tool.schema_digest.clone(), tool.semantics),
                    )
                })
                .collect()
        };
        let first = self.catalog(&route).await?;
        let second = self.catalog(&route).await?;
        if first.tools.is_empty() {
            return Err("the catalog lists no tools".to_owned());
        }
        if first.generation.is_empty() || first.generation != second.generation {
            return Err(format!(
                "generation {:?} then {:?}",
                first.generation, second.generation
            ));
        }
        if pins(&first) != pins(&second) {
            return Err(format!(
                "two answers differ: {:?} then {:?}",
                pins(&first),
                pins(&second)
            ));
        }
        if first.catalog_digest.is_empty() || first.catalog_digest != second.catalog_digest {
            return Err(format!(
                "catalog_digest {:?} then {:?}",
                first.catalog_digest, second.catalog_digest
            ));
        }
        for tool in &first.tools {
            let computed =
                schema_digest(&tool.input_schema).map_err(|e| format!("{}: {e}", tool.name))?;
            if !is_schema_digest(&tool.schema_digest) || tool.schema_digest != computed {
                return Err(format!(
                    "{} carries schema_digest {:?}; the digest of its schema's structure is {computed}",
                    tool.name, tool.schema_digest
                ));
            }
        }
        Ok(())
    }

    async fn catalog_digest_only(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let full = self.catalog(&route).await?;
        let digest = self.catalog_with(&route, true).await?;
        if !digest.tools.is_empty() {
            return Err("a digest_only answer lists tools".to_owned());
        }
        if !digest.user_commands.is_empty() {
            return Err("a digest_only answer lists user commands".into());
        }
        if digest.generation != full.generation {
            return Err(format!(
                "digest_only generation {:?}, full answer {:?}",
                digest.generation, full.generation
            ));
        }
        if digest.catalog_digest.is_empty() || digest.catalog_digest != full.catalog_digest {
            return Err(format!(
                "digest_only catalog_digest {:?}, full answer {:?}",
                digest.catalog_digest, full.catalog_digest
            ));
        }
        Ok(())
    }

    async fn catalog_capability_filter_matches_describe(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let answer = self.describe(&route).await?;
        let describe = check_describe(&answer)
            .map_err(|problems| format!("role.describe refused: {problems:?}"))?;
        if !describe
            .capabilities
            .iter()
            .any(|capability| capability == CATALOG_CAPABILITY_FILTER)
        {
            return Err("role.describe no longer advertises catalog capability filtering".into());
        }
        let all_tags = select_tool_capabilities(&describe, &describe.tool_capabilities)
            .ok_or("the advertised catalog filter was not selected")?;
        if all_tags != describe.tool_capabilities {
            return Err(format!(
                "selecting all advertised tool capabilities returned {all_tags:?}; expected {:?}",
                describe.tool_capabilities
            ));
        }

        let all = self
            .catalog_for(
                &route,
                json!({ "params": params_for_capabilities(&all_tags) }),
            )
            .await?;
        let mut union: Vec<String> = all
            .tools
            .iter()
            .flat_map(|tool| tool.capabilities.iter().cloned())
            .collect();
        union.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        union.dedup();
        if union != describe.tool_capabilities {
            return Err(format!(
                "catalog capability union {union:?} does not match role.describe tool_capabilities {:?}",
                describe.tool_capabilities
            ));
        }
        if all.tools.iter().any(|tool| {
            !tool
                .capabilities
                .iter()
                .any(|tag| all_tags.iter().any(|requested| requested == tag))
        }) {
            return Err("the filtered catalog contains a tool with no requested capability".into());
        }

        let empty = self.catalog_for(&route, json!({ "params": {} })).await?;
        let explicitly_empty = self
            .catalog_for(
                &route,
                json!({ "params": params_for_capabilities(std::iter::empty::<&str>()) }),
            )
            .await?;
        if !empty.tools.is_empty() || !explicitly_empty.tools.is_empty() {
            return Err("a missing or empty capability list must select no tools".into());
        }
        if all.generation != empty.generation
            || all.generation != explicitly_empty.generation
            || empty.catalog_digest != explicitly_empty.catalog_digest
            || all.catalog_digest == empty.catalog_digest
        {
            return Err(format!(
                "filtering changed generation or failed to change the filtered digest: all=({}, {}), empty=({}, {}), explicitly empty=({}, {})",
                all.generation,
                all.catalog_digest,
                empty.generation,
                empty.catalog_digest,
                explicitly_empty.generation,
                explicitly_empty.catalog_digest
            ));
        }

        if let Some(first_tag) = all_tags.first() {
            let single = self
                .catalog_for(
                    &route,
                    json!({ "params": params_for_capabilities([first_tag]) }),
                )
                .await?;
            if single.generation != all.generation {
                return Err(format!(
                    "generation changed with the filter: all {:?}, single {:?}",
                    all.generation, single.generation
                ));
            }
            let all_names: std::collections::BTreeSet<_> =
                all.tools.iter().map(|tool| &tool.name).collect();
            let single_names: std::collections::BTreeSet<_> =
                single.tools.iter().map(|tool| &tool.name).collect();
            if all_names != single_names && all.catalog_digest == single.catalog_digest {
                return Err(
                    "catalog_digest did not change when filtering changed the tools".into(),
                );
            }
        }
        Ok(())
    }

    async fn catalog_unknown_preset_refused(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let mut arguments = self.subject.catalog_arguments();
        self.remove_unadvertised_catalog_filter(&mut arguments);
        arguments[PRESET_FIELD] = json!(UNDEFINED_PRESET);
        let error = expect_error(
            &format!("tool.catalog with preset {UNDEFINED_PRESET:?}"),
            request(&route, op_body(ops::TOOL_CATALOG, arguments)).await?,
        )?;
        expect_invalid_field(&error, PRESET_FIELD)
    }

    /// The subject's catalog arguments without `preset`, `digest_only` or
    /// `system_text`: the request whose answer lists the served presets.
    fn preset_less_arguments(&self) -> Value {
        let mut arguments = self.subject.catalog_arguments();
        self.remove_unadvertised_catalog_filter(&mut arguments);
        if let Some(members) = arguments.as_object_mut() {
            for member in [PRESET_FIELD, "digest_only", "system_text"] {
                members.remove(member);
            }
        }
        arguments
    }

    async fn preset_less_catalog(&self, route: &S::Route) -> Result<CatalogAnswer, String> {
        self.catalog_for(route, self.preset_less_arguments()).await
    }

    async fn catalog_listed_presets_served(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let listing = self.preset_less_catalog(&route).await?;
        check_presets(&listing, None).map_err(|error| {
            format!(
                "the preset-less catalog's presets list is malformed: {} {:?}",
                error.message, error.detail
            )
        })?;
        if listing.presets.is_empty() {
            return Err("the preset-less catalog no longer lists presets".into());
        }
        for preset in &listing.presets {
            let mut arguments = self.preset_less_arguments();
            arguments[PRESET_FIELD] = json!(preset);
            let what = format!("tool.catalog with listed preset {preset:?}");
            let body = expect_response(
                &what,
                request(&route, op_body(ops::TOOL_CATALOG, arguments)).await?,
            )?;
            let answer: CatalogAnswer = serde_json::from_value(body)
                .map_err(|e| format!("{what}: the answer does not decode: {e}"))?;
            check_presets(&answer, Some(preset))
                .map_err(|error| format!("{what}: {}", error.message))?;
        }
        Ok(())
    }

    fn system_text_arguments(&self) -> Result<Value, String> {
        let mut arguments = self
            .subject
            .system_text_catalog_arguments()
            .ok_or("declares system_text but supplies no system-text catalog arguments")?;
        self.remove_unadvertised_catalog_filter(&mut arguments);
        if !arguments["system_text"].is_object() {
            return Err(format!(
                "request system_text received {}; expected {{preset, params}}",
                arguments["system_text"]
            ));
        }
        if arguments["digest_only"] == json!(true) {
            return Err(
                "system-text catalog arguments request digest_only, not a full answer".to_owned(),
            );
        }
        Ok(arguments)
    }

    async fn system_text_catalog(
        &self,
        route: &S::Route,
        arguments: Value,
    ) -> Result<Value, String> {
        expect_response(
            "tool.catalog with system_text",
            request(route, op_body(ops::TOOL_CATALOG, arguments)).await?,
        )
    }

    async fn system_text_digests_match_text(&self) -> CaseResult {
        let arguments = self.system_text_arguments()?;
        let route = self.plain_route().await?;
        let answer = self.system_text_catalog(&route, arguments).await?;
        check_system_text_item(&answer)
    }

    /// The subject's system-text request with the top-level `preset` and
    /// the item's own `preset` removed: a request for the provider's default
    /// catalog variant and its default system text.
    fn preset_less_system_text_arguments(&self) -> Result<Value, String> {
        let mut arguments = self.system_text_arguments()?;
        if let Some(members) = arguments.as_object_mut() {
            members.remove(PRESET_FIELD);
        }
        if let Some(item) = arguments["system_text"].as_object_mut() {
            item.remove(PRESET_FIELD);
        }
        Ok(arguments)
    }

    /// Fetch system text with `arguments`, requiring an answer rather than a
    /// refusal, and a well-formed system-text item in it.
    async fn served_system_text(
        &self,
        route: &S::Route,
        what: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let exchange = request(route, op_body(ops::TOOL_CATALOG, arguments)).await?;
        let answer = match single_terminal(&exchange).map_err(|p| format!("{what}: {p}"))? {
            ObservedFrame::Response(body) => body.clone(),
            ObservedFrame::Error(error) => {
                return Err(format!(
                    "{what} was refused as {} ({}) {:?}; a provider reading system_text.preset \
                     as optional answers with its default system text",
                    error.code, error.message, error.detail
                ))
            }
            other => return Err(format!("{what} ended with {other:?}, not a response")),
        };
        check_system_text_item(&answer).map_err(|reason| format!("{what}: {reason}"))?;
        Ok(answer)
    }

    async fn system_text_preset_optional_served(&self) -> CaseResult {
        let arguments = self.preset_less_system_text_arguments()?;
        let route = self.plain_route().await?;
        let what = "tool.catalog with a system_text item without a preset";
        let first = self
            .served_system_text(&route, what, arguments.clone())
            .await?;
        let second = self
            .served_system_text(&route, what, arguments.clone())
            .await?;
        let digest = &first["system_text"]["item_digest"];
        let repeated = &second["system_text"]["item_digest"];
        if repeated != digest {
            return Err(format!(
                "{what}: a repeat received system_text.item_digest {repeated}; expected digest \
                 {digest} from the identical first request"
            ));
        }
        let Some(default) = self.subject.system_text_default_preset() else {
            return Ok(());
        };
        let mut named = arguments;
        named["system_text"][PRESET_FIELD] = json!(default);
        let named_what = format!("tool.catalog with system_text preset {default:?}");
        let named = self.served_system_text(&route, &named_what, named).await?;
        for field in ["text", "item_digest"] {
            let expected = &named["system_text"][field];
            let received = &first["system_text"][field];
            if received != expected {
                return Err(format!(
                    "{what}: system_text.{field} received {received}; expected {expected}, the \
                     answer for the default preset {default:?}"
                ));
            }
        }
        Ok(())
    }

    async fn system_text_preflight_digest_stable(&self) -> CaseResult {
        let arguments = self.system_text_arguments()?;
        let route = self.plain_route().await?;
        let first = self.system_text_catalog(&route, arguments.clone()).await?;
        let second = self.system_text_catalog(&route, arguments).await?;
        for field in ["item_digest", "preflight_digest"] {
            let expected = &first["system_text"][field];
            if !expected.as_str().is_some_and(is_schema_digest) {
                return Err(format!(
                    "first system_text.{field} received {expected}; expected a digest as 64 lowercase hex characters"
                ));
            }
            let received = &second["system_text"][field];
            if received != expected {
                return Err(format!(
                    "second system_text.{field} received {received}; expected digest {expected} \
                     from the identical first request"
                ));
            }
        }
        Ok(())
    }

    fn disabled_tool(&self) -> Result<String, String> {
        self.subject
            .disabled_tool()
            .ok_or_else(|| "declares disable_tool but names no disabled tool".to_owned())
    }

    async fn catalog_disabled_tool_absent(&self) -> CaseResult {
        let disabled = self.disabled_tool()?;
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        if catalog.tools.iter().any(|tool| tool.name == disabled) {
            return Err(format!("disabled tool {disabled} is in the catalog"));
        }
        let (quick, _) = self.subject.quick_call();
        if !catalog.tools.iter().any(|tool| tool.name == quick) {
            return Err(format!(
                "the quick call's tool {quick} is not in the catalog either, so the absence proves nothing"
            ));
        }
        Ok(())
    }

    async fn call_disabled_tool_refused(&self) -> CaseResult {
        let disabled = self.disabled_tool()?;
        let route = self.plain_route().await?;
        let error = expect_error(
            "a call to the disabled tool",
            request(&route, call_body(&disabled, json!({}), None)).await?,
        )?;
        match errors::disabled_tool(&error) {
            Some(tool) if tool == disabled => Ok(()),
            _ => Err(format!(
                "refused as {} {:?}, not {} {{tool: {disabled}}}",
                error.code,
                error.detail,
                errors::TOOL_DISABLED
            )),
        }
    }

    async fn terminal_frame_on_success(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let (name, arguments) = self.subject.quick_call();
        let exchange = request(&route, call_body(&name, arguments, None)).await?;
        match single_terminal(&exchange).map_err(|p| p.to_string())? {
            ObservedFrame::Error(error) => Err(format!("the quick call failed: {error:?}")),
            _ => Ok(()),
        }
    }

    async fn terminal_frame_on_refusal(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let error = expect_error(
            "a call to an unserved tool",
            request(&route, call_body(UNSERVED_TOOL, json!({}), None)).await?,
        )?;
        expect_code(&error, errors::UNKNOWN_TOOL)
    }

    async fn terminal_frame_on_cancel(&self) -> CaseResult {
        let (name, arguments) = self
            .subject
            .slow_call()
            .ok_or("declares cancellation but supplies no slow call")?;
        let route = self.plain_route().await?;
        let exchange = route
            .request_then_cancel(call_body(&name, arguments, None))
            .await
            .map_err(|e| format!("route failed: {}", e.message))?;
        single_terminal(&exchange)
            .map(drop)
            .map_err(|problem| format!("the cancelled call got {problem}"))
    }

    async fn call_key_malformed_refused(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let (name, arguments) = self.subject.quick_call();
        for key in vector_strings(CALL_KEY_VECTORS, "invalid", "key") {
            let error = expect_error(
                &format!("call_key {key:?}"),
                request(&route, call_body(&name, arguments.clone(), Some(&key))).await?,
            )?;
            expect_invalid_field(&error, CALL_KEY_FIELD)
                .map_err(|e| format!("call_key {key:?}: {e}"))?;
        }
        Ok(())
    }

    async fn call_key_well_formed_accepted(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let (name, arguments) = self.subject.quick_call();
        for key in vector_strings(CALL_KEY_VECTORS, "valid", "key") {
            let exchange = request(&route, call_body(&name, arguments.clone(), Some(&key))).await?;
            if let ObservedFrame::Error(error) = single_terminal(&exchange)
                .map_err(|problem| format!("call_key {key:?}: {problem}"))?
            {
                return Err(format!("call_key {key:?} was refused: {error:?}"));
            }
        }
        Ok(())
    }

    /// The quick call's body, pinned with `pin` as a raw string.
    fn pinned_quick_call(&self, pin: &str) -> Value {
        let (name, arguments) = self.subject.quick_call();
        let mut body = call_body(&name, arguments, None);
        body[SCHEMA_PIN_FIELD] = json!(pin);
        body
    }

    async fn schema_pin_current_accepted(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        let (name, _) = self.subject.quick_call();
        let tool = catalog
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| format!("the quick call's tool {name} is not in the catalog"))?;
        let pin = SchemaPin::new(&name, &tool.schema_digest, tool.semantics)
            .encode()
            .map_err(|e| format!("the current pin cannot be encoded: {e}"))?;
        let exchange = request(&route, self.pinned_quick_call(&pin)).await?;
        match single_terminal(&exchange).map_err(|p| p.to_string())? {
            ObservedFrame::Error(error) => Err(format!("pin {pin} was refused: {error:?}")),
            _ => Ok(()),
        }
    }

    async fn schema_pin_malformed_refused(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        let mut pins = vector_strings(SCHEMA_PIN_VECTORS, "refused", "pin");
        let (quick, _) = self.subject.quick_call();
        let digest = catalog
            .tools
            .iter()
            .find(|tool| tool.name == quick)
            .map(|tool| tool.schema_digest.clone())
            .ok_or_else(|| format!("the quick call's tool {quick} is not in the catalog"))?;
        let other = SchemaPin::new(UNSERVED_TOOL, digest, 0)
            .encode()
            .map_err(|e| e.to_string())?;
        pins.push(other);
        for pin in pins {
            let error = expect_error(
                &format!("schema_pin {pin:?}"),
                request(&route, self.pinned_quick_call(&pin)).await?,
            )?;
            expect_invalid_field(&error, SCHEMA_PIN_FIELD)
                .map_err(|e| format!("schema_pin {pin:?}: {e}"))?;
        }
        Ok(())
    }

    async fn withdraw_unknown_call(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let route = self.route(&principals.stamp(&principals.carrier)).await?;
        let answer = decode_answer(withdraw(&route, json!({ "call_key": key })).await?)?;
        match answer {
            WithdrawAnswer::UnknownCall => Ok(()),
            other => Err(format!(
                "a key never held answered {other:?}, not unknown_call"
            )),
        }
    }

    fn marker(&self, key: &str) -> PathBuf {
        marker_path(self.work_dir, key)
    }

    /// Hold a call raised on `raiser`, run `work` while the provider holds
    /// it, then withdraw it on `raiser` so a provider whose held request only
    /// ends on withdrawal still ends it.
    async fn while_held<'w>(
        &'w self,
        raiser: &'w S::Route,
        key: &'w str,
        work: impl Future<Output = CaseResult> + 'w,
    ) -> CaseResult {
        let (name, arguments) = self
            .subject
            .held_call(&self.marker(key))
            .ok_or("declares held_calls but supplies no held call")?;
        let body = call_body(&name, arguments, Some(key));
        let during = async {
            self.subject
                .await_held(key)
                .await
                .map_err(|e| format!("the provider never held the call: {e}"))?;
            let result = work.await;
            let _ = withdraw(raiser, json!({ "call_key": key })).await;
            result
        };
        let (sent, result) = join(raiser.request(body), during).await;
        result?;
        let sent = sent.map_err(|e| format!("the held call's route failed: {}", e.message))?;
        single_terminal(&sent)
            .map(drop)
            .map_err(|problem| format!("the held call's own request got {problem}"))
    }

    async fn withdraw_identical(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        let owner = self
            .route(&principals.stamp(&principals.scope.owner))
            .await?;
        self.while_held(&carrier, &key, async {
            let first = expect_response(
                "the carrier's withdraw",
                withdraw(&carrier, json!({ "call_key": key })).await?,
            )?;
            let answer = WithdrawAnswer::decode(&first).map_err(|e| e.to_string())?;
            if answer != WithdrawAnswer::Withdrawn {
                return Err(format!(
                    "a held, unapproved call answered {answer:?}, not withdrawn"
                ));
            }
            let repeat = expect_response(
                "the carrier's repeat",
                withdraw(&carrier, json!({ "call_key": key })).await?,
            )?;
            if repeat != first {
                return Err(format!("the repeat answered {repeat}, the first {first}"));
            }
            let by_owner = expect_response(
                "the owner's withdraw",
                withdraw(
                    &owner,
                    json!({ "call_key": key, "carrier": principals.carrier }),
                )
                .await?,
            )?;
            if by_owner != first {
                return Err(format!("the owner got {by_owner}, the carrier {first}"));
            }
            Ok(())
        })
        .await?;
        self.subject.settle().await;
        if self.marker(&key).exists() {
            return Err("the withdrawn call's action ran".to_owned());
        }
        Ok(())
    }

    async fn withdraw_owner_as_carrier(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let owner = self
            .route(&principals.stamp(&principals.scope.owner))
            .await?;
        self.while_held(&owner, &key, async {
            let answer = decode_answer(withdraw(&owner, json!({ "call_key": key })).await?)?;
            if answer != WithdrawAnswer::Withdrawn {
                return Err(format!(
                    "the owner withdrawing its own held call answered {answer:?}, not withdrawn"
                ));
            }
            Ok(())
        })
        .await
    }

    async fn withdraw_owner_without_carrier(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        let owner = self
            .route(&principals.stamp(&principals.scope.owner))
            .await?;
        self.while_held(&carrier, &key, async {
            let error = expect_error(
                "the owner's withdraw without carrier",
                withdraw(&owner, json!({ "call_key": key })).await?,
            )?;
            expect_code(&error, errors::WITHDRAW_CARRIER_REQUIRED)
        })
        .await
    }

    async fn withdraw_carrier_naming_other(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        self.while_held(&carrier, &key, async {
            let error = expect_error(
                "the carrier's withdraw naming another carrier",
                withdraw(
                    &carrier,
                    json!({ "call_key": key, "carrier": principals.other_carrier }),
                )
                .await?,
            )?;
            expect_code(&error, errors::WITHDRAW_CARRIER_MISMATCH)
        })
        .await
    }

    async fn withdraw_scope_mismatch(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        let mut other_scope = principals.scope.clone();
        other_scope.scope_epoch += 1;
        self.while_held(&carrier, &key, async {
            let error = expect_error(
                "a withdraw naming another scope",
                withdraw(&carrier, json!({ "call_key": key, "scope": other_scope })).await?,
            )?;
            expect_code(&error, errors::WITHDRAW_SCOPE_MISMATCH)
        })
        .await
    }

    async fn withdraw_top_level_call_key(&self, key: String, own_key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        self.while_held(&carrier, &key, async {
            let body = json!({
                "name": ops::TOOL_WITHDRAW,
                "arguments": { "call_key": key },
                "call_key": own_key,
            });
            let error = expect_error(
                "a withdraw with its own call_key",
                request(&carrier, body).await?,
            )?;
            expect_invalid_field(&error, CALL_KEY_FIELD)
        })
        .await
    }

    async fn withdraw_malformed_target(&self) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        for key in vector_strings(CALL_KEY_VECTORS, "invalid", "key") {
            let error = expect_error(
                &format!("a withdraw of {key:?}"),
                withdraw(&carrier, json!({ "call_key": key })).await?,
            )?;
            expect_invalid_field(&error, TARGET_CALL_KEY_FIELD)
                .map_err(|e| format!("arguments.call_key {key:?}: {e}"))?;
        }
        Ok(())
    }

    async fn withdraw_not_permitted(&self, key: String) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        let elsewhere = self
            .route(&principals.stamp_in_other_scope(&principals.carrier))
            .await?;
        self.while_held(&carrier, &key, async {
            let error = expect_error(
                "the carrier's withdraw from another scope",
                withdraw(&elsewhere, json!({ "call_key": key })).await?,
            )?;
            expect_code(&error, errors::WITHDRAW_NOT_PERMITTED)
        })
        .await
    }

    /// Raise a held call as the carrier, approve it, and let it settle, so
    /// the provider has a late result for `key`.
    async fn settle_a_call(&self, key: &str) -> CaseResult {
        let principals = self.principals()?;
        let carrier = self.route(&principals.stamp(&principals.carrier)).await?;
        let (name, arguments) = self
            .subject
            .held_call(&self.marker(key))
            .ok_or("declares held_calls but supplies no held call")?;
        let (sent, approved) = join(
            carrier.request(call_body(&name, arguments, Some(key))),
            self.subject.approve(key),
        )
        .await;
        approved.map_err(|e| format!("approving {key}: {e}"))?;
        let sent = sent.map_err(|e| format!("the held call's route failed: {}", e.message))?;
        single_terminal(&sent).map_err(|p| format!("the held call got {p}"))?;
        self.subject.settle().await;
        Ok(())
    }

    async fn owner_route(&self) -> Result<S::Route, String> {
        let principals = self.principals()?;
        self.route(&principals.stamp(&principals.scope.owner)).await
    }

    async fn late_results_round_trip(&self, key: String) -> CaseResult {
        self.settle_a_call(&key).await?;
        let owner = self.owner_route().await?;
        let first = pull(&owner, Value::Null).await?;
        let entry = first
            .entries
            .iter()
            .find(|entry| entry.call_key == key)
            .ok_or_else(|| format!("no late result for the settled call {key}"))?;
        if entry.kind != kinds::RESULT {
            return Err(format!(
                "the settled call's entry is {:?}, not result",
                entry.kind
            ));
        }
        if entry.custodian != self.principals()?.scope.owner {
            return Err(format!("the entry's custodian is {}", entry.custodian));
        }
        let since = serde_json::to_value(&first.cursor).expect("cursor serializes");
        let next = pull(&owner, since.clone()).await?;
        let seen: BTreeSet<&str> = first.entries.iter().map(|e| e.event_id.as_str()).collect();
        if let Some(again) = next
            .entries
            .iter()
            .find(|e| seen.contains(e.event_id.as_str()))
        {
            return Err(format!(
                "reading on from the cursor returned {} again",
                again.event_id
            ));
        }
        let repeat = pull(&owner, since).await?;
        if repeat != next {
            return Err("reading twice from one cursor gave different replies".to_owned());
        }
        Ok(())
    }

    async fn late_results_ack(&self, key: String) -> CaseResult {
        self.settle_a_call(&key).await?;
        let owner = self.owner_route().await?;
        let first = pull(&owner, Value::Null).await?;
        let event = first
            .entries
            .iter()
            .find(|entry| entry.call_key == key)
            .map(|entry| entry.event_id.clone())
            .ok_or_else(|| format!("no late result for the settled call {key}"))?;
        let through = serde_json::to_value(&first.cursor).expect("cursor serializes");
        expect_response(
            "late_results.ack",
            request(
                &owner,
                op_body(ops::LATE_RESULTS_ACK, json!({ "through": through })),
            )
            .await?,
        )?;
        let after = pull(&owner, Value::Null).await?;
        if after.entries.iter().any(|entry| entry.event_id == event) {
            return Err(format!("the acked entry {event} is still served"));
        }
        Ok(())
    }

    async fn late_results_incarnation_change(&self) -> CaseResult {
        let route = self.plain_route().await?;
        let catalog = self.catalog(&route).await?;
        if !catalog.declares(session_capabilities::LATE_RESULTS) {
            return Err("the catalog does not declare late_results".to_owned());
        }
        let owner = self.owner_route().await?;
        let cursor = pull(&owner, Value::Null).await?.cursor;
        let foreign = json!({
            "provider_incarnation": format!("{}-another", cursor.provider_incarnation),
            "seq": cursor.seq,
        });
        let past = json!({
            "provider_incarnation": cursor.provider_incarnation,
            "seq": cursor.seq + 1_000_000,
        });
        for (what, since) in [("another incarnation", foreign), ("past the log", past)] {
            let error = expect_error(
                &format!("a cursor from {what}"),
                request(
                    &owner,
                    op_body(ops::LATE_RESULTS, json!({ "since": since })),
                )
                .await?,
            )?;
            expect_code(&error, CURSOR_INCARNATION_CHANGED)
                .map_err(|e| format!("a cursor from {what}: {e}"))?;
        }
        Ok(())
    }
}

fn principals<S: ToolProviderSubject>(subject: &S) -> Result<ScopedPrincipals, String> {
    subject
        .scoped_principals()
        .ok_or_else(|| "declares scope_stamp but supplies no scoped principals".to_owned())
}

fn marker_path(work_dir: &Path, key: &str) -> PathBuf {
    work_dir
        .join("markers")
        .join(format!("{}.marker", key.replace('/', "_")))
}

/// Kill the provider at `point` while it holds a call, restart it on the same
/// root, and check that it never runs the call, that it reports the call to
/// the owner as a `not_started` late result, and that its withdraw answer
/// guarantees the call will never run.
async fn crash_case<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    mint: &mut KeyMint,
    point: &str,
) -> CaseResult
where
    S: ToolProviderSubject,
    S::Route: ToolRoute,
{
    let principals = principals(subject)?;
    let carrier_stamp = principals.stamp(&principals.carrier);
    let owner_stamp = principals.stamp(&principals.scope.owner);
    let root = work_dir.join(format!("crash-{point}"));
    std::fs::create_dir_all(&root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let key = mint.next(&format!("crash-{point}"));
    let marker = marker_path(work_dir, &key);
    let (name, arguments) = subject
        .held_call(&marker)
        .ok_or("declares held_calls but supplies no held call")?;
    let body = call_body(&name, arguments, Some(&key));

    let handle = driver
        .spawn(&root)
        .await
        .map_err(|e| format!("spawning: {e}"))?;
    let route = driver
        .route(&handle, &carrier_stamp)
        .await
        .map_err(|e| format!("opening the carrier's route: {e}"))?;
    let trigger: Trigger<'_> = if point == points::AUTHORIZED {
        Box::pin(async {
            let _ = join(route.request(body), subject.approve(&key)).await;
        })
    } else {
        Box::pin(async {
            let _ = route.request(body).await;
        })
    };
    driver
        .kill_at(handle, &KillPoint::new(point), trigger)
        .await
        .map_err(|e| format!("killing at {point}: {e}"))?;
    drop(route);

    let handle = driver
        .restart(&root)
        .await
        .map_err(|e| format!("restarting after {point}: {e}"))?;
    subject.settle().await;
    if marker.exists() {
        return Err(format!(
            "the provider ran the call after restarting from {point}"
        ));
    }

    let owner = driver
        .route(&handle, &owner_stamp)
        .await
        .map_err(|e| format!("opening the owner's route after restart: {e}"))?;
    let entries = pull(&owner, Value::Null).await?.entries;
    let mine: Vec<_> = entries.iter().filter(|e| e.call_key == key).collect();
    let not_started = mine
        .iter()
        .find(|e| e.kind == kinds::NOT_STARTED)
        .ok_or_else(|| {
            format!(
                "after restarting from {point} the owner's late results hold no not_started \
                 entry for the call (its entries: {:?})",
                mine.iter().map(|e| &e.kind).collect::<Vec<_>>()
            )
        })?;
    if not_started.reason.as_deref().is_none_or(str::is_empty) {
        return Err("the not_started entry carries no reason".to_owned());
    }
    if let Some(result) = mine.iter().find(|e| e.kind == kinds::RESULT) {
        return Err(format!(
            "the call cut at {point} also has a result entry: {result:?}"
        ));
    }

    let carrier = driver
        .route(&handle, &carrier_stamp)
        .await
        .map_err(|e| format!("opening the carrier's route after restart: {e}"))?;
    let answer = decode_answer(withdraw(&carrier, json!({ "call_key": key })).await?)?;
    if !answer.guarantees_not_run() {
        return Err(format!(
            "after restarting from {point} the withdraw answered {answer:?}; a call cut there \
             was never started, so the answer must be withdrawn or a refusal"
        ));
    }
    subject.settle().await;
    if marker.exists() {
        return Err(format!(
            "the provider ran the call after restarting from {point} and answering {answer:?}"
        ));
    }
    Ok(())
}

fn op_body(op: &str, arguments: Value) -> Value {
    json!({ "name": op, "arguments": arguments })
}

/// A call body built as raw JSON, so the runner can send keys a typed
/// constructor would refuse.
fn call_body(name: &str, arguments: Value, call_key: Option<&str>) -> Value {
    let mut body = json!({ "name": name, "arguments": arguments });
    if let Some(key) = call_key {
        body[CALL_KEY_FIELD] = json!(key);
    }
    body
}

async fn request<R: ToolRoute>(route: &R, body: Value) -> Result<Exchange, String> {
    route
        .request(body)
        .await
        .map_err(|e| format!("route failed: {}", e.message))
}

async fn withdraw<R: ToolRoute>(route: &R, arguments: Value) -> Result<Exchange, String> {
    request(route, op_body(ops::TOOL_WITHDRAW, arguments)).await
}

async fn pull<R: ToolRoute>(route: &R, since: Value) -> Result<LateResultsReply, String> {
    let body = expect_response(
        "late_results",
        request(route, op_body(ops::LATE_RESULTS, json!({ "since": since }))).await?,
    )?;
    serde_json::from_value(body.clone())
        .map_err(|e| format!("late_results reply {body} does not decode: {e}"))
}

fn expect_response(what: &str, exchange: Exchange) -> Result<Value, String> {
    match single_terminal(&exchange).map_err(|problem| format!("{what}: {problem}"))? {
        ObservedFrame::Response(body) => Ok(body.clone()),
        other => Err(format!("{what} ended with {other:?}, not a response")),
    }
}

fn expect_error(what: &str, exchange: Exchange) -> Result<ErrorBody, String> {
    match single_terminal(&exchange).map_err(|problem| format!("{what}: {problem}"))? {
        ObservedFrame::Error(error) => Ok(error.clone()),
        other => Err(format!("{what} ended with {other:?}, not an error frame")),
    }
}

fn expect_code(error: &ErrorBody, code: &str) -> CaseResult {
    if error.code == code {
        Ok(())
    } else {
        Err(format!(
            "refused as {} {:?}, not {code}",
            error.code, error.detail
        ))
    }
}

fn expect_invalid_field(error: &ErrorBody, field: &str) -> CaseResult {
    if errors::invalid_request_field(error) == Some(field) {
        Ok(())
    } else {
        Err(format!(
            "refused as {} {:?}, not {} {{field: \"{field}\"}}",
            error.code,
            error.detail,
            errors::INVALID_REQUEST
        ))
    }
}

/// Check the `system_text` item of a full catalog answer: its text, an
/// `item_digest` that hashes that text, a provider-defined
/// `preflight_digest`, and sorted, deduplicated `tool_names`.
fn check_system_text_item(answer: &Value) -> CaseResult {
    // Inspect the raw fields so even a missing digest reports its received
    // value and the digest expected from the returned text.
    let item = &answer["system_text"];
    if !item.is_object() {
        return Err(format!(
            "system_text received {item}; expected a system-text answer"
        ));
    }
    let text = item["text"].as_str().ok_or_else(|| {
        format!(
            "system_text.text received {}; expected the resolved text as a string",
            item["text"]
        )
    })?;
    let expected = system_text_digest(text);
    let received = &item["item_digest"];
    if !received
        .as_str()
        .is_some_and(|digest| is_schema_digest(digest) && digest == expected)
    {
        return Err(format!(
            "system_text.item_digest received {received}; expected digest {expected} \
             (SHA-256 of the exact UTF-8 text bytes, as 64 lowercase hex characters)"
        ));
    }
    let preflight = &item["preflight_digest"];
    if !preflight.as_str().is_some_and(is_schema_digest) {
        return Err(format!(
            "system_text.preflight_digest received {preflight}; expected a provider-defined \
             digest as 64 lowercase hex characters (not necessarily {expected}, the text digest)"
        ));
    }
    let names: Vec<String> = serde_json::from_value(item["tool_names"].clone()).map_err(|e| {
        format!(
            "system_text.tool_names received {}; expected a sorted, deduplicated list of tool names: {e}",
            item["tool_names"]
        )
    })?;
    let mut ordered = names.clone();
    ordered.sort();
    ordered.dedup();
    if names != ordered {
        return Err(format!(
            "system_text.tool_names received {names:?}; expected sorted, deduplicated {ordered:?}"
        ));
    }
    Ok(())
}

fn decode_answer(exchange: Exchange) -> Result<WithdrawAnswer, String> {
    let body = expect_response("tool.withdraw", exchange)?;
    WithdrawAnswer::decode(&body).map_err(|e| format!("tool.withdraw reply {body}: {e}"))
}

fn vector_strings(vectors: &str, set: &str, member: &str) -> Vec<String> {
    let vectors: Value = serde_json::from_str(vectors).expect("vectors parse");
    vectors[set]
        .as_array()
        .expect("vector set")
        .iter()
        .map(|case| case[member].as_str().expect("vector member").to_owned())
        .collect()
}

impl<S> Case<'_, S>
where
    S: ToolProviderSubject,
    S::Route: ToolRoute,
{
    async fn user_commands(&self, name: &str, key: String) -> CaseResult {
        let catalog = self.catalog(&self.plain_route().await?).await?;
        if name == "user_commands_catalog" {
            return cortexkit_role_tool_provider::user_command::check_catalog(&catalog)
                .map_err(|e| e.message);
        }
        if catalog.user_commands.is_empty() {
            return Err("catalog no longer lists commands".into());
        }
        for (index, command) in catalog.user_commands.iter().enumerate() {
            self.user_command_case(name, format!("{key}/{index}"), &catalog, command)
                .await?;
        }
        Ok(())
    }

    async fn user_command_case(
        &self,
        name: &str,
        key: String,
        catalog: &CatalogAnswer,
        command: &cortexkit_role_tool_provider::user_command::UserCommand,
    ) -> CaseResult {
        use crate::subject::UserCommandScope;
        use cortexkit_role_tool_provider::user_command::UserCommandReply;
        let scope = match name {
            "user_commands_codemode_refused" => UserCommandScope::Codemode,
            "user_commands_flow_refused" => UserCommandScope::Flow,
            _ => UserCommandScope::Session,
        };
        let route = self
            .subject
            .user_command_route(self.handle.inner(), scope)
            .await
            .map_err(|e| e.to_string())?;
        let body = json!({"name": ops::USER_COMMAND_INVOKE, "arguments": {"command": command.name, "input": self.subject.user_command_input(&command.name)}, "call_key": key});
        let (exchange, first_bytes) =
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                if name == "user_commands_repeat" {
                    route
                        .request_with_response_bytes(body.clone())
                        .await
                        .map(|(exchange, bytes)| (exchange, Some(bytes)))
                        .map_err(|error| error.message)
                } else {
                    request(&route, body.clone())
                        .await
                        .map(|exchange| (exchange, None))
                }
            })
            .await
            .map_err(|_| "user command exceeded five seconds")??;
        if scope != UserCommandScope::Session {
            return expect_code(
                &expect_error("user_command.invoke", exchange)?,
                errors::USER_COMMAND_NOT_ADMITTED,
            );
        }
        let first = expect_response("user_command.invoke", exchange)?;
        let reply: UserCommandReply =
            serde_json::from_value(first.clone()).map_err(|e| format!("command reply: {e}"))?;
        reply.check().map_err(|e| e.message)?;
        if !["done", "failed", "accepted"].contains(&reply.state.as_str()) {
            return Err(format!(
                "provider emitted unknown reply state {}",
                reply.state
            ));
        }
        if name == "user_commands_repeat" {
            let (exchange, repeated_bytes) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                route.request_with_response_bytes(body),
            )
            .await
            .map_err(|_| "repeated user command exceeded five seconds")?
            .map_err(|error| error.message)?;
            let repeated = expect_response("repeated user_command.invoke", exchange)?;
            let bytes = first_bytes.ok_or("missing first reply bytes")?;
            let raw: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            let repeated_raw: Value =
                serde_json::from_slice(&repeated_bytes).map_err(|error| error.to_string())?;
            if raw != first || repeated_raw != repeated {
                return Err("raw response bytes do not match decoded responses".into());
            }
            if bytes != repeated_bytes || first != repeated {
                return Err("repeat changed the stored user-command reply".into());
            }
        }
        if name != "user_commands_accepted_late" || reply.state != "accepted" {
            return Ok(());
        }
        if catalog.capabilities.get("late_results") != Some(&json!(true)) {
            return Err("accepted without late_results".into());
        }
        let mut since = None;
        let mut terminal = false;
        for _ in 0..15 {
            let args = match &since {
                Some(cursor) => json!({"since": cursor}),
                None => json!({}),
            };
            let late: LateResultsReply = serde_json::from_value(expect_response(
                "late_results",
                request(&route, op_body(ops::LATE_RESULTS, args)).await?,
            )?)
            .map_err(|e| e.to_string())?;
            for entry in late.entries.iter().filter(|entry| entry.call_key == key) {
                if terminal {
                    return Err("entry after terminal user-command result".into());
                }
                match entry.kind.as_str() {
                    "progress" => {
                        if entry.text.is_none() {
                            return Err("invalid progress text".into());
                        }
                    }
                    "result" => {
                        let result: UserCommandReply = serde_json::from_value(
                            entry.result.clone().ok_or("missing command result")?,
                        )
                        .map_err(|e| e.to_string())?;
                        result.check().map_err(|e| e.message)?;
                        if !["done", "failed"].contains(&result.state.as_str()) {
                            return Err("non-terminal user-command result".into());
                        }
                        terminal = true;
                    }
                    _ => {
                        return Err("accepted user command produced a non-command late entry".into())
                    }
                }
            }
            since = Some(late.cursor);
            if terminal && !late.more {
                return Ok(());
            }
            if late.more {
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Err(
            "accepted user command did not produce a terminal result within the fixture deadline"
                .into(),
        )
    }
}
