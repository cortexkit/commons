//! The role's vectors under `test-vectors/classifier-v1/`, read through the
//! crate's `vectors` module (the `test-vectors` feature). Every file is
//! decoded into the crate's types and re-encoded, and the encoding must equal
//! the file byte for byte: a member a type drops, renames, reorders or
//! reformats (a number included) shows up here. Each file then has its
//! semantic checks: a provider shape that does not match the role-level shape
//! it is wrapped in, or a refusal the crate's constructors would word
//! differently, fails too.

use cortexkit_role_classifier::{
    describe::{check_describe, Price, RoleDescribe},
    errors::{self, ErrorClass, ItemError, Refusal},
    identity::BatchIdentity,
    provider::{
        self, item_code_for_status, workers_ai_item_code, ProviderRequest, ProviderResponse,
    },
    question::{check_question, QuestionType},
    reply::{check_reply, sum_usage, Answer, ClassifyReply, Usage},
    request::{check_request, CatalogRow, ClassifyRequest},
    vectors, IndexMap,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Number;

/// Every vector file this test reads. A test checks this list against
/// `vectors::FILES`, which `tests/vectors_feature.rs` checks against the
/// directory, so a new file cannot land without a test reading it.
const FILES: &[&str] = &[
    "admission-refusals.json",
    "field-paths.json",
    "image-identity.json",
    "item-errors.json",
    "provider-examples.json",
    "question-choice.json",
    "question-noul.json",
    "question-score.json",
    "replay-batch-id-reuse.json",
    "replay-retry.json",
    "role-describe.json",
    "state-array.json",
    "state-object.json",
    "workers-ai-codes.json",
];

/// Decode `name` as `T` and check that its pretty encoding, with a final
/// newline, is the file's exact bytes.
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
struct ProviderExamples {
    about: String,
    clef_request: ProviderRequest,
    typesafe_answers: IndexMap<String, Answer>,
    typesafe_usage: Usage,
}

#[derive(Deserialize, Serialize)]
struct QuestionVectors {
    about: String,
    vectors: Vec<QuestionVector>,
}

#[derive(Deserialize, Serialize)]
struct QuestionVector {
    provider: String,
    provider_request: ProviderRequest,
    provider_response: ProviderResponse,
    request: ClassifyRequest,
    reply: ClassifyReply,
}

#[derive(Deserialize, Serialize)]
struct AdmissionRefusals {
    about: String,
    refusals: Vec<AdmissionRefusal>,
}

#[derive(Deserialize, Serialize)]
struct AdmissionRefusal {
    name: String,
    request: ClassifyRequest,
    refusal: Refusal,
}

#[derive(Deserialize, Serialize)]
struct ItemErrors {
    about: String,
    vectors: Vec<ItemErrorVector>,
}

#[derive(Deserialize, Serialize)]
struct ItemErrorVector {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_status: Option<u16>,
    request: ClassifyRequest,
    reply: ClassifyReply,
}

#[derive(Deserialize, Serialize)]
struct ReplayRetry {
    about: String,
    price: Price,
    request: ClassifyRequest,
    first_reply: ClassifyReply,
    first_provider_calls: u64,
    retry_request: ClassifyRequest,
    retry_reply: ClassifyReply,
    retry_provider_calls: u64,
}

#[derive(Deserialize, Serialize)]
struct ReplayReuse {
    about: String,
    request: ClassifyRequest,
    reply: ClassifyReply,
    reuse_request: ClassifyRequest,
    refusal: Refusal,
}

#[derive(Deserialize, Serialize)]
struct DescribeVector {
    about: String,
    describe: RoleDescribe,
}

fn describe() -> RoleDescribe {
    exact::<DescribeVector>("role-describe.json").describe
}

fn num(value: f64) -> Number {
    Number::from_f64(value).unwrap()
}

#[test]
fn every_vector_file_on_disk_is_listed() {
    // `vectors::FILES` matches the directory exactly; `tests/vectors_feature.rs`
    // checks that.
    let mut on_disk: Vec<String> = vectors::FILES
        .iter()
        .map(|(name, _)| name.to_string())
        .filter(|name| name.ends_with(".json"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = FILES.iter().map(|s| s.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed);
}

/// The file round-trips byte for byte (`exact`) and passes `check_describe`,
/// including the entry whose catalog states no exact context: its
/// `context_tokens` decodes as absent and stays absent when re-encoded.
#[test]
fn role_describe_lists_three_models_one_without_images_one_without_context() {
    let describe = describe();
    assert_eq!(check_describe(&describe), Ok(()));
    assert_eq!(describe.models.len(), 3);
    let clef = describe.model(provider::catalog_ids::CLEF).unwrap();
    assert_eq!(clef.images.map(|images| images.max), Some(4));
    assert_eq!(clef.egress_host, provider::hosts::CLOUDFLARE);
    assert_eq!(clef.context_tokens, Some(8192));
    let jev = describe.model(provider::catalog_ids::JEV_1_13_0).unwrap();
    assert_eq!(jev.images, None);
    assert_eq!(jev.egress_host, provider::hosts::TYPESAFE);
    assert_eq!(jev.context_tokens, Some(4096));
    let flash = describe.model(provider::catalog_ids::CLEF_FLASH).unwrap();
    assert_eq!(flash.context_tokens, None);
    let text = vectors::ROLE_DESCRIBE_JSON;
    assert!(text.contains("\"images\": null"));
    assert_eq!(text.matches("\"context_tokens\"").count(), 2);
}

#[test]
fn provider_examples_are_the_documented_shapes() {
    let examples: ProviderExamples = exact("provider-examples.json");
    assert_eq!(examples.clef_request.model, provider::models::CLEF);
    let kinds: Vec<_> = examples
        .clef_request
        .questions
        .values()
        .map(|question| question.kind.clone())
        .collect();
    assert_eq!(
        kinds,
        [
            QuestionType::Noul,
            QuestionType::Choice,
            QuestionType::Score
        ]
    );
    for (id, question) in &examples.clef_request.questions {
        check_question(id, question).unwrap();
    }
    for (kind, answer) in &examples.typesafe_answers {
        assert_eq!(answer.kind.as_str(), kind);
        assert!(
            answer.extra.is_empty(),
            "{kind} has members the crate does not name"
        );
    }
    assert_eq!(examples.typesafe_answers["noul"].noul, Some(num(0.95)));
    assert_eq!(examples.typesafe_answers["score"].score, Some(num(1.05)));
    assert_eq!(examples.typesafe_usage.input_tokens, Some(307));
}

/// A provider vector's role-level request and reply wrap its provider
/// request and response: the same questions and state, the provider's own
/// model value, the answers and usage passed through untouched.
fn check_wrapping(file: &str, expected_type: Option<QuestionType>) {
    let file_vectors: QuestionVectors = exact(file);
    let describe = describe();
    let mut providers = Vec::new();
    for vector in &file_vectors.vectors {
        let name = format!("{file} {}", vector.provider);
        providers.push(vector.provider.clone());
        assert_eq!(vector.request.model, vector.provider, "{name}");
        let provider_model = match vector.provider.as_str() {
            provider::catalog_ids::CLEF => provider::models::CLEF,
            provider::catalog_ids::JEV_1_13_0 => provider::models::JEV_LATEST,
            other => panic!("{name}: unknown provider {other}"),
        };
        assert_eq!(vector.provider_request.model, provider_model, "{name}");
        assert_eq!(vector.provider_response.model, provider_model, "{name}");
        let model = describe.model(&vector.request.model).unwrap();
        check_request(&vector.request, model).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(vector.request.items.len(), 1, "{name}");
        assert_eq!(
            vector.provider_request.state, vector.request.items[0].state,
            "{name}"
        );
        assert_eq!(
            vector.provider_request.questions, vector.request.questions,
            "{name}"
        );
        if let Some(kind) = &expected_type {
            for question in vector.request.questions.values() {
                assert_eq!(&question.kind, kind, "{name}");
            }
        }
        check_reply(&vector.reply, &vector.request).unwrap_or_else(|e| panic!("{name}: {e}"));
        let item = &vector.reply.items[0];
        assert_eq!(
            item.answers.as_ref(),
            Some(&vector.provider_response.answers),
            "{name}"
        );
        assert_eq!(item.usage, vector.provider_response.usage, "{name}");
        assert_eq!(vector.reply.usage, sum_usage(&vector.reply.items), "{name}");
        assert_eq!(
            vector.reply.cost_usd, None,
            "{name}: unpriced in these vectors"
        );
    }
    if expected_type.is_some() {
        assert_eq!(
            providers,
            [
                provider::catalog_ids::CLEF,
                provider::catalog_ids::JEV_1_13_0
            ],
            "{file}: one vector per provider"
        );
    }
}

#[test]
fn noul_vectors_wrap_the_provider_shapes() {
    check_wrapping("question-noul.json", Some(QuestionType::Noul));
}

#[test]
fn choice_vectors_wrap_the_provider_shapes() {
    check_wrapping("question-choice.json", Some(QuestionType::Choice));
}

#[test]
fn score_vectors_wrap_the_provider_shapes() {
    check_wrapping("question-score.json", Some(QuestionType::Score));
    let vectors: QuestionVectors = exact("question-score.json");
    for vector in &vectors.vectors {
        let answer = &vector.reply.items[0].answers.as_ref().unwrap()[0];
        let levels = match &vector.request.questions[0].criteria {
            Some(cortexkit_role_classifier::question::Criteria::Levels(levels)) => levels.len(),
            other => panic!("score criteria is not levels: {other:?}"),
        };
        let legend = answer.legend.as_ref().unwrap();
        let keys: Vec<String> = (0..levels).map(|i| i.to_string()).collect();
        assert_eq!(legend.keys().cloned().collect::<Vec<_>>(), keys);
        let score = answer.score.as_ref().unwrap().as_f64().unwrap();
        assert!((0.0..=(levels - 1) as f64).contains(&score));
    }
}

#[test]
fn object_and_array_states_are_forwarded_verbatim() {
    check_wrapping("state-object.json", None);
    check_wrapping("state-array.json", None);
    let object: QuestionVectors = exact("state-object.json");
    assert!(object.vectors[0].request.items[0].state.is_object());
    let array: QuestionVectors = exact("state-array.json");
    assert!(array.vectors[0].request.items[0].state.is_array());
}

#[test]
fn one_admission_refusal_per_code_as_the_crate_builds_it() {
    let file: AdmissionRefusals = exact("admission-refusals.json");
    for code in errors::ADMISSION_CODES {
        assert!(
            file.refusals
                .iter()
                .any(|vector| vector.refusal.code == *code),
            "{code} has no admission refusal vector"
        );
    }
    assert!(file
        .refusals
        .iter()
        .any(|vector| vector.name == "model_not_classifier_kindless"));
    let describe = describe();
    for vector in &file.refusals {
        let refusal = &vector.refusal;
        assert!(
            errors::ADMISSION_CODES.contains(&refusal.code.as_str()),
            "{}",
            vector.name
        );
        assert_eq!(
            Some(refusal.detail.class.clone()),
            errors::admission_class(&refusal.code),
            "{}",
            vector.name
        );
        let built = match vector.name.as_str() {
            errors::INVALID_PARAMS => {
                let model = describe.model(&vector.request.model).unwrap();
                check_request(&vector.request, model).unwrap_err()
            }
            errors::BATCH_ID_REUSE => Refusal::batch_id_reuse("items[0]"),
            errors::MODEL_UNKNOWN => {
                assert!(describe.model(&vector.request.model).is_none());
                cortexkit_role_classifier::request::check_catalog_row(
                    &vector.request.model,
                    CatalogRow::Absent,
                )
                .unwrap_err()
            }
            "model_not_classifier" => cortexkit_role_classifier::request::check_catalog_row(
                &vector.request.model,
                CatalogRow::Kind("chat"),
            )
            .unwrap_err(),
            "model_not_classifier_kindless" => {
                cortexkit_role_classifier::request::check_catalog_row(
                    &vector.request.model,
                    CatalogRow::Kindless,
                )
                .unwrap_err()
            }
            errors::COST_EXCEEDED => {
                assert_eq!(vector.request.max_cost_usd, Some(num(0.25)));
                Refusal::cost_exceeded(num(0.42), num(0.25))
            }
            errors::ACCOUNT_WALLED => {
                let refusal = Refusal::account_walled();
                let detail = (*refusal.detail)
                    .clone()
                    .with_resets_at_ms(1_767_225_600_000);
                refusal.with_detail(detail)
            }
            errors::BATCH_IN_PROGRESS => Refusal::batch_in_progress(1000),
            errors::AUTH_FAILED => {
                let refusal = Refusal::auth_failed();
                let detail = (*refusal.detail).clone().with_credential("apikey:typesafe");
                refusal.with_detail(detail)
            }
            errors::MODEL_UNAVAILABLE => {
                assert!(describe.model(&vector.request.model).is_some());
                Refusal::model_unavailable(&vector.request.model)
            }
            other => panic!("unexpected refusal {other}"),
        };
        assert_eq!(&built, refusal, "{}", vector.name);
    }
}

#[test]
fn one_item_error_per_code_beside_an_answered_item() {
    let file: ItemErrors = exact("item-errors.json");
    let names: Vec<&str> = file.vectors.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, errors::ITEM_CODES);
    for vector in &file.vectors {
        let name = &vector.name;
        check_reply(&vector.reply, &vector.request).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(vector.reply.items[0].answers.is_some(), "{name}");
        let error = vector.reply.items[1].error.as_ref().unwrap();
        assert_eq!(&error.code, name);
        assert_eq!(
            Some(error.class.clone()),
            errors::item_class(name),
            "{name}"
        );
        if let Some(status) = vector.provider_status {
            assert_eq!(item_code_for_status(status), Some(name.as_str()), "{name}");
        }
        // A code that stops the call leaves every later item unsent,
        // carrying the same code; other codes leave the batch going.
        for later in &vector.reply.items[2..] {
            let later_error = later.error.as_ref().unwrap();
            assert!(error.stops_the_call(), "{name}: an item after the error");
            assert_eq!(later_error.code, error.code, "{name}");
            assert_eq!(later_error.class, error.class, "{name}");
            assert_eq!(later.usage, None, "{name}: an unsent item reports no usage");
        }
        if matches!(
            name.as_str(),
            errors::AUTH_FAILED | errors::MODEL_UNAVAILABLE
        ) {
            assert!(!error.is_stored(), "{name}");
            assert!(vector.reply.items.len() > 2, "{name}: shows the stop");
        } else {
            assert!(error.is_stored(), "{name}");
        }
        let mut built = ItemError::for_code(name, error.message.clone());
        if let Some(ms) = error.retry_after_ms {
            built = built.with_retry_after_ms(ms);
        }
        assert_eq!(&built, error, "{name}");
        assert_eq!(vector.reply.usage, sum_usage(&vector.reply.items), "{name}");
        assert_eq!(
            error.is_retried_on_resend(),
            error.class == ErrorClass::Transient,
            "{name}"
        );
    }
}

#[test]
fn a_retry_returns_the_stored_answers_and_the_same_cost() {
    let file: ReplayRetry = exact("replay-retry.json");
    assert_eq!(file.retry_request, file.request);
    assert_eq!(file.retry_reply, file.first_reply);
    assert_eq!(file.first_provider_calls, file.request.items.len() as u64);
    assert_eq!(file.retry_provider_calls, 0);
    check_reply(&file.first_reply, &file.request).unwrap();
    let cost = file
        .first_reply
        .cost_usd
        .as_ref()
        .expect("a priced batch has cost_usd");
    assert_eq!(file.retry_reply.cost_usd.as_ref(), Some(cost));
    // cost_usd is the batch total at the vector's price.
    let input = file
        .price
        .input_per_mtok_usd
        .as_ref()
        .unwrap()
        .as_f64()
        .unwrap();
    let output = file
        .price
        .output_per_mtok_usd
        .as_ref()
        .unwrap()
        .as_f64()
        .unwrap();
    let usage = file.first_reply.usage;
    let total = usage.input_tokens.unwrap() as f64 * input / 1e6
        + usage.output_tokens.unwrap() as f64 * output / 1e6;
    assert!((cost.as_f64().unwrap() - total).abs() < 1e-12);
}

#[test]
fn a_reuse_is_refused_naming_the_differing_field() {
    let file: ReplayReuse = exact("replay-batch-id-reuse.json");
    assert_eq!(file.reuse_request.batch_id, file.request.batch_id);
    check_reply(&file.reply, &file.request).unwrap();
    let recorded = BatchIdentity::of(&file.request).unwrap();
    let field = recorded
        .first_difference(&BatchIdentity::of(&file.reuse_request).unwrap())
        .unwrap();
    assert_eq!(file.refusal, Refusal::batch_id_reuse(field));
    assert_eq!(file.refusal.field(), Some("items[1]"));
}

#[test]
fn refusals_name_the_narrowest_field_that_applies() {
    let file: AdmissionRefusals = exact("field-paths.json");
    let names: Vec<&str> = file.refusals.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["unknown_question_type", "narrower_path"]);
    let describe = describe();
    for vector in &file.refusals {
        let name = &vector.name;
        let model = describe.model(&vector.request.model).unwrap();
        let built = check_request(&vector.request, model).unwrap_err();
        assert_eq!(&built, &vector.refusal, "{name}");
        assert_eq!(built.code, errors::INVALID_PARAMS, "{name}");
        let expected = match name.as_str() {
            "unknown_question_type" => {
                // The request decoded: the type is kept open for decoding,
                // and only the validator refuses it.
                assert_eq!(
                    vector.request.questions["priority"].kind,
                    QuestionType::Other("rank".into())
                );
                "questions.priority.type"
            }
            "narrower_path" => "items[1].images[1]",
            other => panic!("unexpected vector {other}"),
        };
        assert_eq!(built.field(), Some(expected), "{name}");
    }
}

#[derive(Deserialize, Serialize)]
struct ImageIdentity {
    about: String,
    request: ClassifyRequest,
    resend: ClassifyRequest,
    malformed: AdmissionRefusalCase,
}

#[derive(Deserialize, Serialize)]
struct AdmissionRefusalCase {
    request: ClassifyRequest,
    refusal: Refusal,
}

/// One image sent as a `data:` URL and re-sent as `{content_type, base64}`
/// is one batch; malformed base64 is refused before any provider call with
/// the code and field any other bad image gets.
#[test]
fn one_image_in_both_encodings_is_one_identity_and_bad_base64_is_refused() {
    let file: ImageIdentity = exact("image-identity.json");
    let describe = describe();
    let model = describe.model(&file.request.model).unwrap();
    check_request(&file.request, model).unwrap();
    check_request(&file.resend, model).unwrap();
    assert_eq!(file.resend.batch_id, file.request.batch_id);
    assert_ne!(file.resend.items, file.request.items, "two spellings");
    let recorded = BatchIdentity::of(&file.request).unwrap();
    assert_eq!(
        recorded.first_difference(&BatchIdentity::of(&file.resend).unwrap()),
        None
    );
    let model = describe.model(&file.malformed.request.model).unwrap();
    let built = check_request(&file.malformed.request, model).unwrap_err();
    let pinned = &file.malformed.refusal;
    assert_eq!(built.code, pinned.code);
    assert_eq!(built.detail, pinned.detail);
    assert_eq!(built.code, errors::INVALID_PARAMS);
    assert_eq!(built.field(), Some("items[0].images[0]"));
    // The words after the colon are the base64 decoder's own.
    let prefix = "the image's base64 does not decode: ";
    assert!(built.message.starts_with(prefix), "{}", built.message);
    assert!(pinned.message.starts_with(prefix), "{}", pinned.message);
}

#[derive(Deserialize, Serialize)]
struct WorkersAiCodes {
    about: String,
    clef: Vec<CodeRow>,
    typesafe: Vec<CodeRow>,
    first_call_refusals: Vec<Refusal>,
}

#[derive(Deserialize, Serialize)]
struct CodeRow {
    status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<u64>,
    item_code: String,
    class: ErrorClass,
    stored: bool,
    stops_the_call: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    was: Option<String>,
}

fn check_row(provider: &str, row: &CodeRow, mapped: Option<&str>) {
    let name = format!("{provider} {} {:?}", row.status, row.code);
    assert_eq!(mapped, Some(row.item_code.as_str()), "{name}");
    assert_eq!(
        errors::item_class(&row.item_code),
        Some(row.class.clone()),
        "{name}"
    );
    let error = ItemError::for_code(&row.item_code, "m");
    assert_eq!(error.is_stored(), row.stored, "{name}: stored");
    assert_eq!(error.stops_the_call(), row.stops_the_call, "{name}: stops");
}

/// clef's errors map by Workers AI code first; `was` is what the 0.1.0
/// status-only mapping gave, where it differs. TypeSafe's are unchanged.
#[test]
fn clef_errors_map_by_workers_ai_code_and_typesafe_by_status() {
    let file: WorkersAiCodes = exact("workers-ai-codes.json");
    for row in &file.clef {
        check_row("clef", row, workers_ai_item_code(row.status, row.code));
        let before = item_code_for_status(row.status);
        match &row.was {
            Some(was) => {
                assert_eq!(before, Some(was.as_str()));
                assert_ne!(was, &row.item_code);
            }
            None => assert_eq!(before, Some(row.item_code.as_str())),
        }
    }
    for code in [3036, 5035, 5016, 3023, 5007, 3042] {
        assert!(
            file.clef.iter().any(|row| row.code == Some(code)),
            "clef code {code} has no vector"
        );
    }
    for row in &file.typesafe {
        assert_eq!(row.code, None);
        assert_eq!(row.was, None);
        check_row("typesafe", row, item_code_for_status(row.status));
    }
    let built = [
        Refusal::account_limit_exhausted(),
        Refusal::account_not_entitled(),
    ];
    assert_eq!(file.first_call_refusals, built);
    for refusal in &file.first_call_refusals {
        assert!(errors::BATCH_LEVEL_CODES.contains(&refusal.code.as_str()));
        assert_eq!(
            Some(refusal.detail.class.clone()),
            errors::admission_class(&refusal.code)
        );
    }
}
