//! The role's vectors under `test-vectors/extractor-v1/`, read through the
//! crate's `vectors` module (the `test-vectors` feature). Every file is
//! decoded and re-encoded, and the encoding must equal the file byte for
//! byte; then each file is checked against the code that states its rule.

use cortexkit_role_extractor::{
    describe::{check_describe, RoleDescribe},
    errors::{self, ErrorClass, ItemError, Refusal},
    field::{types, FieldType},
    identity::BatchIdentity,
    provider::{clairic_limits, models, ProviderRequest},
    reply::ExtractReply,
    request::{admit, estimated_reply_bytes, CatalogRow, ExtractRequest, REPLY_FRAME_BYTES},
    spend::{check_before_call, item_cost_input, usd_to_nanos, ModuleBounds, Rounding},
    vectors,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{value::RawValue, Value};

const FILES: &[&str] = &[
    "admission-refusals.json",
    "answer-too-large.json",
    "field-paths.json",
    "identity.json",
    "reply-size.json",
    "requests.json",
    "role-describe.json",
    "spend.json",
];

fn exact<T: DeserializeOwned + Serialize>(name: &str) -> T {
    assert!(FILES.contains(&name), "{name} is not listed in FILES");
    let text = vectors::file(name).unwrap_or_else(|| panic!("{name} is not in vectors::FILES"));
    let decoded: T = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let encoded = serde_json::to_string_pretty(&decoded).unwrap() + "\n";
    if encoded != text {
        let line = encoded
            .lines()
            .zip(text.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{name}: re-encoding differs from the file at line {}:\n  file:    {:?}\n  encoded: {:?}",
            line + 1,
            text.lines().nth(line),
            encoded.lines().nth(line)
        );
    }
    decoded
}

#[derive(Deserialize, Serialize)]
struct DescribeVector {
    about: String,
    describe: RoleDescribe,
}

fn describe() -> RoleDescribe {
    exact::<DescribeVector>("role-describe.json").describe
}

#[derive(Deserialize, Serialize)]
struct CatalogEntry {
    model: String,
    kind: Option<String>,
}

fn row_of<'a>(catalog: &'a [CatalogEntry], model: &str) -> CatalogRow<'a> {
    match catalog.iter().find(|entry| entry.model == model) {
        None => CatalogRow::Absent,
        Some(CatalogEntry { kind: None, .. }) => CatalogRow::Kindless,
        Some(CatalogEntry {
            kind: Some(kind), ..
        }) => CatalogRow::Kind(kind),
    }
}

/// Run every admission step on `request`, as the text a caller sends.
fn admitted(
    describe: &RoleDescribe,
    catalog: &[CatalogEntry],
    request: &impl Serialize,
) -> Result<ExtractRequest, Refusal> {
    let text = serde_json::to_string(request).unwrap();
    admit(
        &text,
        describe,
        |model| row_of(catalog, model),
        clairic_limits::MAX_BODY_BYTES,
    )
}

#[test]
fn every_vector_file_is_listed() {
    let mut listed: Vec<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    listed.sort();
    assert_eq!(listed, FILES);
}

#[test]
fn role_describe_states_clairic_with_free_output_and_no_context() {
    let describe = describe();
    assert_eq!(check_describe(&describe), Ok(()));
    let clairic = &describe.models[0];
    assert_eq!(clairic.pass_tokens, clairic_limits::PASS_TOKENS);
    let all: Vec<FieldType> = types::ALL.iter().map(|t| FieldType::parse(t)).collect();
    assert_eq!(clairic.field_types, all);
    let images = clairic.images.as_ref().unwrap();
    assert_eq!(images.max_bytes_each, clairic_limits::MAX_IMAGE_BYTES);
    assert_eq!(
        images.max_bytes_total,
        clairic_limits::MAX_IMAGES_TOTAL_BYTES
    );
    let price = clairic.price.as_ref().unwrap();
    assert_eq!(
        price.output_per_mtok_usd.as_ref().unwrap().as_u64(),
        Some(0)
    );
    assert_eq!(clairic.context_tokens, None);
    assert!(vectors::ROLE_DESCRIBE_JSON.contains("\"output_per_mtok_usd\": 0"));
}

#[derive(Deserialize, Serialize)]
struct Refusals {
    about: String,
    catalog: Vec<CatalogEntry>,
    refusals: Vec<RefusalCase>,
}

#[derive(Deserialize, Serialize)]
struct RefusalCase {
    name: String,
    request: Value,
    refusal: Refusal,
}

