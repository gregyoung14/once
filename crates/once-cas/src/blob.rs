use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use zstd::stream::{copy_encode, decode_all, encode_all};

use crate::integrity::{self, CorruptBlob, HashingWriter};
use crate::Digest;

pub(crate) const ZSTD_BLOB_MAGIC: &[u8] = b"once.cas.zstd.v1\0";
pub(crate) const ZSTD_BLOB_HEADER_LEN: usize = ZSTD_BLOB_MAGIC.len() + RAW_SIZE_LEN;
const ZSTD_LEVEL: i32 = 3;
const RAW_SIZE_LEN: usize = 8;

pub(crate) fn encode_bytes(raw: &[u8]) -> io::Result<Cow<'_, [u8]>> {
    let compressed = encode_all(raw, ZSTD_LEVEL)?;
    let wrapped_len = checked_wrapped_len(compressed.len())?;
    if raw.starts_with(ZSTD_BLOB_MAGIC) || wrapped_len < raw.len() {
        let mut out = Vec::with_capacity(wrapped_len);
        out.extend_from_slice(ZSTD_BLOB_MAGIC);
        out.extend_from_slice(&raw_len_header(raw.len())?);
        out.extend_from_slice(&compressed);
        Ok(Cow::Owned(out))
    } else {
        Ok(Cow::Borrowed(raw))
    }
}

pub(crate) fn decode_bytes(stored: Vec<u8>) -> io::Result<Vec<u8>> {
    if !stored.starts_with(ZSTD_BLOB_MAGIC) {
        return Ok(stored);
    }
    let Some(raw_len) = stored
        .get(ZSTD_BLOB_MAGIC.len()..ZSTD_BLOB_HEADER_LEN)
        .map(read_raw_size)
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated zstd blob header",
        ));
    };
    let compressed = stored
        .get(ZSTD_BLOB_HEADER_LEN..)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "truncated zstd blob body"))?;
    let decoded = decode_all(compressed)?;
    if decoded.len() as u64 != raw_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zstd blob decoded size mismatch",
        ));
    }
    Ok(decoded)
}

pub(crate) fn raw_size_from_header(header: &[u8]) -> Option<u64> {
    header
        .starts_with(ZSTD_BLOB_MAGIC)
        .then(|| {
            header
                .get(ZSTD_BLOB_MAGIC.len()..ZSTD_BLOB_HEADER_LEN)
                .map(read_raw_size)
        })
        .flatten()
}

pub(crate) fn encode_file(raw_path: &Path, encoded_path: &Path) -> io::Result<EncodedFile> {
    let raw_size = std::fs::metadata(raw_path)?.len();
    let raw_starts_with_magic = file_starts_with(raw_path, ZSTD_BLOB_MAGIC)?;

    let mut input = File::open(raw_path)?;
    let mut output = File::create(encoded_path)?;
    output.write_all(ZSTD_BLOB_MAGIC)?;
    output.write_all(&raw_size.to_le_bytes())?;
    copy_encode(&mut input, &mut output, ZSTD_LEVEL)?;
    output.sync_all()?;

    let encoded_size = output.metadata()?.len();
    Ok(EncodedFile {
        should_store: raw_starts_with_magic || encoded_size < raw_size,
    })
}

/// Decode the stored blob at `stored_path` into `output_path`, hashing the
/// decoded bytes as they are written. The output is only persisted when they
/// hash to `expected`; undecodable or mismatching content is reported as a
/// corrupt blob and leaves `output_path` untouched.
///
/// `stop` is checked between chunks and once more before the output is
/// published, so a restore whose caller went away does not write the
/// destination afterwards.
pub(crate) fn decode_file(
    stored_path: &Path,
    output_path: &Path,
    expected: &Digest,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut input = File::open(stored_path)?;
    let mut header = vec![0_u8; ZSTD_BLOB_HEADER_LEN];
    let mut header_len = 0;
    while header_len < header.len() {
        let read = input.read(&mut header[header_len..])?;
        if read == 0 {
            break;
        }
        header_len += read;
    }
    input.rewind()?;
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    let mut output = HashingWriter::new(tempfile::NamedTempFile::new_in(parent)?);
    let written = if header_len == ZSTD_BLOB_HEADER_LEN && header.starts_with(ZSTD_BLOB_MAGIC) {
        let raw_size = raw_size_from_header(&header)
            .ok_or_else(|| CorruptBlob::io_error(*expected, "truncated zstd blob header"))?;
        input.seek(io::SeekFrom::Start(ZSTD_BLOB_HEADER_LEN as u64))?;
        let decoder = zstd::stream::read::Decoder::new(input)?;
        let written = copy_chunks(
            decoder.take(raw_size.saturating_add(1)),
            &mut output,
            Some(expected),
            stop,
        )?;
        if written != raw_size {
            return Err(CorruptBlob::io_error(
                *expected,
                "zstd blob decoded size mismatch",
            ));
        }
        written
    } else {
        copy_chunks(input, &mut output, None, stop)?
    };
    integrity::verify(expected, &output.digest())?;
    if stop.load(Ordering::Acquire) {
        return Err(cancelled());
    }
    let output = output.into_inner();
    output.as_file().sync_all()?;
    output.persist(output_path).map_err(|error| error.error)?;
    tracing::trace!(bytes = written, blob_digest = %expected, "restored verified blob");
    Ok(())
}

