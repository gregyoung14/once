//! Reads from the remote tier. Every read is retried as a fresh request and
//! verified before anything is committed to the local store.
//!
//! Only blobs up to `IN_MEMORY_BLOB_LIMIT` are read into memory, through a
//! batch read whose decoded size is capped at that limit. Larger blobs stream
//! into scratch space in bounded chunks, capped at the size their digest
//! declares, and are committed only after their SHA-256 and size match.
//! Memory per transfer is therefore bounded by the limit or by one stream
//! message, and transfers are bounded by the transfer semaphore, regardless
//! of the sizes a remote entry declares.

use sha2::Digest as _;
use tokio::io::AsyncWriteExt;

use super::*;
use crate::store::StagedBlob;

/// Largest blob read whole into memory from the remote tier.
pub(super) const IN_MEMORY_BLOB_LIMIT: i64 = BATCH_BLOB_LIMIT_I64;
/// Room for protobuf framing, digests, and status around a blob's bytes in
/// one response message.
const FRAMING_SLACK: usize = 16 * 1024;
#[allow(clippy::cast_possible_truncation)]
const DUPLEX_BUFFER: usize = crate::budget::STREAM_PIPE_BYTES as usize;

/// Largest `ByteStream` read message accepted for a blob of `declared` bytes:
/// the whole blob plus framing, never more than the transport-wide cap.
fn stream_message_limit(declared: u64) -> usize {
    usize::try_from(declared)
        .unwrap_or(usize::MAX)
        .saturating_add(FRAMING_SLACK)
        .min(BYTE_STREAM_MESSAGE_LIMIT)
}

/// Largest `BatchReadBlobs` response accepted for one blob of `declared`
/// bytes, allowing for a zstd frame that is slightly larger than its input.
fn batch_response_limit(declared: u64) -> usize {
    let declared = usize::try_from(declared).unwrap_or(usize::MAX);
    declared
        .saturating_add(declared / 64)
        .saturating_add(FRAMING_SLACK)
}

// Every reservation a single remote operation makes, other than an
// in-memory upload of a caller's buffer, fits the minimum budget.
const _: () = {
    let min = crate::budget::MIN_TRANSFER_MEMORY_BYTES;
    let batch = BATCH_BLOB_LIMIT as u64;
    assert!(METADATA_RESPONSE_LIMIT as u64 <= min);
    assert!(batch + batch / 64 + FRAMING_SLACK as u64 + batch <= min);
    assert!(3 * BYTE_STREAM_CHUNK_SIZE as u64 <= min);
    assert!((BYTE_STREAM_MESSAGE_LIMIT + DUPLEX_BUFFER) as u64 == min);
};

impl TuistCache {
    pub async fn ensure_blob_local(&self, digest: &Digest) -> Result<()> {
        if self.local.has_blob(digest).await? {
            return Ok(());
        }
        if is_empty_blob(digest) {
            self.local.mirror_blob(digest, &[]).await?;
            return Ok(());
        }
        let remote_digest = self.remote_blob_digest(digest).await?;
        let started = Instant::now();
        let staged = retry::grpc("get blob", || {
            self.stage_remote_blob(&remote_digest, "get blob")
        })
        .await
        .map_err(|error| {
            if error.is_not_found() {
                Error::BlobNotFound(*digest)
            } else {
                error
            }
        })?;
        if staged.digest() != *digest {
            return Err(status::remote(
                "get blob",
                ErrorKind::Corrupt,
                format!(
                    "remote blob {digest} failed verification: content hashes to {}",
                    staged.digest()
                ),
            ));
        }
        let size = staged.len();
        self.local.commit_staged(staged).await?;
        transfer::record(
            TransferDirection::Download,
            *digest,
            size,
            started.elapsed(),
        );
        Ok(())
    }

