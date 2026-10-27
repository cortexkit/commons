//! The transcripts [`super::held_copies_follow_reasoning_pruned`] serves: the
//! generic one of earlier releases, and one per serializer profile the
//! runner contract names (`llm-runner/v1` §4.4), each in that profile's own
//! stored format. Every id, model name, signature and text is invented.
//!
//! An assistant answer carries, where its profile can represent them, the
//! probe's logical parts ([`probe_parts`]): reasoning with a signature,
//! reasoning with an opaque payload beside a readable summary, text, a tool
//! call with its thought signature (its result rides in the same part or
//! in the next message, as the profile stores it), and text carrying a
//! provider-state signature. Each pruned part is served after the prune in
//! the form the per-profile rules of `llm-runner/v1` §9.14.1 give it.

use cortexkit_role_llm_runner::{
    archive_prunes_reasoning::{kinds, kinds_carried, PrunedPartAt, UnsupportedProfile},
    serializer_profile::{ANTHROPIC_MESSAGES, OPENCODE_AISDK, OWNED_BROCA, PI},
};
use serde_json::{json, Value};

use crate::view::CompactionView;

/// The names of the logical parts the held-copies probe's assistant answers
/// carry, as [`super::HeldCopiesReport`] lists them.
pub mod probe_parts {
    /// Reasoning text with a signature, pruned with the `reasoning` kind.
    pub const REASONING: &str = "reasoning";
    /// An opaque (encrypted) reasoning payload beside a readable summary,
    /// pruned with the `opaque_reasoning` kind.
    pub const OPAQUE_REASONING: &str = "opaque_reasoning";
    /// Plain assistant text, never pruned.
    pub const TEXT: &str = "text";
    /// A tool call.
    pub const TOOL_CALL: &str = "tool_call";
    /// That call's result.
    pub const TOOL_RESULT: &str = "tool_result";
    /// A thought signature on the tool call, pruned with the `signature`
    /// kind.
    pub const THOUGHT_SIGNATURE: &str = "thought_signature";
    /// Text carrying a provider-state signature, pruned with the
    /// `signature` kind.
    pub const TEXT_WITH_STATE: &str = "text_with_state";
    /// Every logical part, in the order the reports list them.
    pub const ALL: &[&str] = &[
        REASONING,
        OPAQUE_REASONING,
        TEXT,
        TOOL_CALL,
        TOOL_RESULT,
        THOUGHT_SIGNATURE,
        TEXT_WITH_STATE,
    ];
}

/// A logical part or prune kind the probe did not check, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotApplicable {
    /// A [`probe_parts`] name or a prune kind name.
    pub name: &'static str,
    pub reason: &'static str,
}

/// One part of an assistant answer: its original form and, for each kind
/// that can prune it, the form served after a prune of that kind.
pub(crate) struct ScriptedPart {
    pub original: Value,
    pub pruned: Vec<(&'static str, Value)>,
}

fn kept(original: Value) -> ScriptedPart {
    ScriptedPart {
        original,
        pruned: Vec::new(),
    }
}

fn pruned(original: Value, kind: &'static str, served: Value) -> ScriptedPart {
    ScriptedPart {
        original,
        pruned: vec![(kind, served)],
    }
}

/// An opaque payload beside a readable summary: a `reasoning` prune serves
/// `emptied`, an `opaque_reasoning` prune serves `summary_kept`.
fn opaque(original: Value, emptied: Value, summary_kept: Value) -> ScriptedPart {
    ScriptedPart {
        original,
        pruned: vec![
            (kinds::REASONING, emptied),
            (kinds::OPAQUE_REASONING, summary_kept),
        ],
    }
}

/// Which kinds one prune of the probe applies. A runner's per-message
/// check (`llm-runner/v1` §9.14.1) refuses any readable reasoning in a fold
/// that stands for a `reasoning`-pruned part, so a summary an
/// `opaque_reasoning` prune keeps cannot share a fold with a `reasoning`
/// prune; the probe exercises the two in separate conversations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PruneMode {
    /// `reasoning` and `signature`: every reasoning payload is emptied.
    Reasoning,
    /// `opaque_reasoning` and `signature`: only opaque payloads go, and
    /// readable reasoning stays.
    Opaque,
}

