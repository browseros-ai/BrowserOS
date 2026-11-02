//! Session-owned, immutable tool outputs. One request collects typed parts behind one ID;
//! the manifest is published only after response assembly, including nested script calls.
//! Storage lives independently of an MCP connection so reconnects cannot orphan references.

use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex as StdMutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
};
use uuid::Uuid;

pub const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_PART_BYTES: usize = 64 * 1024 * 1024;
const MAX_SESSION_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PARTS: usize = 64;
const MAX_OUTPUTS: usize = 128;
const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);
const MAX_INLINE_IMAGES: usize = 4;
const MAX_INLINE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MANIFEST_RESERVE: u64 = 64 * 1024;

/// Shared by a host's store clones. Admission scans once per call; subsequent parts
/// update quotas in memory under this root's lock. Other roots never share the lock.
#[derive(Default)]
struct StorageState {
    sessions: HashMap<PathBuf, (usize, u64)>,
    last_cleanup: Option<Instant>,
}

// Stateless requests can outlive every service instance between calls. Retain a
// root's admission state across that gap so cleanup stays throttled, then evict
// idle entries on later admissions. No I/O holds this registry lock.
type SharedStorage = Arc<Mutex<StorageState>>;
struct RootEntry {
    storage: SharedStorage,
    last_used: Instant,
}
static ROOTS: LazyLock<StdMutex<HashMap<PathBuf, RootEntry>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

