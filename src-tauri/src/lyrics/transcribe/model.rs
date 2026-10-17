//! Speech models: which ones, where they live, and the download.
//!
//! Models never ship in the installer. One is downloaded when the user asks,
//! from the whisper.cpp model repository on Hugging Face, pinned to one
//! commit so the bytes can't change under us, and checked against its
//! SHA-256 before it's used. Files sit in `<app data>/models/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Which model transcribes (Settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    /// Faster, smaller download.
    #[default]
    Small,
    /// Better, about three times slower and bigger.
    Medium,
    /// Large v3 turbo: the best of the three, made for the graphics card
    /// (seconds per song there, far too slow on the processor).
    Turbo,
}

impl ModelKind {
    /// Smallest first (the lyrics check takes the first one installed).
    pub const ALL: [ModelKind; 3] = [ModelKind::Small, ModelKind::Medium, ModelKind::Turbo];

    /// The model for someone who hasn't picked one: Turbo with a graphics
    /// card in use, else Small.
    pub fn default_for(gpu: bool) -> ModelKind {
        if gpu { ModelKind::Turbo } else { ModelKind::Small }
    }

    /// The model used: the one picked in Settings, else the default.
    pub fn chosen(picked: Option<ModelKind>, gpu: bool) -> ModelKind {
        picked.unwrap_or_else(|| Self::default_for(gpu))
    }
}

/// One downloadable model file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    pub kind: ModelKind,
    pub file: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

/// The model repository commit the files are pinned to.
const REPO: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";

/// Whisper models, quantized (whisper.cpp's ggml format, MIT): English-only
/// Small and Medium, and the multilingual Large v3 turbo.
pub fn catalogue() -> Vec<ModelSpec> {
    let spec = |kind, file: &str, size, sha: &str| ModelSpec {
        kind,
        file: file.to_owned(),
        url: format!("{REPO}/{file}"),
        size,
        sha256: sha.to_owned(),
    };
    vec![
        spec(
            ModelKind::Small,
            "ggml-small.en-q5_1.bin",
            190_098_681,
            "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        ),
        spec(
            ModelKind::Medium,
            "ggml-medium.en-q5_0.bin",
            539_225_533,
            "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0",
        ),
        spec(
            ModelKind::Turbo,
            "ggml-large-v3-turbo-q5_0.bin",
            574_041_195,
            "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        ),
    ]
}

/// Free space kept on top of the model when checking the disk.
pub const DISK_MARGIN: u64 = 256 * 1024 * 1024;

/// Why a download stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// Network trouble or the server refused.
    Network(String),
    /// The file arrived but isn't the expected one.
    Checksum,
    /// Not enough free space, before or during the download.
    Disk { needed: u64, free: u64 },
    /// Writing the file failed for another reason.
    Io(String),
    Cancelled,
}

/// Downloads (behind a trait so tests can serve fake bytes).
#[async_trait]
pub trait ModelFetch: Send + Sync {
    /// The body as a stream of chunks.
    async fn open(&self, url: &str) -> Result<BoxStream<'static, Result<Bytes, String>>, String>;
}

/// Downloads with reqwest.
pub struct HttpFetch(pub reqwest::Client);

impl HttpFetch {
    /// A client that gives up on a stalled connection instead of waiting
    /// for ever (no overall limit: the files are large).
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self(client)
    }
}

impl Default for HttpFetch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ModelFetch for HttpFetch {
    async fn open(&self, url: &str) -> Result<BoxStream<'static, Result<Bytes, String>>, String> {
        let resp = self.0.get(url).send().await.map_err(|e| e.without_url().to_string())?;
        if !resp.status().is_success() {
            return Err(format!("http {}", resp.status().as_u16()));
        }
        Ok(resp.bytes_stream().map(|r| r.map_err(|e| e.without_url().to_string())).boxed())
    }
}

/// Free bytes on the disk holding `dir`, or None if unknown.
pub type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

pub fn os_free_space() -> FreeSpace {
    Arc::new(|p: &Path| fs4::available_space(p).ok())
}

fn disk_full(e: &std::io::Error) -> bool {
    // ENOSPC is 28 on Unix; Windows says ERROR_DISK_FULL (112) or
    // ERROR_HANDLE_DISK_FULL (39).
    e.kind() == std::io::ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(28) if cfg!(unix)) || matches!(e.raw_os_error(), Some(112 | 39) if cfg!(windows))
}

/// The model folder.
#[derive(Clone)]
pub struct ModelStore {
    dir: PathBuf,
    specs: Arc<Vec<ModelSpec>>,
    free: FreeSpace,
}

impl ModelStore {
    pub fn new(dir: PathBuf, specs: Vec<ModelSpec>, free: FreeSpace) -> Self {
        Self { dir, specs: Arc::new(specs), free }
    }

    pub fn spec(&self, kind: ModelKind) -> Option<&ModelSpec> {
        self.specs.iter().find(|s| s.kind == kind)
    }

    pub fn path(&self, kind: ModelKind) -> Option<PathBuf> {
        self.spec(kind).map(|s| self.dir.join(&s.file))
    }

    fn part_path(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(format!("{}.part", spec.file))
    }

    /// Where the note of a checked file is kept: its checksum, size and
    /// modified time when it was checked.
    fn marker_path(&self, spec: &ModelSpec) -> PathBuf {
        self.dir.join(format!("{}.verified", spec.file))
    }

