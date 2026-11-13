//! Reply blobs have a shorter lifetime than downloads when values are off. The durable
//! deadline must survive a crash before any SQLite run row commits; minute upkeep uses the
//! deadlines registered by writes or startup recovery rather than rescanning output folders.

use std::error::Error;

use neo_trace::{
    BlobId, DownloadStatus, EpochMillis, KeptFile, RunDownload, RunId, RunOutputs, TraceStore,
};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BLOB: BlobId = BlobId::new(0);
const BLOB_BYTES: &[u8] = b"returned image bytes";

fn at(minutes: i64) -> EpochMillis {
    EpochMillis::from_millis(1_800_000_000_000).plus_millis(minutes * 60_000)
}

async fn open(dir: &TempDir) -> TestResult<TraceStore> {
    Ok(TraceStore::open(&dir.path().join("trace.db")).await?)
}

async fn contents(KeptFile { mut file, size }: KeptFile) -> TestResult<Vec<u8>> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    assert_eq!(size, u64::try_from(bytes.len())?);
    Ok(bytes)
}

async fn blob_bytes(outputs: &RunOutputs, run: RunId, blob: BlobId) -> TestResult<Option<Vec<u8>>> {
    match outputs.open_blob(run, blob).await? {
        Some(kept) => Ok(Some(contents(kept).await?)),
        None => Ok(None),
    }
}

#[tokio::test]
async fn off_mode_blobs_expire_after_ten_minutes_and_leave_downloads() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = open(&dir).await?;
    let outputs = store.outputs();
    let run = RunId::new();
    let second_blob = BlobId::new(1);
    outputs.keep_blobs_until(run, at(10)).await?;
    outputs.keep_blob(run, BLOB, BLOB_BYTES).await?;
    outputs.keep_blob(run, second_blob, b"second image").await?;

    let source = dir.path().join("statement.txt");
    tokio::fs::write(&source, b"downloaded statement").await?;
    let size = outputs.keep_download(run, 0, &source).await?;
    let download = RunDownload {
        index: 0,
        url: "https://example.com/statement.txt".to_owned(),
        suggested_filename: "statement.txt".to_owned(),
        path: Some(source.to_string_lossy().into_owned()),
        size: Some(size),
        state: DownloadStatus::Completed,
        failure: None,
    };
    outputs
        .record_downloads(run, std::slice::from_ref(&download))
        .await?;

    for now in [at(0), at(9), at(10).plus_millis(-1)] {
        outputs.expire_blobs(now).await?;
        assert_eq!(
            blob_bytes(outputs, run, BLOB).await?.as_deref(),
            Some(BLOB_BYTES)
        );
        assert_eq!(
            blob_bytes(outputs, run, second_blob).await?.as_deref(),
            Some(b"second image".as_slice())
        );
    }

    // Clones share the registration list used by the app's separate upkeep task.
    outputs.clone().expire_blobs(at(10)).await?;
    assert!(outputs.open_blob(run, BLOB).await?.is_none());
    assert!(outputs.open_blob(run, second_blob).await?.is_none());
    assert_eq!(outputs.downloads(run).await?, vec![download]);
    let kept = outputs
        .open_download(run, 0)
        .await?
        .ok_or("expiry removed the download")?;
    assert_eq!(contents(kept).await?.as_slice(), b"downloaded statement");
    Ok(())
}

#[tokio::test]
async fn startup_expires_orphaned_blobs_without_a_trace_run_row() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    {
        let store = open(&dir).await?;
        store.outputs().keep_blobs_until(run, at(10)).await?;
        store.outputs().keep_blob(run, BLOB, BLOB_BYTES).await?;
        assert!(store.run_summary(run).await?.is_none());
    }

    // The reply's bytes reached disk before the trace writer committed any run record.
    let restarted = open(&dir).await?;
    assert!(restarted.run_summary(run).await?.is_none());
    assert!(restarted.outputs().open_blob(run, BLOB).await?.is_some());
    restarted.outputs().restore_blob_expiries(at(12)).await?;
    assert!(restarted.outputs().open_blob(run, BLOB).await?.is_none());
    restarted.outputs().restore_blob_expiries(at(12)).await?;
    Ok(())
}

