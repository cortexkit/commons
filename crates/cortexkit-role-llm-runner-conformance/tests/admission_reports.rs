use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, Major, RoleDescribe},
    PROVIDES, REQUIRED_OPS,
};
use cortexkit_role_llm_runner_conformance::Capability;

#[test]
fn admission_reports_token_round_trips_without_routed_accounts() {
    assert_eq!(capabilities::ADMISSION_REPORTS, "admission_reports");
    let answer = RoleDescribe::new(
        vec![Major::new(
            PROVIDES,
            REQUIRED_OPS.iter().map(|op| (*op).into()).collect(),
            "alpha",
        )],
        "example",
    )
    .with_capabilities(vec![capabilities::ADMISSION_REPORTS.into()])
    .with_session_capabilities_from("baseline");
    let wire = serde_json::to_value(&answer).unwrap();
    assert_eq!(
        wire["capabilities"],
        serde_json::json!(["admission_reports"])
    );
    assert_eq!(check_describe(&wire).unwrap(), answer);
    assert_eq!(
        serde_json::from_value::<RoleDescribe>(wire).unwrap(),
        answer
    );
    assert_eq!(
        Capability::AdmissionReports.group(),
        Some("admission_reports")
    );
    assert!(capabilities::GROUPS
        .iter()
        .any(|(name, ops)| *name == "admission_reports" && ops.is_empty()));
}
