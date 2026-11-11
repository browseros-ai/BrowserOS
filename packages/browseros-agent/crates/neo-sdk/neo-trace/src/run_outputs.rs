//! The files a run keeps beside the trace, other than screenshots: the downloads Neo copied for
//! it and the byte values its reply was too large to carry, under `outputs/<run>/` beside
//! `trace.db`.
//!
//! ```text
//!  outputs/<run>/downloads.json      the run's downloads (RunDownload), written once at its end
//!  outputs/<run>/downloads/<index>   the bytes of each completed one
//!  outputs/<run>/blobs/<id>          each byte value the reply names as {"$neo":"blob","id"}
//! ```
//!
//! claw-sdk writes them when a run ends, before it answers the run, so a host that reads them
//! right after the reply finds them; nothing here goes through the writer task. The age sweep
//! deletes `outputs/<run>` with the run's rows, so the files live exactly as long as the run's
//! record. The screenshot cap never touches them: it frees screenshot bytes only. Every path is
//! built from a typed id and an index, never from text read back, so no stored value can point
//! a read or a delete outside the directory.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{BlobId, RunDownload, RunId, TraceError};

/// The outputs directory of one trace file. Cheap to clone.
#[derive(Clone, Debug)]
pub struct RunOutputs {
    root: Arc<Path>,
}

impl RunOutputs {
    /// The `outputs` directory beside the trace file at `db`.
    pub(crate) fn beside(db: &Path) -> Self {
        let dir = db.parent().unwrap_or_else(|| Path::new("."));
        Self {
            root: Arc::from(dir.join("outputs")),
        }
    }

    /// The directory holding everything `run` kept.
    pub(crate) fn run_dir(&self, run: RunId) -> PathBuf {
        self.root.join(run.to_string())
    }

    fn manifest(&self, run: RunId) -> PathBuf {
        self.run_dir(run).join("downloads.json")
    }

    fn download_path(&self, run: RunId, index: u32) -> PathBuf {
        self.run_dir(run).join("downloads").join(index.to_string())
    }

    fn blob_path(&self, run: RunId, id: BlobId) -> PathBuf {
        self.run_dir(run).join("blobs").join(id.to_string())
    }

    /// Writes blob `id` of `run`: bytes its reply names instead of carrying them.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub async fn keep_blob(&self, run: RunId, id: BlobId, bytes: &[u8]) -> io::Result<()> {
        let path = self.blob_path(run, id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, bytes).await
    }

    /// Blob `id` of `run`, open for reading, with its length; `None` when the run has none such.
    ///
    /// # Errors
    /// When the file exists but cannot be opened.
    pub async fn open_blob(
        &self,
        run: RunId,
        id: BlobId,
    ) -> Result<Option<(tokio::fs::File, u64)>, TraceError> {
        open(&self.blob_path(run, id)).await
    }

    /// Copies the finished download at `source` in as download `index` of `run`, and answers how
    /// many bytes it kept. A copy, not a link: the browser's file stays the script's to change
    /// or replace, and what the host reads stays what the run downloaded. Only a whole copy
    /// appears under the download's name.
    ///
    /// # Errors
    /// When `source` cannot be read or the copy cannot be written.
    pub async fn keep_download(&self, run: RunId, index: u32, source: &Path) -> io::Result<u64> {
        let target = self.download_path(run, index);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Through a temporary name, so a copy cut short is never served as the download.
        let partial = target.with_extension("partial");
        let size = tokio::fs::copy(source, &partial).await?;
        tokio::fs::rename(&partial, &target).await?;
        Ok(size)
    }

    /// Records `run`'s downloads, once, after every file it kept. It goes through a temporary
    /// file and a rename, so a reader sees the whole list or none.
    ///
    /// # Errors
    /// When the list cannot be written.
    pub async fn record_downloads(&self, run: RunId, downloads: &[RunDownload]) -> io::Result<()> {
        let manifest = self.manifest(run);
        if let Some(parent) = manifest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_vec(downloads).map_err(io::Error::other)?;
        let partial = manifest.with_extension("json.partial");
        tokio::fs::write(&partial, json).await?;
        tokio::fs::rename(&partial, &manifest).await
    }

    /// `run`'s downloads, by index; none when the run recorded none.
    ///
    /// # Errors
    /// When the list exists but cannot be read.
    pub async fn downloads(&self, run: RunId) -> Result<Vec<RunDownload>, TraceError> {
        match tokio::fs::read(self.manifest(run)).await {
            Ok(json) => serde_json::from_slice(&json)
                .map_err(|error| TraceError::Corrupt(format!("run {run}'s downloads: {error}"))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(TraceError::Output(error)),
        }
    }

    /// The bytes Neo kept of download `index` of `run`, open for reading, with their length;
    /// `None` when it kept none.
    ///
    /// # Errors
    /// When the file exists but cannot be opened.
    pub async fn open_download(
        &self,
        run: RunId,
        index: u32,
    ) -> Result<Option<(tokio::fs::File, u64)>, TraceError> {
        open(&self.download_path(run, index)).await
    }
}

