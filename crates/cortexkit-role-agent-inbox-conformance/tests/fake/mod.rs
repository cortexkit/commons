//! In-memory fake for testing the suite, not evidence of live conformance.
//! Restart retains the fake's rows and receipts; it is not a process crash.

use std::collections::BTreeMap;

use cortexkit_role_agent_inbox::{
    neutralize, ops,
    post::{Attention, Content, PostRequest},
    read::{Item, ItemState},
    withdraw::WithdrawRequest,
};
use cortexkit_role_agent_inbox_conformance::{AgentInboxSubject, Caller, Delivery, ScopeStatus};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Defect {
    Repeat,
    Conflict,
    RetryAuth,
    ReplaceLive,
    ReplaceLanded,
    ReplaceRetired,
    Expiry,
    NotBefore,
    Withdraw,
    WithdrawLanded,
    WithdrawMissing,
    Namespace,
    NotYours,
    Unknown,
    Ended,
    Neutralize,
    Cap,
    Pull,
    Full,
}

#[derive(Clone)]
struct Row {
    namespace: String,
    request: PostRequest,
    item: Item,
    attention: Attention,
}

pub struct Fake {
    pub defect: Option<Defect>,
    rows: Vec<Row>,
    receipts: BTreeMap<(String, String), (Value, Value)>,
    now: u64,
    status: ScopeStatus,
    cap: usize,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            defect: None,
            rows: Vec::new(),
            receipts: BTreeMap::new(),
            now: 1000,
            status: ScopeStatus::Served,
            cap: usize::MAX,
        }
    }
}

fn refusal(code: &str) -> Value {
    json!({"code":code,"message":code})
}

impl Fake {
    fn broken(&self, defect: Defect) -> bool {
        self.defect == Some(defect)
    }

    fn namespace(&self, caller: Caller) -> String {
        if self.broken(Defect::Namespace) {
            return "shared".to_owned();
        }
        match caller {
            Caller::Provider => "provider::agent",
            Caller::OtherProvider => "other-provider::agent",
            Caller::FlowA => "basal:flow-a:agent",
            Caller::FlowB => "basal:flow-b:agent",
            Caller::OtherTarget => "basal:flow-a:other-agent",
        }
        .to_owned()
    }

    fn authorize(&self, caller: Caller) -> Option<Value> {
        if !matches!(caller, Caller::Provider | Caller::OtherProvider) {
            return None;
        }
        match self.status {
            ScopeStatus::Served => None,
            ScopeStatus::NotServed => Some(refusal(if self.broken(Defect::NotYours) {
                "scope_unknown"
            } else {
                "scope_not_yours"
            })),
            ScopeStatus::Unknown => Some(refusal(if self.broken(Defect::Unknown) {
                "scope_not_yours"
            } else {
                "scope_unknown"
            })),
            ScopeStatus::Ended => Some(refusal(if self.broken(Defect::Ended) {
                "scope_unknown"
            } else {
                "scope_ended"
            })),
        }
    }

    fn retire_expired(&mut self) {
        if self.broken(Defect::Expiry) {
            return;
        }
        for row in &mut self.rows {
            if row.item.state == ItemState::Pending
                && row
                    .request
                    .notice
                    .expires_at_ms
                    .is_some_and(|t| t <= self.now)
            {
                row.item.state = ItemState::Retired;
            }
        }
    }

    fn post(&mut self, caller: Caller, params: Value) -> Result<Value, String> {
        let namespace = self.namespace(caller);
        let req: PostRequest = match serde_json::from_value(params.clone()) {
            Ok(req) => req,
            Err(_) => return Ok(refusal("invalid_params")),
        };
        let receipt_key = (namespace.clone(), req.notice.idempotency_key.clone());
        if self.broken(Defect::RetryAuth) {
            if let Some(error) = self.authorize(caller) {
                return Ok(error);
            }
        }
        if let Some((body, reply)) = self.receipts.get(&receipt_key) {
            if body != &params {
                return Ok(if self.broken(Defect::Conflict) {
                    reply.clone()
                } else {
                    refusal("idempotency_conflict")
                });
            }
            if self.broken(Defect::Repeat) {
                return Ok(json!({"state":"accepted","item_id":"changed"}));
            }
            return Ok(reply.clone());
        }
        if let Some(error) = self.authorize(caller) {
            return Ok(error);
        }
        if !self.broken(Defect::Cap) {
            if let Err(error) = req.validate() {
                return serde_json::to_value(error).map_err(|e| e.to_string());
            }
        }
        self.retire_expired();
        let count = self
            .rows
            .iter()
            .filter(|r| r.namespace == namespace && r.item.state == ItemState::Pending)
            .count();
        if count >= self.cap && !self.broken(Defect::Full) {
            return Ok(refusal("inbox_full"));
        }
        let mut attention = req.notice.attention;
        if let Some(key) = &req.notice.replace_key {
            let keep_landed = !self.broken(Defect::ReplaceLanded);
            let inherit_retired = self.broken(Defect::ReplaceRetired);
            let lose_attention = self.broken(Defect::ReplaceLive);
            for row in &mut self.rows {
                if row.namespace != namespace
                    || row.request.notice.replace_key.as_ref() != Some(key)
                {
                    continue;
                }
                if row.item.state == ItemState::Landed && !keep_landed {
                    row.item.body = "mutated landed edition".to_owned();
                }
                if row.item.state == ItemState::Pending
                    || (inherit_retired && row.item.state == ItemState::Retired)
                {
                    if !lose_attention {
                        attention = attention.max(row.attention);
                    }
                    row.item.state = ItemState::Retired;
                }
            }
        }
        let id = format!("item-{}", self.rows.len());
        let rendered = self.render(&req);
        let item = Item::new(&id, ItemState::Pending, rendered);
        self.rows.push(Row {
            namespace,
            request: req,
            item,
            attention,
        });
        let reply = json!({"state":"accepted","item_id":id});
        self.receipts.insert(receipt_key, (params, reply.clone()));
        Ok(reply)
    }

