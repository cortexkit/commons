use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{collections::HashSet, fmt, str::FromStr};

use crate::{
    evidence::{decode_evidence, is_id_byte, valid_code, validate_evidence, WireEvidence},
    secrets::secret_free,
    wire::{
        canonical_digest, frame, is_lower_hex, present, require, shape, string_enum, unsigned,
        Object,
    },
    DecodeError, Evidence,
};

/// Stable, module-scoped finding identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct FindingId(String);

impl FindingId {
    pub fn new(id: impl Into<String>) -> Result<Self, DecodeError> {
        let id = id.into();
        require((1..=96).contains(&id.len()) && id.bytes().all(is_id_byte) && secret_free(&id))?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FindingId {
    type Err = DecodeError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::new(id)
    }
}

impl fmt::Display for FindingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Identifier of a module, as declared in its manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct ModuleId(String);

impl ModuleId {
    pub fn new(id: impl Into<String>) -> Result<Self, DecodeError> {
        let id = id.into();
        require(
            (1..=64).contains(&id.len())
                && id.as_bytes()[0].is_ascii_lowercase()
                && id.as_bytes()[1..]
                    .iter()
                    .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && secret_free(&id),
        )?;
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ModuleId {
    type Err = DecodeError;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        Self::new(id)
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FixClass {
    None,
    Safe,
    Risky,
}

/// A plan whose digest binds the finding id, steps, stop requirement, class and timeout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Plan {
    steps: Vec<String>,
    digest: String,
    timeout_ms: u64,
}

impl Plan {
    pub fn new(
        id: &FindingId,
        steps: Vec<String>,
        needs_stop: bool,
        class: FixClass,
        timeout_ms: u64,
    ) -> Result<Self, DecodeError> {
        require(class != FixClass::None)?;
        validate_plan(&steps, timeout_ms)?;
        let digest = plan_digest(id, &steps, needs_stop, class, timeout_ms)?;
        Ok(Self {
            steps,
            digest,
            timeout_ms,
        })
    }

    pub fn steps(&self) -> &[String] {
        &self.steps
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
}

/// Compute the plan digest over the RFC 8785 encoding of its bound members.
pub fn plan_digest(
    id: &FindingId,
    steps: &[String],
    needs_stop: bool,
    class: FixClass,
    timeout_ms: u64,
) -> Result<String, DecodeError> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        id: &'a FindingId,
        steps: &'a [String],
        needs_stop: bool,
        class: FixClass,
        timeout_ms: u64,
    }
    canonical_digest(&DigestInput {
        id,
        steps,
        needs_stop,
        class,
        timeout_ms,
    })
}

/// Check the wire grammar for a `sha256:` digest without verifying its content.
pub fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:")
        .is_some_and(|s| is_lower_hex(s, 64))
}

/// The fix (or absence of one) associated with a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Fix {
    class: FixClass,
    needs_stop: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<Plan>,
}

impl Fix {
    pub fn none(needs_stop: bool) -> Self {
        Self {
            class: FixClass::None,
            needs_stop,
            plan: None,
        }
    }

    pub fn safe(
        id: &FindingId,
        needs_stop: bool,
        steps: Vec<String>,
        timeout_ms: u64,
    ) -> Result<Self, DecodeError> {
        Self::planned(id, needs_stop, steps, timeout_ms, FixClass::Safe)
    }

    pub fn risky(
        id: &FindingId,
        needs_stop: bool,
        steps: Vec<String>,
        timeout_ms: u64,
    ) -> Result<Self, DecodeError> {
        Self::planned(id, needs_stop, steps, timeout_ms, FixClass::Risky)
    }

    fn planned(
        id: &FindingId,
        needs_stop: bool,
        steps: Vec<String>,
        timeout_ms: u64,
        class: FixClass,
    ) -> Result<Self, DecodeError> {
        let plan = Plan::new(id, steps, needs_stop, class, timeout_ms)?;
        Ok(Self {
            class,
            needs_stop,
            plan: Some(plan),
        })
    }

    pub fn class(&self) -> FixClass {
        self.class
    }

    pub fn needs_stop(&self) -> bool {
        self.needs_stop
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    fn verify_digest(&self, id: &FindingId) -> Result<(), DecodeError> {
        if let Some(plan) = &self.plan {
            let expected = plan_digest(
                id,
                &plan.steps,
                self.needs_stop,
                self.class,
                plan.timeout_ms,
            )?;
            if plan.digest != expected {
                return Err(DecodeError::PlanDigest);
            }
        }
        Ok(())
    }
}

/// A diagnostic finding produced by a module's check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Finding {
    id: FindingId,
    severity: Severity,
    summary: String,
    evidence: Vec<Evidence>,
    fix: Fix,
}

impl Finding {
    pub fn new(
        id: FindingId,
        severity: Severity,
        summary: impl Into<String>,
        evidence: Vec<Evidence>,
        fix: Fix,
    ) -> Result<Self, DecodeError> {
        let finding = Self::from_parts(id, severity, summary.into(), evidence, fix)?;
        finding.fix.verify_digest(&finding.id)?;
        Ok(finding)
    }

    fn from_parts(
        id: FindingId,
        severity: Severity,
        summary: String,
        evidence: Vec<Evidence>,
        fix: Fix,
    ) -> Result<Self, DecodeError> {
        require(valid_summary(&summary))?;
        validate_evidence(&evidence)?;
        Ok(Self {
            id,
            severity,
            summary,
            evidence,
            fix,
        })
    }

    pub fn id(&self) -> &FindingId {
        &self.id
    }
    pub fn severity(&self) -> Severity {
        self.severity
    }
    pub fn summary(&self) -> &str {
        &self.summary
    }
    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }
    pub fn fix(&self) -> &Fix {
        &self.fix
    }
}

