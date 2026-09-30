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
        check_flat_schema, is_schema_digest, schema_digest, session_capabilities, CatalogAnswer,
    },
    describe::check_describe,
    errors,
    late_results::{kinds, LateResultsReply, CURSOR_INCARNATION_CHANGED},
    ops, points,
    withdraw::{WithdrawAnswer, TARGET_CALL_KEY_FIELD},
    PROVIDES,
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
const CALL_KEY_VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-vectors/tool-provider-v1/call-key.json"
));
const SCHEMA_PIN_VECTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-vectors/tool-provider-v1/schema-pin.json"
));

/// A tool name no provider serves, for the refusal case.
const UNSERVED_TOOL: &str = "conformance.not-a-served-tool";

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

    let capabilities = subject.capabilities();
    let mut driver = CrashDriver::new(subject);
    let mut mint = KeyMint(0);
    let main = driver
        .spawn(&work_dir.join("main"))
        .await
        .map_err(|e| format!("spawning the provider failed: {e}"));

    let mut cases = Vec::with_capacity(CASES.len());
    for spec in CASES {
        let missing: Vec<Capability> = spec
            .requires
            .iter()
            .filter(|required| !capabilities.contains(required))
            .copied()
            .collect();
        let outcome = if !missing.is_empty() {
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
            "role_describe_shape" => self.role_describe_shape().await,
            "role_describe_cacheable" => self.role_describe_cacheable().await,
            "catalog_schemas_flat" => self.catalog_schemas_flat().await,
            "catalog_schema_digest_stable" => self.catalog_schema_digest_stable().await,
            "catalog_digest_only" => self.catalog_digest_only().await,
            "catalog_disabled_tool_absent" => self.catalog_disabled_tool_absent().await,
            "call_disabled_tool_refused_by_name" => self.call_disabled_tool_refused().await,
            "terminal_frame_on_success" => self.terminal_frame_on_success().await,
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

    fn principals(&self) -> Result<ScopedPrincipals, String> {
        principals(self.subject)
    }

    async fn describe(&self, route: &S::Route) -> Result<Value, String> {
        expect_response(
            "role.describe",
            request(route, op_body(ops::ROLE_DESCRIBE, json!({}))).await?,
        )
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
        let body = expect_response(
            "tool.catalog",
            request(route, op_body(ops::TOOL_CATALOG, arguments)).await?,
        )?;
        serde_json::from_value(body)
            .map_err(|e| format!("tool.catalog answer does not decode: {e}"))
    }

    async fn catalog(&self, route: &S::Route) -> Result<CatalogAnswer, String> {
        self.catalog_with(route, false).await
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
        }
        Ok(())
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