fn shared_storage(root: &Path) -> SharedStorage {
    let mut roots = ROOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    roots.retain(|_, entry| {
        Arc::strong_count(&entry.storage) > 1
            || entry.last_used.elapsed() < CLEANUP_INTERVAL
            // A long-lived service may have swept just before it was dropped.
            // Preserve that timestamp too; try_lock never blocks the registry.
            || entry.storage.try_lock().is_ok_and(|state| {
                state.last_cleanup.is_some_and(|last| last.elapsed() < CLEANUP_INTERVAL)
            })
    });
    if let Some(entry) = roots.get_mut(root) {
        entry.last_used = Instant::now();
        return entry.storage.clone();
    }
    let storage = SharedStorage::default();
    roots.insert(
        root.to_path_buf(),
        RootEntry {
            storage: storage.clone(),
            last_used: Instant::now(),
        },
    );
    storage
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
/// One immutable named file, preserving its encoding and size for remote readers.
pub struct OutputPart {
    pub name: String,
    pub mime_type: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
/// The saved call's public manifest. Expiry is Unix epoch milliseconds, shared by all parts.
pub struct OutputReference {
    pub id: String,
    pub expires_at: u64,
    pub parts: Vec<OutputPart>,
    /// Attachments remain retrievable if retaining the primary result fails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub struct OutputChunk {
    pub part: OutputPart,
    pub bytes: Vec<u8>,
    pub next_offset: Option<u64>,
}

#[derive(Clone)]
pub struct InlineImage {
    pub data: String,
    pub mime_type: String,
}

#[derive(Default)]
struct Collected {
    parts: Vec<OutputPart>,
    images: Vec<InlineImage>,
    published: Option<OutputReference>,
    initialized: bool,
    inline_bytes: usize,
    omitted_images: usize,
    incomplete: bool,
    manifest_reserved: bool,
}

/// A logical session's store, narrowed to one request's collection. Clones share that
/// collection; `begin_call` starts a new one so concurrent calls never mix their parts.
#[derive(Clone)]
pub struct OutputStore {
    root: Arc<PathBuf>,
    session: Arc<str>,
    id: String,
    retention: Duration,
    collected: Arc<Mutex<Collected>>,
    storage: Arc<Mutex<StorageState>>,
}

impl OutputStore {
    #[must_use]
    pub fn new(root: PathBuf, session: &str) -> Self {
        let storage = shared_storage(&root);
        Self {
            root: Arc::new(root),
            session: Arc::from(session),
            id: Uuid::new_v4().to_string(),
            retention: RETENTION,
            collected: Arc::default(),
            storage,
        }
    }

    #[must_use]
    pub fn for_session(&self, session: &str) -> Self {
        Self {
            session: Arc::from(session),
            ..self.begin_call()
        }
    }

    #[must_use]
    pub fn begin_call(&self) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            collected: Arc::default(),
            ..self.clone()
        }
    }

    #[must_use]
    pub fn with_retention(mut self, retention: Duration) -> Self {
        self.retention = retention;
        self
    }

    fn session_dir(&self) -> PathBuf {
        // Lowercase hex stays injective on case-insensitive macOS/Windows filesystems.
        let key: String = self
            .session
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.root.join(format!("s-{key}"))
    }

    fn output_dir(&self, id: &str) -> io::Result<PathBuf> {
        Uuid::parse_str(id).map_err(|_| unavailable())?;
        Ok(self.session_dir().join(id))
    }

    pub async fn write_part(
        &self,
        name: &str,
        extension: &str,
        bytes: &[u8],
    ) -> io::Result<PathBuf> {
        self.record_write(self.write_part_inner(name, extension, bytes).await)
            .await
    }

    // A script may discard an inner tool's status. Keep persistence failure at
    // the outer-call boundary even when it happened before the first saved part.
    async fn record_write<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            let mut collected = self.collected.lock().await;
            if collected.published.is_none() {
                collected.incomplete = true;
            }
        }
        result
    }

    async fn write_part_inner(
        &self,
        name: &str,
        extension: &str,
        bytes: &[u8],
    ) -> io::Result<PathBuf> {
        if !self.root.is_absolute() {
            return Err(io::Error::other(
                "Tool output root must be an absolute, non-root directory",
            ));
        }
        if bytes.len() > MAX_PART_BYTES {
            return Err(io::Error::other(
                "Tool output exceeds the 64 MiB per-part limit",
            ));
        }
        let mut storage = self.storage.lock().await;
        let mut collected = self.collected.lock().await;
        if collected.published.is_some() {
            return Err(io::Error::other("Published output is immutable"));
        }
        let stem = segment(name);
        let extension = segment(extension).to_ascii_lowercase();
        let has_primary = collected
            .parts
            .iter()
            .any(|part| part.name == "result.json");
        // Attachments must leave one slot for response assembly. A batch at its
        // advertised limit must still publish a manifest and a usable output ID.
        let writing_primary = stem == "result" && extension == "json";
        let reserved = usize::from(!(has_primary || writing_primary));
        if collected.parts.len() >= MAX_PARTS - reserved {
            return Err(io::Error::other("Tool output exceeds the 64-part limit"));
        }
        let session = self.session_dir();
        if !collected.initialized || !storage.sessions.contains_key(&session) {
            self.prepare_session(&mut storage, &session).await?;
            collected.initialized = true;
        }
        let (count, used) = storage.sessions[&session];
        // Every active output reserves bounded manifest capacity before accepting
        // payload bytes. Publication can then expose partial results at the quota.
        let reserve = if collected.manifest_reserved {
            0
        } else {
            MANIFEST_RESERVE
        };
        if count > MAX_OUTPUTS
            || used
                .saturating_add(bytes.len() as u64)
                .saturating_add(reserve)
                > MAX_SESSION_BYTES
        {
            return Err(io::Error::other(
                "Tool output session quota exceeded (128 outputs / 128 MiB)",
            ));
        }
        let directory = self.output_dir(&self.id)?;
        let new_output = !fs::try_exists(&directory).await?;
        if count == MAX_OUTPUTS && new_output {
            return Err(io::Error::other(
                "Tool output session quota exceeded (128 outputs)",
            ));
        }
        private_dir(&directory).await?;
        if !collected.manifest_reserved {
            // A real reservation survives another call's directory scan and a
            // reconstructed service. It becomes the manifest at publication.
            if let Err(error) = write_new(
                &directory.join("manifest.pending"),
                &vec![0; MANIFEST_RESERVE as usize],
            )
            .await
            {
                storage.sessions.remove(&session);
                return Err(error);
            }
            collected.manifest_reserved = true;
        }
        let mut filename = format!("{stem}.{extension}");
        let mut number = 2;
        while collected.parts.iter().any(|part| part.name == filename) {
            filename = format!("{stem}-{number}.{extension}");
            number += 1;
        }
        let path = directory.join(&filename);
        if let Err(error) = write_new(&path, bytes).await {
            // A failed flush or permission change may leave a file behind. Rescan
            // before another write rather than trusting an incomplete byte count.
            storage.sessions.remove(&session);
            return Err(error);
        }
        storage.sessions.insert(
            session,
            (
                count + usize::from(new_output),
                used + bytes.len() as u64 + reserve,
            ),
        );
        collected.parts.push(OutputPart {
            name: filename,
            mime_type: mime_type(&extension).to_string(),
            bytes: bytes.len() as u64,
        });
        Ok(path)
    }

    async fn prepare_session(&self, storage: &mut StorageState, session: &Path) -> io::Result<()> {
        private_dir(&self.root).await?;
        // Root cleanup is amortized across calls. Current-task expiry and quota
        // admission run once per call, never once per screenshot/metadata part.
        if storage
            .last_cleanup
            .is_none_or(|last| last.elapsed() >= CLEANUP_INTERVAL)
        {
            let mut sessions = fs::read_dir(&*self.root).await?;
            while let Some(entry) = sessions.next_entry().await? {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("s-"))
                    && entry.file_type().await?.is_dir()
                    && entry
                        .metadata()
                        .await?
                        .modified()
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age >= RETENTION)
                {
                    self.prune_expired(&entry.path()).await?;
                    let _ = fs::remove_dir(entry.path()).await;
                }
            }
            storage.sessions.clear();
            storage.last_cleanup = Some(Instant::now());
        }
        private_dir(session).await?;
        self.prune_expired(session).await?;
        storage
            .sessions
            .insert(session.to_path_buf(), session_usage(session).await?);
        Ok(())
    }

    /// Copies completed downloads into private storage. The browser's original file can
    /// subsequently move or change without altering the bytes an output ID represents.
    pub async fn copy_file(&self, name: &str, source: &Path) -> io::Result<PathBuf> {
        self.record_write(self.copy_file_inner(name, source).await)
            .await
    }

    async fn copy_file_inner(&self, name: &str, source: &Path) -> io::Result<PathBuf> {
        regular_file(source).await?;
        let mut bytes = Vec::new();
        fs::File::open(source)
            .await?
            .take((MAX_PART_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bin");
        self.write_part(name, extension, &bytes).await
    }

    pub async fn image(&self, data: &str, mime: &str, metadata: &Value) -> io::Result<()> {
        self.record_write(self.image_inner(data, mime, metadata).await)
            .await
    }

    async fn image_inner(&self, data: &str, mime: &str, metadata: &Value) -> io::Result<()> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(io::Error::other)?;
        let extension = match mime {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            _ => return Err(io::Error::other("Unsupported screenshot MIME type")),
        };
        // Nested scripts retain only a bounded vision preview in Rust memory.
        // Further captures are still stored and advertised through the manifest.
        let inline = {
            let mut collected = self.collected.lock().await;
            if collected.images.len() < MAX_INLINE_IMAGES
                && collected.inline_bytes.saturating_add(data.len()) <= MAX_INLINE_IMAGE_BYTES
            {
                collected.inline_bytes += data.len();
                collected.images.push(InlineImage {
                    data: data.to_string(),
                    mime_type: mime.to_string(),
                });
                true
            } else {
                false
            }
        };
        self.write_part("screenshot", extension, &bytes).await?;
        if !inline {
            self.collected.lock().await.omitted_images += 1;
        }
        self.write_part(
            "screenshot-metadata",
            "json",
            &serde_json::to_vec(metadata)?,
        )
        .await?;
        Ok(())
    }

    pub async fn images(&self) -> Vec<InlineImage> {
        self.collected.lock().await.images.clone()
    }

    pub async fn omitted_images(&self) -> usize {
        self.collected.lock().await.omitted_images
    }

    /// Publishes only calls that produced stored content. `result.json` retains the primary
    /// typed result; a large run may have written its complete value there before clipping.
    pub async fn finish(&self, primary: &Value) -> io::Result<Option<OutputReference>> {
        let (empty, has_primary, incomplete) = {
            let collected = self.collected.lock().await;
            if let Some(output) = &collected.published {
                return Ok(Some(output.clone()));
            }
            (
                collected.parts.is_empty(),
                collected
                    .parts
                    .iter()
                    .any(|part| part.name == "result.json"),
                collected.incomplete,
            )
        };
        if empty {
            if incomplete {
                return Err(io::Error::other("No output parts could be retained"));
            }
            return Ok(None);
        }
        let mut warnings = Vec::new();
        if self.collected.lock().await.incomplete {
            warnings.push(
                "Some output parts could not be retained; use read_output for the available parts."
                    .to_string(),
            );
        }
        if !has_primary
            && let Err(error) = self
                .write_part("result", "json", &serde_json::to_vec(primary)?)
                .await
        {
            // Quota or I/O failure must not hide attachments already saved by an
            // action that cannot safely be replayed. Publish their available parts.
            warnings.push(format!("Primary result could not be retained: {error}"));
        }
        let mut storage = self.storage.lock().await;
        let mut collected = self.collected.lock().await;
        if let Some(output) = &collected.published {
            return Ok(Some(output.clone()));
        }
        let output = OutputReference {
            id: self.id.clone(),
            expires_at: now_ms()
                .saturating_add(self.retention.as_millis().try_into().unwrap_or(u64::MAX)),
            parts: collected.parts.clone(),
            warnings,
        };
        let directory = self.output_dir(&self.id)?;
        // A reader sees either no manifest or the complete immutable result, never an
        // intermediate list from a nested call still producing attachments.
        let temporary = directory.join("manifest.pending");
        let manifest = serde_json::to_vec(&output)?;
        if manifest.len() as u64 > MANIFEST_RESERVE {
            return Err(io::Error::other(
                "Tool output manifest exceeds its reserved capacity",
            ));
        }
        write_manifest(&temporary, &manifest).await?;
        fs::rename(temporary, directory.join("manifest.json")).await?;
        if let Some((_, bytes)) = storage.sessions.get_mut(&self.session_dir()) {
            *bytes = bytes
                .saturating_sub(MANIFEST_RESERVE)
                .saturating_add(manifest.len() as u64);
        }
        collected.published = Some(output.clone());
        Ok(Some(output))
    }

    pub async fn describe(&self, id: &str) -> io::Result<OutputReference> {
        let directory = self.output_dir(id)?;
        real_dir(&self.root).await?;
        real_dir(&self.session_dir()).await?;
        real_dir(&directory).await?;
        let path = directory.join("manifest.json");
        regular_file(&path).await?;
        let mut bytes = Vec::new();
        fs::File::open(path)
            .await?
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > 64 * 1024 {
            return Err(unavailable());
        }
        let output: OutputReference = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if output.id != id || now_ms() >= output.expires_at {
            return Err(unavailable());
        }
        Ok(output)
    }

    pub async fn read(
        &self,
        id: &str,
        name: &str,
        offset: u64,
        limit: usize,
    ) -> io::Result<OutputChunk> {
        self.read_chunk(id, name, offset, limit, true).await
    }

    /// Binary transfer preserves every byte even for a file whose extension suggests
    /// text but whose contents use a different encoding (for example an exported CSV).
    pub async fn read_bytes(
        &self,
        id: &str,
        name: &str,
        offset: u64,
        limit: usize,
    ) -> io::Result<OutputChunk> {
        self.read_chunk(id, name, offset, limit, false).await
    }

    async fn read_chunk(
        &self,
        id: &str,
        name: &str,
        offset: u64,
        limit: usize,
        align_utf8: bool,
    ) -> io::Result<OutputChunk> {
        let output = self.describe(id).await?;
        let part = output
            .parts
            .into_iter()
            .find(|part| part.name == name)
            .ok_or_else(unavailable)?;
        if name.contains(['/', '\\']) || name.starts_with('.') {
            return Err(unavailable());
        }
        let path = self.output_dir(id)?.join(name);
        regular_file(&path).await?;
        let mut file = fs::File::open(path).await?;
        if file.metadata().await?.len() != part.bytes || offset > part.bytes {
            return Err(unavailable());
        }
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut bytes = Vec::new();
        let limit = limit.clamp(4, MAX_READ_BYTES);
        file.take(limit as u64).read_to_end(&mut bytes).await?;
        if align_utf8
            && (part.mime_type.starts_with("text/") || part.mime_type == "application/json")
        {
            match std::str::from_utf8(&bytes) {
                Ok(_) => {}
                Err(error)
                    if error.error_len().is_none()
                        && offset + (bytes.len() as u64) < part.bytes =>
                {
                    bytes.truncate(error.valid_up_to())
                }
                Err(_) => {
                    return Err(io::Error::other(
                        "Output offset must be a UTF-8 character boundary",
                    ));
                }
            }
        }
        let end = offset + bytes.len() as u64;
        let next_offset = (end < part.bytes).then_some(end);
        Ok(OutputChunk {
            part,
            bytes,
            next_offset,
        })
    }

    async fn prune_expired(&self, session: &Path) -> io::Result<()> {
        let mut entries = fs::read_dir(session).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name() == self.id.as_str() || !entry.file_type().await?.is_dir() {
                continue;
            }
            if entry
                .file_name()
                .to_str()
                .is_none_or(|name| Uuid::parse_str(name).is_err())
            {
                continue; // Never prune unrelated directories on a caller-supplied mount.
            }
            let path = entry.path();
            let expired = match fs::read(path.join("manifest.json")).await {
                Ok(bytes) => {
                    serde_json::from_slice::<OutputReference>(&bytes).is_ok_and(|output| {
                        entry.file_name() == output.id.as_str() && now_ms() >= output.expires_at
                    })
                }
                Err(_) => entry
                    .metadata()
                    .await?
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= RETENTION),
            };
            if expired {
                fs::remove_dir_all(path).await?;
            }
        }
        Ok(())
    }
}