fn valid_summary(s: &str) -> bool {
    !s.is_empty() && s.chars().take(201).count() <= 200 && !s.chars().any(char::is_control)
}

fn validate_plan(steps: &[String], timeout_ms: u64) -> Result<(), DecodeError> {
    require(
        (1..=16).contains(&steps.len())
            && steps.iter().all(|s| valid_summary(s))
            && (1_000..=600_000).contains(&timeout_ms),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum CheckSchema {
    #[serde(rename = "ck.doctor-check/v1")]
    V1,
}

/// A complete `doctor check --json` document.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CheckOutput {
    schema: CheckSchema,
    findings: Vec<Finding>,
}

impl CheckOutput {
    pub fn new(findings: Vec<Finding>) -> Result<Self, DecodeError> {
        let mut ids = HashSet::new();
        require(findings.iter().all(|finding| ids.insert(finding.id())))?;
        Ok(Self {
            schema: CheckSchema::V1,
            findings,
        })
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// Validate framing, shape and every plan digest, in that order.
    pub fn decode(input: &[u8]) -> Result<Self, DecodeError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireCheck {
            #[serde(rename = "schema", deserialize_with = "string_enum")]
            _schema: CheckSchema,
            findings: Vec<Object<WireFinding>>,
        }

        let Object(raw): Object<WireCheck> = shape(frame(input)?)?;
        let findings = raw
            .findings
            .into_iter()
            .map(|Object(finding)| finding.validate())
            .collect::<Result<_, _>>()?;
        let output = Self::new(findings)?;
        for finding in output.findings() {
            finding.fix.verify_digest(finding.id())?;
        }
        Ok(output)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFinding {
    id: String,
    #[serde(deserialize_with = "string_enum")]
    severity: Severity,
    summary: String,
    evidence: Vec<Object<WireEvidence>>,
    fix: Object<WireFix>,
}

impl WireFinding {
    fn validate(self) -> Result<Finding, DecodeError> {
        Finding::from_parts(
            FindingId::new(self.id)?,
            self.severity,
            self.summary,
            decode_evidence(self.evidence)?,
            self.fix.0.validate()?,
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFix {
    #[serde(deserialize_with = "string_enum")]
    class: FixClass,
    needs_stop: bool,
    #[serde(default, deserialize_with = "present")]
    plan: Option<Object<WirePlan>>,
}

impl WireFix {
    fn validate(self) -> Result<Fix, DecodeError> {
        require((self.class == FixClass::None) == self.plan.is_none())?;
        let plan = self.plan.map(|Object(raw)| raw.validate()).transpose()?;
        Ok(Fix {
            class: self.class,
            needs_stop: self.needs_stop,
            plan,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePlan {
    steps: Vec<String>,
    digest: String,
    timeout_ms: Box<RawValue>,
}

impl WirePlan {
    fn validate(self) -> Result<Plan, DecodeError> {
        let timeout_ms = unsigned(&self.timeout_ms, 600_000)?;
        validate_plan(&self.steps, timeout_ms)?;
        require(is_digest(&self.digest))?;
        Ok(Plan {
            steps: self.steps,
            digest: self.digest,
            timeout_ms,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FixResult {
    Fixed,
    Refused,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FixSchema {
    #[serde(rename = "ck.doctor-fix/v1")]
    V1,
}

/// A complete `doctor fix` document.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
pub struct FixOutput {
    schema: FixSchema,
    result: FixResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<Vec<Evidence>>,
}

impl FixOutput {
    pub fn fixed() -> Self {
        Self {
            schema: FixSchema::V1,
            result: FixResult::Fixed,
            code: None,
            evidence: None,
        }
    }

    pub fn refused(code: impl Into<String>) -> Result<Self, DecodeError> {
        Self::with_code(FixResult::Refused, code.into())
    }

    pub fn failed(code: impl Into<String>) -> Result<Self, DecodeError> {
        Self::with_code(FixResult::Failed, code.into())
    }

    fn with_code(result: FixResult, code: String) -> Result<Self, DecodeError> {
        require(valid_code(&code) && secret_free(&code))?;
        Ok(Self {
            schema: FixSchema::V1,
            result,
            code: Some(code),
            evidence: None,
        })
    }

    pub fn with_evidence(mut self, evidence: Vec<Evidence>) -> Result<Self, DecodeError> {
        validate_evidence(&evidence)?;
        self.evidence = Some(evidence);
        Ok(self)
    }

    pub fn result(&self) -> FixResult {
        self.result
    }
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
    pub fn evidence(&self) -> Option<&[Evidence]> {
        self.evidence.as_deref()
    }

    /// Validate framing and shape.
    pub fn decode(input: &[u8]) -> Result<Self, DecodeError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireFixOutput {
            #[serde(rename = "schema", deserialize_with = "string_enum")]
            _schema: FixSchema,
            #[serde(deserialize_with = "string_enum")]
            result: FixResult,
            #[serde(default, deserialize_with = "present")]
            code: Option<String>,
            #[serde(default, deserialize_with = "present")]
            evidence: Option<Vec<Object<WireEvidence>>>,
        }

        let Object(raw): Object<WireFixOutput> = shape(frame(input)?)?;
        let output = match (raw.result, raw.code) {
            (FixResult::Fixed, None) => Self::fixed(),
            (result @ (FixResult::Refused | FixResult::Failed), Some(code)) => {
                Self::with_code(result, code)?
            }
            _ => return Err(DecodeError::Shape),
        };
        match raw.evidence {
            Some(evidence) => output.with_evidence(decode_evidence(evidence)?),
            None => Ok(output),
        }
    }
}
