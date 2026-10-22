//! The role's vectors under `test-vectors/extractor-v1/`, read through the
//! crate's `vectors` module (the `test-vectors` feature). Every file is
//! decoded and re-encoded, and the encoding must equal the file byte for
//! byte; then each file is checked against the code that states its rule.

use cortexkit_role_extractor::{
    describe::{check_describe, RoleDescribe},
    errors::{self, ErrorClass, ItemError, Refusal},
    field::{types, Field, FieldType},
    identity::BatchIdentity,
    provider::{clairic_limits, models, read_reply, ProviderRequest},
    reply::{span_byte_range, Answer, ExtractReply, ItemResult},
    request::{admit, estimated_reply_bytes, CatalogRow, ExtractRequest, REPLY_FRAME_BYTES},
    spend::{check_before_call, item_cost_input, usd_to_nanos, ModuleBounds, Rounding},
    vectors, IndexMap,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{value::RawValue, Value};

const FILES: &[&str] = &[
    "admission-refusals.json",
    "answer-too-large.json",
    "field-paths.json",
    "identity.json",
    "provider-replies.json",
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
    /// The estimate's input tokens, for an item that is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    estimated_input_tokens: Option<u64>,
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
            Outcome::Sent => {
                let estimate = decided
                    .unwrap_or_else(|e| panic!("{}: {e:?}", case.name))
                    .unwrap_or_else(|| panic!("{}: no estimate", case.name));
                assert_eq!(
                    Some(estimate.input_tokens),
                    case.estimated_input_tokens,
                    "{}",
                    case.name
                );
                assert_eq!(estimate.max_passes, case.request.fields.len() as u64);
            }
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

#[derive(Deserialize, Serialize)]
struct ProviderReplies {
    about: String,
    replies: Vec<ReplyCase>,
}

#[derive(Deserialize, Serialize)]
struct ReplyCase {
    name: String,
    status: u16,
    /// How many fields the probe's request sent.
    fields: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sent: Option<Sent>,
    body: String,
    item: ItemResult,
}

/// The probe's request, as role fields and a state.
#[derive(Deserialize, Serialize)]
struct Sent {
    state: String,
    fields: IndexMap<String, Field>,
}

fn replies() -> ProviderReplies {
    exact("provider-replies.json")
}

fn reply<'a>(file: &'a ProviderReplies, name: &str) -> &'a ReplyCase {
    file.replies
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("no reply {name}"))
}

fn answers(case: &ReplyCase) -> &IndexMap<String, Answer> {
    case.item.answers.as_ref().expect("an answered item")
}

fn text(raw: Option<&RawValue>) -> &str {
    raw.map(RawValue::get).unwrap_or("<absent>")
}

#[test]
fn every_provider_reply_reads_as_its_item() {
    let file = replies();
    assert_eq!(file.replies.len(), 12, "every probe request has its reply");
    for case in &file.replies {
        let read = match read_reply(case.status, &case.body) {
            Ok(result) => result.into_item(0),
            Err(error) => ItemResult::failed(0, error),
        };
        assert_eq!(read, case.item, "{}", case.name);
        // `IndexMap` equality ignores order, so compare the text too.
        assert_eq!(
            serde_json::to_string(&read).unwrap(),
            serde_json::to_string(&case.item).unwrap(),
            "{}",
            case.name
        );
    }
}

#[test]
fn every_success_bills_the_schema_once_and_the_state_per_pass() {
    let file = replies();
    for case in file.replies.iter().filter(|case| case.status == 200) {
        let usage = case.item.usage.as_ref().expect("a success reports usage");
        let count =
            |n: Option<u64>| n.unwrap_or_else(|| panic!("{}: a count is absent", case.name));
        let passes = count(usage.passes);
        assert_eq!(
            count(usage.input_tokens),
            count(usage.schema_tokens)
                + passes * count(usage.state_tokens)
                + count(usage.image_tokens),
            "{}",
            case.name
        );
        assert!(
            passes >= 1 && passes <= case.fields,
            "{}: {passes} passes",
            case.name
        );
        assert_eq!(usage.output_tokens, Some(0), "{}", case.name);
    }
    // 40 fields ran in 40 passes, whichever order they were sent in.
    let forward = reply(&file, "05-multi-pass");
    let reversed = reply(&file, "07-multi-pass-reversed");
    assert_eq!(forward.item.usage, reversed.item.usage);
    assert_eq!(forward.item.usage.as_ref().unwrap().passes, Some(40));
    assert_eq!(answers(forward), answers(reversed));
    assert_eq!(
        reply(&file, "01-all-types")
            .item
            .usage
            .as_ref()
            .unwrap()
            .passes,
        Some(2)
    );
    // One 64x64 PNG on probe 06's request: 64 image tokens, 41 schema tokens.
    let plain = reply(&file, "06-one-pass-control")
        .item
        .usage
        .as_ref()
        .unwrap();
    let image = reply(&file, "12-one-image").item.usage.as_ref().unwrap();
    assert_eq!(image.image_tokens, Some(64));
    assert_eq!(
        image.schema_tokens.unwrap() - plain.schema_tokens.unwrap(),
        41
    );
}

