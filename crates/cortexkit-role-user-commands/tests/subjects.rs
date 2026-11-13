use cortexkit_role_user_commands::{Subject, SubjectKind};
use serde_json::json;

#[test]
fn project_and_workspace_subjects_validate_and_preserve_ids() {
    for (make, kind, field, name) in [
        (
            Subject::project as fn(String) -> Subject,
            SubjectKind::Project,
            "project_id",
            "project",
        ),
        (
            Subject::workspace as fn(String) -> Subject,
            SubjectKind::Workspace,
            "workspace_id",
            "workspace",
        ),
    ] {
        for id in ["a".to_owned(), "é".repeat(128), "  mixed/Id  ".to_owned()] {
            let subject = make(id.clone());
            assert_eq!(subject.kind(), kind);
            subject.check().unwrap();
            let encoded = serde_json::to_value(&subject).unwrap();
            assert_eq!(encoded, json!({"kind": name, field: id}));
            assert_eq!(serde_json::from_value::<Subject>(encoded).unwrap(), subject);
        }
        for id in [String::new(), "é".repeat(129)] {
            assert_eq!(make(id).check().unwrap_err().field, field);
        }
        assert!(serde_json::from_value::<Subject>(
            json!({"kind": name, field: "id", "extra": true})
        )
        .is_err());
        assert!(serde_json::from_value::<Subject>(json!({"kind": name})).is_err());
    }
    assert!(serde_json::from_value::<SubjectKind>(json!("future_kind")).is_err());
    assert!(serde_json::from_value::<Subject>(json!({"kind": "future_kind"})).is_err());
}
