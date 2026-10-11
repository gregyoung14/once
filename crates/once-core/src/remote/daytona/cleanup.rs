use super::client::Client;
use crate::{Error, ExecutionPermit, Result};

const PROVIDER: &str = "daytona";

/// Deletes a hosted sandbox and holds its admission lease until the deletion
/// finishes, so a cancelled caller never frees quota for a machine that is
/// still running.
pub(super) struct SandboxCleanup {
    client: Client,
    id: Option<String>,
    lease: Option<ExecutionPermit>,
}

impl SandboxCleanup {
    pub(super) fn new(client: Client, id: String, lease: Option<ExecutionPermit>) -> Self {
        Self {
            client,
            id: Some(id),
            lease,
        }
    }

    /// Deletion runs on a detached task: cancelling this future leaves the
    /// task, and the lease it owns, running until the provider answers.
    pub(super) async fn run(mut self) -> Result<()> {
        let id = self.id.take().expect("cleanup sandbox is present");
        let deletion = delete(self.client.clone(), id, self.lease.take());
        tokio::spawn(deletion)
            .await
            .map_err(|source| Error::RemoteProviderApi {
                provider: PROVIDER.to_string(),
                message: format!("cleanup task failed: {source}"),
            })?
    }
}

impl Drop for SandboxCleanup {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let deletion = delete(self.client.clone(), id, self.lease.take());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = deletion.await {
                    tracing::warn!(provider = PROVIDER, %error, "failed to delete remote sandbox");
                }
            });
        } else {
            tracing::warn!(
                provider = PROVIDER,
                "skipping remote sandbox deletion because no Tokio runtime is available"
            );
        }
    }
}

async fn delete(client: Client, id: String, lease: Option<ExecutionPermit>) -> Result<()> {
    let result = client.delete(&id).await;
    tracing::debug!(
        provider = PROVIDER,
        sandbox = %id,
        deleted = result.is_ok(),
        "releasing remote admission lease after sandbox cleanup"
    );
    drop(lease);
    result
}
