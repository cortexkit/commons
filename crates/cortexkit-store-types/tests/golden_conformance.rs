//! Conformance against the SUPERVISOR'S data-home resolution rules.
//!
//! The golden fixture is authored in subconscious next to `default_data_home`
//! (`crates/subc-core/tests/golden/data_home_resolution.json`) and vendored
//! here byte-identically; the daemon asserts the same rows against its own
//! resolver. A row failing HERE means this crate diverged from the authority
//! and any module using it resolves a different directory than the descriptor
//! its supervisor serves (the 2026-08 Windows divergence: no APPDATA arm, so a
//! supervised module's self-resolved path split from the daemon's on Windows
//! always).

use cortexkit_store_types::{module_store_path, resolve_data_home};

const VARS: [&str; 4] = ["XDG_DATA_HOME", "APPDATA", "USERPROFILE", "HOME"];

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("golden/data_home_resolution.json"))
        .expect("vendored golden parses")
}

fn platform_matches(p: &str) -> bool {
    p == "any" || p == if cfg!(windows) { "windows" } else { "unix" }
}

/// Apply one case's env under a saved/restored snapshot; returns resolver output.
fn with_case_env<T>(case: &serde_json::Value, f: impl FnOnce() -> T) -> T {
    let saved: Vec<(&str, Option<std::ffi::OsString>)> =
        VARS.iter().map(|v| (*v, std::env::var_os(v))).collect();
    for v in VARS {
        std::env::remove_var(v);
    }
    for (k, v) in case["env"].as_object().expect("env map") {
        std::env::set_var(k, v.as_str().expect("env value"));
    }
    let out = f();
    for (k, v) in saved {
        match v {
            Some(val) => std::env::set_var(k, val),
            None => std::env::remove_var(k),
        }
    }
    out
}

// One test rather than per-row tests: env is process-global and integration
// tests in one binary run threaded; a single test is the serialization.
#[test]
fn resolver_matches_supervisor_golden_fixture() {
    let doc = fixture();
    let mut ran = 0usize;

    for case in doc["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        if !platform_matches(case["platform"].as_str().expect("platform")) {
            continue;
        }
        let got = with_case_env(case, resolve_data_home);
        assert_eq!(
            got,
            case["expect"].as_str().expect("expect"),
            "golden case '{name}' diverged from the supervisor's rule"
        );
        ran += 1;
    }

    for case in doc["composed"].as_array().expect("composed") {
        let name = case["name"].as_str().expect("name");
        if !platform_matches(case["platform"].as_str().expect("platform")) {
            continue;
        }
        let module_id = case["module_id"].as_str().expect("module_id");
        let got = with_case_env(case, || module_store_path(module_id));
        assert_eq!(
            got,
            case["expect_store"].as_str().expect("expect_store"),
            "composed golden case '{name}' diverged"
        );
        ran += 1;
    }

    // Vacuity floor: the three 'any' rows, this platform's rows, and the
    // composed row must all have run.
    assert!(
        ran >= 7,
        "only {ran} golden rows ran; fixture or filter broken"
    );
}