/// Copies bytes in bounded chunks until `stop` is set. When `decoding` names
/// the blob being decoded, a decoder failure that is not an operating system
/// error is reported as corruption of that stored blob.
fn copy_chunks(
    mut input: impl Read,
    output: &mut impl Write,
    decoding: Option<&Digest>,
    stop: &AtomicBool,
) -> io::Result<u64> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut written = 0_u64;
    loop {
        if stop.load(Ordering::Acquire) {
            return Err(cancelled());
        }
        let read = match input.read(&mut buffer) {
            Ok(0) => return Ok(written),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => match decoding {
                Some(expected) if error.raw_os_error().is_none() => {
                    return Err(CorruptBlob::io_error(
                        *expected,
                        &format!("decoding stored blob: {error}"),
                    ));
                }
                _ => return Err(error),
            },
        };
        output.write_all(&buffer[..read])?;
        written += read as u64;
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "blob restore was cancelled")
}

/// Sets its flag when dropped, telling blocking work that its caller is gone.
pub(crate) struct StopOnDrop(pub(crate) Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(crate) struct EncodedFile {
    pub(crate) should_store: bool,
}

fn read_raw_size(bytes: &[u8]) -> u64 {
    let mut raw = [0_u8; RAW_SIZE_LEN];
    raw.copy_from_slice(bytes);
    u64::from_le_bytes(raw)
}

fn checked_wrapped_len(compressed_len: usize) -> io::Result<usize> {
    ZSTD_BLOB_HEADER_LEN
        .checked_add(compressed_len)
        .ok_or_else(blob_too_large)
}

fn raw_len_header(raw_len: usize) -> io::Result<[u8; RAW_SIZE_LEN]> {
    let raw_len = u64::try_from(raw_len).map_err(|_| blob_too_large())?;
    Ok(raw_len.to_le_bytes())
}

#[cfg(test)]
fn raw_len_header_from_u128(raw_len: u128) -> io::Result<[u8; RAW_SIZE_LEN]> {
    let raw_len = u64::try_from(raw_len).map_err(|_| blob_too_large())?;
    Ok(raw_len.to_le_bytes())
}

fn blob_too_large() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "blob is too large")
}

fn file_starts_with(path: &Path, needle: &[u8]) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut prefix = vec![0_u8; needle.len()];
    match file.read_exact(&mut prefix) {
        Ok(()) => Ok(prefix == needle),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_payload_stays_raw_when_compression_would_grow_it() {
        let raw = b"abc";

        let encoded = encode_bytes(raw).unwrap();

        assert_eq!(encoded.as_ref(), raw);
        assert!(matches!(encoded, Cow::Borrowed(_)));
        assert_eq!(decode_bytes(encoded.into_owned()).unwrap(), raw);
    }

    #[test]
    fn compressible_payload_uses_zstd_wrapper() {
        let raw = b"same line\n".repeat(1024);

        let encoded = encode_bytes(&raw).unwrap();

        assert!(encoded.starts_with(ZSTD_BLOB_MAGIC));
        assert!(encoded.len() < raw.len());
        assert!(matches!(encoded, Cow::Owned(_)));
        assert_eq!(decode_bytes(encoded.into_owned()).unwrap(), raw);
    }

    #[test]
    fn magic_prefixed_payload_gets_wrapped_even_when_small() {
        let mut raw = Vec::from(ZSTD_BLOB_MAGIC);
        raw.extend_from_slice(b"literal");

        let encoded = encode_bytes(&raw).unwrap();

        assert!(encoded.starts_with(ZSTD_BLOB_MAGIC));
        assert!(matches!(encoded, Cow::Owned(_)));
        assert_eq!(decode_bytes(encoded.into_owned()).unwrap(), raw);
    }

    #[test]
    fn a_stopped_restore_never_publishes_its_output() {
        let temp = tempfile::TempDir::new().unwrap();
        let payload = b"same line\n".repeat(64 * 1024);
        let digest = Digest::of_bytes(&payload);
        for stored in [
            encode_bytes(&payload).unwrap().into_owned(),
            payload.clone(),
        ] {
            let stored_path = temp.path().join("stored");
            std::fs::write(&stored_path, stored).unwrap();
            let output = temp.path().join("output");
            let stop = AtomicBool::new(true);

            let error = decode_file(&stored_path, &output, &digest, &stop).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert!(!output.exists());
            let left: Vec<_> = std::fs::read_dir(temp.path()).unwrap().collect();
            assert_eq!(left.len(), 1, "only the stored blob remains");
            decode_file(&stored_path, &output, &digest, &AtomicBool::new(false)).unwrap();
            assert_eq!(std::fs::read(&output).unwrap(), payload);
            std::fs::remove_file(&output).unwrap();
        }
    }

    #[test]
    fn checked_wrapped_len_rejects_overflow() {
        let err = checked_wrapped_len(usize::MAX).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "blob is too large");
    }

    #[test]
    fn raw_len_header_rejects_u64_overflow() {
        let err = raw_len_header_from_u128(u128::from(u64::MAX) + 1).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "blob is too large");
    }
}
