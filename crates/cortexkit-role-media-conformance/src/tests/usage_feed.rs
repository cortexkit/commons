use super::{run_case, Fake, MediaSubject, Outcome};
use cortexkit_role_media::usage::{
    check_usage_answer, ChargeBasis, UsageReadAnswer, UsageRecord, UsageStatus,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[tokio::test]
async fn media_kit_core_usage_feed_identity_answer() {
    let case = "usage_feed_identity_answer";
    // A legacy provider can still pass the case without sending an identity.
    assert_eq!(
        run_case(&Fake::new(None, true, true), case).await,
        Outcome::Passed
    );
    for fault in [
        None,
        Some("usage_feed_identity_bad_id"),
        Some("usage_feed_identity_bad_head"),
    ] {
        let subject = Fake::new(fault, true, true).with_feed_identity();
        subject
            .seed_usage_entry(b"not-json".to_vec())
            .await
            .unwrap();
        let outcome = run_case(&subject, case).await;
        match fault {
            None => assert_eq!(outcome, Outcome::Passed),
            Some(fault) => assert_eq!(
                outcome,
                Outcome::Failed(format!("{case}: invalid_usage_answer")),
                "{fault}"
            ),
        }
    }
}

fn read_store(id: &str, record_ids: &[&str], after_seq: u64) -> UsageReadAnswer {
    let entries: Vec<Value> = (1..)
        .zip(record_ids)
        .filter(|(seq, _)| *seq > after_seq)
        .map(|(seq, record_id)| {
            let record = UsageRecord::new(
                (*record_id).into(),
                "alice".into(),
                "fixture".into(),
                "speech".into(),
                ChargeBasis::Vendor,
                UsageStatus::Done,
            )
            .with_job_id(format!("job-{record_id}"));
            json!({"seq":seq,"record":record})
        })
        .collect();
    let mut wire = json!({"entries":entries,"feed":{"id":id,"head_seq":record_ids.len()}});
    if let Some(last) = entries.last() {
        wire["last_seq"] = last["seq"].clone();
    }
    let answer = serde_json::from_value(wire).unwrap();
    check_usage_answer(&answer).unwrap();
    answer
}

#[test]
fn media_kit_usage_feed_reader_resets_after_renumbering() {
    // An ID change must work even if new writes already caught up to the old head.
    // A lower head must also trigger a re-read on its own.
    for (store_id, record_ids, new_ids, ack) in [
        (
            "replacement",
            &["shared", "new", "newer"][..],
            vec!["new", "newer"],
            3,
        ),
        ("original", &["shared", "new"][..], vec!["new"], 2),
    ] {
        let mut reads = Vec::new();
        let mut cursor = 0;
        let mut last_id: Option<String> = None;
        let mut seen = BTreeSet::new();
        let mut poll = |id, record_ids: &[&str]| {
            let mut read = |after_seq| {
                reads.push(after_seq);
                read_store(id, record_ids, after_seq)
            };
            let mut page = read(cursor);
            let feed = page.feed.as_ref().unwrap();
            if last_id.as_deref().is_some_and(|last| last != feed.id) || feed.head_seq < cursor {
                cursor = 0;
                page = read(cursor);
            }
            last_id = page.feed.map(|feed| feed.id);
            let accepted = page
                .entries
                .into_iter()
                .map(|entry| entry.record.unwrap().record_id)
                .filter(|id| seen.insert(id.clone()))
                .collect::<Vec<_>>();
            cursor = page.last_seq.unwrap_or(cursor);
            (accepted, page.last_seq)
        };
        let (accepted, through_seq) = poll("original", &["shared", "old", "older"]);
        assert_eq!(accepted, ["shared", "old", "older"]);
        assert_eq!(through_seq, Some(3));
        let (accepted, through_seq) = poll(store_id, record_ids);
        assert_eq!(accepted, new_ids);
        assert_eq!(through_seq, Some(ack));
        assert_eq!(reads, [0, 3, 0]);
    }
}
