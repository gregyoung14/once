use std::io::{self, Cursor};
use std::time::Duration;

use bazel_remote_apis::google::bytestream::ReadResponse;
use futures::stream;
use tokio_util::io::StreamReader;
use tonic::{Status, Streaming};

use super::{retry, stall};
use crate::{Cas, Digest, Error};

pub(super) async fn store(
    cache: &Cas,
    stream: Streaming<ReadResponse>,
    timeout: Duration,
    expected: u64,
    compressed: bool,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> std::result::Result<Digest, retry::Failure> {
    let chunks = stream::try_unfold(stream, move |mut stream| async move {
        let message = stall::next_message(&mut stream, timeout)
            .await
            .map_err(io::Error::other)?;
        Ok::<_, io::Error>(message.map(|response| (Cursor::new(response.data), stream)))
    });
    let reader = StreamReader::new(Box::pin(chunks));
    cache
        .mirror_remote_stream(reader, expected, compressed, permit)
        .await
        .map_err(read_error)
}

fn read_error(error: Error) -> retry::Failure {
    if let Error::Io { source, .. } = &error {
        if let Some(status) = source
            .get_ref()
            .and_then(|source| source.downcast_ref::<Status>())
        {
            return retry::Failure::Rpc(status.clone());
        }
    }
    error.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn transport_status_survives_native_store_io_errors() {
        let status =
            Status::with_details(Code::Unavailable, "interrupted", b"details".to_vec().into());
        let error = Error::Io {
            path: "scratch".into(),
            source: io::Error::other(status),
        };
        let retry::Failure::Rpc(status) = read_error(error) else {
            panic!("transport status was lost");
        };
        assert_eq!(status.code(), Code::Unavailable);
        assert_eq!(status.message(), "interrupted");
        assert_eq!(status.details(), b"details");
        assert!(retry::retryable(&status));
    }

    #[test]
    fn filesystem_errors_keep_their_local_path_and_kind() {
        let error = Error::Io {
            path: "scratch".into(),
            source: io::ErrorKind::PermissionDenied.into(),
        };
        let retry::Failure::Local(Error::Io { path, source }) = read_error(error) else {
            panic!("filesystem error was lost");
        };
        assert_eq!(path, std::path::Path::new("scratch"));
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
    }
}
