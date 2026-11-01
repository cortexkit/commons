use super::*;

fn device(kind: DeviceKind, state: State) -> Device {
    Device {
        device_id: format!("{kind:?}"),
        device_kind: kind,
        report: Report {
            state,
            focus: vec!["agent-b".into(), "agent-a".into()],
            shows_inline: vec!["ask".into()],
            reported_at_ms: 1_000,
            expires_at_ms: 2_000,
        },
    }
}

#[test]
fn where_precedence() {
    let phone = device(DeviceKind::Phone, State::Active);
    let computer = device(DeviceKind::Computer, State::Background);
    let locked = device(DeviceKind::Phone, State::Locked);
    assert_eq!(
        summarize([&phone, &computer, &locked], 1_500).location,
        Where::Phone
    );
    assert_eq!(
        summarize([&computer, &locked], 1_500).location,
        Where::Computer
    );
    assert_eq!(summarize([&locked], 1_500).location, Where::Away);
    assert_eq!(
        summarize([&phone, &computer], 2_000).location,
        Where::Unknown
    );
    assert_eq!(summarize([], 0).location, Where::Unknown);
    let background_phone = device(DeviceKind::Phone, State::Background);
    assert_eq!(summarize([&background_phone], 1_500).location, Where::Away);
}

#[test]
fn watching_union_and_expiry() {
    let first = device(DeviceKind::Computer, State::Active);
    let mut second = device(DeviceKind::Phone, State::Active);
    second.report.reported_at_ms = 1_100;
    second.report.expires_at_ms = 1_900;
    second.report.focus = vec!["agent-a".into()];
    second.report.shows_inline = vec!["card".into(), "ask".into()];
    let inactive = device(DeviceKind::Computer, State::Idle);
    let summary = summarize([&first, &second, &inactive], 1_500);
    assert_eq!(
        summary.watching,
        vec![
            Watching {
                agent_id: "agent-a".into(),
                shows_inline: vec!["ask".into(), "card".into()],
                seen_at_ms: 1_100
            },
            Watching {
                agent_id: "agent-b".into(),
                shows_inline: vec!["ask".into()],
                seen_at_ms: 1_000
            },
        ]
    );
    let expired = summarize([&first, &second], 1_900);
    assert_eq!(expired.location, Where::Computer);
    assert_eq!(expired.watching[0].shows_inline, ["ask"]);
    let mut refreshed = expired.clone();
    refreshed.watching[0].seen_at_ms += 1;
    assert!(expired.same_attention(&refreshed));
    refreshed.watching[0].shows_inline.push("mention".into());
    assert!(!expired.same_attention(&refreshed));
}

#[test]
fn route_every_branch() {
    let mut summary = summarize([&device(DeviceKind::Computer, State::Active)], 1_500);
    assert_eq!(route(&summary, "ask", "agent-a", 61_000), Deliver::None);
    assert_eq!(route(&summary, "ask", "agent-a", 61_001), Deliver::Desktop);
    assert_eq!(route(&summary, "card", "agent-a", 1_500), Deliver::Desktop);
    assert_eq!(route(&summary, "ask", "agent-c", 1_500), Deliver::Desktop);
    for location in [Where::Phone, Where::Away, Where::Unknown] {
        summary.location = location;
        assert_eq!(route(&summary, "ask", "agent-a", 1_500), Deliver::None);
        assert_eq!(route(&summary, "card", "agent-a", 1_500), Deliver::Phone);
        assert_eq!(route(&summary, "ask", "agent-a", 61_001), Deliver::Phone);
    }
}