    fn render(&self, req: &PostRequest) -> String {
        let mut parts = match &req.notice.content {
            Content::Text(text) => vec![text.text.clone()],
            Content::Template(template) => template
                .template
                .slots
                .values()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        };
        if let Some(links) = &req.notice.links {
            for link in links {
                parts.push(link.label.clone());
                parts.push(link.url.clone());
            }
        }
        if let Some(data) = &req.notice.data {
            parts.extend(data.values().filter_map(Value::as_str).map(str::to_owned));
        }
        let body = parts.join("\n");
        let safe = if self.broken(Defect::Neutralize) {
            body
        } else {
            neutralize(&body)
        };
        format!("<system-reminder>\nFrom: authenticated fixture sender\n{safe}\n</system-reminder>")
    }

    fn withdraw(&mut self, caller: Caller, params: Value) -> Result<Value, String> {
        let selector: WithdrawRequest = match serde_json::from_value(params) {
            Ok(selector) => selector,
            Err(_) => return Ok(refusal("invalid_params")),
        };
        self.retire_expired();
        let namespace = self.namespace(caller);
        let pending_defect = self.broken(Defect::Withdraw);
        let landed_defect = self.broken(Defect::WithdrawLanded);
        for row in self.rows.iter_mut().rev() {
            if row.namespace != namespace {
                continue;
            }
            let matches = match &selector {
                WithdrawRequest::Idempotency(key) => {
                    row.request.notice.idempotency_key == key.idempotency_key
                }
                WithdrawRequest::Replace(key) => {
                    row.request.notice.replace_key.as_ref() == Some(&key.replace_key)
                }
            };
            if !matches {
                continue;
            }
            if row.item.state == ItemState::Pending {
                if pending_defect {
                    return Ok(json!({"state":"not_found"}));
                }
                row.item.state = ItemState::Retired;
                return Ok(json!({"state":"withdrawn"}));
            }
            if row.item.state == ItemState::Landed {
                return Ok(
                    json!({"state":if landed_defect { "not_found" } else { "already_delivered" }}),
                );
            }
        }
        Ok(
            json!({"state":if self.broken(Defect::WithdrawMissing) { "withdrawn" } else { "not_found" }}),
        )
    }
}

impl AgentInboxSubject for Fake {
    fn reset(&mut self, _case: &'static str) -> Result<(), String> {
        *self = Self {
            defect: self.defect,
            ..Self::default()
        };
        Ok(())
    }
    fn call(&mut self, caller: Caller, method: &str, params: Value) -> Result<Value, String> {
        match method {
            ops::INBOX_POST => self.post(caller, params),
            ops::INBOX_WITHDRAW => self.withdraw(caller, params),
            ops::INBOX_LIST => {
                let other = caller == Caller::OtherTarget;
                let items: Vec<_> = self
                    .rows
                    .iter()
                    .filter(|r| {
                        r.namespace.ends_with("other-agent") == other
                            && (r.item.state == ItemState::Pending
                                || (r.item.state == ItemState::Unconfirmed
                                    && !self.broken(Defect::Pull)))
                    })
                    .map(|r| &r.item)
                    .collect();
                Ok(json!({"items":items}))
            }
            ops::INBOX_READ => {
                let item = self.rows.iter().find(|r| {
                    r.item.item_id == params["item_id"]
                        && r.namespace.ends_with("other-agent") == (caller == Caller::OtherTarget)
                });
                Ok(json!({"item":item.map(|r| &r.item)}))
            }
            _ => Err(format!("unknown op {method}")),
        }
    }
    fn set_scope_status(&mut self, status: ScopeStatus) -> Result<(), String> {
        self.status = status;
        Ok(())
    }
    fn restart(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn advance_to(&mut self, now_ms: u64) -> Result<(), String> {
        self.now = now_ms;
        self.retire_expired();
        Ok(())
    }
    fn deliver(&mut self) -> Result<Vec<Delivery>, String> {
        self.retire_expired();
        let not_before_defect = self.broken(Defect::NotBefore);
        let mut deliveries = Vec::new();
        for row in &mut self.rows {
            if row.item.state != ItemState::Pending {
                continue;
            }
            if !not_before_defect
                && row
                    .request
                    .notice
                    .deliver_after_ms
                    .is_some_and(|t| t > self.now)
            {
                continue;
            }
            deliveries.push(Delivery::new(
                &row.item.item_id,
                &row.item.body,
                row.attention,
            ));
            row.item.state = ItemState::Landed;
        }
        Ok(deliveries)
    }
    fn make_unconfirmed(&mut self) -> Result<(), String> {
        for row in &mut self.rows {
            if row.item.state == ItemState::Pending {
                row.item.state = ItemState::Unconfirmed;
            }
        }
        Ok(())
    }
    fn set_pending_cap(&mut self, count: usize) -> Result<(), String> {
        self.cap = count;
        Ok(())
    }
}
