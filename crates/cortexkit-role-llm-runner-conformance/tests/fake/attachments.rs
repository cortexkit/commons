//! The fake keeps original bytes separately from the media blocks in history.
//! Its upload journal survives a simulated restart; open data deliberately does
//! not, so status can distinguish a lost upload from a new empty one.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_role_llm_runner::{
    attachments as wire,
    content_digest::{content_digest, sha256_hex},
    read::MediaMode,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
struct Upload {
    session: String,
    mime: String,
    size: u64,
    sha256: String,
    #[serde(skip)]
    data: Vec<u8>,
    state: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    uploads: BTreeMap<String, Upload>,
    #[serde(skip)]
    media: BTreeMap<String, (String, Vec<u8>)>,
    files: BTreeMap<String, String>,
    #[serde(skip)]
    path: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Self {
        Self::reopen(root, false)
    }

    pub fn reopen(root: &Path, broken: bool) -> Self {
        let path = root.join("uploads.json");
        let mut store: Self = std::fs::read(&path)
            .ok()
            .map(|bytes| serde_json::from_slice(&bytes).unwrap())
            .unwrap_or_default();
        store.path = path;
        for (hash, mime) in &store.files {
            let bytes = std::fs::read(root.join(format!("media-{hash}"))).unwrap();
            store.media.insert(hash.clone(), (mime.clone(), bytes));
        }
        for upload in store
            .uploads
            .values_mut()
            .filter(|u| u.state == "committed")
        {
            upload.data = store.media[&upload.sha256].1.clone();
        }
        store.lose_open(broken);
        store
    }

    fn save(&self) {
        for (hash, (_, bytes)) in &self.media {
            let path = self.path.parent().unwrap().join(format!("media-{hash}"));
            if !path.exists() {
                std::fs::write(path, bytes).unwrap();
            }
        }
        std::fs::write(&self.path, serde_json::to_vec(self).unwrap()).unwrap();
    }

    pub fn lose_open(&mut self, broken: bool) {
        for upload in self.uploads.values_mut().filter(|u| u.state == "open") {
            upload.data.clear();
            upload.state = if broken { "open" } else { "gone" }.into();
        }
        self.save();
    }
}

fn fault(module: &Module, session: &str, name: &str) -> bool {
    module.world.defects.attachments == name && session.contains(name)
}

fn media_block(mime: &str, bytes: &[u8]) -> Value {
    json!({"type":"media","kind":if mime.starts_with("image/") {"image"} else {"document"},"mime":mime,"bytes":bytes.len(),"sha256":sha256_hex(bytes)})
}

fn has_hash(value: &Value, hash: &str) -> bool {
    if value["type"] == "media" && value["sha256"] == hash {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(|v| has_hash(v, hash)),
        Value::Object(map) => map.values().any(|v| has_hash(v, hash)),
        _ => false,
    }
}

