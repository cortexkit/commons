//! Run every shared golden vector under `test-vectors/` through the public API.

use base64::Engine;
use cortexkit_chatgpt_policy::{
    account_id, classify_response, vectors as shared_vectors, Classification, MissingAccountId,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    time::{Duration, UNIX_EPOCH},
};

fn load_vectors(file: &str) -> Vec<Value> {
    let (_, text) = shared_vectors::FILES
        .iter()
        .find(|(name, _)| *name == file)
        .unwrap_or_else(|| panic!("unknown vector file {file}"));
    let document: Value =
        serde_json::from_str(text).unwrap_or_else(|error| panic!("{file} is not JSON: {error}"));
    document["vectors"]
        .as_array()
        .unwrap_or_else(|| panic!("{file} has no vectors array"))
        .clone()
}

/// The vector-file spelling of a class, plus its reset fields when it has them.
fn describe(classification: Classification) -> Value {
    let class = match classification {
        Classification::AccountLimitExhausted {
            resets_at_ms,
            resets_at_source,
        } => {
            return serde_json::json!({
                "class": "account_limit_exhausted",
                "resets_at_ms": resets_at_ms,
                "resets_at_source": resets_at_source.map(|source| source.as_str()),
            })
        }
        Classification::UsageNotIncluded => "usage_not_included",
        Classification::AuthFailure => "auth_failure",
        Classification::UpstreamKeyRejection => "upstream_key_rejection",
        Classification::RateLimited => "rate_limited",
        Classification::Overloaded => "overloaded",
        Classification::Other => "other",
    };
    serde_json::json!({ "class": class })
}

#[test]
fn every_response_vector_gets_its_expected_class() {
    let vectors = load_vectors("responses.json");
    // The file holds at least one vector per class (two for some); requiring seven keeps an
    // emptied or truncated file from passing.
    assert!(
        vectors.len() >= 7,
        "only {} response vectors",
        vectors.len()
    );
    for vector in &vectors {
        let name = vector["name"].as_str().expect("vector name");
        let provenance = vector["provenance"].as_str().expect("vector provenance");
        assert!(
            provenance.starts_with("CAPTURED: ") || provenance.starts_with("RECONSTRUCTED: "),
            "{name}: provenance must say CAPTURED or RECONSTRUCTED"
        );
        let status = u16::try_from(vector["status"].as_u64().expect("status")).unwrap();
        let headers: Vec<(String, String)> = vector["headers"]
            .as_array()
            .expect("headers")
            .iter()
            .map(|pair| {
                (
                    pair[0].as_str().expect("header name").to_owned(),
                    pair[1].as_str().expect("header value").to_owned(),
                )
            })
            .collect();
        let body = vector["body"].as_str().expect("body");
        let now = UNIX_EPOCH + Duration::from_millis(vector["now_ms"].as_u64().expect("now_ms"));
        let actual = describe(classify_response(status, headers, body, now));
        assert_eq!(actual, vector["expected"], "{name}");
    }
}

#[test]
fn every_account_id_vector_resolves_as_expected() {
    let vectors = load_vectors("account_ids.json");
    assert!(
        vectors.len() >= 4,
        "only {} account-id vectors",
        vectors.len()
    );
    for vector in &vectors {
        let name = vector["name"].as_str().expect("vector name");
        let token = vector["access_token"].as_str().expect("access_token");
        let actual = account_id(vector["vault_account_id"].as_str(), token);
        let expected = &vector["expected"];
        match expected["account_id"].as_str() {
            Some(account) => assert_eq!(actual.as_deref(), Ok(account), "{name}"),
            None => {
                assert_eq!(expected["error"], "missing_account_id", "{name}");
                assert_eq!(actual, Err(MissingAccountId), "{name}");
            }
        }
    }
}

#[test]
fn sha256sums_cover_every_shipped_file_and_match_its_bytes() {
    use sha2::{Digest, Sha256};

    let mut listed = BTreeSet::new();
    for line in shared_vectors::SHA256SUMS
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let (digest, name) = line.split_once("  ").expect("sha256sum line");
        let (_, contents) = shared_vectors::FILES
            .iter()
            .find(|(file, _)| *file == name)
            .unwrap_or_else(|| {
                panic!("SHA256SUMS names {name}, which vectors::FILES does not ship")
            });
        assert_eq!(
            format!("{:x}", Sha256::digest(contents.as_bytes())),
            digest,
            "{name} does not match its SHA256SUMS entry"
        );
        listed.insert(name.to_owned());
    }
    let shipped: BTreeSet<_> = shared_vectors::FILES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    assert_eq!(listed, shipped, "every shipped file must have a checksum");
}

#[test]
fn published_vectors_use_placeholder_account_ids_and_credentials() {
    const PLACEHOLDER_ACCOUNT_IDS: &[&str] =
        &["acct-vault", "acct-top", "acct-nested", "org-first"];

    for vector in load_vectors("account_ids.json") {
        let name = vector["name"].as_str().expect("vector name");
        for id in [
            vector["vault_account_id"].as_str(),
            vector
                .pointer("/expected/account_id")
                .and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        .filter(|id| !id.trim().is_empty())
        {
            assert!(
                PLACEHOLDER_ACCOUNT_IDS.contains(&id),
                "{name}: non-placeholder account id {id}"
            );
        }

        let token = vector["access_token"].as_str().expect("access_token");
        if token == "not-a-jwt" {
            continue;
        }
        let parts: Vec<_> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "{name}: token must be a synthetic JWT");
        assert_eq!(
            parts[0], "eyJhbGciOiJub25lIn0",
            "{name}: token is not synthetic"
        );
        assert_eq!(
            parts[2], "sig",
            "{name}: token signature must be a placeholder"
        );
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap_or_else(|error| panic!("{name}: invalid synthetic JWT payload: {error}"));
        let claims: Value = serde_json::from_slice(&payload)
            .unwrap_or_else(|error| panic!("{name}: invalid synthetic JWT claims: {error}"));
        let claim_ids = [
            claims.get("chatgpt_account_id").and_then(Value::as_str),
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .and_then(Value::as_str),
        ];
        for id in claim_ids.into_iter().flatten() {
            assert!(
                PLACEHOLDER_ACCOUNT_IDS.contains(&id),
                "{name}: non-placeholder token account id {id}"
            );
        }
        if let Some(organizations) = claims.get("organizations").and_then(Value::as_array) {
            for id in organizations
                .iter()
                .filter_map(|organization| organization.get("id"))
                .filter_map(Value::as_str)
            {
                assert!(
                    PLACEHOLDER_ACCOUNT_IDS.contains(&id),
                    "{name}: non-placeholder organization id {id}"
                );
            }
        }
    }

    for vector in load_vectors("responses.json") {
        let name = vector["name"].as_str().expect("vector name");
        let body = vector["body"].as_str().expect("body");
        for (start, _) in body.match_indices("sk-") {
            let token = body[start..]
                .split(|character: char| {
                    !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '*'))
                })
                .next()
                .unwrap_or_default();
            assert!(
                token.contains("***********"),
                "{name}: unredacted API token {token}"
            );
        }
    }
}
