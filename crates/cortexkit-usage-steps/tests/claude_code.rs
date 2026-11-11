use cortexkit_usage_steps::{claude_code_step_ref, live_row_identity};

const REQUEST_ID: &str = "req_011CgwTtWTJC7pMsgg6bxRCn";

#[test]
fn claude_code_step_ref_keeps_a_request_id_verbatim() {
    assert_eq!(
        claude_code_step_ref(REQUEST_ID).as_deref(),
        Some(REQUEST_ID)
    );
}

#[test]
fn claude_code_step_ref_refuses_ids_that_are_not_plain_identifiers() {
    let refused = [
        ("empty", String::new()),
        ("space only", " ".to_owned()),
        ("leading space", format!(" {REQUEST_ID}")),
        ("trailing newline", format!("{REQUEST_ID}\n")),
        ("inner space", "req_011 CgwTtWTJC7pMsgg6bxRCn".to_owned()),
        ("tab", format!("{REQUEST_ID}\t")),
        ("no-break space", format!("{REQUEST_ID}\u{a0}")),
        ("nul", format!("{REQUEST_ID}\0")),
        ("escape", format!("\u{1b}{REQUEST_ID}")),
        ("delete", format!("{REQUEST_ID}\u{7f}")),
        ("c1 control", format!("{REQUEST_ID}\u{85}")),
    ];
    for (name, id) in refused {
        assert_eq!(claude_code_step_ref(&id), None, "{name}");
    }
}

/// The runner sends the key as `harness_step_ref` under the `claude-code`
/// harness; the live row must carry the same identity the provider records
/// from the transcript, so the recorded row replaces it.
#[test]
fn live_row_identity_passes_the_claude_code_key_through_unchanged() {
    let key = claude_code_step_ref(REQUEST_ID).unwrap();
    assert_eq!(
        live_row_identity("example-runner", "claude-code", Some(&key), None),
        Some(("claude-code".to_owned(), REQUEST_ID.to_owned()))
    );
}
