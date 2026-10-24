//! Attachment admission is observed over the bound management route. Fixtures
//! configure model facts and restarts; they never manufacture protocol replies.
use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read, wait_run_end,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_role_llm_runner::{
    attachments::CHUNK_MAX,
    content_digest::{content_digest, sha256_hex},
    errors, ops,
    read::{MediaMode, ReadRequest},
    send::SendReply,
};
use serde_json::{json, Value};

type Check = Result<(), String>;

async fn ok<R: RunnerRoute>(route: &R, method: &str, params: Value) -> Result<Value, String> {
    expect_ok(method, call(route, method, params).await?)
}

async fn refused<R: RunnerRoute>(route: &R, method: &str, params: Value, code: &str) -> Check {
    expect_code(
        method,
        &expect_error(method, call(route, method, params).await?)?,
        code,
    )
}

fn check(condition: bool, message: &str) -> Check {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn png() -> Vec<u8> {
    STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap()
}

async fn session<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    label: &str,
    accepts: bool,
) -> Result<Session<S::Route>, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            label,
            Script {
                turns: (0..8)
                    .map(|_| {
                        ScriptedTurn::new(vec![ScriptedPart::Text("attachment-answer".into())])
                    })
                    .collect(),
            },
        )
        .await?;
    let mut params = case.params(
        &session,
        "attachment-session",
        &case.mint.next("send"),
        None,
    );
    let fields = case
        .subject
        .attachment_model_fields(&session.name, accepts)
        .ok_or("attachments declared but attachment_model_fields is not implemented")?;
    params.as_object_mut().unwrap().extend(fields);
    let id = params["send_id"].as_str().unwrap().to_owned();
    let reply: SendReply = decode(
        "admit attachment session",
        ok(&session.route, ops::SESSION_SEND, params).await?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    session.first = Some((id, "attachment-session".into()));
    Ok(session)
}

async fn begin<R: RunnerRoute>(
    route: &R,
    mime: &str,
    bytes: &[u8],
    hash: Option<&str>,
    size: Option<u64>,
) -> Result<String, String> {
    let body = ok(route, ops::SESSION_UPLOAD_BEGIN, json!({"mime":mime,"bytes":size.unwrap_or(bytes.len() as u64),"sha256":hash.map(str::to_owned).unwrap_or_else(|| sha256_hex(bytes))})).await?;
    body["upload_id"]
        .as_str()
        .map(str::to_owned)
        .filter(|id| !id.is_empty())
        .ok_or("upload_begin has no upload_id".into())
}

async fn append<R: RunnerRoute>(
    route: &R,
    id: &str,
    offset: usize,
    bytes: &[u8],
) -> Result<Value, String> {
    ok(
        route,
        ops::SESSION_UPLOAD_APPEND,
        json!({"upload_id":id,"offset":offset,"data":STANDARD.encode(bytes)}),
    )
    .await
}

async fn status<R: RunnerRoute>(route: &R, id: &str, state: &str, received: usize) -> Check {
    let value = ok(route, ops::SESSION_UPLOAD_STATUS, json!({"upload_id":id})).await?;
    check(
        value["state"] == state && value["received"] == received,
        &format!("upload_status expected {state}/{received}, got {value}"),
    )
}

async fn upload<R: RunnerRoute>(route: &R, bytes: &[u8]) -> Result<String, String> {
    let id = begin(route, "image/png", bytes, None, None).await?;
    for (i, chunk) in bytes.chunks(CHUNK_MAX).enumerate() {
        append(route, &id, i * CHUNK_MAX, chunk).await?;
    }
    let reply = ok(route, ops::SESSION_UPLOAD_COMMIT, json!({"upload_id":id})).await?;
    check(
        reply
            == json!({"state":"committed","sha256":sha256_hex(bytes),"mime":"image/png","bytes":bytes.len()}),
        "commit metadata differs from uploaded bytes",
    )?;
    Ok(id)
}

