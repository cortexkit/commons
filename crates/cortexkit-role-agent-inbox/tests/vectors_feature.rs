use cortexkit_role_agent_inbox::vectors;

#[test]
fn packaged_vectors_are_public() {
    assert_eq!(vectors::DIR, "agent-inbox-v1");
    assert_eq!(vectors::FILES.len(), 6);
    assert_eq!(vectors::SHA256SUMS.lines().count(), vectors::FILES.len());
    assert_eq!(vectors::file("README.md"), Some(vectors::README));
    assert!(vectors::file("posts.json").is_some());
    assert!(vectors::file("missing").is_none());
}