    /// Streams one remote blob into scratch space. Each attempt opens a fresh
    /// read stream, so a dropped HTTP/2 stream can be retried. The staged
    /// blob is returned only if its size and SHA-256 match `remote_digest`;
    /// a confirmed missing blob fails with kind not found.
    pub(super) async fn stage_remote_blob(
        &self,
        remote_digest: &reapi::Digest,
        operation: &'static str,
    ) -> std::result::Result<StagedBlob, retry::Failure> {
        let declared = declared_size(remote_digest, operation)?;
        // Disk for the whole declared size is reserved before the stream is
        // opened, and staging stops one byte past it, so a size the remote
        // declares cannot write more than the staging budget allows. Budgets
        // are taken before the transfer slot, the order every transfer uses,
        // and are returned when this attempt ends or is cancelled.
        let _staging = self.budget.reserve_staging(declared).await?;
        // The decoder never accepts a message larger than the blob plus its
        // framing, and that cap is exactly what is reserved, so the buffers
        // this read can hold are the ones the budget admitted.
        let message_limit = stream_message_limit(declared);
        let _memory = self
            .budget
            .reserve_memory(message_limit as u64 + DUPLEX_BUFFER as u64)
            .await?;
        let _permit = self
            .transfer_limit
            .acquire()
            .await
            .expect("transfer semaphore remains open");
        let channel = self.grpc_channel().await?;
        let mut client = ByteStreamClient::new(channel).max_decoding_message_size(message_limit);
        let request = bytestream::ReadRequest {
            resource_name: byte_stream_read_resource(
                &self.instance_name(),
                remote_digest,
                reapi::compressor::Value::Identity as i32,
            ),
            read_offset: 0,
            read_limit: 0,
        };
        let response = match client
            .read(self.authorized_grpc_request(request, operation).await?)
            .await
        {
            Ok(response) => response,
            Err(status) if status.code() == Code::NotFound => {
                return Err(status::remote(
                    operation,
                    ErrorKind::NotFound,
                    format!("remote blob {} is missing", digest_key(remote_digest)),
                )
                .into());
            }
            Err(status) => return Err(status.into()),
        };
        let mut stream = response.into_inner();
        let (mut writer, reader) = tokio::io::duplex(DUPLEX_BUFFER);
        let stall_timeout = self.stream_stall_timeout;
        // Owns the writer so the store sees end of input whenever the
        // download stops, successfully or not.
        let download = async move {
            let mut sha256 = Sha256::new();
            while let Some(response) = stall::next_message(&mut stream, stall_timeout).await? {
                sha256.update(&response.data);
                writer
                    .write_all(&response.data)
                    .await
                    .map_err(|source| local_mirror_error(&source))?;
            }
            writer
                .shutdown()
                .await
                .map_err(|source| local_mirror_error(&source))?;
            Ok::<_, retry::Failure>(hex::encode(sha256.finalize()))
        };
        let store = async {
            self.local
                .stage_stream(reader, declared)
                .await
                .map_err(|error| match error {
                    error if error.is_corrupt() => retry::Failure::Local(status::remote(
                        operation,
                        ErrorKind::Corrupt,
                        format!(
                            "remote blob {} failed verification: {error}",
                            digest_key(remote_digest)
                        ),
                    )),
                    error => retry::Failure::Local(error),
                })
        };
        let (staged, sha256) = match tokio::join!(store, download) {
            // A broken transfer explains whatever the store saw, so it wins
            // and is retried. Otherwise a store failure means the content was
            // wrong and the store stopped reading.
            (stored, Err(failure @ retry::Failure::Rpc(_))) => {
                drop(stored);
                return Err(failure);
            }
            (Err(failure), _) | (Ok(_), Err(failure)) => return Err(failure),
            (Ok(staged), Ok(sha256)) => (staged, sha256),
        };
        if staged.len() != declared || sha256 != remote_digest.hash {
            return Err(status::remote(
                operation,
                ErrorKind::Corrupt,
                format!(
                    "remote blob {} failed verification: received {} bytes hashing to {sha256}",
                    digest_key(remote_digest),
                    staged.len()
                ),
            )
            .into());
        }
        Ok(staged)
    }