async fn send<S: LlmRunnerSubject>(case: &Case<'_, S>, s: &Session<S::Route>, id: &str) -> Check
where
    S::Route: RunnerRoute,
{
    let before = head(&s.route).await?.last_run_state.map(|r| r.run_id);
    let mut params = case.params(s, "look at media", &case.mint.next("send"), None);
    params["content"] = json!([{"type":"upload","upload_id":id}]);
    let reply: SendReply = decode(
        "attachment send",
        ok(&s.route, ops::SESSION_SEND, params).await?,
    )?;
    wait_run_end(
        case.subject,
        &s.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    Ok(())
}

async fn media<R: RunnerRoute>(route: &R, hash: &str, bytes: &[u8]) -> Check {
    let request =
        |offset: usize, length: usize| json!({"sha256":hash,"offset":offset,"length":length});
    let probe = ok(route, ops::SESSION_MEDIA_READ, request(0, 0)).await?;
    check(
        probe["size"] == bytes.len() && probe["mime"] == "image/png" && probe.get("data").is_none(),
        "zero-length probe must omit data and return full size/mime",
    )?;
    let mut actual = Vec::new();
    while actual.len() < bytes.len() {
        let reply = ok(
            route,
            ops::SESSION_MEDIA_READ,
            request(actual.len(), CHUNK_MAX),
        )
        .await?;
        check(
            reply["size"] == bytes.len() && reply["mime"] == "image/png",
            "media chunk metadata changed",
        )?;
        let decoded = STANDARD
            .decode(reply["data"].as_str().ok_or("media chunk has no data")?)
            .map_err(|e| e.to_string())?;
        check(
            !decoded.is_empty()
                && decoded.len() <= CHUNK_MAX
                && decoded.len() <= bytes.len() - actual.len(),
            "media chunk is empty, over cap, or beyond EOF",
        )?;
        actual.extend(decoded);
    }
    check(actual == bytes, "media_read returned different bytes")?;
    check(
        ok(route, ops::SESSION_MEDIA_READ, request(bytes.len(), 1)).await?["data"] == "",
        "EOF must return empty data",
    )?;
    for (offset, length, field) in [(bytes.len() + 1, 1, "offset"), (0, CHUNK_MAX + 1, "length")] {
        let error = expect_error(
            "media bounds",
            call(route, ops::SESSION_MEDIA_READ, request(offset, length)).await?,
        )?;
        expect_field("media bounds", &error, errors::INVALID_PARAMS, field)?;
    }
    Ok(())
}

async fn no_send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    s: &Session<S::Route>,
    id: &str,
    code: &str,
    joining: bool,
) -> Check
where
    S::Route: RunnerRoute,
{
    let before = read(&s.route, &ReadRequest::range(0)).await?;
    let before_head = head(&s.route).await?;
    let mut params = case.params(
        s,
        "refused-media",
        &case.mint.next("send"),
        joining.then_some("steer"),
    );
    params["content"] = json!([{"type":"upload","upload_id":id}]);
    refused(&s.route, ops::SESSION_SEND, params, code).await?;
    check(
        read(&s.route, &ReadRequest::range(0)).await? == before
            && head(&s.route).await? == before_head,
        "refused attachment send wrote transcript or head state",
    )
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let result = match name {
        "attachments_frozen_model" => frozen(case).await,
        "attachments_tool_result" => tool_result(case).await,
        "attachments_plan_sender" => plan_sender(case).await,
        _ => ordinary(case, name).await,
    };
    result.map(|()| Ending::Passed)
}

async fn ordinary<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Check
where
    S::Route: RunnerRoute,
{
    let s = session(case, name, name != "attachments_model_unsupported").await?;
    let bytes = png();
    let hash = sha256_hex(&bytes);
    match name {
        "attachments_lifecycle" => {
            // A valid PNG larger than a frame proves that the send names an
            // upload instead of carrying the original bytes inline.
            let bytes = crate::media::padded_png(CHUNK_MAX * 2 + 128);
            let id = begin(&s.route, "image/png", &bytes, None, None).await?;
            status(&s.route, &id, "open", 0).await?;
            let first = append(&s.route, &id, 0, &bytes[..CHUNK_MAX]).await?;
            check(
                first["received"] == CHUNK_MAX,
                "append did not acknowledge the chunk",
            )?;
            let lost = expect_error(
                "lost append reply",
                call(
                    &s.route,
                    ops::SESSION_UPLOAD_APPEND,
                    json!({"upload_id":id,"offset":0,"data":STANDARD.encode(&bytes[..CHUNK_MAX])}),
                )
                .await?,
            )?;
            expect_code("lost append reply", &lost, errors::UPLOAD_OFFSET_MISMATCH)?;
            let received = lost
                .detail
                .as_ref()
                .and_then(|d| d["received"].as_u64())
                .ok_or("upload_offset_mismatch has no received count")?
                as usize;
            check(
                received == CHUNK_MAX,
                "lost append reply gave wrong resume offset",
            )?;
            for (i, chunk) in bytes[received..].chunks(CHUNK_MAX).enumerate() {
                append(&s.route, &id, received + i * CHUNK_MAX, chunk).await?;
            }
            status(&s.route, &id, "open", bytes.len()).await?;
            let committed = ok(
                &s.route,
                ops::SESSION_UPLOAD_COMMIT,
                json!({"upload_id":id}),
            )
            .await?;
            check(
                committed
                    == json!({"state":"committed","sha256":sha256_hex(&bytes),"mime":"image/png","bytes":bytes.len()}),
                "commit metadata differs",
            )?;
            check(
                ok(
                    &s.route,
                    ops::SESSION_UPLOAD_COMMIT,
                    json!({"upload_id":id}),
                )
                .await?
                    == committed,
                "repeat commit changed metadata",
            )?;
            status(&s.route, &id, "committed", bytes.len()).await?;
            send(case, &s, &id).await?;
            media(&s.route, &sha256_hex(&bytes), &bytes).await
        }
        "attachments_restart_gone" | "attachments_send_gone" => {
            let id = begin(&s.route, "image/png", &bytes, None, None).await?;
            append(&s.route, &id, 0, &bytes[..3]).await?;
            let committed = upload(&s.route, &bytes).await?;
            send(case, &s, &committed).await?;
            let route = case
                .subject
                .restart_attachments(case.handle.inner(), &s.name)
                .await
                .map_err(|e| e.to_string())?;
            if name == "attachments_restart_gone" {
                status(&route, &id, "gone", 0).await?;
                status(&route, &committed, "committed", bytes.len()).await?;
                media(&route, &hash, &bytes).await
            } else {
                let s = Session {
                    name: s.name,
                    route,
                    first: s.first,
                };
                no_send(case, &s, &id, errors::UPLOAD_GONE, false).await
            }
        }
        "attachments_chunk_cap" => {
            let data = vec![0; CHUNK_MAX + 1];
            let id = begin(&s.route, "application/pdf", &data, None, None).await?;
            refused(
                &s.route,
                ops::SESSION_UPLOAD_APPEND,
                json!({"upload_id":id,"offset":0,"data":STANDARD.encode(&data)}),
                errors::UPLOAD_CHUNK_TOO_LARGE,
            )
            .await?;
            status(&s.route, &id, "open", 0).await?;
            append(&s.route, &id, 0, &data[..CHUNK_MAX]).await?;
            status(&s.route, &id, "open", CHUNK_MAX).await
        }
        "attachments_commit_sha256" | "attachments_commit_size" => {
            let bad_hash = sha256_hex(b"different content");
            let wrong_hash = name.ends_with("sha256");
            let id = begin(
                &s.route,
                "image/png",
                &bytes,
                wrong_hash.then_some(bad_hash.as_str()),
                (!wrong_hash).then_some(bytes.len() as u64 + 1),
            )
            .await?;
            append(&s.route, &id, 0, &bytes).await?;
            refused(
                &s.route,
                ops::SESSION_UPLOAD_COMMIT,
                json!({"upload_id":id}),
                if wrong_hash {
                    errors::UPLOAD_SHA256_MISMATCH
                } else {
                    errors::UPLOAD_SIZE_MISMATCH
                },
            )
            .await?;
            status(&s.route, &id, "open", bytes.len()).await?;
            no_send(case, &s, &id, errors::UPLOAD_NOT_COMMITTED, false).await?;
            if !wrong_hash {
                refused(
                    &s.route,
                    ops::SESSION_UPLOAD_APPEND,
                    json!({"upload_id":id,"offset":bytes.len(),"data":STANDARD.encode([0,0])}),
                    errors::UPLOAD_SIZE_MISMATCH,
                )
                .await?;
                status(&s.route, &id, "open", bytes.len()).await?;
            }
            Ok(())
        }
        "attachments_model_mime" => {
            let bytes = STANDARD
                .decode("R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==")
                .unwrap();
            let input = head(&s.route)
                .await?
                .input
                .ok_or("session.head has no model input facts")?;
            check(
                input.mime_types.as_ref().is_some_and(|types| {
                    types.iter().any(|mime| mime == "image/png")
                        && !types.iter().any(|mime| mime == "image/gif")
                }),
                "fixture must report PNG support but not GIF support",
            )?;
            let id = begin(&s.route, "image/gif", &bytes, None, None).await?;
            append(&s.route, &id, 0, &bytes).await?;
            ok(
                &s.route,
                ops::SESSION_UPLOAD_COMMIT,
                json!({"upload_id":id}),
            )
            .await?;
            no_send(case, &s, &id, errors::MEDIA_UNSUPPORTED, false).await
        }
        "attachments_model_unsupported" => {
            let id = upload(&s.route, &bytes).await?;
            let input = head(&s.route)
                .await?
                .input
                .ok_or("session.head has no model input facts")?;
            check(
                input.attachments == "no"
                    || input
                        .modalities
                        .as_ref()
                        .is_some_and(|m| !m.iter().any(|m| m == "image")),
                "text-only fixture did not report its known model input restriction",
            )?;
            no_send(case, &s, &id, errors::MEDIA_UNSUPPORTED, false).await
        }
        "attachments_send_uncommitted" => {
            let id = begin(&s.route, "image/png", &bytes, None, None).await?;
            no_send(case, &s, &id, errors::UPLOAD_NOT_COMMITTED, false).await
        }
        "attachments_send_unknown" => {
            no_send(
                case,
                &s,
                "never-issued-upload-id",
                errors::UNKNOWN_UPLOAD,
                false,
            )
            .await
        }
        "attachments_send_other_session" | "attachments_media_other_session" => {
            let other = session(case, "other-attachment-session", true).await?;
            let id = upload(&other.route, &bytes).await?;
            if name == "attachments_send_other_session" {
                no_send(case, &s, &id, errors::UNKNOWN_UPLOAD, false).await
            } else {
                send(case, &other, &id).await?;
                media(&other.route, &hash, &bytes).await?;
                refused(
                    &s.route,
                    ops::SESSION_MEDIA_READ,
                    json!({"sha256":hash,"offset":0,"length":1}),
                    errors::UNKNOWN_MEDIA,
                )
                .await
            }
        }
        "attachments_upload_other_session_status" | "attachments_upload_other_session_commit" => {
            let other = session(case, "other-attachment-upload", true).await?;
            let id = upload(&other.route, &bytes).await?;
            if name.ends_with("status") {
                return status(&s.route, &id, "gone", 0).await;
            }
            for method in [ops::SESSION_UPLOAD_COMMIT, ops::SESSION_UPLOAD_APPEND] {
                let params = if method == ops::SESSION_UPLOAD_APPEND {
                    json!({"upload_id":id,"offset":0,"data":""})
                } else {
                    json!({"upload_id":id})
                };
                refused(&s.route, method, params, errors::UNKNOWN_UPLOAD).await?;
            }
            status(&other.route, &id, "committed", bytes.len()).await
        }
        "attachments_media_read_admission" => {
            let id = upload(&s.route, &bytes).await?;
            send(case, &s, &id).await?;
            let stranger = case
                .subject
                .session_route(case.handle.inner(), &s.name, &case.subject.stranger_stamp())
                .await
                .map_err(|e| e.to_string())?;
            match call(&stranger, ops::SESSION_READ, json!({})).await? {
                crate::route::Reply::Response(_) => media(&stranger, &hash, &bytes).await,
                crate::route::Reply::Error(error) => {
                    refused(
                        &stranger,
                        ops::SESSION_MEDIA_READ,
                        json!({"sha256":hash,"offset":0,"length":1}),
                        &error.code,
                    )
                    .await
                }
            }
        }
        "attachments_media_unreferenced" => {
            upload(&s.route, &bytes).await?;
            refused(
                &s.route,
                ops::SESSION_MEDIA_READ,
                json!({"sha256":hash,"offset":0,"length":1}),
                errors::UNKNOWN_MEDIA,
            )
            .await
        }
        "attachments_default_read" => {
            let id = upload(&s.route, &bytes).await?;
            send(case, &s, &id).await?;
            rendered(case, &s.route, &bytes, false).await
        }
        "attachments_admission" => {
            let empty = case.open(name, Script::default()).await?;
            refused(
                &empty.route,
                ops::SESSION_UPLOAD_BEGIN,
                json!({"mime":"image/png","bytes":bytes.len(),"sha256":hash}),
                errors::UNKNOWN_SESSION,
            )
            .await?;
            let stranger = case
                .subject
                .session_route(case.handle.inner(), &s.name, &case.subject.stranger_stamp())
                .await
                .map_err(|e| e.to_string())?;
            refused(
                &stranger,
                ops::SESSION_UPLOAD_BEGIN,
                json!({"mime":"image/png","bytes":bytes.len(),"sha256":hash}),
                errors::SCOPE_OWNER_MISMATCH,
            )
            .await?;
            let id = upload(&s.route, &bytes).await?;
            for method in [
                ops::SESSION_UPLOAD_APPEND,
                ops::SESSION_UPLOAD_STATUS,
                ops::SESSION_UPLOAD_COMMIT,
            ] {
                refused(
                    &stranger,
                    method,
                    json!({"upload_id":id,"offset":0,"data":""}),
                    errors::SCOPE_OWNER_MISMATCH,
                )
                .await?;
            }
            Ok(())
        }
        "attachments_retention" => {
            let id = upload(&s.route, &bytes).await?;
            case.subject
                .advance_retention_clock(&s.name, 3_599_999)
                .await
                .map_err(|e| e.to_string())?;
            status(&s.route, &id, "committed", bytes.len()).await?;
            send(case, &s, &id).await?;
            case.subject
                .advance_retention_clock(&s.name, 7_200_000)
                .await
                .map_err(|e| e.to_string())?;
            let route = case
                .subject
                .restart_attachments(case.handle.inner(), &s.name)
                .await
                .map_err(|e| e.to_string())?;
            media(&route, &hash, &bytes).await
        }
        other => Err(format!("no attachment case named {other}")),
    }
}

fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

async fn rendered<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    bytes: &[u8],
    nested: bool,
) -> Check
where
    S::Route: RunnerRoute,
{
    let hash = sha256_hex(bytes);
    let marker = format!(
        "[media not shown: kind=image mime=image/png bytes={} sha256={} reason=not_requested]",
        bytes.len(),
        &hash[..12]
    );
    let inline = read(route, &ReadRequest::range(0)).await?;
    let explicit_inline = read(route, &ReadRequest::range(0).with_media(MediaMode::Inline)).await?;
    check(
        inline == explicit_inline,
        "default and explicit inline reads differ",
    )?;
    let blocks = read(route, &ReadRequest::range(0).with_media(MediaMode::Blocks)).await?;
    let descriptors = read(
        route,
        &ReadRequest::range(0).with_media(MediaMode::Descriptors),
    )
    .await?;
    let mut texts = Vec::new();
    for m in &inline.messages {
        strings(&m.message, &mut texts);
        check(
            !m.message.to_string().contains(&hash),
            "default read leaked a media block instead of a marker",
        )?;
    }
    check(
        texts.iter().any(|t| t.contains(&marker)),
        "default read has no exact not_requested marker",
    )?;
    if case.declared.contains(&Capability::ModelView) {
        let page = ok(
            route,
            ops::SESSION_READ,
            json!({"view":"model","from_ordinal":0}),
        )
        .await?;
        let mut model_texts = Vec::new();
        strings(&page, &mut model_texts);
        check(
            model_texts.iter().any(|text| text.contains(&marker))
                && !page.to_string().contains(&hash),
            "default model-view read did not render safe media markers",
        )?;
    }
    if nested {
        check(
            texts
                .iter()
                .any(|t| t == &format!("before-media\n{marker}\nafter-media")),
            "multipart tool result did not render as today's text output in part order",
        )?;
    }
    // The default read replaced every media block with the marker, so its
    // exact text names the message that holds the media. Blocks and
    // descriptors mode are then matched by that `mid`: a runner's stored
    // schema contains the media its own way and need not carry the digest.
    let mid = inline
        .messages
        .iter()
        .find(|m| {
            let mut found = Vec::new();
            strings(&m.message, &mut found);
            found.iter().any(|text| text.contains(&marker))
        })
        .map(|m| m.mid.clone())
        .ok_or("default read has no exact not_requested marker")?;
    check(
        descriptors.messages.iter().any(|m| {
            m.mid == mid
                && m.message.to_string().contains("media_ref")
                && m.message.to_string().contains(&hash)
        }),
        "descriptors mode lost media_ref",
    )?;
    let message = blocks
        .messages
        .iter()
        .find(|m| m.mid == mid)
        .ok_or("media blocks opt-in did not return the stored message")?;
    let stored_texts = {
        let mut found = Vec::new();
        strings(&message.message, &mut found);
        found
    };
    check(
        !stored_texts.iter().any(|text| text.contains(&marker)),
        "blocks mode skipped markers instead of serving the stored message",
    )?;
    check(
        message.digest.as_deref() == Some(content_digest(&message.message).as_str()),
        "blocks digest does not identify the stored message",
    )?;
    let expected: Vec<_> = case
        .subject
        .read_parts(&message.message)
        .iter()
        .map(|(key, value)| {
            cortexkit_role_llm_runner::read::descriptors::PartDigest::new(key, value)
        })
        .collect();
    check(
        message.part_digests.as_ref() == Some(&expected),
        "part digests do not identify stored parts",
    )?;
    for (page, required) in [(&inline, false), (&descriptors, true)] {
        let other = page
            .messages
            .iter()
            .find(|m| m.mid == message.mid)
            .ok_or("rendered read lost a stored message")?;
        // Inline reads may omit either identity independently; descriptors
        // must carry both. A supplied identity always names the stored content.
        if required || other.digest.is_some() {
            check(
                other.digest == message.digest,
                "media rendering changed stored content digest",
            )?;
        }
        if required || other.part_digests.is_some() {
            check(
                other.part_digests == message.part_digests,
                "media rendering changed stored part digests",
            )?;
        }
    }
    media(route, &hash, bytes).await
}

