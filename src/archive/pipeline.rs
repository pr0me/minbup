use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use camino::Utf8Path;
use tracing::{debug, warn};

use crate::config::{MANIFEST_NAME, READ_BUFFER_BYTES};
use crate::error::{Error, Result};
use crate::hashing::BlakeTee;
use crate::tui::state::ProgressState;
use crate::walk::Project;
use crate::walk::gitmeta;

use super::manifest::{ManifestBuilder, ManifestEntry};

#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveStats {
    pub files: u64,
    pub uncompressed_bytes: u64,
    pub manifest_entries: u64,
}

pub struct ArchiveWriter<W: Write> {
    tar: tar::Builder<zstd::Encoder<'static, W>>,
    manifest: ManifestBuilder,
    state: ProgressState,
    stats: ArchiveStats,
}

impl<W: Write> ArchiveWriter<W> {
    pub fn new(zstd_enc: zstd::Encoder<'static, W>, state: ProgressState) -> Self {
        let mut tar = tar::Builder::new(zstd_enc);
        tar.mode(tar::HeaderMode::Deterministic);
        tar.follow_symlinks(false);
        Self {
            tar,
            manifest: ManifestBuilder::new(),
            state,
            stats: ArchiveStats::default(),
        }
    }

    /// Append a regular file. hashes contents on the fly into the manifest.
    pub fn add_file(
        &mut self,
        abs: &Utf8Path,
        name_in_archive: impl AsRef<Path>,
        mtime: SystemTime,
        size: u64,
    ) -> Result<()> {
        let f = match File::open(abs.as_std_path()) {
            Ok(f) => f,
            Err(e) => {
                warn!("open {abs}: {e}");
                self.state.errors_skipped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        };
        let buf = BufReader::with_capacity(READ_BUFFER_BYTES, f);

        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o644);
        if let Ok(secs) = mtime.duration_since(SystemTime::UNIX_EPOCH) {
            header.set_mtime(secs.as_secs());
        }
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();

        let mut hasher = blake3::Hasher::new();
        let tee = BlakeTee::new(buf, &mut hasher);

        let name = name_in_archive.as_ref();
        if let Err(e) = self.tar.append_data(&mut header, name, tee) {
            return Err(Error::Archive(format!("append {abs}: {e}")));
        }

        let path_str = path_to_archive_str(name)?;
        self.manifest.push(ManifestEntry {
            blake3_hex: hasher.finalize().to_hex().to_string(),
            size,
            path: path_str,
        });
        self.stats.files += 1;
        self.stats.uncompressed_bytes = self.stats.uncompressed_bytes.saturating_add(size);
        self.state.files_done.fetch_add(1, Ordering::Relaxed);
        self.state.bytes_archived_uncompressed.fetch_add(size, Ordering::Relaxed);
        Ok(())
    }

    /// Append a small in-memory blob (used for `.gitmeta`). hashes and adds to manifest.
    pub fn add_blob(&mut self, name_in_archive: impl AsRef<Path>, data: &[u8]) -> Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(now_secs());
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();

        let name = name_in_archive.as_ref();
        self.tar
            .append_data(&mut header, name, data)
            .map_err(|e| Error::Archive(format!("append blob: {e}")))?;

        let hash = blake3::hash(data);
        let path_str = path_to_archive_str(name)?;
        self.manifest.push(ManifestEntry {
            blake3_hex: hash.to_hex().to_string(),
            size: data.len() as u64,
            path: path_str,
        });
        self.stats.files += 1;
        self.stats.uncompressed_bytes = self.stats.uncompressed_bytes.saturating_add(data.len() as u64);
        self.state.files_done.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn add_gitmeta(&mut self, project: &Project) -> Result<()> {
        let toml = gitmeta::serialize(&project.gitmeta)?;
        let name = if project.root_rel.as_str().is_empty() {
            String::from(".gitmeta")
        } else {
            format!("{}/.gitmeta", project.root_rel)
        };
        debug!("writing {name}");
        self.add_blob(name, toml.as_bytes())
    }

    /// Append manifest as the final tar entry, then close zstd and return the inner writer.
    pub fn finalize(mut self) -> Result<(W, ArchiveStats)> {
        let entries = self.manifest.len() as u64;
        let bytes = self.manifest.into_bytes()?;

        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(now_secs());
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        self.tar
            .append_data(&mut header, MANIFEST_NAME, bytes.as_slice())
            .map_err(|e| Error::Archive(format!("append manifest: {e}")))?;

        let zstd_enc = self
            .tar
            .into_inner()
            .map_err(|e| Error::Archive(format!("tar finish: {e}")))?;
        let inner = zstd_enc
            .finish()
            .map_err(|e| Error::Archive(format!("zstd finish: {e}")))?;
        self.stats.manifest_entries = entries;
        Ok((inner, self.stats))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn path_to_archive_str(p: &Path) -> Result<String> {
    p.to_str()
        .ok_or_else(|| Error::Archive("non-utf8 archive entry name".into()))
        .map(|s| s.replace(std::path::MAIN_SEPARATOR, "/"))
}
