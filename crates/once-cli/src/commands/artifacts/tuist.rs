mod transport;

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use once_cas::{TuistAuth, TuistCacheConfig};
use once_core::artifacts::{read_artifact_json, ArtifactExport, ArtifactManifest};
use once_core::Xdg;
use reqwest::{Client, Url};
use serde_json::{json, Value};

use super::Publication;
use transport::{post, upload_parts, PART_BYTES};

pub(super) async fn publish(
    package: &ArtifactExport,
    config: &TuistCacheConfig,
    xdg: &Xdg,
    publication: &Publication,
) -> Result<String> {
    ensure!(package.artifacts.members.len() == 1, "the legacy Tuist preview API cannot atomically publish artifact sets; export one artifact per publication");
    let member = package
        .artifacts
        .members
        .values()
        .next()
        .context("empty artifact set")?;
    let manifest: ArtifactManifest =
        read_artifact_json(&package.directory, &member.artifact).await?;
    let interface = manifest.interfaces.get("dev.tuist.preview.v1")
        .context("Tuist's current preview API requires the dev.tuist.preview.v1 interface; use local publication for other artifact types")?;
    let payload = manifest
        .payloads
        .get(&interface.payload)
        .context("missing preview payload")?;
    ensure!(
        (1..=PART_BYTES * 10_000).contains(&payload.size_bytes),
        "preview payload must be nonempty and fit the multipart upload part limit"
    );
    let metadata_name = interface
        .spec
        .get("metadata_payload")
        .and_then(Value::as_str)
        .context("preview interface requires metadata_payload")?;
    let metadata = manifest
        .payloads
        .get(metadata_name)
        .context("missing preview metadata payload")?;
    let mut params: BTreeMap<String, Value> =
        read_artifact_json(&package.directory, metadata).await?;
    for (name, value) in [
        ("track", &publication.track),
        ("git_branch", &publication.git_branch),
        ("git_commit_sha", &publication.git_commit),
    ] {
        if let Some(value) = value {
            params.insert(name.into(), Value::String(value.clone()));
        }
    }
    let base = endpoint(config)?;
    let client = Client::builder()
        .timeout(Duration::from_mins(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let auth = TuistAuth::new(crate::cache_provider::credentials_root(xdg), config);
    let token = tokio::task::spawn_blocking(move || auth.token()).await??;
    let start = post(
        &client,
        &base,
        "start",
        &token,
        serde_json::to_value(params)?,
    )
    .await?;
    let upload_id = start
        .pointer("/data/upload_id")
        .and_then(Value::as_str)
        .context("Tuist did not return an upload id")?;
    let build_id = start
        .pointer("/data/app_build_id")
        .and_then(Value::as_str)
        .context("Tuist did not return an app build id")?;
    let parts = upload_parts(
        &client, &base, &token, package, payload, upload_id, build_id,
    )
    .await
    .context(
        "preview upload incomplete; the legacy API cannot safely retry an interrupted publication",
    )?;
    let completed = post(
        &client,
        &base,
        "complete",
        &token,
        json!({
            "preview_id": build_id,
            "app_build_id": build_id,
            "multipart_upload_parts": {"upload_id": upload_id, "parts": parts},
        }),
    )
    .await
    .context(
        "preview finalization was not confirmed; do not treat this publication as successful",
    )?;
    let url = completed
        .get("url")
        .and_then(Value::as_str)
        .context("Tuist did not return a preview URL")?;
    let parsed = Url::parse(url)?;
    ensure!(
        parsed.scheme() == "https" || parsed.origin() == base.origin(),
        "invalid preview URL"
    );
    Ok(url.into())
}

fn endpoint(config: &TuistCacheConfig) -> Result<Url> {
    let account = config
        .account
        .as_deref()
        .context("artifact infrastructure requires an account")?;
    let project = config
        .project
        .as_deref()
        .context("artifact infrastructure requires a project")?;
    let mut base = Url::parse(&format!("{}/", config.url.trim_end_matches('/')))?;
    ensure!(
        base.scheme() == "https"
            || base
                .host_str()
                .is_some_and(|host| host == "localhost" || host == "127.0.0.1"),
        "artifact publication requires HTTPS"
    );
    ensure!(
        base.username().is_empty()
            && base.password().is_none()
            && base.query().is_none()
            && base.fragment().is_none(),
        "artifact provider URL must not contain credentials, a query or a fragment"
    );
    base.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("invalid artifact provider URL"))?
        .pop_if_empty()
        .extend(["api", "projects", account, project, "previews"]);
    Ok(base)
}