#[test]
fn every_path_refusal_is_what_admission_builds() {
    let file: Refusals = exact("field-paths.json");
    let describe = describe();
    for case in &file.refusals {
        let built = admitted(&describe, &file.catalog, &case.request)
            .err()
            .unwrap_or_else(|| panic!("{}: admitted", case.name));
        assert_eq!(
            built,
            case.refusal,
            "{}: built {}",
            case.name,
            serde_json::to_string(&built).unwrap()
        );
        assert_eq!(built.code, errors::INVALID_PARAMS, "{}", case.name);
    }
}

#[test]
fn one_admission_refusal_per_code_and_the_kindless_row() {
    let file: Refusals = exact("admission-refusals.json");
    let describe = describe();
    let mut expected: Vec<&str> = errors::ADMISSION_CODES.to_vec();
    let at = expected
        .iter()
        .position(|code| *code == errors::MODEL_NOT_EXTRACTOR)
        .unwrap();
    expected.insert(at + 1, "model_not_extractor_kindless");
    let names: Vec<&str> = file
        .refusals
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(names, expected);
    for case in &file.refusals {
        let name = case.name.as_str();
        let refusal = &case.refusal;
        assert_eq!(
            Some(refusal.detail.class.clone()),
            errors::admission_class(&refusal.code),
            "{name}"
        );
        let admitted = admitted(&describe, &file.catalog, &case.request);
        let built = match name {
            errors::INVALID_PARAMS
            | errors::MODEL_UNKNOWN
            | errors::MODEL_NOT_EXTRACTOR
            | "model_not_extractor_kindless" => admitted.unwrap_err(),
            other => {
                // Decided by state the request does not show, so the
                // request itself is admitted.
                let request = admitted.unwrap_or_else(|e| panic!("{other}: {e}"));
                match other {
                    errors::BATCH_ID_REUSE => Refusal::batch_id_reuse("fields"),
                    errors::ACCOUNT_WALLED => {
                        let refusal = Refusal::account_walled();
                        let detail = (*refusal.detail)
                            .clone()
                            .with_resets_at_ms(1_798_761_600_000);
                        refusal.with_detail(detail)
                    }
                    errors::BATCH_IN_PROGRESS => Refusal::batch_in_progress(1000),
                    errors::AUTH_FAILED => {
                        let refusal = Refusal::auth_failed();
                        let detail = (*refusal.detail)
                            .clone()
                            .with_credential("apikey:cloudflare");
                        refusal.with_detail(detail)
                    }
                    errors::MODEL_UNAVAILABLE => Refusal::model_unavailable(&request.model),
                    errors::ACCOUNT_LIMIT_EXHAUSTED => Refusal::account_limit_exhausted(),
                    errors::ACCOUNT_NOT_ENTITLED => Refusal::account_not_entitled(),
                    other => panic!("unexpected refusal {other}"),
                }
            }
        };
        assert_eq!(&built, refusal, "{name}");
    }
    let kindless = &file.refusals[at + 1].refusal;
    assert_eq!(kindless.code, errors::MODEL_NOT_EXTRACTOR);
    assert_eq!(kindless.detail.kind, None);
}

#[derive(Deserialize, Serialize)]
struct Requests {
    about: String,
    requests: Vec<RequestCase>,
}

#[derive(Deserialize, Serialize)]
struct RequestCase {
    name: String,
    request: ExtractRequest,
    provider_requests: Vec<ProviderRequest>,
}

#[test]
fn each_item_maps_to_one_clairic_request_with_every_field() {
    let file: Requests = exact("requests.json");
    let describe = describe();
    let catalog = [CatalogEntry {
        model: describe.models[0].model.clone(),
        kind: Some("extraction".into()),
    }];
    let mut types_seen = std::collections::BTreeSet::new();
    for case in &file.requests {
        let name = &case.name;
        let admitted =
            admitted(&describe, &catalog, &case.request).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(admitted, case.request, "{name}");
        assert_eq!(case.provider_requests.len(), case.request.items.len());
        for (index, body) in case.provider_requests.iter().enumerate() {
            let mapped = ProviderRequest::for_item(&case.request, index, models::CLAIRIC).unwrap();
            assert_eq!(&mapped, body, "{name} item {index}");
            assert_eq!(
                body.fields.keys().collect::<Vec<_>>(),
                case.request.fields.keys().collect::<Vec<_>>(),
                "{name}: fields keep the caller's order"
            );
        }
        for field in case.request.fields.values() {
            types_seen.insert(field.kind.as_str().to_owned());
        }
    }
    assert_eq!(
        types_seen.len(),
        types::ALL.len(),
        "every type has a vector"
    );
}

#[derive(Deserialize, Serialize)]
struct Identity {
    about: String,
    cases: Vec<IdentityCase>,
}

#[derive(Deserialize, Serialize)]
struct IdentityCase {
    name: String,
    recorded: ExtractRequest,
    sent: ExtractRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    difference: Option<String>,
}

