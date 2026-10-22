//! The shared vectors under `test-vectors/model-batch-v1/`, read through the
//! crate's `vectors` module (the `test-vectors` feature). Every file is
//! decoded and re-encoded byte for byte, then checked against the code that
//! states the same rule.

use cortexkit_model_batch::{
    errors::{self, ErrorClass, ItemError},
    estimate::{estimate_item, Estimate, EstimateError, ItemCostInput, Tokens},
    identity::item_digest,
    image::{check_image, Image, InlineImage},
    status::{item_code_for_status, workers_ai_item_code},
    vectors,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;

const FILES: &[&str] = &[
    "estimates.json",
    "images.json",
    "status-mapping.json",
    "vocabulary.json",
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

#[test]
fn every_vector_file_is_listed() {
    let mut listed: Vec<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    listed.sort();
    assert_eq!(listed, FILES);
}

#[derive(Deserialize, Serialize)]
struct Vocabulary {
    about: String,
    admission: Vec<AdmissionCode>,
    item: Vec<ItemCode>,
    answer_too_large: ItemError,
}

#[derive(Deserialize, Serialize)]
struct AdmissionCode {
    code: String,
    class: ErrorClass,
}

#[derive(Deserialize, Serialize)]
struct ItemCode {
    code: String,
    class: ErrorClass,
    stored: bool,
    stops_the_call: bool,
}

#[test]
fn the_vocabulary_states_each_class_storage_and_stop() {
    let file: Vocabulary = exact("vocabulary.json");
    let admission: Vec<&str> = file.admission.iter().map(|c| c.code.as_str()).collect();
    assert_eq!(admission, errors::ADMISSION_VOCABULARY);
    for code in &file.admission {
        assert_eq!(
            errors::admission_class(&code.code),
            Some(code.class.clone()),
            "{}",
            code.code
        );
    }
    let item: Vec<&str> = file.item.iter().map(|c| c.code.as_str()).collect();
    assert_eq!(item, errors::ITEM_VOCABULARY);
    for code in &file.item {
        let name = &code.code;
        assert_eq!(errors::item_class(name), Some(code.class.clone()), "{name}");
        let error = ItemError::for_code(name, "m");
        assert_eq!(error.is_stored(), code.stored, "{name}: stored");
        assert_eq!(
            error.stops_the_call(),
            code.stops_the_call,
            "{name}: stops the call"
        );
        assert_eq!(
            errors::is_batch_level(name),
            !code.stored,
            "{name}: batch-level"
        );
    }
    assert_eq!(
        file.answer_too_large,
        ItemError::answer_too_large("fields.summary", 65536, 70000)
    );
}

#[derive(Deserialize, Serialize)]
struct StatusMapping {
    about: String,
    workers_ai: Vec<StatusRow>,
    status_only: Vec<StatusRow>,
}

#[derive(Deserialize, Serialize)]
struct StatusRow {
    status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<u64>,
    item_code: String,
}

#[test]
fn provider_errors_map_to_item_codes() {
    let file: StatusMapping = exact("status-mapping.json");
    for row in &file.workers_ai {
        assert_eq!(
            workers_ai_item_code(row.status, row.code),
            Some(row.item_code.as_str()),
            "Workers AI {} {:?}",
            row.status,
            row.code
        );
    }
    for code in cortexkit_model_batch::status::workers_ai_codes::ALL {
        assert!(
            file.workers_ai.iter().any(|row| row.code == Some(*code)),
            "Workers AI code {code} has no vector"
        );
    }
    for row in &file.status_only {
        assert_eq!(row.code, None);
        assert_eq!(
            item_code_for_status(row.status),
            Some(row.item_code.as_str()),
            "status {}",
            row.status
        );
    }
}

#[derive(Deserialize, Serialize)]
struct Images {
    about: String,
    same_image: Vec<Image>,
    canonical: InlineImage,
    refused: Vec<RefusedImage>,
}

#[derive(Deserialize, Serialize)]
struct RefusedImage {
    image: Image,
    because: String,
}

#[test]
fn one_image_in_any_spelling_is_one_identity_and_malformed_ones_are_refused() {
    let file: Images = exact("images.json");
    let digests: Vec<String> = file
        .same_image
        .iter()
        .map(|image| {
            check_image(image).unwrap();
            assert_eq!(image.canonical().unwrap(), file.canonical);
            let images = vec![image.clone()];
            item_digest(&json!({"state": "s", "images": images}), Some(&images)).unwrap()
        })
        .collect();
    assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
    for refused in &file.refused {
        let why = check_image(&refused.image).unwrap_err();
        if let Some(prefix) = refused.because.strip_suffix(':') {
            assert!(why.starts_with(prefix), "{why}");
        } else {
            assert_eq!(why, refused.because);
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Estimates {
    about: String,
    cases: Vec<EstimateCase>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Count {
    Exact(u64),
    Bound(u64),
}

impl From<&Count> for Tokens {
    fn from(count: &Count) -> Self {
        match count {
            Count::Exact(n) => Tokens::Exact(*n),
            Count::Bound(n) => Tokens::Bound(*n),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct EstimateCase {
    name: String,
    state: Count,
    schema: Count,
    images: u64,
    image_tokens_each: u64,
    overhead_tokens: u64,
    max_passes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pass_tokens: Option<u64>,
    rate_nanos_per_mtok: i64,
    expected: Expected,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Expected {
    Estimate {
        max_passes: u64,
        input_tokens: u64,
        cost_nanos: u64,
    },
    DoesNotFit {
        state_tokens: u64,
        pass_tokens: u64,
    },
    /// At least this many input tokens: a byte-bound case checked against
    /// what the provider was measured to bill for the same request.
    AtLeast {
        input_tokens: u64,
    },
}

#[test]
fn estimates_bound_each_item_and_say_when_they_cannot() {
    let file: Estimates = exact("estimates.json");
    for case in &file.cases {
        let mut input = ItemCostInput::new(
            (&case.state).into(),
            (&case.schema).into(),
            case.image_tokens_each,
            case.overhead_tokens,
        )
        .with_images(case.images)
        .with_max_passes(case.max_passes);
        if let Some(pass_tokens) = case.pass_tokens {
            input = input.with_pass_tokens(pass_tokens);
        }
        let estimated = estimate_item(&input, case.rate_nanos_per_mtok);
        match case.expected {
            Expected::Estimate {
                max_passes,
                input_tokens,
                cost_nanos,
            } => assert_eq!(
                estimated,
                Ok(Estimate::new(max_passes, input_tokens, cost_nanos)),
                "{}",
                case.name
            ),
            Expected::DoesNotFit {
                state_tokens,
                pass_tokens,
            } => assert_eq!(
                estimated,
                Err(EstimateError::DoesNotFit {
                    state_tokens,
                    pass_tokens,
                }),
                "{}",
                case.name
            ),
            Expected::AtLeast { input_tokens } => {
                let estimate = estimated.unwrap_or_else(|e| panic!("{}: {e}", case.name));
                assert!(
                    estimate.input_tokens >= input_tokens,
                    "{}: {} < {input_tokens}",
                    case.name,
                    estimate.input_tokens
                );
            }
        }
    }
}
