use cortexkit_role_compaction::{
    notice::{Conditions, NoticeRequest},
    part::{FileMedia, Part, PartBody, Seal, ToolPart, ToolStatus},
    read::{Cursor, Entry, Head, ReadPage, Role},
    rules::{EditCost, ProviderRules, SignedParts},
    setup::ModelRef,
    tokens::{PartTokens, ReasoningTokens, SurfaceTokens},
};
use serde_json::json;

pub fn counted_entries() -> Vec<Entry> {
    vec![
        Entry::new(
            1,
            "message-counted-a",
            Role::Assistant,
            1_700_000_000_000,
            vec![
                Part::new(
                    "text-a",
                    PartBody::Text {
                        text: "An invented reply.".into(),
                    },
                )
                .with_tag(1)
                .with_tokens(PartTokens::count(4)),
                Part::new(
                    "tool-a",
                    PartBody::Tool(
                        ToolPart::new(
                            "call-a",
                            "example-tool",
                            json!({"query": "invented", "tokens": {"count": 7}}),
                            ToolStatus::Completed,
                        )
                        .with_output("An invented result.")
                        .with_output_bytes(19),
                    ),
                )
                .with_tag(2)
                .with_tokens(PartTokens::tool(12, 5)),
                Part::new(
                    "file-a",
                    PartBody::File(
                        FileMedia::inline("image/png", Some(64))
                            .with_width(32)
                            .with_height(32),
                    ),
                )
                .with_tokens(PartTokens::count(85)),
                Part::new(
                    "reasoning-a",
                    PartBody::Reasoning {
                        text: Some("An invented thought.".into()),
                        text_bytes: Some(20),
                    },
                )
                .with_seal(Seal::Signed),
            ],
        )
        .with_reasoning_tokens(ReasoningTokens::new(20, false)),
        Entry::new(
            2,
            "message-counted-b",
            Role::Assistant,
            1_700_000_001_000,
            vec![Part::new(
                "text-b",
                PartBody::Text {
                    text: "Another invented reply.".into(),
                },
            )
            .with_tag(3)],
        )
        .with_reasoning_tokens(ReasoningTokens::new(18, true)),
    ]
}

pub fn counted_page() -> ReadPage {
    let mut entries = counted_entries();
    for entry in &mut entries {
        entry.parts = entry.parts.iter().map(Part::compact).collect();
    }
    let last = entries.last().unwrap();
    let newest = Cursor::new(last.ordinal, &last.mid, &last.digest);
    ReadPage::new(entries, Head::new().with_newest(newest))
        .with_token_family("claude-v5/1")
        .with_token_family_exact(true)
}

pub fn counted_notice() -> NoticeRequest {
    let conditions = Conditions::new(
        ModelRef::new("example-provider", "example-model"),
        200_000,
        10_000,
        ProviderRules::new(SignedParts::Bound, EditCost::Suffix),
    )
    .with_system_tokens(SurfaceTokens::new(120, true))
    .with_tools_tokens(SurfaceTokens::new(230, false));
    let page = counted_page();
    NoticeRequest::new(1, conditions)
        .with_newest(page.head.newest.unwrap())
        .with_entries(page.entries)
        .with_token_family("claude-v5/1")
        .with_token_family_exact(false)
}
