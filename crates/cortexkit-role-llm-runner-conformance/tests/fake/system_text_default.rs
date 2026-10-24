//! Scripted provider catalog and received-item capture for system-text admission.

use super::*;
use cortexkit_role_llm_runner_conformance::{
    wire::SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY, SystemTextDefaultObservation,
};

pub(super) struct Provider {
    provides: Vec<String>,
    received: Vec<Vec<u8>>,
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
        Ok(observation)
    }
}

impl Module {
    pub(super) fn admit_system_text_default(&self, session: &str, params: &Value) -> Option<Reply> {
        let entry = params.get("plan")?.get("system_text")?;
        let mut providers = self.world.system_text_default.lock().unwrap();
        let provider = providers.get_mut(session)?;
        if !provider
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
        if self.world.defects.system_text_default_invents_preset {
            item["preset"] = json!("invented-default");
        }
        provider.received.push(serde_json::to_vec(&item).unwrap());
        None
    }
}
