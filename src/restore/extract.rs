use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};

use age::secrecy::SecretString;
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use tracing::{info, warn};

use crate::archive::manifest;
use crate::config::{MANIFEST_NAME, READ_BUFFER_BYTES};
use crate::error::{Error, Result};
use crate::hashing::BlakeTeeWriter;

use super::RestoreSummary;

pub fn extract(
    archive: &Utf8Path,
    dest: &Utf8Path,
    passphrase: Option<SecretString>,
    skip_verify: bool,
) -> Result<RestoreSummary> {
    fs::create_dir_all(dest.as_std_path())?;

    let file = File::open(archive.as_std_path())?;
    let buf = BufReader::with_capacity(READ_BUFFER_BYTES, file);

    let mut summary = RestoreSummary::default();
    if let Some(secret) = passphrase {
        let stream = decrypt_age(buf, secret)?;
        let zstd_dec = zstd::Decoder::new(stream)?;
        run_extract(zstd_dec, dest, &mut summary, skip_verify)?;
    } else {
        let zstd_dec = zstd::Decoder::new(buf)?;
        run_extract(zstd_dec, dest, &mut summary, skip_verify)?;
    }
    Ok(summary)
}

fn decrypt_age<R: Read + 'static>(reader: R, secret: SecretString) -> Result<Box<dyn Read>> {
    let dec = age::Decryptor::new(reader)
        .map_err(|e| Error::Encrypt(format!("decrypt header: {e}")))?;
    match dec {
        age::Decryptor::Passphrase(pd) => {
            let stream = pd
                .decrypt(&secret, None)
                .map_err(|e| Error::Encrypt(format!("decrypt: {e}")))?;
            Ok(Box::new(stream))
        }
        age::Decryptor::Recipients(_) => {
            Err(Error::Encrypt("expected passphrase-encrypted archive".into()))
        }
    }
}

fn run_extract<R: Read>(
    reader: R,
    dest: &Utf8Path,
    summary: &mut RestoreSummary,
    skip_verify: bool,
) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_mtime(true);
    archive.set_preserve_permissions(false);

    let mut computed = HashMap::<String, (String, u64)>::new();
    let mut manifest_entries: Option<Vec<manifest::ManifestEntry>> = None;

    for entry_res in archive.entries()? {
        let mut entry = entry_res?;
        let path_in_archive = match entry.path() {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => {
                warn!("skipping entry with bad path: {e}");
                continue;
            }
        };
        let normalized = normalize_archive_path(&path_in_archive);

        if normalized == MANIFEST_NAME {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            manifest_entries = Some(manifest::parse(&buf)?);
            summary.manifest_entries = manifest_entries.as_ref().map(|v| v.len() as u64).unwrap_or(0);
            continue;
        }

        let safe_rel = match safe_relative(&normalized) {
            Some(p) => p,
            None => {
                warn!("rejecting entry with unsafe path: {normalized}");
                continue;
            }
        };

        let dest_path = dest.join(&safe_rel);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(dest_path.as_std_path())?;
            continue;
        }
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent.as_std_path())?;
        }

        let header_size = entry.header().size().unwrap_or(0);
        let f = File::create(dest_path.as_std_path())?;
        let mut hasher = blake3::Hasher::new();
        let mut tee = BlakeTeeWriter::new(f, &mut hasher);
        let copied = io::copy(&mut entry, &mut tee)?;
        tee.flush().ok();

        let hex = hasher.finalize().to_hex().to_string();
        computed.insert(normalized, (hex, copied));
        summary.files_extracted += 1;
        summary.bytes_extracted = summary.bytes_extracted.saturating_add(copied);
        let _ = header_size;
    }

    if !skip_verify {
        if let Some(entries) = manifest_entries {
            for e in entries {
                match computed.get(&e.path) {
                    Some((hex, size)) if *hex == e.blake3_hex && *size == e.size => {}
                    Some((hex, size)) => {
                        warn!(
                            "verification mismatch for {}: hash {} (manifest {}), size {} (manifest {})",
                            e.path, hex, e.blake3_hex, size, e.size
                        );
                        summary.verification_failures += 1;
                    }
                    None => {
                        warn!("manifest entry missing from archive: {}", e.path);
                        summary.verification_failures += 1;
                    }
                }
            }
            if summary.verification_failures > 0 {
                return Err(Error::Manifest(format!(
                    "{} verification failure(s)",
                    summary.verification_failures
                )));
            }
            info!("verification ok ({} entries)", summary.manifest_entries);
        } else {
            warn!("archive has no manifest; skipping verification");
        }
    }
    Ok(())
}

fn normalize_archive_path(s: &str) -> String {
    s.trim_end_matches('/').replace('\\', "/")
}

fn safe_relative(p: &str) -> Option<Utf8PathBuf> {
    if p.is_empty() {
        return None;
    }
    let path = Utf8PathBuf::from(p);
    if path.is_absolute() {
        return None;
    }
    for c in path.components() {
        match c {
            Utf8Component::ParentDir => return None,
            Utf8Component::RootDir | Utf8Component::Prefix(_) => return None,
            _ => {}
        }
    }
    Some(path)
}

