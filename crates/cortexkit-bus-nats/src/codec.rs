use async_nats::header::{HeaderMap, HeaderName};
use cortexkit_bus_trait::{BusError, BusResult, ContentDigest, Headers, Message};

pub(crate) const MESSAGE_ID_HEADER: &str = "Nats-Msg-Id";
pub(crate) const CONTENT_DIGEST_HEADER: &str = "Ck-Content-Digest";

pub(crate) fn encode_headers(
    id: &str,
    digest: ContentDigest,
    headers: Headers,
) -> BusResult<HeaderMap> {
    let mut encoded = HeaderMap::new();
    encoded.insert(MESSAGE_ID_HEADER, id);
    encoded.insert(CONTENT_DIGEST_HEADER, digest.to_string());
    for (name, value) in headers {
        if name.eq_ignore_ascii_case(MESSAGE_ID_HEADER)
            || name.eq_ignore_ascii_case(CONTENT_DIGEST_HEADER)
        {
            return Err(BusError::denied(
                name,
                "caller header collides with a message-plane wire header",
            ));
        }
        let name = name.parse::<HeaderName>().map_err(|error| {
            BusError::denied(name, format!("invalid NATS header name: {error}"))
        })?;
        encoded.insert(name, value);
    }
    Ok(encoded)
}

pub(crate) fn decode_message(message: &async_nats::Message) -> BusResult<Message> {
    let headers = message
        .headers
        .as_ref()
        .ok_or_else(|| BusError::absent("message-plane wire headers"))?;
    let id = headers
        .get_last(MESSAGE_ID_HEADER)
        .map(|value| value.as_str().to_owned())
        .ok_or_else(|| BusError::absent(format!("header {MESSAGE_ID_HEADER}")))?;
    let digest = headers
        .get_last(CONTENT_DIGEST_HEADER)
        .ok_or_else(|| BusError::absent(format!("header {CONTENT_DIGEST_HEADER}")))?
        .as_str()
        .parse::<ContentDigest>()
        .map_err(BusError::unavailable)?;
    let mut decoded = Headers::new();
    for (name, values) in headers.iter() {
        let name: &str = name.as_ref();
        if name.eq_ignore_ascii_case(MESSAGE_ID_HEADER)
            || name.eq_ignore_ascii_case(CONTENT_DIGEST_HEADER)
        {
            continue;
        }
        if let Some(value) = values.last() {
            decoded.insert(name.to_owned(), value.as_str().to_owned());
        }
    }
    Ok(Message {
        subject: message.subject.to_string(),
        id,
        digest,
        headers: decoded,
    })
}
