use super::{failures, outcome, Fixture};
use cortexkit_doctor::FindingId;
use cortexkit_doctor_conformance::{run, Liveness, Outcome};
use std::collections::HashSet;
use std::fs;

const RULES: [&str; 4] = [
    "fix.stale_digest",
    "fix.module_running",
    "fix.no_terminal",
    "fix.no_change_on_refusal",
];

fn with_fixes(mode: &str) -> Fixture {
    let mut fixture = Fixture::new(mode);
    fixture.non_prompting = true;
    fixture.fixable = Some(FindingId::new("history.backlog").unwrap());
    fixture.needs_stop = Some(FindingId::new("history.compact").unwrap());
    fixture.liveness = Some(Liveness::PidFile("nested/doctor.pid".into()));
    fs::write(fixture.program.with_file_name("observe-fixes"), "").unwrap();
    fixture
}

fn invocations(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.program.with_file_name("observe-fixes")).unwrap()
}

#[test]
fn correct_fixture_passes_every_rule_with_fresh_homes_and_either_signal() {
    for liveness in [
        Liveness::PidFile("nested/doctor.pid".into()),
        Liveness::FlockFile("nested/doctor.lock".into()),
    ] {
        let mut fixture = with_fixes("correct");
        fixture.liveness = Some(liveness);
        let runs = run(&mut fixture);
        assert_eq!(runs.len(), 8);
        for rule in runs {
            assert_eq!(rule.outcome, Outcome::Passed, "{}", rule.name);
        }
        let seen = invocations(&fixture);
        assert_eq!(seen.lines().count(), 3);
        assert_eq!(seen.lines().collect::<HashSet<_>>().len(), 3);
        assert_eq!(fixture.seeded.len(), 6);
        assert_eq!(fixture.seeded.iter().collect::<HashSet<_>>().len(), 6);
        let seeded: HashSet<_> = fixture
            .seeded
            .iter()
            .map(|(data, config)| format!("{}\t{}", data.display(), config.display()))
            .collect();
        for homes in seen.lines() {
            assert!(seeded.contains(homes));
        }
    }
}

#[test]
fn no_fix_is_invoked_without_a_non_prompting_declaration() {
    let mut fixture = with_fixes("correct");
    fixture.non_prompting = false;
    let runs = run(&mut fixture);
    assert!(invocations(&fixture).is_empty());
    for rule in RULES {
        assert_eq!(
            outcome(&runs, rule),
            Outcome::Skipped {
                reason: "non_prompting_not_declared"
            },
            "{rule}"
        );
    }
}

#[test]
fn missing_inputs_and_failed_prerequisite_skip_fixes() {
    let mut missing = with_fixes("correct");
    missing.fixable = None;
    missing.liveness = None;
    let runs = run(&mut missing);
    assert!(invocations(&missing).is_empty());
    for rule in &RULES[..3] {
        assert_eq!(
            outcome(&runs, rule),
            Outcome::Skipped {
                reason: "missing_adapter_input"
            }
        );
    }
    assert_eq!(
        outcome(&runs, RULES[3]),
        Outcome::Skipped {
            reason: "no_fix_run"
        }
    );

    let mut invalid = with_fixes("invalid-check");
    let runs = run(&mut invalid);
    assert!(invocations(&invalid).is_empty());
    assert_eq!(
        failures(&runs),
        [("output.valid", Outcome::Failed { reason: "not_json" })]
    );
    for rule in RULES {
        assert_eq!(
            outcome(&runs, rule),
            Outcome::Skipped {
                reason: "prerequisite_failed"
            }
        );
    }
}

#[test]
fn named_ids_must_be_present_and_fixable_and_needs_stop_must_be_true() {
    let bad_input = Outcome::Failed {
        reason: "bad_adapter_input",
    };
    for (fixable, needs_stop) in [
        ("history.missing", "history.missing"),
        ("history.info", "history.info"),
        ("history.info", "history.backlog"),
    ] {
        let mut fixture = with_fixes("correct");
        fixture.fixable = Some(FindingId::new(fixable).unwrap());
        fixture.needs_stop = Some(FindingId::new(needs_stop).unwrap());
        let runs = run(&mut fixture);
        assert_eq!(
            failures(&runs),
            [
                (RULES[0], bad_input),
                (RULES[1], bad_input),
                (RULES[2], bad_input)
            ]
        );
        assert!(invocations(&fixture).is_empty());
        assert_eq!(
            outcome(&runs, RULES[3]),
            Outcome::Skipped {
                reason: "no_fix_run"
            }
        );
    }
}

#[test]
fn module_running_skips_when_stand_in_hides_or_changes_the_plan() {
    for mode in ["store-in-use", "different-plan-while-running"] {
        let mut fixture = with_fixes(mode);
        let runs = run(&mut fixture);
        assert!(failures(&runs).is_empty());
        assert_eq!(
            outcome(&runs, RULES[1]),
            Outcome::Skipped {
                reason: "setup_not_observable"
            }
        );
        assert_eq!(outcome(&runs, RULES[3]), Outcome::Passed);
        assert_eq!(invocations(&fixture).lines().count(), 2);
    }
}

#[test]
fn rejects_faulty_programs_by_refusal_rule() {
    for (mode, rule) in [
        ("ignores-stale", RULES[0]),
        ("confirmation-before-preflight", RULES[1]),
        ("applies-without-terminal", RULES[2]),
        ("fails-with-no-terminal-code", RULES[2]),
    ] {
        let runs = run(&mut with_fixes(mode));
        assert_eq!(
            failures(&runs),
            [(
                rule,
                Outcome::Failed {
                    reason: "unexpected_fix_result"
                }
            )],
            "{mode}"
        );
        assert_eq!(outcome(&runs, RULES[3]), Outcome::Passed, "{mode}");
    }
}

#[test]
fn every_fix_is_measured_even_on_expected_refusal_or_crash() {
    let changed = Outcome::Failed {
        reason: "home_changed",
    };
    for mode in [
        "mutates-before-stale",
        "mutates-before-running",
        "mutates-before-no-terminal",
    ] {
        let runs = run(&mut with_fixes(mode));
        assert_eq!(failures(&runs), [(RULES[3], changed)], "{mode}");
        for rule in &RULES[..3] {
            assert_eq!(outcome(&runs, rule), Outcome::Passed, "{mode} {rule}");
        }
    }
    let runs = run(&mut with_fixes("mutates-before-crash"));
    assert_eq!(
        failures(&runs),
        [
            (RULES[2], Outcome::Failed { reason: "crashed" }),
            (RULES[3], changed)
        ]
    );
}