fn unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "Output unavailable: unknown, expired, or owned by another session",
    )
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
fn segment(value: &str) -> String {
    let value: String = value
        .chars()
        .take(80)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if value.is_empty() {
        "output".to_string()
    } else {
        value
    }
}
fn mime_type(extension: &str) -> &str {
    match extension {
        "md" => "text/markdown",
        "txt" => "text/plain",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
async fn real_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).await?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(unavailable());
    }
    Ok(())
}
async fn regular_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(unavailable());
    }
    Ok(())
}
async fn private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path).await?;
    real_dir(path).await?;
    if fs::canonicalize(path).await?.parent().is_none() {
        return Err(io::Error::other(
            "Tool output directory must not be the filesystem root",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}
async fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    if let Err(error) = file.write_all(bytes).await {
        drop(file);
        let _ = fs::remove_file(path).await;
        return Err(error);
    }
    file.flush().await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    Ok(())
}

/// Replace reserved bytes before the atomic rename, so even publication's temporary
/// file fits the admitted quota. Readers only open the final manifest name.
async fn write_manifest(path: &Path, bytes: &[u8]) -> io::Result<()> {
    regular_file(path).await?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .await?;
    file.write_all(bytes).await?;
    file.flush().await
}
async fn session_usage(session: &Path) -> io::Result<(usize, u64)> {
    let (mut count, mut bytes) = (0, 0u64);
    let mut entries = fs::read_dir(session).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir()
            || entry
                .file_name()
                .to_str()
                .is_none_or(|name| Uuid::parse_str(name).is_err())
        {
            continue;
        }
        count += 1;
        let mut parts = fs::read_dir(entry.path()).await?;
        while let Some(part) = parts.next_entry().await? {
            if part.file_type().await?.is_file() {
                let size = part.metadata().await?.len();
                bytes = bytes.saturating_add(if part.file_name() == "manifest.pending" {
                    size.max(MANIFEST_RESERVE)
                } else {
                    size
                });
            }
        }
    }
    Ok((count, bytes))
}
