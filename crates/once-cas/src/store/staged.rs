use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::Cas;
use crate::{blob, Digest, Error, Result};

/// An owned, read-only staging lease for a decoded blob.
///
/// Raw storage may be pinned without copying. No mutable file handle or path is
/// exposed, and dropping the lease removes its temporary storage.
pub struct StagedBlob {
    directory: Arc<tempfile::TempDir>,
    path: PathBuf,
}

impl std::fmt::Debug for StagedBlob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("StagedBlob").finish_non_exhaustive()
    }
}

impl StagedBlob {
    /// Open a bounded-memory reader, retaining the lease until it is dropped.
    pub fn into_reader(self) -> Result<StagedBlobReader> {
        let file = std::fs::File::open(&self.path).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        Ok(StagedBlobReader { file, _lease: self })
    }
}

/// Read and seek within a staged blob without exposing mutable storage access.
pub struct StagedBlobReader {
    file: std::fs::File,
    _lease: StagedBlob,
}

impl Read for StagedBlobReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}

impl Seek for StagedBlobReader {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl Cas {
    /// Stage a blob for read-only access under `parent`.
    ///
    /// Uses bounded decoding when raw storage cannot be pinned on this filesystem.
    pub async fn stage_blob(&self, digest: &Digest, parent: &Path) -> Result<StagedBlob> {
        self.stage_blob_guarded(digest, parent, ()).await
    }

    /// Stage a blob while retaining `guard` through any blocking decoding work.
    ///
    /// Cancellation does not release the guard or temporary directory while a
    /// decoder still uses them. The completed lease does not retain the guard.
    pub async fn stage_blob_guarded<G: Send + 'static>(
        &self,
        digest: &Digest,
        parent: &Path,
        guard: G,
    ) -> Result<StagedBlob> {
        self.stage_blob_with(digest, parent, guard, link_source, |reader, writer| {
            let mut reader = io::BufReader::with_capacity(512 * 1024, reader);
            io::copy(&mut reader, writer).map(|_| ())
        })
        .await
    }

    async fn stage_blob_with<G: Send + 'static>(
        &self,
        digest: &Digest,
        parent: &Path,
        guard: G,
        link: fn(&Path, &Path) -> io::Result<()>,
        copy: impl FnOnce(&mut dyn Read, &mut dyn io::Write) -> io::Result<()> + Send + 'static,
    ) -> Result<StagedBlob> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(".once-blob-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = Arc::new(builder.tempdir_in(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?);
        let original = self.blob_path(digest);
        let linked = directory.path().join("stored");
        let pinned = match link(&original, &linked) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Error::BlobNotFound(*digest));
            }
            Err(_) => false,
        };
        let source = if pinned { linked } else { original };
        if pinned {
            let file = std::fs::File::open(&source).map_err(|source_error| Error::Io {
                path: source.clone(),
                source: source_error,
            })?;
            let mut prefix = Vec::with_capacity(blob::ZSTD_BLOB_MAGIC.len());
            file.take(u64::try_from(blob::ZSTD_BLOB_MAGIC.len()).unwrap_or(u64::MAX))
                .read_to_end(&mut prefix)
                .map_err(|source_error| Error::Io {
                    path: source.clone(),
                    source: source_error,
                })?;
            if prefix != blob::ZSTD_BLOB_MAGIC {
                return Ok(StagedBlob {
                    directory,
                    path: source,
                });
            }
        }
        let destination = directory.path().join("decoded");
        let error_path = destination.clone();
        let lease = StagedBlob {
            directory,
            path: destination.clone(),
        };
        let owner = Arc::clone(&lease.directory);
        tokio::task::spawn_blocking(move || {
            let _owners = (owner, guard);
            blob::decode_file(&source, &destination, copy)
        })
        .await
        .map_err(|source| Error::Io {
            path: error_path.clone(),
            source: io::Error::other(source.to_string()),
        })?
        .map_err(|source| Error::Io {
            path: error_path,
            source,
        })?;
        Ok(lease)
    }
}

fn link_source(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::hard_link(source, destination)
}

#[cfg(test)]
#[path = "staged_tests.rs"]
mod tests;