#[test]
fn identity_names_the_first_differing_part() {
    let file: Identity = exact("identity.json");
    for case in &file.cases {
        // `IndexMap` equality ignores order, so compare the text.
        assert_ne!(
            serde_json::to_string(&case.recorded).unwrap(),
            serde_json::to_string(&case.sent).unwrap(),
            "{}: two different bodies",
            case.name
        );
        let recorded = BatchIdentity::of(&case.recorded).unwrap();
        let sent = BatchIdentity::of(&case.sent).unwrap();
        assert_eq!(
            recorded.first_difference(&sent),
            case.difference,
            "{}",
            case.name
        );
    }
}

#[derive(Deserialize, Serialize)]
struct ReplySize {
    about: String,
    at_the_frame: ExtractRequest,
    over_the_frame: ExtractRequest,
    refusal: Refusal,
}

#[test]
fn a_reply_estimated_past_the_frame_is_refused() {
    let file: ReplySize = exact("reply-size.json");
    let describe = describe();
    let catalog = [CatalogEntry {
        model: describe.models[0].model.clone(),
        kind: Some("extraction".into()),
    }];
    let model = &describe.models[0];
    assert_eq!(
        estimated_reply_bytes(&file.at_the_frame, model),
        REPLY_FRAME_BYTES
    );
    admitted(&describe, &catalog, &file.at_the_frame).unwrap();
    assert_eq!(
        estimated_reply_bytes(&file.over_the_frame, model),
        REPLY_FRAME_BYTES + 1
    );
    let refusal = admitted(&describe, &catalog, &file.over_the_frame).unwrap_err();
    assert_eq!(refusal, file.refusal);
    assert_eq!(refusal.field(), Some("items"));
}

#[derive(Deserialize, Serialize)]
struct Spend {
    about: String,
    bounds: Bounds,
    cases: Vec<SpendCase>,
}

#[derive(Deserialize, Serialize)]
struct Bounds {
    image_tokens_each: u64,
    overhead_tokens: u64,
}

#[derive(Deserialize, Serialize)]
struct SpendCase {
    name: String,
    request: ExtractRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exact_state_tokens: Option<u64>,
    outcome: Outcome,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Sent,
    Error(ItemError),
}

#[test]
fn the_pre_call_check_sends_or_refuses_each_item() {
    let file: Spend = exact("spend.json");
    let describe = describe();
    let model = &describe.models[0];
    let rate = model
        .price
        .as_ref()
        .and_then(|price| price.input_per_mtok_usd.as_ref())
        .and_then(|usd| usd_to_nanos(usd, Rounding::Up))
        .map(|nanos| i64::try_from(nanos).unwrap());
    assert_eq!(rate, Some(110_000_000));
    let bounds = ModuleBounds {
        image_tokens_each: file.bounds.image_tokens_each,
        overhead_tokens: file.bounds.overhead_tokens,
    };
    for case in &file.cases {
        let input = item_cost_input(
            &case.request,
            0,
            model,
            bounds,
            case.exact_state_tokens,
            None,
        )
        .unwrap();
        let ceiling = case
            .request
            .max_cost_usd
            .as_ref()
            .map(|usd| usd_to_nanos(usd, Rounding::Down).unwrap());
        let decided = check_before_call(&input, rate, 0, ceiling);
        match &case.outcome {
            Outcome::Sent => assert!(decided.is_ok(), "{}: {decided:?}", case.name),
            Outcome::Error(error) => {
                assert_eq!(decided.as_ref().err(), Some(error), "{}", case.name);
                assert_eq!(error.class, ErrorClass::Permanent);
                assert!(error.is_stored());
            }
        }
    }
}

#[derive(Deserialize, Serialize)]
struct AnswerTooLarge {
    about: String,
    request: ExtractRequest,
    reply: ExtractReply,
}

#[test]
fn an_answer_over_the_bound_is_a_stored_item_error() {
    let file: AnswerTooLarge = exact("answer-too-large.json");
    let describe = describe();
    let bound = describe.models[0].max_answer_bytes;
    let error = file.reply.items[1].error.as_ref().unwrap();
    assert_eq!(
        error,
        &ItemError::answer_too_large("fields.summary", bound, bound + 577)
    );
    assert!(error.is_stored());
    assert!(!error.stops_the_call());
    let answers = file.reply.items[0].answers.as_ref().unwrap();
    assert_eq!(
        answers["vendor"].p_null.as_deref().map(RawValue::get),
        Some("0.010"),
        "number text is kept as written"
    );
    assert_eq!(
        answers["summary"].value.as_deref().map(RawValue::get),
        Some("null")
    );
    for (index, item) in file.reply.items.iter().enumerate() {
        assert_eq!(item.index, index as u64);
        assert!(item.answers.is_some() != item.error.is_some());
    }
    assert_eq!(file.reply.items.len(), file.request.items.len());
}