pub fn handle(
    module: &Module,
    stamp: &RouteStamp,
    session: &str,
    method: &str,
    params: Value,
) -> Reply {
    let sess = module.sessions.lock().unwrap().get(session).cloned();
    let media_read = method == ops::SESSION_MEDIA_READ;
    if media_read
        && fault(module, session, "attachments_media_read_admission")
        && sess
            .as_ref()
            .is_some_and(|s| stamp.principal.as_str() != s.owner)
    {
        return refuse(errors::SCOPE_OWNER_MISMATCH, None);
    }
    if !media_read {
        if let Some(sess) = &sess {
            let admitted = stamp.principal.as_str() == sess.owner
                || sess
                    .senders
                    .iter()
                    .any(|s| s.module == stamp.principal.as_str());
            if !admitted
                || (fault(module, session, "attachments_plan_sender")
                    && stamp.principal.as_str() != sess.owner)
            {
                return refuse(errors::SCOPE_OWNER_MISMATCH, None);
            }
        } else if !fault(module, session, "attachments_admission") {
            return refuse(errors::UNKNOWN_SESSION, None);
        }
    }
    let fields: &[&str] = match method {
        ops::SESSION_UPLOAD_BEGIN => &["mime", "bytes", "sha256"],
        ops::SESSION_UPLOAD_APPEND => &["upload_id", "offset", "data"],
        ops::SESSION_MEDIA_READ => &["sha256", "offset", "length"],
        _ => &["upload_id"],
    };
    if let Some(field) = unknown_field(&params, fields) {
        return invalid(&field);
    }
    if matches!(
        method,
        ops::SESSION_UPLOAD_STATUS | ops::SESSION_UPLOAD_COMMIT
    ) && !params["upload_id"].is_string()
    {
        return invalid("upload_id");
    }
    let mut store = module.attachments.lock().unwrap();
    if media_read {
        let request: wire::MediaReadRequest = match serde_json::from_value(params) {
            Ok(r) => r,
            Err(_) => return invalid("sha256"),
        };
        let referenced = sess.as_ref().is_some_and(|s| {
            s.messages
                .iter()
                .chain(s.archive.iter())
                .any(|m| has_hash(&m.body, &request.sha256))
        });
        if !referenced
            && !fault(module, session, "attachments_media_unreferenced")
            && !fault(module, session, "attachments_media_other_session")
        {
            return refuse(errors::UNKNOWN_MEDIA, None);
        }
        let Some((mime, bytes)) = store.media.get(&request.sha256) else {
            return refuse(errors::UNKNOWN_MEDIA, None);
        };
        if let Err(field) = request.check(bytes.len() as u64) {
            return invalid(field);
        }
        let mut reply = wire::MediaReadReply::new(bytes.len() as u64, mime);
        if request.length > 0 {
            let end = (request.offset + request.length).min(bytes.len() as u64) as usize;
            reply = reply.with_data(STANDARD.encode(&bytes[request.offset as usize..end]));
        }
        return respond(reply);
    }
    if method == ops::SESSION_UPLOAD_BEGIN {
        let request: wire::UploadBeginRequest = match serde_json::from_value(params) {
            Ok(r) => r,
            Err(_) => return invalid("mime"),
        };
        if let Err(field) = request.check() {
            return invalid(field);
        }
        let id = format!("upload-{}", store.uploads.len() + 1);
        store.uploads.insert(
            id.clone(),
            Upload {
                session: session.into(),
                mime: request.mime,
                size: request.bytes,
                sha256: request.sha256,
                data: Vec::new(),
                state: "open".into(),
            },
        );
        store.save();
        return respond(json!({"upload_id":id}));
    }
    let id = params["upload_id"].as_str().unwrap_or_default();
    let Some(upload) = store.uploads.get_mut(id) else {
        return if method == ops::SESSION_UPLOAD_STATUS {
            respond(json!({"state":"gone","received":0}))
        } else {
            refuse(errors::UNKNOWN_UPLOAD, None)
        };
    };
    if upload.session != session
        && !fault(module, session, "attachments_upload_other_session_status")
        && !fault(module, session, "attachments_upload_other_session_commit")
    {
        return if method == ops::SESSION_UPLOAD_STATUS {
            respond(json!({"state":"gone","received":0}))
        } else {
            refuse(errors::UNKNOWN_UPLOAD, None)
        };
    }
    if method == ops::SESSION_UPLOAD_STATUS {
        if fault(module, session, "attachments_retention") && upload.state == "committed" {
            return respond(json!({"state":"gone","received":0}));
        }
        return respond(json!({"state":upload.state,"received":upload.data.len()}));
    }
    if upload.state == "gone" {
        return refuse(errors::UPLOAD_GONE, None);
    }
    if method == ops::SESSION_UPLOAD_APPEND {
        let request: wire::UploadAppendRequest = match serde_json::from_value(params) {
            Ok(r) => r,
            Err(_) => return invalid("offset"),
        };
        if upload.state != "open" {
            return invalid("upload_id");
        }
        let bytes = match STANDARD.decode(&request.data) {
            Ok(bytes) => bytes,
            Err(_) => return invalid("data"),
        };
        let admission = wire::check_append(
            upload.data.len() as u64,
            upload.size,
            request.offset,
            bytes.len(),
        );
        if let Err(code) = admission {
            if !(fault(module, session, "attachments_chunk_cap")
                && code == errors::UPLOAD_CHUNK_TOO_LARGE)
                && !(fault(module, session, "attachments_lifecycle")
                    && code == errors::UPLOAD_OFFSET_MISMATCH)
            {
                return refuse(
                    code,
                    (code == errors::UPLOAD_OFFSET_MISMATCH)
                        .then(|| json!({"received":upload.data.len()})),
                );
            }
        }
        upload.data.extend(bytes);
        let received = upload.data.len();
        store.save();
        return respond(json!({"received":received}));
    }
    if method == ops::SESSION_UPLOAD_COMMIT {
        if let Err(code) = wire::verify_commit(upload.size, &upload.sha256, &upload.data) {
            if !fault(module, session, "attachments_commit_sha256")
                && !fault(module, session, "attachments_commit_size")
            {
                return refuse(code, None);
            }
        }
        upload.state = "committed".into();
        let reply = json!({"state":"committed","sha256":upload.sha256,"mime":upload.mime,"bytes":upload.size});
        let (hash, mime, bytes) = (
            upload.sha256.clone(),
            upload.mime.clone(),
            upload.data.clone(),
        );
        store.files.insert(hash.clone(), mime.clone());
        store.media.insert(hash, (mime, bytes));
        store.save();
        return respond(reply);
    }
    invalid("method")
}