impl PruneMode {
    fn kinds(self) -> [&'static str; 2] {
        match self {
            Self::Reasoning => [kinds::REASONING, kinds::SIGNATURE],
            Self::Opaque => [kinds::OPAQUE_REASONING, kinds::SIGNATURE],
        }
    }
}

/// `value` without member `key`.
fn without(mut value: Value, key: &str) -> Value {
    if let Some(members) = value.as_object_mut() {
        members.remove(key);
    }
    value
}

/// An OpenAI Responses reasoning item serialized whole, with or without its
/// encrypted content: the opaque payload the profiles that carry one hold.
fn reasoning_item(ordinal: u64, encrypted: bool) -> String {
    let mut item = json!({
        "id": format!("rs_probe_{ordinal}"),
        "summary": [{"text": format!("A readable summary of thought {ordinal}."), "type": "summary_text"}],
        "type": "reasoning",
    });
    if encrypted {
        item["encrypted_content"] = json!(format!("invented-ciphertext-{ordinal}"));
    }
    item.to_string()
}

fn tool_output(ordinal: u64) -> String {
    format!("Door {ordinal} is open, and here is a long listing of what is behind it.")
}

const AT_MS: u64 = 1_767_225_600_000;

/// Which transcript the probe serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transcript {
    /// The generic transcript of earlier releases, for a provider set up
    /// with no serializer profile.
    Generic,
    /// One profile's native transcript.
    Profile(&'static str),
}

impl Transcript {
    /// The transcript for a provider set up with `profile`.
    pub fn for_profile(profile: Option<&str>) -> Result<Self, UnsupportedProfile> {
        let Some(profile) = profile else {
            return Ok(Self::Generic);
        };
        kinds_carried(profile)?;
        Ok(Self::Profile(
            [ANTHROPIC_MESSAGES, OPENCODE_AISDK, OWNED_BROCA, PI]
                .into_iter()
                .find(|named| *named == profile)
                .ok_or_else(|| UnsupportedProfile::new(profile))?,
        ))
    }

