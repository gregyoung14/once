use super::*;
use serde_json::json;
use std::sync::Arc;

fn info(path: &str) -> ArtifactInfo {
    serde_json::from_value(json!({
        "schema": "once.artifact_info.v1",
        "artifacts": [{
            "name": "product",
            "payloads": {"content": {"path": path, "media_type": "application/octet-stream"}},
            "interfaces": {"example.unknown.v7": {"payload": "content", "spec": {"argv": ["never-execute"]}}},
            "selectors": {"os": "linux", "arch": "aarch64"},
        }],
    })).unwrap()
}

async fn permit() -> crate::ResourcePermit {
    Arc::new(crate::ResourcePool::new(crate::ResourceLimits::default()))
        .acquire_required(crate::ResourceRequest::new(1, 4 * 1024 * 1024))
        .await
        .unwrap()
}

#[tokio::test]
async fn export_is_self_contained_and_content_addressed_not_workspace_addressed() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("payload"), b"hello").unwrap();
    std::fs::write(second.path().join("other-path"), b"hello").unwrap();
    let permit = permit().await;
    let a = export_artifacts(
        first.path(),
        &info("payload"),
        &["payload".into()],
        destination.path(),
        &permit,
    )
    .await
    .unwrap();
    let b = export_artifacts(
        second.path(),
        &info("other-path"),
        &["other-path".into()],
        destination.path(),
        &permit,
    )
    .await
    .unwrap();
    assert_eq!(a.root, b.root);
    let manifest: ArtifactManifest =
        read_artifact_json(&a.directory, &a.artifacts.members["product"].artifact)
            .await
            .unwrap();
    let payload = &manifest.payloads["content"];
    assert_eq!(
        payload.digest,
        "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(
        manifest.interfaces["example.unknown.v7"].spec["argv"],
        json!(["never-execute"])
    );
    first.close().unwrap();
    second.close().unwrap();
    assert_eq!(
        std::fs::read(content_path(&a.directory, payload).unwrap()).unwrap(),
        b"hello"
    );
    let index: ArtifactSet =
        serde_json::from_slice(&std::fs::read(a.directory.join("index.json")).unwrap()).unwrap();
    assert_eq!(
        index.members["product"].artifact,
        a.artifacts.members["product"].artifact
    );
    let mut changed = info("payload");
    changed.artifacts[0]
        .interfaces
        .get_mut("example.unknown.v7")
        .unwrap()
        .spec
        .insert("argv".into(), json!(["changed"]));
    let third = tempfile::tempdir().unwrap();
    std::fs::write(third.path().join("payload"), b"hello").unwrap();
    let c = export_artifacts(
        third.path(),
        &changed,
        &["payload".into()],
        destination.path(),
        &permit,
    )
    .await
    .unwrap();
    assert_ne!(a.root, c.root);
}

#[tokio::test]
async fn export_rejects_undeclared_payloads_and_detects_tampered_metadata() {
    let root = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload"), b"abc").unwrap();
    let permit = permit().await;
    let error = export_artifacts(
        root.path(),
        &info("payload"),
        &[],
        destination.path(),
        &permit,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("not a declared output"));
    let package = export_artifacts(
        root.path(),
        &info("payload"),
        &["payload".into()],
        destination.path(),
        &permit,
    )
    .await
    .unwrap();
    let descriptor = &package.artifacts.members["product"].artifact;
    std::fs::write(content_path(&package.directory, descriptor).unwrap(), b"{}").unwrap();
    assert!(
        read_artifact_json::<ArtifactManifest>(&package.directory, descriptor)
            .await
            .is_err()
    );
    let repaired = export_artifacts(
        root.path(),
        &info("payload"),
        &["payload".into()],
        destination.path(),
        &permit,
    )
    .await
    .unwrap();
    assert_eq!(package.root, repaired.root);
    assert!(
        read_artifact_json::<ArtifactManifest>(&repaired.directory, descriptor)
            .await
            .is_ok()
    );
}

#[test]
fn artifact_contract_bounds_and_interface_references_are_validated() {
    let mut declaration = info("payload");
    let template = declaration.artifacts[0].clone();
    declaration.artifacts = (0..128)
        .map(|index| {
            let mut artifact = template.clone();
            artifact.name = format!("variant-{index}");
            artifact
        })
        .collect();
    assert!(declaration.validate(&["payload".into()]).is_ok());
    declaration.artifacts.push(template);
    assert!(declaration.validate(&["payload".into()]).is_err());
    let mut declaration = info("payload");
    declaration.artifacts[0]
        .interfaces
        .get_mut("example.unknown.v7")
        .unwrap()
        .payload = "missing".into();
    assert!(declaration
        .validate(&["payload".into()])
        .unwrap_err()
        .to_string()
        .contains("unknown payload"));
    assert!(info("../outside").validate(&["../outside".into()]).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn export_rejects_output_symlinks_that_escape_workspace() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"private").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("payload")).unwrap();
    let error = export_artifacts(
        root.path(),
        &info("payload"),
        &["payload".into()],
        &root.path().join("export"),
        &permit().await,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("escapes workspace"));
    assert!(!root.path().join("export").exists());
}