async fn frozen<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Check
where
    S::Route: RunnerRoute,
{
    let mut tool = case.mint.call("attachment-held-call");
    tool.hold = true;
    let mut s = case
        .open(
            "attachments_frozen_model",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("done".into())]),
                ],
            },
        )
        .await?;
    let send_id = case.mint.next("send");
    let mut params = case.params(&s, "text-only-run", &send_id, None);
    params.as_object_mut().unwrap().extend(
        case.subject
            .attachment_model_fields(&s.name, false)
            .ok_or("no attachment model fixture")?,
    );
    let first: SendReply = decode(
        "start frozen text-only run",
        ok(&s.route, ops::SESSION_SEND, params).await?,
    )?;
    s.first = Some((send_id, "text-only-run".into()));
    case.subject
        .await_tool_call(&tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let outcome = async {
        let id = upload(&s.route, &png()).await?;
        no_send(case, &s, &id, errors::MEDIA_UNSUPPORTED, true).await
    }
    .await;
    case.subject
        .release_tool_call(&tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    wait_run_end(case.subject, &s.route, case.declared, None, &first).await?;
    outcome
}

async fn tool_result<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Check
where
    S::Route: RunnerRoute,
{
    let bytes = png();
    let mut tool = case.mint.call("attachment-tool-media");
    tool.result = case
        .subject
        .attachment_tool_result(&bytes)
        .ok_or("attachments declared but attachment_tool_result is not implemented")?;
    let mut s = case
        .open(
            "attachments_tool_result",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool)]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("done".into())]),
                ],
            },
        )
        .await?;
    case.run_prompt(&mut s, "read the tool's image").await?;
    rendered(case, &s.route, &bytes, true).await
}

async fn plan_sender<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Check
where
    S::Route: RunnerRoute,
{
    let (s, route) = crate::delivery_order::sender_session(
        case,
        "attachments_plan_sender",
        vec![ScriptedTurn::new(vec![ScriptedPart::Text("done".into())])],
        true,
        false,
    )
    .await?;
    let bytes = png();
    let id = upload(&route, &bytes).await?;
    status(&route, &id, "committed", bytes.len()).await?;
    let sender_session = Session {
        name: s.name,
        route,
        first: s.first,
    };
    send(case, &sender_session, &id).await?;
    media(&s.route, &sha256_hex(&bytes), &bytes).await
}
