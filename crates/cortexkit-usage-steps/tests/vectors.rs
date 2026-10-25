//! Every vector decodes and round-trips byte for byte, every file is indexed
//! in `cases.json` and pinned by `SHA256SUMS`, and the refusal and value files
//! cover every code and value this version knows.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cortexkit_usage_steps::{
    codes, Notice, ReadReply, ReadRequest, Refusal, RequestSize, StepRow, StepSource,
    SubscribeRequest, Verdict,
};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/usage-steps-v1")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(dir().join(rel)).unwrap_or_else(|err| panic!("{rel}: {err}"))
}

fn cases() -> serde_json::Value {
    serde_json::from_str(&read("cases.json")).unwrap()
}

fn entries<'a>(cases: &'a serde_json::Value, list: &str) -> Vec<&'a serde_json::Value> {
    cases[list].as_array().unwrap().iter().collect()
}

fn recode<T: Serialize + DeserializeOwned>(text: &str) -> Result<String, serde_json::Error> {
    let value: T = serde_json::from_str(text)?;
    serde_json::to_string(&value)
}

/// Decodes `text` as the named contract type and re-encodes it.
fn recode_as(ty: &str, text: &str) -> Result<String, serde_json::Error> {
    match ty {
        "SubscribeRequest" => recode::<SubscribeRequest>(text),
        "Notice" => recode::<Notice>(text),
        "ReadRequest" => recode::<ReadRequest>(text),
        "ReadReply" => recode::<ReadReply>(text),
        "StepRow" => recode::<StepRow>(text),
        "Refusal" => recode::<Refusal>(text),
        "Verdict" => recode::<Verdict>(text),
        "RequestSize" => recode::<RequestSize>(text),
        "StepSource" => recode::<StepSource>(text),
        other => panic!("cases.json names an unknown type {other}"),
    }
}

fn files_under(root: &Path, here: &Path, out: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(here).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_str().unwrap();
            out.insert(rel.replace('\\', "/"));
        }
    }
}

fn all_files() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    files_under(&dir(), &dir(), &mut out);
    out
}

#[test]
fn every_round_trip_vector_decodes_and_re_encodes_to_the_same_bytes() {
    let cases = cases();
    let round_trip = entries(&cases, "round_trip");
    assert!(round_trip.len() >= 30, "round_trip lost cases");
    for case in round_trip {
        let file = case["file"].as_str().unwrap();
        let ty = case["type"].as_str().unwrap();
        let text = read(file);
        let again = recode_as(ty, &text).unwrap_or_else(|err| panic!("{file}: {err}"));
        assert_eq!(again, text, "{file} did not round-trip as {ty}");
    }
}

#[test]
fn every_tolerant_input_decodes_and_re_encodes_as_its_canonical_vector() {
    let cases = cases();
    for case in entries(&cases, "normalize") {
        let file = case["file"].as_str().unwrap();
        let ty = case["type"].as_str().unwrap();
        let again = recode_as(ty, &read(file)).unwrap_or_else(|err| panic!("{file}: {err}"));
        let expected = read(case["encodes_as"].as_str().unwrap());
        assert_eq!(again, expected, "{file}");
        assert_ne!(read(file), expected, "{file} is not a different input");
    }
}

#[test]
fn every_rejected_input_fails_to_decode_or_the_provider_refuses_it() {
    let cases = cases();
    for case in entries(&cases, "reject") {
        let file = case["file"].as_str().unwrap();
        let ty = case["type"].as_str().unwrap();
        let decoded = recode_as(ty, &read(file));
        // A request naming only one of `harness` and `session` decodes, as a
        // request with any `limit` does; the contract has the provider refuse
        // it instead, and the case says so with `decodes: true`.
        let decodes = case["decodes"].as_bool().unwrap_or(false);
        assert_eq!(
            decoded.is_ok(),
            decodes,
            "{file} as {ty}: the contract refuses it {}, but this crate {}",
            if decodes {
                "at the provider"
            } else {
                "undecodable"
            },
            if decoded.is_ok() {
                "decoded it"
            } else {
                "did not decode it"
            }
        );
        // A refusal code must be one the operation it names may send, so
        // check each request against its own operation's list, not the union
        // of both: the union hid a subscribe code missing from its list.
        if let Some(code) = case.get("provider_code") {
            let own: &[&str] = match ty {
                "SubscribeRequest" => codes::SUBSCRIBE_REFUSALS,
                "ReadRequest" => codes::READ_REFUSALS,
                other => {
                    panic!("{file}: provider_code on a {other} case, which no operation refuses")
                }
            };
            let code = code.as_str().unwrap();
            assert!(
                own.contains(&code),
                "{file}: {code} is not among the {ty} refusals {own:?}"
            );
        }
    }
}

