//! Content from an untrusted source is staged in scratch space, checked, and
//! only then committed, so nothing unverified is ever visible in the store.

use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{Cas, Digest, Error, Result};

/// A blob written to scratch space but not yet visible in the store. Dropping
/// it removes the scratch file.
#[derive(Debug)]
pub(crate) struct StagedBlob {
    tmp: tempfile::TempPath,
    digest: Digest,
    len: u64,
}

impl StagedBlob {
    pub(crate) fn digest(&self) -> Digest {
        self.digest
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }
}

impl Cas {
    /// Stream `reader` into scratch space, hashing it, and fail as corrupt
    /// once it passes `max_bytes`. Reading stops one byte past the limit, so
    /// an oversized source never costs more than the limit in disk space.
    pub(crate) async fn stage_stream<R: AsyncRead + Unpin>(
        &self,
        reader: R,
        max_bytes: u64,
    ) -> Result<StagedBlob> {
        let (tmp, digest) = self
            .stream_to_tmp("staged", reader.take(max_bytes.saturating_add(1)))
            .await?;
        let len = fs::metadata(&tmp)
            .await
            .map_err(|source| Error::Io {
                path: tmp.to_path_buf(),
                source,
            })?
            .len();
        if len > max_bytes {
            return Err(Error::corrupt_blob(
                tmp.to_path_buf(),
                digest,
                &format!("stream exceeded its declared {max_bytes} bytes"),
            ));
        }
        Ok(StagedBlob { tmp, digest, len })
    }

    /// Make a staged blob visible under its digest.
    pub(crate) async fn commit_staged(&self, staged: StagedBlob) -> Result<Digest> {
        let digest = staged.digest;
        self.commit_stream(staged.tmp, &digest).await?;
        Ok(digest)
    }
}
