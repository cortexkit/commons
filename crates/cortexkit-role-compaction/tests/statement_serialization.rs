use cortexkit_role_compaction::{
    notice::Conditions,
    read::Cursor,
    ride::RideCause,
    rules::{EditCost, ProviderRules, SignedParts},
    setup::{Limits, ModelRef, PageLimits, SetupRequest, WindowLimits},
    tokens::SurfaceTokens,
    view::AppliedView,
    SignedFloor,
};

// Captured from the 0.3.7 derived serializers with every non-ordering optional
// field set. Literal bytes pin key order, which a serde_json::Value would lose.
const SETUP_LEGACY: &str = r#"{"signed_floor":{"through":"floor-entry"},"preset":"example-preset","child_harness":"example-harness","model":{"provider":"example-provider","model":"example-model","variant":"example-variant","service_tier":"example-tier"},"accepts_images":true,"view_anchor":true,"limits":{"page":{"max_entries":10,"max_bytes":20},"select_max":30,"window":{"entries":40,"bytes":50}},"context_window":200000,"output_reserve":8000,"rules":{"signed_parts":"bound","history_edit_cost":"suffix"},"detects":["over_limit"],"applied_view":{"version":3,"checksum":"view-checksum"},"newest":{"ordinal":7,"mid":"newest-entry","digest":"entry-digest"},"now":17}"#;
const CONDITIONS_LEGACY: &str = r#"{"model":{"provider":"example-provider","model":"example-model","variant":"example-variant","service_tier":"example-tier"},"signed_floor":{"through":"floor-entry"},"accepts_images":true,"context_window":200000,"output_reserve":8000,"rules":{"signed_parts":"bound","history_edit_cost":"suffix"},"last_prompt_tokens":1234,"last_prompt_view":3,"system_tokens":{"count":111,"exact":true},"tools_tokens":{"count":222,"exact":false},"notes_tokens":{"count":333,"exact":true},"window_over_budget":true}"#;

fn model() -> ModelRef {
    ModelRef::new("example-provider", "example-model")
        .with_variant("example-variant")
        .with_service_tier("example-tier")
}

fn rules() -> ProviderRules {
    ProviderRules::new(SignedParts::Bound, EditCost::Suffix)
}

fn setup() -> SetupRequest {
    let mut setup = SetupRequest::new(
        "example-preset",
        model(),
        200_000,
        8_000,
        rules(),
        vec![RideCause::OverLimit],
        17,
    );
    setup.signed_floor = Some(SignedFloor::new("floor-entry"));
    setup.child_harness = Some("example-harness".into());
    setup.accepts_images = Some(true);
    setup.view_anchor = Some(true);
    setup.limits = Some(
        Limits::new()
            .with_page(PageLimits::new().with_max_entries(10).with_max_bytes(20))
            .with_select_max(30)
            .with_window(WindowLimits::new().with_entries(40).with_bytes(50)),
    );
    setup.applied_view = Some(AppliedView::new(3, "view-checksum"));
    setup.newest = Some(Cursor::new(7, "newest-entry", "entry-digest"));
    setup
}

fn conditions() -> Conditions {
    let mut conditions = Conditions::new(model(), 200_000, 8_000, rules());
    conditions.signed_floor = Some(SignedFloor::new("floor-entry"));
    conditions.accepts_images = Some(true);
    conditions.last_prompt_tokens = Some(1_234);
    conditions.last_prompt_view = Some(3);
    conditions.system_tokens = Some(SurfaceTokens::new(111, true));
    conditions.tools_tokens = Some(SurfaceTokens::new(222, false));
    conditions.notes_tokens = Some(SurfaceTokens::new(333, true));
    conditions.window_over_budget = Some(true);
    conditions
}

#[test]
fn legacy_statements_keep_derive_key_order() {
    assert_eq!(
        [
            serde_json::to_string(&setup()).unwrap(),
            serde_json::to_string(&conditions()).unwrap()
        ],
        [SETUP_LEGACY, CONDITIONS_LEGACY],
    );
}

#[test]
fn stamped_statements_put_epoch_and_stamp_first() {
    let setup = setup()
        .with_statement_order("89abcdef0123456789abcdef01234567", 42, true)
        .unwrap();
    let conditions = conditions()
        .with_statement_order("89abcdef0123456789abcdef01234567", 42, true)
        .unwrap();
    let prefix = r#"{"statement_epoch":"89abcdef0123456789abcdef01234567","statement_seq":42,"#;
    assert_eq!(
        serde_json::to_string(&setup).unwrap(),
        format!("{prefix}{}", &SETUP_LEGACY[1..])
    );
    assert_eq!(
        serde_json::to_string(&conditions).unwrap(),
        format!("{prefix}{}", &CONDITIONS_LEGACY[1..])
    );
}
