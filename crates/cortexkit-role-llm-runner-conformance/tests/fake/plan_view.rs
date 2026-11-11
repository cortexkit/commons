use super::*;

fn fault(module: &Module) -> Option<&'static str> {
    module.world.defects.plan_view.and_then(|(case, fault)| {
        (*module.world.current_case.lock().unwrap() == Some(case)).then_some(fault)
    })
}

pub(super) fn admit(module: &Module, params: &Value) -> Option<Reply> {
    if params.get("plan").is_none() {
        match fault(module) {
            Some("require_plan") => return Some(invalid("plan")),
            Some("refuse_other") => return Some(invalid("other")),
            _ => {}
        }
    }
    None
}

pub(super) fn read(module: &Module, session: &str, params: &Value) -> Reply {
    use cortexkit_role_llm_runner::plan_view::{
        PlanView, PlanViewRequest, System, SystemItem, Tool,
    };

    if serde_json::from_value::<PlanViewRequest>(params.clone()).is_err() {
        return invalid("params");
    }
    let sessions = module.sessions.lock().unwrap();
    let Some(session) = sessions.get(session) else {
        return refuse(errors::UNKNOWN_SESSION, None);
    };
    if session.plan_digest.is_none() && fault(module) != Some("placeholder") {
        return refuse(errors::NO_PLAN, None);
    }
    let view = PlanView::new(
        0,
        System::new("é\nplan").with_items(vec![
            SystemItem::new("first", 0, 2),
            SystemItem::new("second", 3, 7),
        ]),
        session
            .tools
            .iter()
            .map(|name| Tool::new(name, "fixture tool", json!({}), "fixture-tools"))
            .collect(),
    );
    let mut value = serde_json::to_value(view).unwrap();
    match fault(module) {
        Some("not_planned") => value["as_planned"] = json!(false),
        Some("empty_range") => value["system"]["items"][0]["end"] = json!(0),
        Some("split_utf8") => value["system"]["items"][0]["end"] = json!(1),
        Some("outside_text") => value["system"]["items"][1]["end"] = json!(8),
        Some("overlap") => value["system"]["items"][1]["start"] = json!(0),
        Some("unordered") => value["system"]["items"].as_array_mut().unwrap().reverse(),
        _ => {}
    }
    respond(value)
}
