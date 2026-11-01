//! A small delayed-storage runner: subscriber ids depend on native storage state.

use cortexkit_role_llm_runner::live::LiveSubscribeRequest;
use cortexkit_role_llm_runner_conformance::LiveIdsObservation;
use serde_json::{json, Value};

use super::Defects;

struct Runner<'a> {
    defects: &'a Defects,
    opt_in: bool,
    stored: Option<String>,
    held: Vec<Value>,
    frames: Vec<Value>,
    seq: u64,
}

impl Runner<'_> {
    fn emit(&mut self, control: bool, event: Value) {
        self.seq += 1;
        let mut frame =
            json!({"kind": if control {"control"} else {"display"}, "seq":self.seq, "event":event});
        if control {
            frame["cursor"] = json!({"record":self.seq});
        }
        self.frames.push(frame);
    }

    fn delta(&mut self, kind: &str) {
        let mut event = json!({"type":kind,"message_id":"","tool_call_id":"call-1","delta":"words","part":"part-1"});
        if self.stored.is_none() || self.defects.live_ids_after_storage {
            if self.opt_in || self.defects.live_ids_without_opt_in {
                event["live_id"] = "live-1".into();
            } else {
                self.held.push(event);
                return;
            }
        } else {
            event["mid"] = self.stored.clone().unwrap().into();
        }
        self.emit(false, event);
    }

    fn started(&mut self) {
        let mut event = json!({"type":"message_started","mid":"stored-1"});
        if self.opt_in {
            event["live_id"] = if self.defects.live_ids_rekey_mismatch {
                "live-other"
            } else {
                "live-1"
            }
            .into();
        }
        self.emit(true, event);
    }
}

pub(super) fn observe(
    request: LiveSubscribeRequest,
    store: bool,
    defects: &Defects,
) -> LiveIdsObservation {
    let mut runner = Runner {
        defects,
        opt_in: request.live_ids,
        stored: None,
        held: Vec::new(),
        frames: Vec::new(),
        seq: 0,
    };
    runner.emit(false, json!({"type":"display_reset"}));
    if defects.live_ids_started_before_storage {
        runner.started();
    }
    runner.delta("reasoning_delta");
    runner.delta("text_delta");
    runner.delta("tool_input_delta");
    let before = std::mem::take(&mut runner.frames);
    if store {
        runner.stored = Some("stored-1".into());
        runner.started();
        for mut event in std::mem::take(&mut runner.held) {
            event["mid"] = "stored-1".into();
            runner.emit(false, event);
        }
        runner.emit(false, json!({"type":"display_reset"}));
        if !defects.live_ids_store_at_end {
            runner.delta("text_delta");
            runner.delta("reasoning_delta");
            runner.delta("tool_input_delta");
        }
        runner.emit(true, json!({"type":"message_finished","mid":"stored-1"}));
    } else if !defects.live_ids_no_reset {
        runner.emit(false, json!({"type":"display_reset"}));
    }
    LiveIdsObservation::new(before, runner.frames, runner.stored)
}
