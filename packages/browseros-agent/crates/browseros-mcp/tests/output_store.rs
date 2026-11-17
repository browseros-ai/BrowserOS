use browseros_mcp::output_store::OutputStore;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn image_retrieval_accepts_four_mib_and_keeps_larger_parts_readable_as_chunks()
-> anyhow::Result<()> {
    use base64::Engine;
    use browseros_mcp::{ToolExecution, execute_tool};
    let directory = tempfile::tempdir()?;
    let store = OutputStore::new(directory.path().join("outputs"), "session");
    for size in [3 * 1024 * 1024, 4 * 1024 * 1024, 4 * 1024 * 1024 + 1] {
        let call = store.begin_call();
        // Retrieval transfers already-captured bytes without rendering or re-encoding them.
        let bytes = vec![42; size];
        call.write_part("capture", "png", &bytes).await?;
        let output = call
            .finish(&json!({"done":true}))
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing image output"))?;
        let result = execute_tool(
            &browseros_mcp::tools::read_output::definition(),
            json!({"outputId": output.id, "part":"capture.png"}),
            ToolExecution::Outputs(&store),
        )
        .await;
        if size <= 4 * 1024 * 1024 {
            let result = result.map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let image = result
                .content
                .iter()
                .find_map(|block| block.as_image())
                .ok_or_else(|| anyhow::anyhow!("missing inline image"))?;
            assert!(base64::engine::general_purpose::STANDARD.decode(&image.data)? == bytes);
        } else {
            assert!(
                result.is_err(),
                "oversized image should require chunked retrieval"
            );
            let chunk = execute_tool(
                &browseros_mcp::tools::read_output::definition(),
                json!({"outputId": output.id, "part":"capture.png", "encoding":"base64", "limit":1024}),
                ToolExecution::Outputs(&store),
            ).await.map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let value = chunk
                .structured_content
                .ok_or_else(|| anyhow::anyhow!("missing chunk"))?;
            assert_eq!(value["nextOffset"], 1024);
            assert!(
                base64::engine::general_purpose::STANDARD.decode(
                    value["data"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing bytes"))?
                )? == bytes[..1024]
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_outputs_reserve_manifest_bytes_inside_the_session_limit() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("outputs");
    let first = OutputStore::new(root.clone(), "session");
    let bytes = vec![0; 64 * 1024 * 1024];
    let path = first.write_part("first", "bin", &bytes).await?;
    let second = OutputStore::new(root, "session");
    second.write_part("peer", "txt", b"peer").await?;
    // Leave just 64 payload bytes after both manifests' reservations. A fresh
    // service's admission scan must preserve the first active reservation too.
    first
        .write_part(
            "second",
            "bin",
            &bytes[..bytes.len() - 2 * 64 * 1024 - 4 - 64],
        )
        .await?;
    assert!(first.write_part("overflow", "txt", &[0; 65]).await.is_err());
    let output = first
        .finish(&json!({"done":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing bounded output"))?;
    second.finish(&json!({"done":true})).await?;
    assert!(first.describe(&output.id).await.is_ok());
    let session = path
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| anyhow::anyhow!("missing session directory"))?;
    let mut outputs = tokio::fs::read_dir(session).await?;
    let mut total = 0;
    while let Some(output) = outputs.next_entry().await? {
        let mut parts = tokio::fs::read_dir(output.path()).await?;
        while let Some(part) = parts.next_entry().await? {
            total += part.metadata().await?.len();
        }
    }
    assert!(total <= 128 * 1024 * 1024, "session bytes: {total}");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn root_cleanup_throttle_survives_the_last_service_being_dropped() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("outputs");
    let abandoned = OutputStore::new(root.clone(), "abandoned").with_retention(Duration::ZERO);
    let path = abandoned.write_part("part", "txt", b"expired").await?;
    abandoned.finish(&json!({"done":true})).await?;
    let session = path
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| anyhow::anyhow!("missing session directory"))?;
    std::fs::File::open(session)?
        .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))?;
    drop(abandoned);
    let next = OutputStore::new(root, "active");
    next.write_part("part", "txt", b"new").await?;
    assert!(
        path.exists(),
        "a fresh service repeated root cleanup inside its five-minute interval"
    );
    Ok(())
}

#[tokio::test]
async fn total_write_failure_is_distinct_from_an_ordinary_inline_call() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let occupied = directory.path().join("occupied");
    tokio::fs::write(&occupied, b"file").await?;
    let call = OutputStore::new(occupied, "session");
    assert!(call.write_part("capture", "png", b"bytes").await.is_err());
    assert!(call.finish(&json!({"done":true})).await.is_err());

    let copied = OutputStore::new(directory.path().join("copy"), "session");
    assert!(
        copied
            .copy_file("download", &directory.path().join("missing"))
            .await
            .is_err()
    );
    assert!(copied.finish(&json!({"done":true})).await.is_err());

    let image = OutputStore::new(directory.path().join("image"), "session");
    assert!(
        image
            .image("invalid base64", "image/png", &json!({}))
            .await
            .is_err()
    );
    assert!(image.finish(&json!({"done":true})).await.is_err());

    let inline = OutputStore::new(directory.path().join("inline"), "session");
    assert!(inline.finish(&json!({"done":true})).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn full_attachment_batches_keep_the_primary_slot_and_remain_retrievable() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir()?;
    let call = OutputStore::new(directory.path().join("outputs"), "session");
    for index in 0..63 {
        call.write_part("attachment", "txt", index.to_string().as_bytes())
            .await?;
    }
    assert!(
        call.write_part("overflow", "txt", b"overflow")
            .await
            .is_err()
    );
    let output = call
        .finish(&json!({"done":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("full batch was not published"))?;
    assert_eq!(output.parts.len(), 64);
    assert!(!output.warnings.is_empty());
    assert_eq!(
        call.read(&output.id, "attachment-63.txt", 0, 100)
            .await?
            .bytes,
        b"62"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &call.read(&output.id, "result.json", 0, 100).await?.bytes
        )?,
        json!({"done":true})
    );
    Ok(())
}

#[tokio::test]
async fn primary_write_failure_still_publishes_completed_attachments() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let call = OutputStore::new(directory.path().join("outputs"), "session");
    let path = call.write_part("capture", "png", b"capture bytes").await?;
    // An occupied primary path injects an actual create_new failure without
    // filling the developer's disk or weakening the store's production limits.
    tokio::fs::write(path.with_file_name("result.json"), b"occupied").await?;
    let output = call
        .finish(&json!({"done":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("attachments were hidden by primary failure"))?;
    assert_eq!(output.parts.len(), 1);
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("Primary result"))
    );
    assert_eq!(
        call.read_bytes(&output.id, "capture.png", 0, 100)
            .await?
            .bytes,
        b"capture bytes"
    );
    assert!(call.read(&output.id, "result.json", 0, 100).await.is_err());
    Ok(())
}

#[tokio::test]
async fn screenshot_previews_are_bounded_while_extra_captures_stay_readable() -> anyhow::Result<()>
{
    use base64::Engine;
    let directory = tempfile::tempdir()?;
    let call = OutputStore::new(directory.path().join("outputs"), "session");
    for index in 0..6 {
        let data = base64::engine::general_purpose::STANDARD.encode(format!("image {index}"));
        call.image(&data, "image/png", &json!({"index":index}))
            .await?;
    }
    assert_eq!(call.images().await.len(), 4);
    assert_eq!(call.omitted_images().await, 2);
    let output = call
        .finish(&json!({"done":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing screenshots"))?;
    assert_eq!(
        call.read_bytes(&output.id, "screenshot-6.png", 0, 100)
            .await?
            .bytes,
        b"image 5"
    );

    // A single capture can exceed the aggregate preview allowance. It remains
    // available as exact bytes without cloning another large Rust-owned string.
    let large = call.begin_call();
    let bytes = vec![0; 7 * 1024 * 1024];
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    large.image(&data, "image/png", &json!({})).await?;
    assert!(large.images().await.is_empty());
    assert_eq!(large.omitted_images().await, 1);
    let output = large
        .finish(&json!({"done":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing large screenshot"))?;
    assert_eq!(
        output
            .parts
            .iter()
            .find(|part| part.name == "screenshot.png")
            .map(|part| part.bytes),
        Some(bytes.len() as u64)
    );
    Ok(())
}

#[tokio::test]
async fn fresh_services_share_quota_admission_for_the_last_output_slot() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("outputs");
    let owner = OutputStore::new(root.clone(), "session");
    for _ in 0..127 {
        let call = owner.begin_call();
        call.write_part("part", "txt", b"saved").await?;
        call.finish(&json!({"done":true})).await?;
    }
    let first = OutputStore::new(root.clone(), "session");
    let second = OutputStore::new(root, "session");
    let (first, second) = tokio::join!(
        first.write_part("part", "txt", b"one"),
        second.write_part("part", "txt", b"two"),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    Ok(())
}

#[tokio::test]
async fn saved_call_retains_typed_parts_across_reconnect_and_rejects_other_sessions()
-> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("neo-output-test-{}", uuid::Uuid::new_v4()));
    let call = OutputStore::new(root.clone(), "owner-session");
    call.write_part("snapshot", "md", "hello 🦀 world".as_bytes())
        .await?;
    call.write_part("pdf", "pdf", b"%PDF-1.7\0\xff").await?;
    let output = call
        .finish(&json!({"value": [1, 2], "logs": ["done"]}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    assert_eq!(output.parts.len(), 3);

    let reconnected = OutputStore::new(root.clone(), "owner-session");
    let text = reconnected.read(&output.id, "snapshot.md", 0, 8).await?;
    assert_eq!(text.bytes, b"hello ");
    assert_eq!(text.next_offset, Some(6));
    let rest = reconnected.read(&output.id, "snapshot.md", 6, 64).await?;
    assert_eq!(String::from_utf8(rest.bytes)?, "🦀 world");
    assert!(rest.next_offset.is_none());
    let pdf = reconnected.read(&output.id, "pdf.pdf", 0, 64).await?;
    assert_eq!(pdf.bytes, b"%PDF-1.7\0\xff");
    let primary = reconnected.read(&output.id, "result.json", 0, 1024).await?;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&primary.bytes)?,
        json!({"value": [1, 2], "logs": ["done"]})
    );
    assert!(
        OutputStore::new(root.clone(), "other-session")
            .describe(&output.id)
            .await
            .is_err()
    );
    assert!(
        reconnected
            .read(&output.id, "../result.json", 0, 100)
            .await
            .is_err()
    );
    assert!(reconnected.describe("../../escape").await.is_err());
    tokio::fs::remove_dir_all(root).await?;
    Ok(())
}

#[tokio::test]
async fn expired_outputs_are_unreadable_and_removed_on_the_next_publish() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!("neo-output-test-{}", uuid::Uuid::new_v4()));
    let call = OutputStore::new(root.clone(), "session").with_retention(Duration::ZERO);
    let path = call.write_part("pdf", "pdf", b"pdf").await?;
    let output = call
        .finish(&json!({"ok":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    assert!(call.describe(&output.id).await.is_err());
    let next = OutputStore::new(root.clone(), "session");
    next.write_part("pdf", "pdf", b"new").await?;
    assert!(!path.exists());
    tokio::fs::remove_dir_all(root).await?;
    Ok(())
}

#[tokio::test]
async fn published_downloads_are_immutable_and_binary_transfer_ignores_text_extensions()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("export.txt");
    tokio::fs::write(&source, b"\xff\0original").await?;
    let call = OutputStore::new(directory.path().join("outputs"), "session");
    call.copy_file("download", &source).await?;
    let output = call
        .finish(&json!({"ok":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    tokio::fs::write(&source, b"changed").await?;
    assert_eq!(
        call.read_bytes(&output.id, "download.txt", 0, 100)
            .await?
            .bytes,
        b"\xff\0original"
    );
    assert!(call.write_part("late", "txt", b"late").await.is_err());
    assert_eq!(
        call.finish(&json!({"changed":true}))
            .await?
            .map(|output| output.id),
        Some(output.id)
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn private_outputs_refuse_symlink_substitution() -> anyhow::Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir()?;
    let call = OutputStore::new(directory.path().join("outputs"), "session");
    let path = call.write_part("document", "txt", b"original").await?;
    let output = call
        .finish(&json!({"ok":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    assert_eq!(
        tokio::fs::metadata(&path).await?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        tokio::fs::metadata(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("missing parent"))?
        )
        .await?
        .permissions()
        .mode()
            & 0o777,
        0o700
    );
    let secret = directory.path().join("secret.txt");
    // Same length as the original: size validation alone must not make this pass.
    tokio::fs::write(&secret, b"secret!!").await?;
    tokio::fs::remove_file(&path).await?;
    symlink(secret, path)?;
    assert!(call.read(&output.id, "document.txt", 0, 100).await.is_err());
    Ok(())
}

#[tokio::test]
#[cfg(unix)]
async fn retention_preserves_unrelated_mount_directories_and_rejects_relative_roots()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("outputs");
    let call = OutputStore::new(root.clone(), "session");
    let path = call.write_part("document", "txt", b"first").await?;
    let owner_dir = path
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| anyhow::anyhow!("missing session directory"))?;
    let unrelated = owner_dir.join("keep-me");
    tokio::fs::create_dir(&unrelated).await?;
    tokio::fs::write(unrelated.join("notes.txt"), b"owner data").await?;
    std::fs::File::open(&unrelated)?.set_times(
        std::fs::FileTimes::new()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(3 * 24 * 60 * 60)),
    )?;
    call.begin_call()
        .write_part("document", "txt", b"next")
        .await?;
    assert_eq!(
        tokio::fs::read(unrelated.join("notes.txt")).await?,
        b"owner data"
    );
    assert!(
        OutputStore::new(".".into(), "session")
            .write_part("bad", "txt", b"bad")
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn output_tool_runs_without_browser_dependencies_and_supports_content_only_clients()
-> anyhow::Result<()> {
    use browseros_mcp::{ToolExecution, execute_tool};
    let directory = tempfile::tempdir()?;
    let store = OutputStore::new(directory.path().join("outputs"), "session");
    store
        .write_part("document", "txt", "hello 🦀 world".as_bytes())
        .await?;
    let output = store
        .finish(&json!({"ok":true}))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing output"))?;
    let result = execute_tool(
        &browseros_mcp::tools::read_output::definition(),
        json!({"outputId": output.id, "part":"document.txt", "limit":8}),
        ToolExecution::Outputs(&store),
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .ok_or_else(|| anyhow::anyhow!("missing text"))?;
    let metadata: serde_json::Value = serde_json::from_str(
        text.text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Output chunk: "))
            .ok_or_else(|| anyhow::anyhow!("missing cursor metadata"))?,
    )?;
    assert_eq!(metadata["nextOffset"], 6);
    assert!(text.text.contains("hello "));
    let binary = execute_tool(
        &browseros_mcp::tools::read_output::definition(),
        json!({"outputId": output.id,"part":"document.txt","encoding":"base64","limit":8}),
        ToolExecution::Outputs(&store),
    )
    .await
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let text = binary
        .content
        .first()
        .and_then(|block| block.as_text())
        .ok_or_else(|| anyhow::anyhow!("missing binary text"))?;
    let displayed: serde_json::Value = serde_json::from_str(&text.text)?;
    assert_eq!(
        displayed,
        binary
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing binary metadata"))?
    );
    assert_eq!(displayed["data"], "aGVsbG8g8J8=");
    assert!(matches!(
        execute_tool(
            &browseros_mcp::tools::snapshot::definition(),
            json!({"page":1}),
            ToolExecution::Outputs(&store)
        )
        .await,
        Err(browseros_mcp::framework::ToolError::BrowserRequired)
    ));
    Ok(())
}