#[test]
fn every_answer_type_has_its_measured_shape() {
    let file = replies();
    let all = answers(reply(&file, "01-all-types"));
    let kinds: Vec<&str> = all.values().map(|answer| answer.kind.as_str()).collect();
    assert_eq!(kinds, types::ALL, "probe 01 answers every type, in order");
    for answer in all.values() {
        assert!(answer.extra.is_empty(), "{}: unnamed members", answer.kind);
    }
    assert_eq!(
        text(all["inspection_start"].value.as_deref()),
        r#""09:15:00""#
    );
    assert_eq!(
        text(all["restarted_at"].value.as_deref()),
        r#""2026-03-06T16:40:00""#
    );
    assert_eq!(text(all["downtime"].unit.as_deref()), r#""minutes""#);
    assert_eq!(text(all["technician"].span.as_deref()), "[60,71]");
    for name in [
        "flow_rate",
        "inspection_date",
        "inspection_start",
        "restarted_at",
        "downtime",
    ] {
        assert!(all[name].span.is_none(), "{name} has no span");
        assert!(
            all[name].p_null.is_some() && all[name].confidence.is_some(),
            "{name}"
        );
    }
    let estimate = &all["seal_life_months"];
    assert!(estimate.p_null.is_none() && estimate.confidence.is_none());
    let quantiles: Vec<Value> = serde_json::from_str(text(estimate.quantiles.as_deref())).unwrap();
    assert_eq!(quantiles.len(), 99);
    let forecast = &all["throughput_forecast"];
    let steps: Vec<IndexMap<String, Value>> =
        serde_json::from_str(text(forecast.steps.as_deref())).unwrap();
    assert_eq!(steps.len(), 3, "horizon 3");
    for step in &steps {
        let members: Vec<&str> = step.keys().map(String::as_str).collect();
        assert_eq!(
            members,
            [
                "value",
                "mean",
                "median",
                "p10",
                "p90",
                "quantiles",
                "p_below",
                "p_above"
            ]
        );
        assert_eq!(step["quantiles"].as_array().unwrap().len(), 99);
    }
    for parallel in [
        &forecast.value,
        &forecast.median,
        &forecast.p10,
        &forecast.p90,
    ] {
        let values: Vec<Value> = serde_json::from_str(text(parallel.as_deref())).unwrap();
        assert_eq!(values.len(), 3);
    }
    // `decimals: 1` on a list of numbers is applied, with a confidence per
    // element.
    let rainfall = &answers(reply(&file, "03-list-decimals"))["rainfall_mm"];
    assert_eq!(text(rainfall.value.as_deref()), "[2.3,0.1,11.8,4.5,7.2]");
    assert!(rainfall.confidences.is_some());
    // A date estimate with the month step answers ISO dates throughout.
    let date = &answers(reply(&file, "04-date-estimate"))["completion_date"];
    assert_eq!(text(date.value.as_deref()), r#""2026-05-01""#);
    let dates: Vec<String> = serde_json::from_str(text(date.quantiles.as_deref())).unwrap();
    assert_eq!(dates.len(), 99);
    assert!(dates
        .iter()
        .all(|d| cortexkit_role_extractor::request::is_calendar_date(d)));
}

#[test]
fn spans_count_scalar_values_and_state_text_is_the_state() {
    let file = replies();
    for name in ["01-all-types", "02-span-offsets"] {
        let case = reply(&file, name);
        let sent = case.sent.as_ref().unwrap();
        assert_eq!(
            case.item.state_text.as_deref(),
            Some(sent.state.as_str()),
            "{name}"
        );
    }
    let case = reply(&file, "02-span-offsets");
    let state = &case.sent.as_ref().unwrap().state;
    // Read the body again rather than trusting the stored item, so this
    // checks what `read_reply` and `into_item` forward.
    let item = read_reply(case.status, &case.body).unwrap().into_item(0);
    let answer = &item.answers.as_ref().unwrap()["tracking_code"];
    let span = answer.span.as_deref().unwrap();
    assert_eq!(span.get(), "[53,61]", "forwarded as the provider wrote it");
    assert_eq!(item.state_text.as_deref(), Some(state.as_str()));
    let range = span_byte_range(state, span).unwrap();
    assert_eq!(range, 65..73);
    assert_eq!(&state[range], "QK-58213");
}

#[test]
fn what_clairic_refuses_is_refused_at_admission_and_maps_to_invalid_item() {
    let file = replies();
    let describe = describe();
    let catalog = [CatalogEntry {
        model: describe.models[0].model.clone(),
        kind: Some("extraction".into()),
    }];
    for (name, status, code, path) in [
        ("08-unknown-type", 400, 5006, "fields.desk_colour.type"),
        ("09-unknown-option", 422, 3030, "fields.desk_price.decimalz"),
        ("11-empty-state", 422, 3030, "items[0].state"),
    ] {
        let case = reply(&file, name);
        assert_eq!(case.status, status);
        let sent = case.sent.as_ref().unwrap();
        let request = serde_json::json!({
            "batch_id": "probe",
            "model": describe.models[0].model,
            "fields": sent.fields,
            "items": [{"state": sent.state}],
        });
        let refusal = admitted(&describe, &catalog, &request).unwrap_err();
        assert_eq!(refusal.code, errors::INVALID_PARAMS, "{name}");
        assert_eq!(refusal.field(), Some(path), "{name}");
        let error = case.item.error.as_ref().unwrap();
        assert_eq!(error.code, errors::INVALID_ITEM, "{name}");
        assert_eq!(error.class, ErrorClass::Permanent, "{name}");
        assert!(
            error.message.contains(&format!("Workers AI code {code}")),
            "{name}"
        );
        assert!(
            case.item.usage.is_none(),
            "{name}: an error reports no usage"
        );
        assert!(case.body.contains(r#""result":{}"#), "{name}");
    }
}
