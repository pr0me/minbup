use std::path::Path;
use std::time::SystemTime;

use camino::{Utf8Path, Utf8PathBuf};
use humansize::{format_size, BINARY};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{Error, Result};

pub fn human_bytes(n: u64) -> String {
    format_size(n, BINARY)
}

pub fn rfc3339_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

pub fn systemtime_to_rfc3339(t: SystemTime) -> String {
    let odt: OffsetDateTime = t.into();
    odt.format(&Rfc3339).unwrap_or_else(|_| "?".into())
}

/// Refuse to back up when the output archive would land inside the target tree.
pub fn ensure_output_outside_target(
    output: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Result<()> {
    let output = output.as_ref();
    let target = target.as_ref();
    let target_canon = std::fs::canonicalize(target)?;

    let output_canon = if let Ok(c) = std::fs::canonicalize(output) {
        c
    } else {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let parent_canon = std::fs::canonicalize(&parent)?;
        parent_canon.join(output.file_name().unwrap_or_default())
    };

    if output_canon.starts_with(&target_canon) {
        return Err(Error::OutputInsideTarget {
            output: output_canon,
            target: target_canon,
        });
    }
    Ok(())
}

pub fn append_partial_suffix(p: &Utf8Path) -> Utf8PathBuf {
    let mut s = p.as_str().to_owned();
    s.push_str(".partial");
    Utf8PathBuf::from(s)
}

/// Default output name when -o is omitted: <target-basename>-<UTC-stamp>.tar.zst[.age].
pub fn default_output_name(target: &Utf8Path, encrypt: bool) -> Utf8PathBuf {
    let basename = target.file_name().unwrap_or("backup");
    let stamp = OffsetDateTime::now_utc()
        .format(&time::macros::format_description!(
            "[year][month][day]-[hour][minute][second]"
        ))
        .unwrap_or_else(|_| "now".into());
    let ext = if encrypt { "tar.zst.age" } else { "tar.zst" };
    Utf8PathBuf::from(format!("{basename}-{stamp}.{ext}"))
}

pub fn relativize(abs: &Utf8Path, root: &Utf8Path) -> Utf8PathBuf {
    abs.strip_prefix(root).map(Utf8PathBuf::from).unwrap_or_else(|_| abs.to_path_buf())
}
