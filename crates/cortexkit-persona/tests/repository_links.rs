//! Repository tiers never read through a link: no path component below the
//! tier root may be a symbolic link (on Windows, a junction counts as one), or
//! a repository could pull any file the process can read into its persona.
//! The root itself may be reached through a link.
//!
//! On Windows the links are junctions made with `mklink /J`, which needs no
//! special privilege; std reports them as links because `is_symlink` is true
//! for every name-surrogate reparse point.

mod common;

use std::{fs, path::Path};

use common::*;
use cortexkit_persona::{
    Approval, ApprovalSet, ApprovalState, FragmentSources, LoadRequest, PersonaError, ProjectKey,
    SourceRef, Tier, TrustClass,
};

/// Create a directory link at `link` that points to `target` (the argument
/// order of `ln -s`).
fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // cmd reads a `/` inside a path as a switch, so hand it backslashes.
        let native = |p: &Path| p.to_string_lossy().replace('/', "\\");
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mklink /J {} {} failed: {}{}",
            link.display(),
            target.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

const SECRET: &str = "SECRET FROM OUTSIDE";

fn persona() -> String {
    full("fragments: [notes]\n", "[{% include \"notes\" %}]")
}

/// A real layer at `dir` (the role directory), with its fragment.
fn write_layer(dir: &Path, notes: &str) {
    fs::create_dir_all(dir.join("fragments")).unwrap();
    fs::write(dir.join("PERSONA.md"), persona()).unwrap();
    fs::write(dir.join("fragments/notes.md"), notes).unwrap();
}

fn project_tier(root: &Path) -> Tier {
    Tier::directory(
        "project",
        TrustClass::Repository,
        root,
        ".cortexkit/demo/roles",
    )
    .unwrap()
}

fn refused_at(root: &Path, component: &str) {
    let key = ProjectKey::new("pj-1").unwrap();
    let tiers = [project_tier(root)];
    match engine().load(LoadRequest::new(ROLE, &tiers, &ApprovalSet::new()).project_key(&key)) {
        Err(PersonaError::LinkInRepositoryTier { component: found }) => assert_eq!(
            found,
            SourceRef::File {
                tier: "project".into(),
                path: component.into()
            }
        ),
        other => panic!("expected a refusal at {component}, got {other:?}"),
    }
}

#[test]
fn a_linked_fragments_directory_in_a_repository_tier_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("notes.md"), SECRET).unwrap();
    let role_dir = t.path().join("project/.cortexkit/demo/roles/writer");
    fs::create_dir_all(&role_dir).unwrap();
    fs::write(role_dir.join("PERSONA.md"), persona()).unwrap();
    link_dir(&outside, &role_dir.join("fragments"));
    refused_at(
        &t.path().join("project"),
        ".cortexkit/demo/roles/writer/fragments",
    );
}

#[test]
fn a_linked_role_directory_in_a_repository_tier_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside/writer");
    write_layer(&outside, SECRET);
    let roles = t.path().join("project/.cortexkit/demo/roles");
    fs::create_dir_all(&roles).unwrap();
    link_dir(&outside, &roles.join("writer"));
    refused_at(&t.path().join("project"), ".cortexkit/demo/roles/writer");
}

#[test]
fn a_linked_module_directory_in_a_repository_tier_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside/demo");
    write_layer(&outside.join("roles/writer"), SECRET);
    let dot = t.path().join("project/.cortexkit");
    fs::create_dir_all(&dot).unwrap();
    link_dir(&outside, &dot.join("demo"));
    refused_at(&t.path().join("project"), ".cortexkit/demo");
}

#[test]
fn a_repository_root_reached_through_a_link_is_allowed() {
    let t = tempfile::tempdir().unwrap();
    let real = t.path().join("checkouts/project");
    write_layer(&real.join(".cortexkit/demo/roles/writer"), "inside");
    let linked = t.path().join("linked-project");
    link_dir(&real, &linked);

    let key = ProjectKey::new("pj-1").unwrap();
    // Find the layer digest through an unapproved load of the linked root.
    let probe = engine()
        .load(LoadRequest::new(
            ROLE,
            &[
                base(&[("PERSONA.md", &full("", "base"))]),
                project_tier(&linked),
            ],
            &ApprovalSet::new(),
        ))
        .unwrap();
    let digest = probe.tiers()[1].layer_digest.expect("the layer is present");
    let approvals: ApprovalSet = [Approval {
        module: "demo".into(),
        role: ROLE.into(),
        project_key: key.clone(),
        relative_path: ".cortexkit/demo/roles/writer".into(),
        layer_digest: digest,
    }]
    .into_iter()
    .collect();
    let role = load_approved(&[project_tier(&linked)], &approvals, &key).unwrap();
    assert_eq!(role.tiers()[0].approval, ApprovalState::Approved);
    assert_eq!(
        role.render(&context(), &FragmentSources::new())
            .unwrap()
            .text,
        "[inside]"
    );
}

#[test]
fn an_operator_tier_still_follows_links() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("dotfiles/writer");
    write_layer(&outside, "from dotfiles");
    let roles = t.path().join("config/cortexkit/demo/roles");
    fs::create_dir_all(&roles).unwrap();
    link_dir(&outside, &roles.join("writer"));
    let tiers = [Tier::directory(
        "user",
        TrustClass::Operator,
        t.path().join("config"),
        "cortexkit/demo/roles",
    )
    .unwrap()];
    assert_eq!(text(&tiers), "[from dotfiles]");
}