#[test]
fn cases_json_indexes_every_vector_file_exactly_once() {
    let cases = cases();
    let mut indexed = Vec::new();
    for list in ["round_trip", "normalize", "reject"] {
        for case in entries(&cases, list) {
            indexed.push(case["file"].as_str().unwrap().to_owned());
        }
    }
    let unique: BTreeSet<String> = indexed.iter().cloned().collect();
    assert_eq!(unique.len(), indexed.len(), "a file is indexed twice");
    let mut on_disk = all_files();
    for meta in ["README.md", "SHA256SUMS", "cases.json"] {
        assert!(on_disk.remove(meta), "{meta} is missing");
    }
    assert_eq!(on_disk, unique);
}

#[test]
fn sha256sums_pins_every_file() {
    let sums = read("SHA256SUMS");
    let mut listed = BTreeSet::new();
    for line in sums.lines() {
        let (hash, file) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(dir().join(file)).unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)), "{file}");
        assert!(listed.insert(file.to_owned()), "{file} listed twice");
    }
    let mut on_disk = all_files();
    on_disk.remove("SHA256SUMS");
    assert_eq!(listed, on_disk);
}

#[test]
fn every_row_vector_with_reasoning_and_output_has_reasoning_at_most_output() {
    let cases = cases();
    let mut checked = 0;
    for list in ["round_trip", "normalize", "reject"] {
        for case in entries(&cases, list) {
            let file = case["file"].as_str().unwrap();
            let value: serde_json::Value = serde_json::from_str(&read(file)).unwrap();
            let rows: Vec<&serde_json::Value> = match case["type"].as_str().unwrap() {
                "StepRow" => vec![&value],
                "ReadReply" => value["rows"].as_array().into_iter().flatten().collect(),
                _ => continue,
            };
            for row in rows {
                let (Some(reasoning), Some(output)) =
                    (row["reasoning"].as_u64(), row["output"].as_u64())
                else {
                    continue;
                };
                checked += 1;
                assert!(
                    reasoning <= output,
                    "{file}: row {} {} has reasoning {reasoning} above output {output}; \
                     reasoning is a breakdown of output",
                    row["harness"],
                    row["step_ref"],
                );
                if let Ok(decoded) = serde_json::from_value::<StepRow>(row.clone()) {
                    assert_eq!(decoded.check_counts(), Ok(()), "{file}");
                }
            }
        }
    }
    assert!(checked >= 5, "too few rows carry both counts: {checked}");
}

fn stems(subdir: &str, prefix: &str) -> BTreeSet<String> {
    all_files()
        .into_iter()
        .filter_map(|file| {
            file.strip_prefix(&format!("{subdir}/{prefix}"))?
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn refusal_and_ending_vectors_cover_exactly_the_contract_codes() {
    let refusals: BTreeSet<String> = codes::SUBSCRIBE_REFUSALS
        .iter()
        .chain(codes::READ_REFUSALS)
        .map(|code| code.to_string())
        .collect();
    assert_eq!(stems("refusals", ""), refusals);
    let endings: BTreeSet<String> = codes::SUBSCRIBE_ENDINGS
        .iter()
        .map(|code| code.to_string())
        .collect();
    assert_eq!(stems("endings", ""), endings);
    for code in refusals.iter().chain(&endings) {
        let dir = if endings.contains(code) {
            "endings"
        } else {
            "refusals"
        };
        let refusal: Refusal = serde_json::from_str(&read(&format!("{dir}/{code}.json"))).unwrap();
        assert_eq!(&refusal.code, code);
    }
}

fn value_stems(prefix: &str, known: &[&str]) {
    let mut expected: BTreeSet<String> = known.iter().map(|v| v.to_string()).collect();
    expected.insert("future-value".to_owned());
    assert_eq!(stems("values", prefix), expected, "{prefix}");
}

#[test]
fn value_vectors_cover_every_known_value_and_one_future_value() {
    value_stems("verdict-", Verdict::KNOWN);
    value_stems("request_size-", RequestSize::KNOWN);
    value_stems("source-", StepSource::KNOWN);
    for (prefix, file_value) in [
        ("verdict-", "cold_start"),
        ("verdict-", "unknown"),
        ("request_size-", "not_smaller"),
        ("source-", "runner"),
    ] {
        let text = read(&format!("values/{prefix}{file_value}.json"));
        assert_eq!(text, format!("\"{file_value}\""));
    }
    let future: Verdict = serde_json::from_str(&read("values/verdict-future-value.json")).unwrap();
    assert!(matches!(future, Verdict::Other(_)));
    let future: RequestSize =
        serde_json::from_str(&read("values/request_size-future-value.json")).unwrap();
    assert!(matches!(future, RequestSize::Other(_)));
    let future: StepSource =
        serde_json::from_str(&read("values/source-future-value.json")).unwrap();
    assert!(matches!(future, StepSource::Other(_)));
}
