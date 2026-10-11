use std::collections::BTreeSet;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use once_cas::{ActionResult, CacheProvider};
use once_core::{ResourcePermit, ResourcePool, ResourceRequest};

use super::{admission, paths, SourceDigestCache};

pub(super) async fn restore(
    results: &[ActionResult],
    workspace: &Path,
    cache: &CacheProvider,
    digests: Option<&SourceDigestCache>,
    resources: &Arc<ResourcePool>,
) -> Result<()> {
    run(results, workspace, resources, |index, permit| {
        once_core::with_resource_permit(
            permit,
            super::restore_one(&results[index], workspace, cache, digests),
        )
    })
    .await
}

async fn run<F, W>(
    results: &[ActionResult],
    workspace: &Path,
    resources: &Arc<ResourcePool>,
    execute: F,
) -> Result<()>
where
    F: Fn(usize, ResourcePermit) -> W,
    W: Future<Output = Result<()>>,
{
    let mut running = FuturesUnordered::new();
    let mut active = BTreeSet::new();
    let mut next = 0;
    let mut failure = None;
    while !running.is_empty() || (failure.is_none() && next < results.len()) {
        if active.is_empty() && failure.is_none() {
            let batch = admission::acquire(&results[next..], workspace, resources).await;
            for (index, permit) in (next..next + batch.count).zip(batch.permits) {
                active.insert(index);
                running.push(indexed(index, execute(index, permit)));
            }
            next += batch.count;
        }
        if failure.is_none() && next < results.len() {
            if let Some(permit) = resources.acquire(ResourceRequest::default()).now_or_never() {
                let candidate = results[next].outputs.keys().cloned().collect();
                let disjoint = active.iter().all(|index| {
                    let outputs = results[*index].outputs.keys().cloned().collect();
                    paths::disjoint(workspace, &candidate, &outputs)
                });
                if disjoint {
                    active.insert(next);
                    running.push(indexed(next, execute(next, permit)));
                    next += 1;
                    continue;
                }
            }
        }
        let (index, outcome) = running.next().await.expect("admitted restoration window");
        active.remove(&index);
        if let Err(error) = outcome {
            failure.get_or_insert(error);
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn indexed<F: Future>(index: usize, future: F) -> (usize, F::Output) {
    (index, future.await)
}

#[cfg(all(test, not(windows)))]
#[path = "window_tests.rs"]
mod tests;