#[tokio::test]
async fn startup_tracks_future_expiry_without_resetting_the_deadline() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    {
        let store = open(&dir).await?;
        store.outputs().keep_blobs_until(run, at(10)).await?;
        store.outputs().keep_blob(run, BLOB, BLOB_BYTES).await?;
    }

    let restarted = open(&dir).await?;
    let outputs = restarted.outputs();
    outputs.restore_blob_expiries(at(9)).await?;
    outputs.expire_blobs(at(9)).await?;
    assert_eq!(
        blob_bytes(outputs, run, BLOB).await?.as_deref(),
        Some(BLOB_BYTES)
    );
    outputs.expire_blobs(at(10)).await?;
    assert!(outputs.open_blob(run, BLOB).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn startup_removes_malformed_expiries_and_keeps_downloads() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    let sidecar = dir
        .path()
        .join("outputs")
        .join(run.to_string())
        .join("blob-expiry");
    for invalid in [
        b"".as_slice(),
        b"not a timestamp".as_slice(),
        b"9223372036854775808",
        b"\xff",
    ] {
        {
            let store = open(&dir).await?;
            let outputs = store.outputs();
            outputs.keep_blobs_until(run, at(10)).await?;
            outputs.keep_blob(run, BLOB, BLOB_BYTES).await?;
            let source = dir.path().join("statement.txt");
            tokio::fs::write(&source, b"downloaded statement").await?;
            outputs.keep_download(run, 0, &source).await?;
            outputs.record_downloads(run, &[]).await?;
            tokio::fs::write(&sidecar, invalid).await?;
        }

        // A fresh process must serve even if a sidecar was truncated, overflowed or not UTF-8.
        let restarted = open(&dir).await?;
        restarted.outputs().restore_blob_expiries(at(1)).await?;
        assert!(restarted.outputs().open_blob(run, BLOB).await?.is_none());
        assert!(!tokio::fs::try_exists(&sidecar).await?);
        assert!(restarted.outputs().open_download(run, 0).await?.is_some());
        assert!(tokio::fs::try_exists(sidecar.with_file_name("downloads.json")).await?);
    }
    Ok(())
}

#[tokio::test]
async fn on_mode_blobs_survive_upkeep_and_startup() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    {
        let store = open(&dir).await?;
        // The default mode writes no expiry: its ordinary retention remains unchanged.
        store.outputs().keep_blob(run, BLOB, BLOB_BYTES).await?;
        store.outputs().expire_blobs(at(20)).await?;
        assert_eq!(
            blob_bytes(store.outputs(), run, BLOB).await?.as_deref(),
            Some(BLOB_BYTES)
        );
    }

    let restarted = open(&dir).await?;
    restarted.outputs().restore_blob_expiries(at(30)).await?;
    restarted.outputs().expire_blobs(at(60)).await?;
    assert_eq!(
        blob_bytes(restarted.outputs(), run, BLOB).await?.as_deref(),
        Some(BLOB_BYTES)
    );
    Ok(())
}

#[tokio::test]
async fn an_expiry_without_any_blob_is_safe_to_restore_and_clean() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    {
        let store = open(&dir).await?;
        // Arming expiry precedes writing bytes, so a crash can leave only this sidecar.
        store.outputs().keep_blobs_until(run, at(10)).await?;
    }

    let restarted = open(&dir).await?;
    let outputs = restarted.outputs();
    outputs.restore_blob_expiries(at(9)).await?;
    outputs.expire_blobs(at(10)).await?;
    outputs.expire_blobs(at(11)).await?;
    outputs.restore_blob_expiries(at(12)).await?;
    assert!(outputs.open_blob(run, BLOB).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn upkeep_discovers_no_sidecars_until_startup_restores_them() -> TestResult {
    let dir = tempfile::tempdir()?;
    let run = RunId::new();
    let writer = open(&dir).await?;
    // A separately opened store has its own registration list, unlike a cheap store clone.
    let unaware = open(&dir).await?;
    writer.outputs().keep_blobs_until(run, at(10)).await?;
    writer.outputs().keep_blob(run, BLOB, BLOB_BYTES).await?;

    unaware.outputs().expire_blobs(at(10)).await?;
    assert_eq!(
        blob_bytes(unaware.outputs(), run, BLOB).await?.as_deref(),
        Some(BLOB_BYTES)
    );
    unaware.outputs().restore_blob_expiries(at(10)).await?;
    assert!(unaware.outputs().open_blob(run, BLOB).await?.is_none());
    Ok(())
}
