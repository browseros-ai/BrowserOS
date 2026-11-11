//! A run's downloads and reply blobs as a host reads them: downloads listed on
//! `GET /v1/runs/{id}` with their kept bytes at `GET /v1/runs/{id}/downloads/{index}`, and blobs
//! at `GET /v1/runs/{id}/blobs/{blobId}`. claw-sdk writes them beside the trace when the run
//! ends (`RunOutputs`), as these tests do.

use crate::support::{Start, TestResult, end_run, harness, start_run};
use axum::http::StatusCode;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use neo_trace::{BlobId, DownloadStatus, RunDownload, TraceStore};

#[tokio::test]
async fn a_runs_kept_download_is_listed_and_served() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    end_run(&h.sink, run, None);
    let store = TraceStore::open(&h.dir.path().join("trace.db")).await?;
    let source = h.dir.path().join("statement (1).txt");
    tokio::fs::write(&source, b"balance 42").await?;
    let size = store.outputs().keep_download(run, 0, &source).await?;
    let completed = RunDownload {
        index: 0,
        url: "https://bank.example/statement.txt".to_owned(),
        suggested_filename: "statement ü.txt".to_owned(),
        path: Some(source.to_string_lossy().into_owned()),
        size: Some(size),
        state: DownloadStatus::Completed,
        failure: None,
    };
    let canceled = RunDownload {
        index: 1,
        url: "https://bank.example/big.zip".to_owned(),
        suggested_filename: "big.zip".to_owned(),
        path: None,
        size: None,
        state: DownloadStatus::Failed,
        failure: Some("canceled".to_owned()),
    };
    store
        .outputs()
        .record_downloads(run, &[completed, canceled])
        .await?;
    h.flushed().await?;

    let reply = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let downloads = &reply.body["downloads"];
    assert_eq!(downloads[0]["state"], "completed", "{downloads}");
    assert_eq!(downloads[0]["size"], 10);
    assert_eq!(downloads[0]["suggestedFilename"], "statement ü.txt");
    assert_eq!(downloads[1]["state"], "failed");
    assert_eq!(downloads[1]["failure"], "canceled");

    let (status, headers, bytes) = h.get_bytes(&format!("/v1/runs/{run}/downloads/0")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"balance 42");
    assert_eq!(
        headers.get(CONTENT_TYPE).map(|value| value.as_bytes()),
        Some(&b"application/octet-stream"[..])
    );
    assert_eq!(
        headers
            .get(CONTENT_DISPOSITION)
            .map(|value| value.as_bytes()),
        Some(&b"attachment; filename*=UTF-8''statement%20%C3%BC.txt"[..])
    );
    for missing in ["1", "7", "x"] {
        let (status, _, _) = h
            .get_bytes(&format!("/v1/runs/{run}/downloads/{missing}"))
            .await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "download {missing}");
    }
    Ok(())
}

#[tokio::test]
async fn a_run_handed_no_download_lists_none() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    end_run(&h.sink, run, None);
    h.flushed().await?;
    let reply = h.get(&format!("/v1/runs/{run}")).await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(reply.body.get("downloads").is_none(), "{}", reply.body);
    Ok(())
}

/// A reply's blob: written beside the trace before the reply, served by its id, nothing else.
#[tokio::test]
async fn a_runs_blob_is_served_by_its_id() -> TestResult {
    let h = harness().await?;
    let run = start_run(&h.sink, Start::default());
    end_run(&h.sink, run, None);
    let store = TraceStore::open(&h.dir.path().join("trace.db")).await?;
    let png = [&b"\x89PNG\r\n\x1a\n"[..], &[7_u8; 300_000][..]].concat();
    store.outputs().keep_blob(run, BlobId::new(0), &png).await?;
    let (status, headers, bytes) = h.get_bytes(&format!("/v1/runs/{run}/blobs/b0")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png);
    assert_eq!(
        headers.get(CONTENT_TYPE).map(|value| value.as_bytes()),
        Some(&b"application/octet-stream"[..])
    );
    for missing in ["b1", "b00", "0", "../b0"] {
        let (status, _, _) = h
            .get_bytes(&format!("/v1/runs/{run}/blobs/{missing}"))
            .await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "blob {missing}");
    }
    Ok(())
}
