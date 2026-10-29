use anyhow::{bail, ensure, Context, Result};
use once_core::artifacts::{content_path, ArtifactDescriptor, ArtifactExport};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

pub(super) const PART_BYTES: u64 = 8 * 1024 * 1024;

pub(super) async fn upload_parts(
    client: &Client,
    base: &Url,
    token: &str,
    package: &ArtifactExport,
    payload: &ArtifactDescriptor,
    upload_id: &str,
    build_id: &str,
) -> Result<Vec<Value>> {
    let path = content_path(&package.directory, payload)?;
    let mut parts = Vec::new();
    let mut offset = 0;
    while offset < payload.size_bytes {
        let length = (payload.size_bytes - offset).min(PART_BYTES);
        let number = parts.len() + 1;
        let signed = post(client, base, "generate-url", token, json!({
            "preview_id": build_id,
            "app_build_id": build_id,
            "multipart_upload_part": {"upload_id": upload_id, "part_number": number, "content_length": length},
        })).await?;
        let url = signed
            .pointer("/data/url")
            .and_then(Value::as_str)
            .context("Tuist did not return an upload URL")?;
        let parsed = Url::parse(url)?;
        ensure!(
            parsed.scheme() == "https" || parsed.origin() == base.origin(),
            "insecure artifact upload URL"
        );
        let mut file = tokio::fs::File::open(&path).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let body =
            reqwest::Body::wrap_stream(ReaderStream::with_capacity(file.take(length), 64 * 1024));
        let response = client
            .put(parsed)
            .header("content-length", length)
            .header("content-type", &payload.media_type)
            .body(body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("artifact part transfer failed"))?;
        ensure!(
            response.status().is_success(),
            "artifact part upload returned HTTP {}",
            response.status().as_u16()
        );
        let etag = response
            .headers()
            .get("etag")
            .context("artifact part upload did not return an ETag")?
            .to_str()?
            .to_string();
        parts.push(json!({"part_number": number, "etag": etag}));
        offset += length;
    }
    Ok(parts)
}

pub(super) async fn post(
    client: &Client,
    base: &Url,
    operation: &str,
    token: &str,
    params: Value,
) -> Result<Value> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("invalid artifact service URL"))?
        .push(operation);
    let mut response = client
        .post(url)
        .bearer_auth(token)
        .json(&params)
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("Tuist artifact {operation} request failed; its outcome may be unknown")
        })?;
    if response.status() == reqwest::StatusCode::CONFLICT {
        bail!("Tuist rejected a duplicate preview build; its legacy API does not return an idempotent publication receipt");
    }
    ensure!(
        response.status().is_success(),
        "Tuist artifact {operation} returned HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 1024 * 1024,
            "artifact service metadata exceeds one mebibyte"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
