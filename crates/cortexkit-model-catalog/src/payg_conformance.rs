use serde::Deserialize;
use serde_json::Value;

use crate::{CatalogDoc, PaygModelId, PaygRemapDoc};

/// One expected classification outcome from the PAYG conformance matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaygOutcome {
    Priced,
    NotSoldPerToken,
    TargetNotInCatalog,
    TargetNotPriceable,
    DeclarationSuperseded,
    NoEntry,
}

/// One classification vector supplied by a conformance corpus.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PaygVector {
    pub name: String,
    pub cell: String,
    #[serde(deserialize_with = "deserialize_remap_doc")]
    pub remap: PaygRemapDoc,
    #[serde(deserialize_with = "deserialize_catalog_doc")]
    pub catalog: CatalogDoc,
    #[serde(deserialize_with = "deserialize_model_id")]
    pub model: PaygModelId,
    pub expected: PaygOutcome,
}

/// A complete, ordered classification-vector corpus.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PaygVectorSuite {
    pub vectors: Vec<PaygVector>,
}

/// One vector whose classifier result differed from the declared expectation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorFailure {
    pub vector: String,
    pub expected: PaygOutcome,
    pub actual: PaygOutcome,
}

/// Execute every vector against the caller's classification implementation.
pub fn run_vectors<F>(vectors: &PaygVectorSuite, classify: F) -> Vec<VectorFailure>
where
    F: Fn(&PaygRemapDoc, &CatalogDoc, &PaygModelId) -> PaygOutcome,
{
    vectors
        .vectors
        .iter()
        .filter_map(|vector| {
            let actual = classify(&vector.remap, &vector.catalog, &vector.model);
            (actual != vector.expected).then(|| VectorFailure {
                vector: vector.name.clone(),
                expected: vector.expected,
                actual,
            })
        })
        .collect()
}

fn deserialize_remap_doc<'de, D>(deserializer: D) -> Result<PaygRemapDoc, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    PaygRemapDoc::parse(&value.to_string()).map_err(serde::de::Error::custom)
}

fn deserialize_catalog_doc<'de, D>(deserializer: D) -> Result<CatalogDoc, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    CatalogDoc::parse(&value.to_string()).map_err(serde::de::Error::custom)
}

fn deserialize_model_id<'de, D>(deserializer: D) -> Result<PaygModelId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let id = String::deserialize(deserializer)?;
    PaygModelId::parse(&id).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use crate::{CatalogDoc, PaygModelId, PaygRemapDoc};

    use super::{run_vectors, PaygOutcome, PaygVector, PaygVectorSuite};

    #[test]
    fn reports_a_mismatch_from_the_caller_supplied_classifier() {
        let vectors = PaygVectorSuite {
            vectors: vec![PaygVector {
                name: "priced-vector".into(),
                cell: "overrides_unpriced/absent".into(),
                remap: PaygRemapDoc::parse(
                    r#"{
                        "schema": 1,
                        "counterfactual": "same_platform_list",
                        "providers": {},
                        "entries": {}
                    }"#,
                )
                .unwrap(),
                catalog: CatalogDoc::parse("{}").unwrap(),
                model: PaygModelId::parse("provider/model").unwrap(),
                expected: PaygOutcome::Priced,
            }],
        };

        assert_eq!(
            run_vectors(&vectors, |_, _, _| PaygOutcome::NoEntry),
            vec![super::VectorFailure {
                vector: "priced-vector".into(),
                expected: PaygOutcome::Priced,
                actual: PaygOutcome::NoEntry,
            }]
        );
    }

    #[test]
    fn parses_vectors_with_their_catalog_and_remap_documents() {
        let suite: PaygVectorSuite = serde_json::from_str(
            r#"{
                "vectors": [{
                    "name": "priced-vector",
                    "cell": "overrides_unpriced/absent",
                    "remap": {
                        "schema": 1,
                        "counterfactual": "same_platform_list",
                        "providers": {},
                        "entries": {}
                    },
                    "catalog": {},
                    "model": "provider/model",
                    "expected": "priced"
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(suite.vectors[0].model.as_str(), "provider/model");
        assert_eq!(suite.vectors[0].expected, PaygOutcome::Priced);
    }
}
