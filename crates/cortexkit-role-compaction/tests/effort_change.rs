use cortexkit_role_compaction::{
    notice::Conditions,
    ride::{RideCause, RideRequest},
};
use serde_json::{json, Value};

#[test]
fn effort_change_is_a_known_cause_that_loses_the_cache() {
    let notice: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-request.json"
    ))
    .unwrap();
    let conditions: Conditions =
        serde_json::from_value(notice["params"]["conditions"].clone()).unwrap();
    let cause: RideCause = serde_json::from_value(json!("effort_change")).unwrap();
    assert_eq!(cause, RideCause::EffortChange);
    assert_eq!(
        serde_json::to_value(&cause).unwrap(),
        json!("effort_change")
    );
    assert!(RideRequest::new(vec![cause], 1_000, conditions).cache_lost());
}
