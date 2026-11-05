use super::*;
use cortexkit_role_llm_runner::{
    read::RideFact,
    rides::{self, Ride},
};

pub(super) fn apply(sess: &mut Sess, record: &Value, world: &World) -> bool {
    if !world
        .groups
        .iter()
        .any(|group| group == groups::RESULT_RIDES)
        || record["delivered_as"]["mode"] != "tool_result"
    {
        return false;
    }
    let id = record["id"].as_str().unwrap();
    let queued = sess.ordered[id].clone();
    let key = record["delivered_as"]["call_key"].as_str().unwrap();
    let message = sess
        .messages
        .iter_mut()
        .find(|message| message.body["call_key"] == key)
        .unwrap();
    message.rides.push(RideFact::new(
        0,
        &queued.send_id,
        &queued.sender,
        &queued.text,
    ));
    let row = message.mid.clone();
    if world.defects.result_rides == Some("stored") {
        message.host_body["content"] = json!(format!(
            "{}\n\n{}",
            result_text(&message.host_body["content"]),
            queued.text
        ));
    }
    if world.defects.result_rides == Some("entry") {
        let ordinal = sess.push(
            json!({"role":"user", "text":queued.text}),
            record["run_id"].as_str().unwrap(),
            Vec::new(),
        );
        sess.at_mut(ordinal).delivered_as = Some(record["delivered_as"].clone());
    }
    let queued = sess.ordered.get_mut(id).unwrap();
    queued.state = "delivered".into();
    queued.delivery = Some(QueuedDelivery {
        r#as: delivered_as::STEP,
        r#ref: row,
        run_id: record["run_id"].as_str().unwrap().into(),
    });
    true
}

pub(super) fn facts(message: &Msg, defects: &Defects) -> Vec<RideFact> {
    let mut facts = message.rides.clone();
    if defects.result_rides == Some("fact") {
        for fact in &mut facts {
            fact.part += 1;
        }
    }
    facts
}

pub(super) fn observe(observation: &mut ModelRequestObservation, sess: &Sess, defects: &Defects) {
    let mut system_text: Vec<String> = sess
        .messages
        .iter()
        .filter(|message| message.body["role"] == "system")
        .filter_map(|message| message.body["text"].as_str().map(str::to_owned))
        .collect();
    for message in sess
        .messages
        .iter()
        .filter(|message| !message.rides.is_empty())
    {
        let output = result_text(&message.body["content"]);
        let rides: Vec<_> = message
            .rides
            .iter()
            .map(|ride| {
                Ride::new(
                    &ride.send_id,
                    &ride.sender,
                    message.body["call_key"].as_str().unwrap(),
                    &ride.text,
                )
            })
            .collect();
        if let Some(text) = observation
            .rendered
            .iter_mut()
            .find(|text| **text == output)
        {
            *text = if defects.result_rides == Some("bytes") {
                format!(
                    "{}\n{}",
                    output,
                    rides
                        .iter()
                        .map(|ride| ride.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            } else {
                rides::append(&output, &rides)
            };
        }
        if defects.result_rides == Some("system") {
            system_text.extend(message.rides.iter().map(|ride| ride.text.clone()));
        }
    }
    observation.system_text = Some(system_text);
}