/// Called before any send record is written, including its idempotency key.
pub fn admit_send(
    module: &Module,
    session: &str,
    request: &SendRequest,
) -> Result<Vec<Value>, Reply> {
    if !module.world.serves(groups::ATTACHMENTS) {
        return Ok(Vec::new());
    }
    let Some(content) = request.content() else {
        return Ok(Vec::new());
    };
    let sess = module.sessions.lock().unwrap().get(session).cloned();
    let accepts = sess
        .as_ref()
        .and_then(|s| s.attachment_accepts)
        .unwrap_or_else(|| {
            request
                .runner_params
                .get("attachment_accepts")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
    let store = module.attachments.lock().unwrap();
    let mut parts = Vec::new();
    for part in content {
        let upload = store.uploads.get(part.upload_id());
        let code = match upload {
            None => Some(errors::UNKNOWN_UPLOAD),
            Some(u) if u.session != session => Some(errors::UNKNOWN_UPLOAD),
            Some(u) if u.state == "gone" => Some(errors::UPLOAD_GONE),
            Some(u) if u.state != "committed" => Some(errors::UPLOAD_NOT_COMMITTED),
            Some(u) if !accepts || !["image/png", "application/pdf"].contains(&u.mime.as_str()) => {
                Some(errors::MEDIA_UNSUPPORTED)
            }
            _ => None,
        };
        if let Some(code) = code {
            let broken = [
                "attachments_send_unknown",
                "attachments_send_uncommitted",
                "attachments_send_other_session",
                "attachments_send_gone",
                "attachments_model_unsupported",
                "attachments_model_mime",
                "attachments_frozen_model",
            ]
            .iter()
            .any(|name| fault(module, session, name));
            if !broken {
                return Err(refuse(code, None));
            }
        }
        if let Some(upload) = upload {
            parts.push(media_block(&upload.mime, &upload.data));
        }
    }
    Ok(parts)
}

pub fn add_parts(body: &mut Value, record: &Value) {
    if let Some(parts) = record["attachment_parts"]
        .as_array()
        .filter(|p| !p.is_empty())
    {
        let prompt = record.get("prompt").unwrap_or(&body["text"]);
        let mut all = vec![json!({"type":"text","text":prompt})];
        all.extend(parts.iter().cloned());
        body.as_object_mut().unwrap().remove("text");
        body["parts"] = json!(all);
    }
}

pub fn queued_parts(module: &Module, session: &str, request: &Value) -> Vec<Value> {
    let request: SendRequest = serde_json::from_value(request.clone()).unwrap();
    admit_send(module, session, &request).expect("queued uploads were admitted before writing")
}

/// Ingest provider bytes once; read representations contain only metadata.
pub fn ingest(module: &Module, body: &mut Value) {
    if body["type"] == "image" && body["source"]["type"] == "base64" {
        let bytes = STANDARD
            .decode(body["source"]["data"].as_str().unwrap())
            .unwrap();
        let mime = body["source"]["media_type"].as_str().unwrap().to_owned();
        let mut store = module.attachments.lock().unwrap();
        store.files.insert(sha256_hex(&bytes), mime.clone());
        store
            .media
            .insert(sha256_hex(&bytes), (mime.clone(), bytes.clone()));
        store.save();
        *body = media_block(&mime, &bytes);
        return;
    }
    match body {
        Value::Array(items) => items.iter_mut().for_each(|v| ingest(module, v)),
        Value::Object(map) => map.values_mut().for_each(|v| ingest(module, v)),
        _ => {}
    }
}

fn contains_media(value: &Value) -> bool {
    if value["type"] == "media" {
        return true;
    }
    match value {
        Value::Array(items) => items.iter().any(contains_media),
        Value::Object(map) => map.values().any(contains_media),
        _ => false,
    }
}

pub(super) fn render(value: &mut Value) {
    if value["type"] == "media" {
        let text = wire::marker(
            value["kind"].as_str().unwrap(),
            value["mime"].as_str().unwrap(),
            value["bytes"].as_u64().unwrap(),
            value["sha256"].as_str().unwrap(),
            "not_requested",
        );
        *value = json!({"type":"text","text":text});
        return;
    }
    if value["type"] == "tool_result"
        && value["content"].is_array()
        && contains_media(&value["content"])
    {
        let mut parts = value["content"].as_array().unwrap().clone();
        parts.iter_mut().for_each(render);
        value["content"] = json!(wire::join_tool_result(
            &parts
                .iter()
                .map(|p| p["text"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        ));
        return;
    }
    match value {
        Value::Array(items) => items.iter_mut().for_each(render),
        Value::Object(map) => map.values_mut().for_each(render),
        _ => {}
    }
}

pub fn page(page: &mut ReadPage, mode: Option<MediaMode>, defects: &Defects) {
    for message in &mut page.messages {
        if !contains_media(&message.message) {
            continue;
        }
        message.digest = Some(content_digest(&message.message));
        message.part_digests = Some(
            media::parts(&message.message)
                .iter()
                .map(|(key, value)| {
                    cortexkit_role_llm_runner::read::descriptors::PartDigest::new(key, value)
                })
                .collect(),
        );
        let broken = matches!(
            defects.attachments,
            "attachments_default_read" | "attachments_tool_result"
        ) && message.mid.contains(defects.attachments);
        if mode != Some(MediaMode::Blocks) && !broken {
            render(&mut message.message);
        }
    }
}
