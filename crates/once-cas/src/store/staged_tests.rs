use super::*;

#[tokio::test]
async fn staged_raw_storage_is_pinned_and_reader_owns_its_lease() {
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let parent = temp.path().join("staging");
    std::fs::create_dir(&parent).unwrap();
    let payload = b"immutable raw payload";
    let digest = cas.put_blob(payload).await.unwrap();
    let lease = cas.stage_blob(&digest, &parent).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        assert_eq!(
            std::fs::metadata(&lease.path).unwrap().ino(),
            std::fs::metadata(cas.blob_path(&digest)).unwrap().ino()
        );
        assert_eq!(
            std::fs::metadata(lease.directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::remove_file(cas.blob_path(&digest)).unwrap();
    let mut reader = lease.into_reader().unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, payload);
    reader.rewind().unwrap();
    bytes.clear();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, payload);
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
    drop(reader);
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[tokio::test]
async fn staged_decoding_covers_compression_literal_magic_empty_and_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let parent = temp.path().join("staging");
    std::fs::create_dir(&parent).unwrap();
    for payload in [
        Vec::new(),
        vec![b'x'; 1024 * 1024],
        blob::ZSTD_BLOB_MAGIC.to_vec(),
    ] {
        let digest = cas.put_blob(&payload).await.unwrap();
        let mut reader = cas
            .stage_blob(&digest, &parent)
            .await
            .unwrap()
            .into_reader()
            .unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, payload);
        drop(reader);
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }
    let digest = cas.put_blob(&vec![b'x'; 1024 * 1024]).await.unwrap();
    let stored = cas.blob_path(&digest);
    let original = std::fs::read(&stored).unwrap();
    for truncated in [true, false] {
        let mut encoded = original.clone();
        if truncated {
            encoded.truncate(blob::ZSTD_BLOB_HEADER_LEN + 1);
        } else {
            encoded[blob::ZSTD_BLOB_MAGIC.len()..blob::ZSTD_BLOB_HEADER_LEN]
                .copy_from_slice(&(1024_u64 * 1024 + 1).to_le_bytes());
        }
        std::fs::write(&stored, encoded).unwrap();
        assert!(cas.stage_blob(&digest, &parent).await.is_err());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }
    assert!(matches!(
        cas.stage_blob(&Digest::of_bytes(b"missing"), &parent).await,
        Err(Error::BlobNotFound(_))
    ));
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[tokio::test]
async fn mutable_copies_cannot_change_a_pinned_blob() {
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let digest = cas.put_blob(b"immutable").await.unwrap();
    let lease = cas.stage_blob(&digest, temp.path()).await.unwrap();
    let destination = temp.path().join("mutable");
    cas.copy_blob_to_file(&digest, &destination).await.unwrap();
    std::fs::write(&destination, b"changed").unwrap();
    assert_eq!(cas.get_blob(&digest).await.unwrap(), b"immutable");
    let mut reader = lease.into_reader().unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"immutable");
}

#[tokio::test]
async fn decode_write_and_publication_failures_remove_every_lease() {
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let digest = cas.put_blob(&vec![b'x'; 1024 * 1024]).await.unwrap();
    let parent = temp.path().join("staging");
    std::fs::create_dir(&parent).unwrap();
    let failed = cas
        .stage_blob_with(&digest, &parent, (), link_source, |reader, writer| {
            io::copy(&mut reader.take(1024), writer)?;
            Err(io::Error::other("injected copy failure"))
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    let owned_parent = parent.clone();
    let failed = cas
        .stage_blob_with(&digest, &parent, (), link_source, move |reader, writer| {
            let mut reader = io::BufReader::with_capacity(512 * 1024, reader);
            io::copy(&mut reader, writer)?;
            let directory = std::fs::read_dir(owned_parent)?.next().unwrap()?.path();
            std::fs::create_dir(directory.join("decoded"))
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
}

#[tokio::test]
async fn unsupported_links_fall_back_to_independent_bounded_decoding() {
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let parent = temp.path().join("staging");
    std::fs::create_dir(&parent).unwrap();
    for payload in [b"raw".to_vec(), vec![b'x'; 1024 * 1024]] {
        let digest = cas.put_blob(&payload).await.unwrap();
        let lease = cas
            .stage_blob_with(
                &digest,
                &parent,
                (),
                |_, _| Err(io::Error::other("links unsupported")),
                |reader, writer| {
                    let mut reader = io::BufReader::with_capacity(512 * 1024, reader);
                    io::copy(&mut reader, writer).map(|_| ())
                },
            )
            .await
            .unwrap();
        std::fs::remove_file(cas.blob_path(&digest)).unwrap();
        let mut reader = lease.into_reader().unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, payload);
        drop(reader);
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn representation_is_classified_on_the_pinned_inode_after_republication() {
    fn replacing_link(source: &Path, destination: &Path) -> io::Result<()> {
        link_source(source, destination)?;
        std::fs::remove_file(source)?;
        std::fs::write(source, blob::ZSTD_BLOB_MAGIC)
    }
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let digest = cas.put_blob(b"original raw").await.unwrap();
    let lease = cas
        .stage_blob_with(&digest, temp.path(), (), replacing_link, |_, _| {
            panic!("the pinned raw blob does not need decoding")
        })
        .await
        .unwrap();
    let mut reader = lease.into_reader().unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"original raw");
}

#[tokio::test]
async fn cancellation_retains_decoder_owner_and_private_lease_until_worker_finishes() {
    struct Guard(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let cas = Cas::open(temp.path().join("cas"));
    let parent_owner = Arc::new(tempfile::tempdir_in(temp.path()).unwrap());
    let parent = parent_owner.path().to_path_buf();
    let digest = cas.put_blob(&vec![b'x'; 1024 * 1024]).await.unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let marker = Arc::clone(&dropped);
    let task_parent = parent.clone();
    let task = tokio::spawn(async move {
        cas.stage_blob_with(
            &digest,
            &task_parent,
            (parent_owner, Guard(marker)),
            link_source,
            move |reader, writer| {
                let mut reader = io::BufReader::with_capacity(512 * 1024, reader);
                io::copy(&mut reader, writer)?;
                entered.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                Ok(())
            },
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let directory = std::fs::read_dir(&parent)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(std::fs::read_dir(directory).unwrap().any(|entry| {
        std::fs::File::open(entry.unwrap().path())
            .unwrap()
            .metadata()
            .unwrap()
            .len()
            == 1024 * 1024
    }));
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while parent.exists() || !dropped.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
