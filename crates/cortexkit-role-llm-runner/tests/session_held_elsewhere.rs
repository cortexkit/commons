use cortexkit_role_llm_runner::{errors, session_retention::SessionHeldElsewhereDetail};
use serde_json::{json, Value};

#[test]
fn session_held_elsewhere_samples_preserve_optional_holder() {
    let samples: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/session-held-elsewhere.json"
    ))
    .unwrap();
    for sample in samples["samples"].as_array().unwrap() {
        let error = &sample["error"];
        assert_eq!(error["code"], errors::SESSION_HELD_ELSEWHERE);
        assert_eq!(sample["retryable_here"], false);
        assert!(!errors::is_retryable(error["code"].as_str().unwrap()));
        let raw = error.get("detail").cloned().unwrap_or_else(|| json!({}));
        let detail: SessionHeldElsewhereDetail = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&detail).unwrap(), raw);
        let expected = match &detail.holder {
            Some(holder) => {
                assert_eq!(holder.len(), 32);
                assert!(holder.bytes().all(|byte| byte.is_ascii_hexdigit()));
                SessionHeldElsewhereDetail::new().with_holder(holder)
            }
            None => SessionHeldElsewhereDetail::new(),
        };
        assert_eq!(detail, expected);
    }
}

#[test]
fn session_held_elsewhere_is_not_the_retryable_release_refusal() {
    assert!(errors::CODES.contains(&errors::SESSION_HELD_ELSEWHERE));
    assert_ne!(errors::SESSION_HELD_ELSEWHERE, errors::SESSION_RELEASED);
    assert!(!errors::is_retryable(errors::SESSION_HELD_ELSEWHERE));
    assert!(errors::is_retryable(errors::SESSION_RELEASED));
    assert_eq!(
        serde_json::to_value(SessionHeldElsewhereDetail::new()).unwrap(),
        json!({})
    );
}
