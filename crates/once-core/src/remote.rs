use std::collections::BTreeMap;
use std::path::Path;

use once_cas::{ActionResult, CacheProvider};

use crate::{ExecutionPermit, Executor, RemoteExecution, ResourceRequest, Result, WorkspacePath};

mod archive;
mod daytona;
mod e2b;
mod microsandbox;
mod output_install;
mod path;
#[cfg(test)]
mod test_server;

pub(crate) struct PreparedCommand<'a> {
    pub lease: Option<ExecutionPermit>,
    pub argv: &'a [String],
    pub env: &'a BTreeMap<String, String>,
    pub cwd: Option<&'a WorkspacePath>,
    pub inputs: &'a [WorkspacePath],
    pub outputs: &'a [WorkspacePath],
    pub resources: &'a ResourceRequest,
    pub timeout_ms: Option<u64>,
    pub success_exit_codes: &'a [i32],
}

impl PreparedCommand<'_> {
    pub fn accepts_exit_code(&self, exit_code: i32) -> bool {
        self.success_exit_codes.contains(&exit_code)
    }
}

pub(crate) async fn execute_command(
    executor: Executor,
    remote: &RemoteExecution,
    command: PreparedCommand<'_>,
    workspace_root: &Path,
    cache: &CacheProvider,
    stream_to_parent: bool,
) -> Result<ActionResult> {
    tracing::debug!(
        provider = %remote.provider,
        executor = executor.name(),
        environment = remote.environment.as_deref().unwrap_or("provider-default"),
        inputs = command.inputs.len(),
        outputs = command.outputs.len(),
        "dispatching remote command"
    );
    match executor {
        Executor::Microsandbox => {
            microsandbox::execute_command(remote, command, workspace_root, cache, stream_to_parent)
                .await
        }
        Executor::Daytona => {
            daytona::execute_command(remote, command, workspace_root, cache, stream_to_parent).await
        }
        Executor::E2b => {
            e2b::execute_command(remote, command, workspace_root, cache, stream_to_parent).await
        }
        Executor::Local => unreachable!("local execution does not use a remote transport"),
    }
}

pub(super) fn join_path(root: &str, rel: &str) -> String {
    if root.ends_with('/') {
        format!("{root}{rel}")
    } else {
        format!("{root}/{rel}")
    }
}