    pub fn profile(self) -> Option<&'static str> {
        match self {
            Self::Generic => None,
            Self::Profile(profile) => Some(profile),
        }
    }

    /// A message's top-level parts, from the profile's part container.
    pub fn parts(self, message: &Value) -> Vec<Value> {
        let container = match self {
            Self::Profile(OPENCODE_AISDK) => message.get("parts"),
            Self::Profile(PI) => message.pointer("/message/content"),
            _ => message.get("content"),
        };
        container
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// The message's part array, for replacing a part in place.
    pub fn parts_mut(self, message: &mut Value) -> Option<&mut Vec<Value>> {
        let container = match self {
            Self::Profile(OPENCODE_AISDK) => message.get_mut("parts"),
            Self::Profile(PI) => message.pointer_mut("/message/content"),
            _ => message.get_mut("content"),
        };
        container.and_then(Value::as_array_mut)
    }

    /// Whether pages carry `tags` rows, as a descriptors read does. Profile
    /// transcripts do, so a provider can run `check_descriptor_rows`; the
    /// generic transcript keeps the bytes of earlier releases.
    pub fn serves_tags(self) -> bool {
        self != Self::Generic
    }

    /// Message `ordinal`: a prompt or tool result at even ordinals, an
    /// assistant answer at odd ones.
    pub fn message(self, ordinal: u64) -> Value {
        if ordinal.is_multiple_of(2) {
            return self.even_message(ordinal);
        }
        let parts: Vec<Value> = self
            .answer_parts(ordinal)
            .into_iter()
            .map(|p| p.original)
            .collect();
        self.wrap_answer(ordinal, parts)
    }

    /// The parts a prune in `mode` changes in answer `ordinal`: their
    /// index, kind and served form, in the order the prune records them.
    pub fn prunes(self, ordinal: u64, mode: PruneMode) -> Vec<(usize, &'static str, Value)> {
        self.answer_parts(ordinal)
            .into_iter()
            .enumerate()
            .filter_map(|(index, part)| {
                let (kind, served) = part
                    .pruned
                    .into_iter()
                    .find(|(kind, _)| mode.kinds().contains(kind))?;
                Some((index, kind, served))
            })
            .collect()
    }

    /// Whether the probe runs a third conversation pruning in
    /// [`PruneMode::Opaque`]. The generic transcript has no opaque payload,
    /// so it runs only the `reasoning` mode, as in earlier releases.
    pub fn opaque_mode(self) -> bool {
        self != Self::Generic
    }

    /// Answer `ordinal`'s parts, original and pruned.
    pub fn answer_parts(self, o: u64) -> Vec<ScriptedPart> {
        let thought = format!("Private thought {o}.");
        let signature = format!("invented-signature-{o}");
        let call_signature = format!("invented-call-signature-{o}");
        let state_signature = format!("invented-state-signature-{o}");
        let noted = format!("Noted door {o}, with a long explanation.");
        let answer = format!("Answer {o}.");
        match self {
            Self::Generic => {
                let call = json!({"type":"tool_call","id":format!("call-{o}"),"name":"open_door",
                    "arguments":format!("{{\"door\": {o}}}"),"thought_signature":call_signature,
                    "result":{"output":tool_output(o)}});
                let state = signed_state(&noted, "gemini", &state_signature);
                let mut state_served = state.clone();
                state_served["provider_state"]["data"] = json!("");
                vec![
                    pruned(
                        json!({"type":"reasoning","text":thought,"signature":signature}),
                        kinds::REASONING,
                        json!({"type":"reasoning","text":""}),
                    ),
                    kept(json!({"type":"text","text":answer})),
                    pruned(
                        call.clone(),
                        kinds::SIGNATURE,
                        without(call, "thought_signature"),
                    ),
                    pruned(state, kinds::SIGNATURE, state_served),
                ]
            }
            Self::Profile(OWNED_BROCA) => {
                let call = json!({"type":"tool_call","tool_call_id":format!("call-{o}"),
                    "tool_name":"open_door","input":{"door":o},"thought_signature":call_signature});
                // Answers alternate between the two signed provider-state
                // formats, and each also carries a Responses item state,
                // which no prune touches and which never counts as restored.
                let (signed_format, item_format) = if o % 4 == 1 {
                    ("gemini", "open_ai_responses")
                } else {
                    ("google_code_assist", "open_ai_responses_chatgpt")
                };
                let state = signed_state(&noted, signed_format, &state_signature);
                let mut state_served = state.clone();
                state_served["provider_state"]["data"] = json!("");
                let item_state = json!({"type":"text_with_state","text":format!("Closing note {o}."),
                    "provider_state":{"format":item_format,
                        "data":json!({"id":format!("msg_probe_{o}"),"phase":"final_answer"}).to_string()}});
                vec![
                    // The signature is the opaque payload, so an
                    // `opaque_reasoning` prune removes it and keeps the text.
                    opaque(
                        json!({"type":"reasoning","text":thought,"signature":signature}),
                        json!({"type":"reasoning","text":""}),
                        json!({"type":"reasoning","text":thought}),
                    ),
                    opaque(
                        json!({"type":"redacted_reasoning","data":reasoning_item(o, true),
                            "provider_format":"probe-item"}),
                        json!({"type":"redacted_reasoning","data":"",
                            "provider_format":"probe-item"}),
                        json!({"type":"redacted_reasoning","data":reasoning_item(o, false),
                            "provider_format":"probe-item"}),
                    ),
                    kept(json!({"type":"text","text":answer})),
                    pruned(
                        call.clone(),
                        kinds::SIGNATURE,
                        without(call, "thought_signature"),
                    ),
                    pruned(state, kinds::SIGNATURE, state_served),
                    kept(item_state),
                ]
            }
            Self::Profile(ANTHROPIC_MESSAGES) => vec![
                // The signature is the encrypted full reasoning and the
                // text its readable summary, so an `opaque_reasoning` prune
                // empties only the signature.
                opaque(
                    json!({"type":"thinking","thinking":thought,"signature":signature}),
                    json!({"type":"thinking","thinking":"","signature":""}),
                    json!({"type":"thinking","thinking":thought,"signature":""}),
                ),
                // Redacted thinking is opaque through and through: both
                // kinds empty its data.
                opaque(
                    json!({"type":"redacted_thinking","data":format!("invented-redacted-data-{o}")}),
                    json!({"type":"redacted_thinking","data":""}),
                    json!({"type":"redacted_thinking","data":""}),
                ),
                kept(json!({"type":"text","text":answer})),
                kept(
                    json!({"type":"tool_use","id":format!("toolu_probe_{o}"),"name":"open_door",
                    "input":{"door":o}}),
                ),
            ],
            Self::Profile(OPENCODE_AISDK) => {
                let part = |index: usize, rest: Value| {
                    let mut part = json!({"id":format!("prt_probe_{o}_{index}"),
                        "sessionID":"ses_probe","messageID":format!("msg_probe_{o}")});
                    part.as_object_mut()
                        .unwrap()
                        .extend(rest.as_object().unwrap().clone());
                    part
                };
                let time = json!({"start": AT_MS + o * 1000, "end": AT_MS + o * 1000 + 500});
                let reasoning = part(
                    0,
                    json!({"type":"reasoning","text":thought,
                        "metadata":{"anthropic":{"signature":signature}},"time":time}),
                );
                let mut reasoning_served = reasoning.clone();
                reasoning_served["text"] = json!("");
                reasoning_served["metadata"] = json!({"anthropic":{}});
                let opaque_part = part(
                    1,
                    json!({"type":"reasoning","text":format!("A readable summary of thought {o}."),
                        "metadata":{"openai":{"itemId":format!("rs_probe_{o}"),
                            "reasoningEncryptedContent":format!("invented-ciphertext-{o}")}},
                        "time":time}),
                );
                let mut opaque_served = opaque_part.clone();
                opaque_served["metadata"] = json!({"openai":{"itemId":format!("rs_probe_{o}")}});
                let mut opaque_emptied = opaque_served.clone();
                opaque_emptied["text"] = json!("");
                let tool = part(
                    3,
                    json!({"type":"tool","callID":format!("call-{o}"),"tool":"open_door",
                        "state":{"status":"completed","input":{"door":o},"output":tool_output(o),
                            "title":"open_door","metadata":{},"time":time},
                        "metadata":{"google":{"thoughtSignature":call_signature}}}),
                );
                let mut tool_served = tool.clone();
                tool_served["metadata"] = json!({"google":{}});
                let state = part(
                    4,
                    json!({"type":"text","text":noted,
                        "metadata":{"google":{"thoughtSignature":state_signature}}}),
                );
                let mut state_served = state.clone();
                state_served["metadata"] = json!({"google":{}});
                vec![
                    pruned(reasoning, kinds::REASONING, reasoning_served),
                    opaque(opaque_part, opaque_emptied, opaque_served),
                    kept(part(2, json!({"type":"text","text":answer}))),
                    pruned(tool, kinds::SIGNATURE, tool_served),
                    pruned(state, kinds::SIGNATURE, state_served),
                ]
            }
            Self::Profile(PI) => {
                let call = json!({"type":"toolCall","id":format!("call-{o}"),"name":"open_door",
                    "arguments":{"door":o},"thoughtSignature":call_signature});
                let state = json!({"type":"text","text":noted,"textSignature":state_signature});
                vec![
                    pruned(
                        json!({"type":"thinking","thinking":thought,"thinkingSignature":signature}),
                        kinds::REASONING,
                        json!({"type":"thinking","thinking":""}),
                    ),
                    opaque(
                        json!({"type":"thinking","thinking":format!("A readable summary of thought {o}."),
                            "thinkingSignature":reasoning_item(o, true)}),
                        json!({"type":"thinking","thinking":""}),
                        json!({"type":"thinking","thinking":format!("A readable summary of thought {o}."),
                            "thinkingSignature":reasoning_item(o, false)}),
                    ),
                    kept(json!({"type":"text","text":answer})),
                    pruned(
                        call.clone(),
                        kinds::SIGNATURE,
                        without(call, "thoughtSignature"),
                    ),
                    pruned(
                        state.clone(),
                        kinds::SIGNATURE,
                        without(state, "textSignature"),
                    ),
                    // An OpenAI Responses text block's `textSignature` is
                    // the message item's id and phase, which no prune
                    // touches and which never counts as restored.
                    kept(json!({"type":"text","text":format!("Closing note {o}."),
                        "textSignature":json!({"v":1,"id":format!("msg_probe_{o}"),
                            "phase":"final_answer"}).to_string()})),
                ]
            }
            Self::Profile(other) => unreachable!("no transcript for profile {other}"),
        }
    }

    /// The prompt or tool-result message at even `ordinal`. Above 0 it
    /// carries the result of the call in the answer before it, where the
    /// profile stores results in a message of their own.
    fn even_message(self, o: u64) -> Value {
        let question = format!("Question {o}.");
        let call = o.checked_sub(1);
        match self {
            Self::Generic => json!({"role":"user","content":[{"type":"text","text":question}]}),
            Self::Profile(OWNED_BROCA) => {
                let mut content = Vec::new();
                if let Some(c) = call {
                    content.push(
                        json!({"type":"tool_result","tool_call_id":format!("call-{c}"),
                        "output":[{"type":"text","text":tool_output(c)}]}),
                    );
                }
                content.push(json!({"type":"text","text":question}));
                json!({"role":"user","content":content})
            }
            Self::Profile(ANTHROPIC_MESSAGES) => {
                let mut content = Vec::new();
                if let Some(c) = call {
                    content.push(
                        json!({"type":"tool_result","tool_use_id":format!("toolu_probe_{c}"),
                        "content":tool_output(c)}),
                    );
                }
                content.push(json!({"type":"text","text":question}));
                json!({"role":"user","content":content})
            }
            Self::Profile(OPENCODE_AISDK) => json!({
                "info":{"id":format!("msg_probe_{o}"),"sessionID":"ses_probe","role":"user",
                    "time":{"created":AT_MS + o * 1000},"agent":"probe",
                    "model":{"providerID":"probe-provider","modelID":"probe-model"}},
                "parts":[{"id":format!("prt_probe_{o}_0"),"sessionID":"ses_probe",
                    "messageID":format!("msg_probe_{o}"),"type":"text","text":question}],
            }),
            Self::Profile(PI) => {
                let message = match call {
                    None => json!({"role":"user","content":[{"type":"text","text":question}],
                        "timestamp":AT_MS}),
                    Some(c) => json!({"role":"toolResult","toolCallId":format!("call-{c}"),
                        "toolName":"open_door","content":[{"type":"text","text":tool_output(c)}],
                        "isError":false,"timestamp":AT_MS + o * 1000}),
                };
                pi_entry(o, message)
            }
            Self::Profile(other) => unreachable!("no transcript for profile {other}"),
        }
    }

    fn wrap_answer(self, o: u64, parts: Vec<Value>) -> Value {
        match self {
            Self::Generic | Self::Profile(ANTHROPIC_MESSAGES) => {
                json!({"role":"assistant","content":parts})
            }
            Self::Profile(OWNED_BROCA) => json!({"role":"assistant","content":parts,
                "origin":{"provider_module_id":"probe-model-provider","model_id":"probe-model"}}),
            Self::Profile(OPENCODE_AISDK) => json!({
                "info":{"id":format!("msg_probe_{o}"),"sessionID":"ses_probe","role":"assistant",
                    "time":{"created":AT_MS + o * 1000,"completed":AT_MS + o * 1000 + 900},
                    "parentID":format!("msg_probe_{}", o - 1),"providerID":"probe-provider",
                    "modelID":"probe-model","mode":"probe","path":{"cwd":"/probe","root":"/probe"},
                    "cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}},
                "parts":parts,
            }),
            Self::Profile(PI) => pi_entry(
                o,
                json!({"role":"assistant","content":parts,"api":"probe-api",
                    "provider":"probe-provider","model":"probe-model",
                    "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                    "stopReason":"toolUse","timestamp":AT_MS + o * 1000}),
            ),
            Self::Profile(other) => unreachable!("no transcript for profile {other}"),
        }
    }

    /// The logical parts this transcript carries and the probe checks.
    pub fn parts_checked(self) -> Vec<&'static str> {
        let missing: Vec<&str> = self.parts_not_applicable().iter().map(|n| n.name).collect();
        probe_parts::ALL
            .iter()
            .copied()
            .filter(|p| !missing.contains(p))
            .collect()
    }

    /// The logical parts this transcript cannot carry, by name.
    pub fn parts_not_applicable(self) -> Vec<NotApplicable> {
        match self {
            Self::Generic => vec![NotApplicable {
                name: probe_parts::OPAQUE_REASONING,
                reason: "the generic transcript carries no opaque reasoning payload; set the \
                         adapter's serializer profile to check one",
            }],
            Self::Profile(ANTHROPIC_MESSAGES) => vec![
                NotApplicable {
                    name: probe_parts::THOUGHT_SIGNATURE,
                    reason: "an anthropic-messages tool_use block has no signature",
                },
                NotApplicable {
                    name: probe_parts::TEXT_WITH_STATE,
                    reason: "an anthropic-messages text block has no provider-state signature",
                },
            ],
            Self::Profile(_) => Vec::new(),
        }
    }

    /// The prune kinds this transcript exercises.
    pub fn kinds_checked(self) -> Vec<&'static str> {
        match self {
            Self::Generic => vec![kinds::REASONING, kinds::SIGNATURE],
            Self::Profile(profile) => kinds_carried(profile).unwrap_or_default().to_vec(),
        }
    }

    /// The prune kinds this transcript cannot exercise, by name.
    pub fn kinds_not_applicable(self) -> Vec<NotApplicable> {
        let checked = self.kinds_checked();
        kinds::ALL
            .iter()
            .copied()
            .filter(|kind| !checked.contains(kind))
            .map(|name| NotApplicable {
                name,
                reason: match self {
                    Self::Generic => "the generic transcript carries no content of this kind",
                    Self::Profile(_) => {
                        "the profile's stored format carries no content of this kind"
                    }
                },
            })
            .collect()
    }

    /// The pruned part `view` restores, by the rule a runner of this
    /// profile applies.
    pub fn restored<'a>(
        self,
        view: &CompactionView,
        pruned: &'a [PrunedPartAt],
        first_ordinal: u64,
    ) -> Result<Option<&'a PrunedPartAt>, UnsupportedProfile> {
        match self {
            Self::Generic => Ok(view.restored_pruned_part(pruned, first_ordinal)),
            Self::Profile(profile) => view.restored_pruned_part_in(profile, pruned, first_ordinal),
        }
    }
}

/// An `owned-broca` `text_with_state` part whose provider state is a signed
/// `format`: `data` is the JSON string `{"thoughtSignature": ...}`.
fn signed_state(text: &str, format: &str, signature: &str) -> Value {
    json!({"type":"text_with_state","text":text,"provider_state":{"format":format,
        "data":json!({"thoughtSignature":signature}).to_string()}})
}

/// A `pi` journal entry wrapping `message`.
fn pi_entry(o: u64, message: Value) -> Value {
    let parent = o
        .checked_sub(1)
        .map_or(Value::Null, |p| json!(format!("probe-entry-{p}")));
    json!({"type":"message","id":format!("probe-entry-{o}"),"parentId":parent,
        "timestamp":"2026-01-01T00:00:00.000Z","message":message})
}
