use super::*;

#[tokio::test]
async fn different_available_digest_cannot_reuse_a_prior_input_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    let cache_root = tempfile::tempdir().unwrap();
    let cache = CacheProvider::Local(once_cas::Cas::open(cache_root.path()));
    let old = cache.put_blob(b"old").await.unwrap();
    let wanted = cache.put_blob(b"wanted").await.unwrap();
    let pending = vec![ActionResult {
        exit_code: 0,
        stdout: None,
        stderr: None,
        outputs: BTreeMap::from([("input.txt".into(), old)]),
    }];
    let available = BTreeMap::from([(
        "input.txt".into(),
        AvailableInput {
            blob_digest: wanted,
            same_target: true,
            materialized: false,
        },
    )]);
    let declared = serde_json::from_value(serde_json::json!({
        "inputs": ["input.txt"], "outputs": ["copy.txt"], "depends_on_prior_actions": false
    }))
    .unwrap();
    restore(
        workspace.path(),
        &cache,
        &pending,
        &available,
        &declared,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(workspace.path().join("input.txt")).unwrap(),
        b"wanted"
    );
}

#[test]
fn child_overlays_prevent_reusing_an_enclosing_input_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    let root = once_cas::Digest::of_bytes(b"root");
    let child = once_cas::Digest::of_bytes(b"child");
    let pending = vec![ActionResult {
        exit_code: 0,
        stdout: None,
        stderr: None,
        outputs: BTreeMap::from([("tree".into(), root), ("tree/child".into(), child)]),
    }];
    let available = BTreeMap::from([(
        "tree".into(),
        AvailableInput {
            blob_digest: root,
            same_target: true,
            materialized: false,
        },
    )]);
    assert!(reusable_inputs(
        workspace.path(),
        &pending,
        &available,
        &BTreeSet::from(["tree".into(), "tree/child".into()]),
        &BTreeSet::from(["tree".into()])
    )
    .is_empty());
}
