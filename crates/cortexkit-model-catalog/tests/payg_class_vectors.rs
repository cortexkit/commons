//! Validate the frozen PAYG classification-vector corpus without classifying it.
//!
//! Any classifier implementation MUST execute this suite through `run_vectors`; a
//! classifier that does not is nonconforming. This crate owns the corpus shape, while
//! the classifier's home owns execution of the matrix outcomes.
//! The qwen schedules retain only the live rates needed by their cells, not a complete
//! models.dev record; context bands are represented by `tiers` in this crate.

use std::collections::BTreeSet;

use cortexkit_model_catalog::PaygVectorSuite;
use serde::Deserialize;

const VECTORS: &str = include_str!("golden/payg-class-vectors.json");

const MATRIX_CELLS: &[&str] = &[
    "resolves_to/priced",
    "resolves_to/all-zero/target-priced",
    "resolves_to/all-zero/target-all-none",
    "resolves_to/all-zero/target-all-zero",
    "resolves_to/all-zero/target-absent",
    "resolves_to/all-none/target-priced",
    "resolves_to/all-none/target-all-none",
    "resolves_to/all-none/target-all-zero",
    "resolves_to/all-none/target-absent",
    "resolves_to/absent/target-priced",
    "resolves_to/absent/target-all-none",
    "resolves_to/absent/target-all-zero",
    "resolves_to/absent/target-absent",
    "overrides_unpriced/priced",
    "overrides_unpriced/all-zero",
    "overrides_unpriced/all-none",
    "overrides_unpriced/absent",
    "not_sold_per_token/priced",
    "not_sold_per_token/all-zero",
    "not_sold_per_token/all-none",
    "not_sold_per_token/absent",
    "zeros_are_not_prices/priced",
    "zeros_are_not_prices/all-zero",
    "zeros_are_not_prices/all-none",
    "zeros_are_not_prices/absent",
    "no_declaration/priced",
    "no_declaration/all-zero",
    "no_declaration/all-none",
    "no_declaration/absent",
];

const LEGAL_OUTCOMES: &[&str] = &[
    "priced",
    "not_sold_per_token",
    "target_not_in_catalog",
    "target_not_priceable",
    "declaration_superseded",
    "no_entry",
];

const CELL_CONTRACT: &[(&str, &str)] = &[
    ("resolves_to/priced", "declaration_superseded"),
    ("resolves_to/all-zero/target-priced", "priced"),
    (
        "resolves_to/all-zero/target-all-none",
        "target_not_priceable",
    ),
    (
        "resolves_to/all-zero/target-all-zero",
        "target_not_priceable",
    ),
    (
        "resolves_to/all-zero/target-absent",
        "target_not_in_catalog",
    ),
    ("resolves_to/all-none/target-priced", "priced"),
    (
        "resolves_to/all-none/target-all-none",
        "target_not_priceable",
    ),
    (
        "resolves_to/all-none/target-all-zero",
        "target_not_priceable",
    ),
    (
        "resolves_to/all-none/target-absent",
        "target_not_in_catalog",
    ),
    ("resolves_to/absent/target-priced", "priced"),
    ("resolves_to/absent/target-all-none", "target_not_priceable"),
    ("resolves_to/absent/target-all-zero", "target_not_priceable"),
    ("resolves_to/absent/target-absent", "target_not_in_catalog"),
    ("overrides_unpriced/priced", "declaration_superseded"),
    ("overrides_unpriced/all-zero", "priced"),
    ("overrides_unpriced/all-none", "priced"),
    ("overrides_unpriced/absent", "priced"),
    ("not_sold_per_token/priced", "declaration_superseded"),
    ("not_sold_per_token/all-zero", "not_sold_per_token"),
    ("not_sold_per_token/all-none", "not_sold_per_token"),
    ("not_sold_per_token/absent", "not_sold_per_token"),
    ("zeros_are_not_prices/priced", "no_entry"),
    ("zeros_are_not_prices/all-zero", "not_sold_per_token"),
    ("zeros_are_not_prices/all-none", "not_sold_per_token"),
    ("zeros_are_not_prices/absent", "not_sold_per_token"),
    ("no_declaration/priced", "no_entry"),
    ("no_declaration/all-zero", "no_entry"),
    ("no_declaration/all-none", "no_entry"),
    ("no_declaration/absent", "no_entry"),
];

#[derive(Debug, Deserialize)]
struct RawVectorSuite {
    vectors: Vec<RawVector>,
}

#[derive(Debug, Deserialize)]
struct RawVector {
    cell: String,
    expected: String,
}

#[test]
fn classification_vectors_are_a_complete_well_formed_matrix_corpus() {
    let vectors: PaygVectorSuite = serde_json::from_str(VECTORS)
        .expect("every classification vector parses through the public conformance type");
    let raw: RawVectorSuite = serde_json::from_str(VECTORS)
        .expect("read classification vector cell references and outcome names");

    let failures = validation_failures(&raw.vectors, vectors.vectors.len());
    assert!(
        failures.is_empty(),
        "PAYG classification vector corpus is malformed:\n{}",
        failures.join("\n")
    );
}

