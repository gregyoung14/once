use super::*;
use crate::{Executor, ExecutorResources, ResourceLimits, ResourcePool, ResourceRequest};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn cancelled_installer_keeps_staging_and_admission_until_blocking_work_finishes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let workspace = tempfile::tempdir().unwrap();
        let staging = OutputStaging::create(workspace.path()).unwrap();
        std::fs::write(staging.files().join("out"), b"complete").unwrap();
        let staging_path = staging.root.clone();
        let resources =
            ExecutorResources::new(Arc::new(ResourcePool::new(ResourceLimits::new(1, 0))));
        let lease = resources
            .acquire(Executor::Local, &ResourceRequest::default())
            .await;
        let phase = PhasePermit {
            _action: Some(lease),
            ..PhasePermit::default()
        };
        let (release, gate) = std::sync::mpsc::channel();
        let (arrived, arrival) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            arrived.send(()).unwrap();
            gate.recv().unwrap();
        });
        arrival.await.unwrap();
        let root = workspace.path().to_path_buf();
        let install = tokio::spawn(async move {
            staging
                .install_admitted(&[WorkspacePath::try_from("out").unwrap()], &root, phase)
                .await
        });
        tokio::task::yield_now().await;
        install.abort();
        assert!(install.await.unwrap_err().is_cancelled());
        assert!(
            staging_path.exists(),
            "cancelled observer must not remove staging from the queued installer"
        );
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            resources.acquire(Executor::Local, &ResourceRequest::default())
        )
        .await
        .is_err());
        release.send(()).unwrap();
        blocker.await.unwrap();
        let _next = tokio::time::timeout(
            Duration::from_secs(5),
            resources.acquire(Executor::Local, &ResourceRequest::default()),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(workspace.path().join("out")).unwrap(),
            b"complete"
        );
        assert!(!staging_path.exists());
    });
}

#[tokio::test]
async fn failed_multi_output_install_rolls_back_every_published_output() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("first"), b"old first").unwrap();
    std::fs::write(workspace.path().join("second"), b"old second").unwrap();
    let staging = OutputStaging::create(workspace.path()).unwrap();
    std::fs::write(staging.files().join("first"), b"new first").unwrap();
    let outputs = [
        WorkspacePath::try_from("first").unwrap(),
        WorkspacePath::try_from("second").unwrap(),
    ];
    assert!(staging.install(&outputs, workspace.path()).await.is_err());
    assert_eq!(
        std::fs::read(workspace.path().join("first")).unwrap(),
        b"old first"
    );
    assert_eq!(
        std::fs::read(workspace.path().join("second")).unwrap(),
        b"old second"
    );
}