    /// The model's file when it's fully there: as it was when its checksum
    /// was checked (same size and modified time), or, for a file downloaded
    /// before those notes were kept, the right size until [`Self::verify`]
    /// checks it once.
    pub fn installed(&self, kind: ModelKind) -> Option<PathBuf> {
        let spec = self.spec(kind)?;
        let p = self.dir.join(&spec.file);
        std::fs::metadata(&p).ok().filter(|m| m.is_file() && m.len() == spec.size)?;
        match std::fs::read_to_string(self.marker_path(spec)) {
            Ok(note) => (Some(note.trim()) == stamp(&p, &spec.sha256).as_deref()).then_some(p),
            Err(_) => Some(p),
        }
    }

    /// The file is there but was never checked here (downloaded before
    /// checks were noted).
    pub fn needs_verify(&self, kind: ModelKind) -> bool {
        self.spec(kind).is_some_and(|s| self.installed(kind).is_some() && !self.marker_path(s).exists())
    }

    /// Check the file's checksum now (slow: reads the whole file; call off
    /// the async threads). A good file is noted; a bad one is deleted.
    pub fn verify(&self, kind: ModelKind) -> bool {
        let Some(spec) = self.spec(kind) else { return false };
        let p = self.dir.join(&spec.file);
        let ok = (|| -> std::io::Result<bool> {
            let mut f = std::fs::File::open(&p)?;
            let mut h = Sha256::new();
            std::io::copy(&mut f, &mut h)?;
            Ok(hex(&h.finalize()) == spec.sha256)
        })()
        .unwrap_or(false);
        if ok {
            if let Some(s) = stamp(&p, &spec.sha256) {
                let _ = std::fs::write(self.marker_path(spec), s);
            }
        } else {
            tracing::warn!(model = %spec.file, "model file failed its checksum; removed");
            let _ = self.delete(kind);
        }
        ok
    }

    /// Delete the model (and any half-finished download of it).
    pub fn delete(&self, kind: ModelKind) -> std::io::Result<()> {
        let Some(spec) = self.spec(kind) else { return Ok(()) };
        for p in [self.dir.join(&spec.file), self.part_path(spec), self.marker_path(spec)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Download and verify a model. `progress(done, total)` follows along.
    pub async fn download(
        &self,
        kind: ModelKind,
        fetch: &dyn ModelFetch,
        cancel: &AtomicBool,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<PathBuf, DownloadError> {
        let spec = self.spec(kind).ok_or_else(|| DownloadError::Io("unknown model".into()))?.clone();
        tokio::fs::create_dir_all(&self.dir).await.map_err(|e| DownloadError::Io(e.to_string()))?;
        if let Some(free) = (self.free)(&self.dir) {
            let needed = spec.size + DISK_MARGIN;
            if free < needed {
                return Err(DownloadError::Disk { needed, free });
            }
        }
        let part = self.part_path(&spec);
        let result = self.fetch_to(&spec, &part, fetch, cancel, progress).await;
        match result {
            Ok(()) => {
                let dest = self.dir.join(&spec.file);
                tokio::fs::rename(&part, &dest).await.map_err(|e| DownloadError::Io(e.to_string()))?;
                // The checksum was checked on the way in: note the file as it is now.
                if let Some(s) = stamp(&dest, &spec.sha256) {
                    tokio::fs::write(self.marker_path(&spec), s).await.map_err(|e| DownloadError::Io(e.to_string()))?;
                }
                Ok(dest)
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    async fn fetch_to(
        &self,
        spec: &ModelSpec,
        part: &Path,
        fetch: &dyn ModelFetch,
        cancel: &AtomicBool,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<(), DownloadError> {
        let io = |e: std::io::Error| {
            if disk_full(&e) {
                let free = (self.free)(&self.dir).unwrap_or(0);
                DownloadError::Disk { needed: spec.size + DISK_MARGIN, free }
            } else {
                DownloadError::Io(e.to_string())
            }
        };
        let mut body = fetch.open(&spec.url).await.map_err(DownloadError::Network)?;
        let mut file = tokio::fs::File::create(part).await.map_err(io)?;
        let mut hash = Sha256::new();
        let mut done = 0u64;
        progress(0, spec.size);
        while let Some(chunk) = body.next().await {
            if cancel.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            let chunk = chunk.map_err(DownloadError::Network)?;
            done += chunk.len() as u64;
            if done > spec.size {
                return Err(DownloadError::Checksum);
            }
            hash.update(&chunk);
            file.write_all(&chunk).await.map_err(io)?;
            progress(done, spec.size);
        }
        file.flush().await.map_err(io)?;
        file.sync_all().await.map_err(io)?;
        drop(file);
        if cancel.load(Ordering::Relaxed) {
            return Err(DownloadError::Cancelled);
        }
        let got = hex(&hash.finalize());
        if done != spec.size || got != spec.sha256 {
            tracing::warn!(model = %spec.file, done, "model checksum mismatch");
            return Err(DownloadError::Checksum);
        }
        Ok(())
    }
}

/// A checked file's note: checksum, size and modified time.
fn stamp(p: &Path, sha: &str) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("{sha} {} {t}", m.len()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of some bytes as lower-case hex (tests build fake models with it).
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
