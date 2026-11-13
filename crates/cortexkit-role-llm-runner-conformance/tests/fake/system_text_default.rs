//! Scripted provider catalog and received-item capture for system-text admission.

use super::*;
use cortexkit_role_llm_runner_conformance::{
    wire::SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY, SystemTextDefaultObservation,
};

pub(super) struct Provider {
    provides: Vec<String>,
    received: Vec<Vec<u8>>,
    facts: Vec<Option<Vec<u8>>>,
}

impl FakeSubject {
    pub(super) fn prepare_default_text(
        &self,
        session: &str,
        provides: &[String],
        item: &Value,
    ) -> Result<Map<String, Value>, HarnessError> {
        self.world().system_text_default.lock().unwrap().insert(
            session.into(),
            Provider {
                provides: provides.to_vec(),
                received: vec![],
                facts: vec![],
            },
        );
        let mut fields = self.send_fields(session, true);
        fields
            .get_mut("plan")
            .ok_or_else(|| HarnessError::new("fixture requires plans"))?["system_text"] =
            json!({"provider": TOOL_MODULE, "item": item});
        Ok(fields)
    }

    pub(super) fn observe_default_text(
        &self,
        handle: &FakeHandle,
        session: &str,
    ) -> Result<SystemTextDefaultObservation, HarnessError> {
        let mut observation = SystemTextDefaultObservation::default();
        observation.provider_items = self
            .world()
            .system_text_default
            .lock()
            .unwrap()
            .get(session)
            .map(|p| p.received.clone())
            .unwrap_or_default();
        observation.durable_state = match std::fs::read(&handle.0.log) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(HarnessError::new(e.to_string())),
        };
        observation.provider_calls = observation.provider_items.len();
        observation.provider_facts = self
            .world()
            .system_text_default
            .lock()
            .unwrap()
            .get(session)
            .map(|p| p.facts.clone())
            .unwrap_or_default();
        Ok(observation)
    }
}

impl Module {
    pub(super) fn admit_system_text_default(&self, session: &str, params: &Value) -> Option<Reply> {
        let entry = params.get("plan")?.get("system_text")?;
        let mut providers = self.world.system_text_default.lock().unwrap();
        let provider = providers.get_mut(session)?;
        if entry["item"].get("preset").is_none()
            && !provider
                .provides
                .iter()
                .any(|name| name == SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY)
            && !self.world.defects.system_text_default_ignores_catalog
        {
            return Some(refuse(
                errors::SYSTEM_TEXT_PRESET_REQUIRED,
                Some(json!({"provider": entry["provider"]})),
            ));
        }
        let mut item = entry["item"].clone();
        let facts = if self.world.defects.system_text_facts_accepts_malformed
            || self.world.defects.system_text_facts_ignores_opt_in
        {
            item.get("facts").cloned()
        } else {
            match cortexkit_role_llm_runner::system_text_facts::for_provider(
                &item,
                &provider.provides,
            ) {
                Ok(facts) => facts.cloned(),
                Err(field) => {
                    return Some(refuse(errors::INVALID_PARAMS, Some(json!({"field":field}))))
                }
            }
        };
        let mut facts = facts;
        if let (true, Some(facts)) = (self.world.defects.system_text_facts_rewrites, &mut facts) {
            facts["model"]["id"] = json!("rewritten");
        }
        provider
            .facts
            .push(facts.map(|facts| serde_json::to_vec(&facts).unwrap()));
        item.as_object_mut().unwrap().remove("facts");
        if self.world.defects.system_text_default_invents_preset {
            item["preset"] = json!("invented-default");
        }
        provider.received.push(serde_json::to_vec(&item).unwrap());
        None
    }
}
