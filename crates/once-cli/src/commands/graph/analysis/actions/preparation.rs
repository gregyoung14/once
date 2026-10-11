use anyhow::{Context, Result};
use once_frontend::analysis::DeclaredAction;

use super::{pending_outputs, prepare_declared_command_paths, DeclaredActionContext};

pub(super) async fn restore(
    context: &DeclaredActionContext<'_>,
    declared: &DeclaredAction,
) -> Result<pending_outputs::Restoration> {
    let restored = pending_outputs::restore(
        context.workspace,
        context.cache,
        context.prior_cached_results,
        context.available_inputs,
        declared,
        context.source_digest_cache,
    )
    .await
    .with_context(|| {
        format!(
            "materializing prerequisites for action {} for {} ({})",
            context.index, context.target_id, context.identifier
        )
    })?;
    prepare_declared_command_paths(context.workspace, declared)
        .await
        .with_context(|| {
            format!(
                "preparing action {} for {} ({})",
                context.index, context.target_id, context.identifier
            )
        })?;
    Ok(restored)
}
