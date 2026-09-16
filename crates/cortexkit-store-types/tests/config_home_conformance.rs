//! Conformance against the SUPERVISOR'S config-home resolution rules.
//!
//! The golden fixture is authored in subconscious next to `default_config_home`
//! (`crates/subc-core/tests/golden/config_home_resolution.json`) and vendored
//! here byte-identically; the daemon asserts the same rows against its own
//! resolver. It shares a row shape with the data-home golden on purpose: a
//! divergence between the two ladders (one honouring a variable the other does
//! not) is exactly the class that produced the doubled-path store defect, and a
//! shared harness makes it a fixture diff rather than a runtime surprise.
//!
//! Separate test binary from the data-home conformance because env is
//! process-global and the two suites mutate disjoint variable sets; one binary
//! per variable set is the serialization.

use cortexkit_store_types::resolve_config_home;

const VARS: [&str; 4] = ["XDG_CONFIG_HOME", "APPDATA", "USERPROFILE", "HOME"];

fn platform_matches(p: &str) -> bool {
    p == "any" || p == if cfg!(windows) { "windows" } else { "unix" }
}

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

#[test]
fn config_home_resolver_matches_supervisor_golden_fixture() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("golden/config_home_resolution.json"))
            .expect("vendored golden parses");
    let mut ran = 0usize;

    for case in doc["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        if !platform_matches(case["platform"].as_str().expect("platform")) {
            continue;
        }
        let got = with_case_env(case, resolve_config_home);
        assert_eq!(
            got,
            case["expect"].as_str().expect("expect"),
            "golden case '{name}' diverged from the supervisor's rule"
        );
        ran += 1;
    }

    // Vacuity floor: 'any' rows plus this platform's rows must both run.
    assert!(
        ran >= 6,
        "only {ran} golden cases ran; fixture or filter broken"
    );
}
