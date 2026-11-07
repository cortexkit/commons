use super::*;

pub(super) async fn run<S: MediaSubject>(s: &S) -> Result<(), String> {
    let mut configured = engines();
    configured.push(
        serde_json::from_value(json!({
            "kind":"future_kind", "model":"future-model", "provider":"fixture",
            "local":false, "formats":["audio/wav"], "latency_class":"future_latency",
            "bias_limit":{"unit":"future_bias_unit", "max":3}
        }))
        .map_err(err)?,
    );
    s.configure_engines(configured).await.map_err(err)?;
    let describe = call(s, "alice", "role.describe", json!({})).await?;
    let engine = describe["engines"]
        .as_array()
        .and_then(|engines| engines.iter().find(|e| e["model"] == "future-model"))
        .ok_or("describe omitted the engine with unknown enum values")?;
    require(
        engine["kind"] == "future_kind",
        "unknown engine kind changed",
    )?;
    require(
        engine["latency_class"] == "future_latency",
        "unknown latency class changed",
    )?;
    require(
        engine["bias_limit"]["unit"] == "future_bias_unit",
        "unknown bias unit changed",
    )?;

    s.hold_engine("local").await.map_err(err)?;
    let mut req = request("wire", "transcribe");
    req["model"] = json!("local");
    let id = submit(s, req).await?;
    status(s, &id).await?;
    refusal(
        s,
        "alice",
        "media.status",
        json!({"job_id":id,"future_member":true}),
        "invalid_params",
        None,
    )
    .await?;
    Ok(())
}