async fn open(path: &Path) -> Result<Option<(tokio::fs::File, u64)>, TraceError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(TraceError::Output(error)),
    };
    let size = file.metadata().await.map_err(TraceError::Output)?.len();
    Ok(Some((file, size)))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use tokio::io::AsyncReadExt;

    use super::RunOutputs;
    use crate::{BlobId, DownloadStatus, RunDownload, RunId};

    #[test]
    fn a_runs_outputs_live_in_its_own_directory_beside_the_trace_file() {
        let outputs = RunOutputs::beside(std::path::Path::new("/data/claw-sdk/trace.db"));
        let run = RunId::new();
        assert_eq!(
            outputs.run_dir(run),
            PathBuf::from(format!("/data/claw-sdk/outputs/{run}"))
        );
        assert_eq!(
            outputs.download_path(run, 2),
            PathBuf::from(format!("/data/claw-sdk/outputs/{run}/downloads/2"))
        );
        assert_eq!(
            outputs.blob_path(run, BlobId::new(1)),
            PathBuf::from(format!("/data/claw-sdk/outputs/{run}/blobs/b1"))
        );
    }

    #[tokio::test]
    async fn a_kept_blob_reads_back() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let outputs = RunOutputs::beside(&dir.path().join("trace.db"));
        let run = RunId::new();
        assert!(outputs.open_blob(run, BlobId::new(0)).await?.is_none());
        outputs.keep_blob(run, BlobId::new(0), b"\x89PNG").await?;
        let (mut file, length) = outputs
            .open_blob(run, BlobId::new(0))
            .await?
            .ok_or("kept")?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await?;
        assert_eq!((bytes.as_slice(), length), (&b"\x89PNG"[..], 4));
        Ok(())
    }

    #[tokio::test]
    async fn a_kept_download_reads_back_with_its_record() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let outputs = RunOutputs::beside(&dir.path().join("trace.db"));
        let run = RunId::new();
        assert!(outputs.downloads(run).await?.is_empty());
        assert!(outputs.open_download(run, 0).await?.is_none());
        let source = dir.path().join("statement.txt");
        tokio::fs::write(&source, b"balance 42").await?;
        let size = outputs.keep_download(run, 0, &source).await?;
        // The browser's file is the script's: changing it later leaves the kept bytes alone.
        tokio::fs::write(&source, b"changed").await?;
        let record = RunDownload {
            index: 0,
            url: "https://example.com/statement.txt".to_owned(),
            suggested_filename: "statement.txt".to_owned(),
            path: Some(source.to_string_lossy().into_owned()),
            size: Some(size),
            state: DownloadStatus::Completed,
            failure: None,
        };
        outputs
            .record_downloads(run, std::slice::from_ref(&record))
            .await?;
        assert_eq!(outputs.downloads(run).await?, vec![record]);
        let (mut file, length) = outputs.open_download(run, 0).await?.ok_or("kept")?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await?;
        assert_eq!((bytes.as_slice(), length), (&b"balance 42"[..], 10));
        Ok(())
    }
}