#[test]
fn cell_reference_guard_rejects_an_unknown_target_state() {
    let vectors = vec![raw_vector(
        "overrides_unpriced/all-zero/target-priced",
        "priced",
    )];

    assert!(validate_cell_references(&vectors).is_err());
}

#[test]
fn legal_outcome_guard_rejects_a_prefix_of_a_real_outcome() {
    let vectors = vec![raw_vector("no_declaration/absent", "priced-but-not-legal")];

    assert!(validate_expected_outcomes(&vectors).is_err());
}

#[test]
fn cell_contract_guard_rejects_a_legal_but_wrong_target_state_outcome() {
    let vectors = vec![raw_vector(
        "resolves_to/all-zero/target-all-zero",
        "declaration_superseded",
    )];

    assert!(validate_cell_contract(&vectors).is_err());
}

#[test]
fn coverage_guard_rejects_a_duplicate_cell() {
    let mut vectors = complete_matrix_vectors();
    vectors.push(raw_vector("no_declaration/absent", "no_entry"));

    assert!(validate_exact_once_coverage(&vectors).is_err());
}

#[test]
fn coverage_guard_rejects_a_missing_cell() {
    let mut vectors = complete_matrix_vectors();
    vectors.pop();

    assert!(validate_exact_once_coverage(&vectors).is_err());
}

#[test]
fn coverage_guard_rejects_an_extra_cell() {
    let mut vectors = complete_matrix_vectors();
    vectors.push(raw_vector("outside-the-matrix", "no_entry"));

    assert!(validate_exact_once_coverage(&vectors).is_err());
}

#[test]
fn validation_diagnostics_collect_independent_failures() {
    let vectors = vec![raw_vector("outside-the-matrix", "not-an-outcome")];

    let failures = validation_failures(&vectors, 0);

    assert_eq!(failures.len(), 5, "{failures:#?}");
    assert!(failures
        .iter()
        .any(|failure| failure.contains("expected 29 vectors")));
    assert!(failures
        .iter()
        .any(|failure| failure.contains("unknown matrix cell")));
    assert!(failures
        .iter()
        .any(|failure| failure.contains("illegal PAYG outcome")));
}

fn validate_cell_references(vectors: &[RawVector]) -> Result<(), String> {
    for vector in vectors {
        if !MATRIX_CELLS.contains(&vector.cell.as_str()) {
            return Err(format!("unknown matrix cell: {}", vector.cell));
        }
    }
    Ok(())
}

fn validation_failures(vectors: &[RawVector], parsed_vector_count: usize) -> Vec<String> {
    let mut failures = Vec::new();
    let matrix_count_matches = vectors.len() == MATRIX_CELLS.len();
    if !matrix_count_matches {
        failures.push(format!(
            "expected {} vectors, found {}",
            MATRIX_CELLS.len(),
            vectors.len()
        ));
    }
    if vectors.len() != parsed_vector_count {
        failures.push(format!(
            "raw fixture has {} vectors but public parsing produced {parsed_vector_count}",
            vectors.len()
        ));
    }
    for validation in [
        validate_cell_references(vectors),
        validate_expected_outcomes(vectors),
        validate_cell_contract(vectors),
    ] {
        if let Err(error) = validation {
            failures.push(error);
        }
    }
    if matrix_count_matches {
        if let Err(error) = validate_exact_once_coverage(vectors) {
            failures.push(error);
        }
    }
    failures
}

fn validate_expected_outcomes(vectors: &[RawVector]) -> Result<(), String> {
    for vector in vectors {
        if !LEGAL_OUTCOMES.contains(&vector.expected.as_str()) {
            return Err(format!("illegal PAYG outcome: {}", vector.expected));
        }
    }
    Ok(())
}

fn validate_cell_contract(vectors: &[RawVector]) -> Result<(), String> {
    for vector in vectors {
        let expected = CELL_CONTRACT
            .iter()
            .find_map(|(cell, expected)| (*cell == vector.cell).then_some(*expected))
            .ok_or_else(|| format!("matrix cell has no expected outcome: {}", vector.cell))?;
        if vector.expected != expected {
            return Err(format!(
                "matrix cell {} requires {expected}, found {}",
                vector.cell, vector.expected
            ));
        }
    }
    Ok(())
}

fn validate_exact_once_coverage(vectors: &[RawVector]) -> Result<(), String> {
    let seen = vectors
        .iter()
        .map(|vector| vector.cell.as_str())
        .collect::<BTreeSet<_>>();

    if vectors.len() != MATRIX_CELLS.len() || seen.len() != vectors.len() {
        return Err("matrix cells are missing or duplicated".into());
    }
    if MATRIX_CELLS.iter().any(|cell| !seen.contains(cell)) {
        return Err("matrix cells are missing".into());
    }
    Ok(())
}

fn complete_matrix_vectors() -> Vec<RawVector> {
    MATRIX_CELLS
        .iter()
        .map(|cell| raw_vector(cell, "no_entry"))
        .collect()
}

fn raw_vector(cell: &str, expected: &str) -> RawVector {
    RawVector {
        cell: cell.into(),
        expected: expected.into(),
    }
}
