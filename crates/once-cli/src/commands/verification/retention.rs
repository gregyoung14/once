use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use once_cas::CacheProvider;
use once_core::{ReproducibilityTrial, ResourcePool, ResourceRequest};

pub(super) async fn retain(
    directory: &Path,
    execution_root: &Path,
    cache: &CacheProvider,
    resources: &Arc<ResourcePool>,
    trial: &mut ReproducibilityTrial,
) -> Result<()> {
    if let Some(result) = &trial.result {
        let permit = resources.acquire(ResourceRequest::default()).await;
        let retained = once_core::with_resource_permit(permit, async {
            once_core::materialize_outputs(result, &directory.join("outputs"), cache).await?;
            for (name, digest) in [("stdout", result.stdout), ("stderr", result.stderr)] {
                if let Some(digest) = digest {
                    cache
                        .copy_blob_to_file(&digest, &directory.join(name))
                        .await?;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        if let Err(error) = retained {
            trial.error = Some(format!("retaining artifacts and logs: {error}"));
        }
    }
    std::fs::create_dir_all(directory)?;
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&trial)?,
    )?;
    if trial.error.is_some()
        || trial
            .result
            .as_ref()
            .is_some_and(|result| result.exit_code != 0)
    {
        std::fs::rename(execution_root, directory.join("failed-workspace"))?;
    }
    Ok(())
}