    /// Reads a blob small enough to hold in memory. Larger blobs must go
    /// through [`stage_remote_blob`](Self::stage_remote_blob).
    pub(super) async fn read_reapi_blob(
        &self,
        digest: &reapi::Digest,
        operation: &'static str,
    ) -> Result<Option<Vec<u8>>> {
        if digest.size_bytes > IN_MEMORY_BLOB_LIMIT {
            return Err(status::remote(
                operation,
                ErrorKind::Protocol,
                format!(
                    "remote blob {} is too large to read into memory",
                    digest_key(digest)
                ),
            ));
        }
        // The encoded response, capped by the client's decoding limit, and the
        // decoded blob, capped at its declared size. Both limits are derived
        // from the blob before reserving, and the reservation is exactly them.
        let declared = declared_size(digest, operation)?;
        let response_limit = batch_response_limit(declared);
        let _memory = self
            .budget
            .reserve_memory(response_limit as u64 + declared)
            .await?;
        let _permit = self
            .transfer_limit
            .acquire()
            .await
            .expect("transfer semaphore remains open");
        let capabilities = self.remote_capabilities().await;
        let channel = self.grpc_channel().await?;
        let client =
            ContentAddressableStorageClient::new(channel).max_decoding_message_size(response_limit);
        let mut acceptable_compressors = vec![reapi::compressor::Value::Identity as i32];
        if capabilities.zstd_batches {
            acceptable_compressors.push(reapi::compressor::Value::Zstd as i32);
        }
        let request = reapi::BatchReadBlobsRequest {
            instance_name: self.instance_name(),
            digests: vec![digest.clone()],
            acceptable_compressors,
            digest_function: SHA256_DIGEST_FUNCTION,
        };
        retry::grpc(operation, || {
            let mut client = client.clone();
            let request = request.clone();
            async move {
                let response = client
                    .batch_read_blobs(self.authorized_grpc_request(request, operation).await?)
                    .await?
                    .into_inner();
                batch_read_outcome(response, digest, operation)
            }
        })
        .await
    }
}

/// One blob's answer from `BatchReadBlobs`. Only a per-blob `NOT_FOUND`
/// confirms absence; a transient per-blob status is retried like a failed RPC.
fn batch_read_outcome(
    response: reapi::BatchReadBlobsResponse,
    digest: &reapi::Digest,
    operation: &'static str,
) -> std::result::Result<Option<Vec<u8>>, retry::Failure> {
    let Some(blob) = response.responses.into_iter().next() else {
        return Err(status::remote(
            operation,
            ErrorKind::Protocol,
            "Kura returned no blob response",
        )
        .into());
    };
    let Some(rpc) = blob.status.as_ref().filter(|s| s.code != REAPI_STATUS_OK) else {
        return Ok(Some(decode_reapi_blob(
            blob.data,
            blob.compressor,
            digest,
            operation,
        )?));
    };
    if rpc.code == REAPI_STATUS_NOT_FOUND {
        return Ok(None);
    }
    let transient = super::super::retry_hint::from_rpc(rpc);
    if retry::retryable(&transient) {
        Err(transient.into())
    } else {
        Err(status::rpc_status_error(operation, rpc).into())
    }
}

fn declared_size(digest: &reapi::Digest, operation: &'static str) -> Result<u64> {
    u64::try_from(digest.size_bytes).map_err(|_| {
        status::remote(
            operation,
            ErrorKind::Protocol,
            format!("remote digest {} has a negative size", digest_key(digest)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_reads_distinguish_absence_from_failure() {
        let digest = sha256_digest(b"blob").unwrap();
        let response = |code: Code, data: &[u8]| reapi::BatchReadBlobsResponse {
            responses: vec![reapi::batch_read_blobs_response::Response {
                digest: Some(digest.clone()),
                data: data.to_vec(),
                compressor: reapi::compressor::Value::Identity as i32,
                status: Some(bazel_remote_apis::google::rpc::Status {
                    code: code as i32,
                    message: "injected".into(),
                    details: vec![],
                }),
            }],
        };
        assert!(matches!(
            batch_read_outcome(response(Code::Ok, b"blob"), &digest, "get blob"),
            Ok(Some(bytes)) if bytes == b"blob"
        ));
        assert!(matches!(
            batch_read_outcome(response(Code::NotFound, b""), &digest, "get blob"),
            Ok(None)
        ));
        assert!(matches!(
            batch_read_outcome(response(Code::Unavailable, b""), &digest, "get blob"),
            Err(retry::Failure::Rpc(status)) if status.code() == Code::Unavailable
        ));
        assert!(matches!(
            batch_read_outcome(response(Code::PermissionDenied, b""), &digest, "get blob"),
            Err(retry::Failure::Local(error)) if error.kind() == ErrorKind::Refused
        ));
        assert!(matches!(
            batch_read_outcome(response(Code::Ok, b"tampered"), &digest, "get blob"),
            Err(retry::Failure::Local(error)) if error.is_corrupt()
        ));
        assert!(matches!(
            batch_read_outcome(reapi::BatchReadBlobsResponse::default(), &digest, "get blob"),
            Err(retry::Failure::Local(error)) if error.kind() == ErrorKind::Protocol
        ));
    }
}
