use cortexkit_role_compaction::{notice::NoticeRequest, ops, wire};
use serde_json::Value;

#[test]
fn last_prompt_view_notice_vectors_round_trip() {
    for (bytes, version) in [
        (
            include_str!("../test-vectors/compaction-v2/notice-last-prompt-view-request.json"),
            7,
        ),
        (
            include_str!("../test-vectors/compaction-v2/notice-last-prompt-no-view-request.json"),
            0,
        ),
    ] {
        let expected: Value = serde_json::from_str(bytes).unwrap();
        assert_eq!(
            expected["params"]["conditions"]["last_prompt_tokens"],
            1_024
        );
        assert_eq!(
            expected["params"]["conditions"]["last_prompt_view"],
            version
        );
        let (method, params) = wire::decode(bytes.as_bytes()).unwrap();
        assert_eq!(method, ops::COMPACTION_NOTICE);
        let decoded: NoticeRequest = wire::params(params).unwrap();
        let reencoded: Value = serde_json::from_slice(&wire::encode(&method, &decoded)).unwrap();
        assert_eq!(reencoded, expected);
        assert_eq!(
            format!("{}\n", serde_json::to_string_pretty(&reencoded).unwrap()),
            bytes
        );
    }
}
