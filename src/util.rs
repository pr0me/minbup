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

pub fn human_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{}.{:01}s", s, d.subsec_millis() / 100)
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m{:02}s", s / 3600, (s % 3600) / 60, s % 60)
    }
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

/// Refuse to back up when the output archive would land inside the target tree —
/// unless it's inside an excluded subtree, in which case it won't be archived.
pub fn ensure_output_outside_target(
    output: impl AsRef<Path>,
    target: impl AsRef<Path>,
    excluded_subtrees: &[std::path::PathBuf],
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
        let in_excluded = excluded_subtrees
            .iter()
            .any(|ex| output_canon.starts_with(ex));
        if !in_excluded {
            return Err(Error::OutputInsideTarget {
                output: output_canon,
                target: target_canon,
            });
        }
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

/// Result of normalizing one `--exclude` value.
#[derive(Debug)]
pub struct NormalizedExclude {
    /// Pattern to feed into `OverrideBuilder` (gitignore syntax).
    pub pattern: String,
    /// Canonical absolute path when the input was a filesystem path that resolved
    /// inside the target. used by the output-location safety check.
    pub canonical_path: Option<std::path::PathBuf>,
}

/// Normalize a `--exclude` value. paths (containing `/` or starting with `./`/`../`/abs)
/// are resolved against cwd and converted to root-anchored gitignore patterns relative
/// to `target_canon`. Anything else is treated as a literal gitignore glob.
/// Returns `None` if the path resolves outside the target tree.
pub fn normalize_exclude(target_canon: &Utf8Path, raw: &str) -> Option<NormalizedExclude> {
    let (negate, pat) = match raw.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let _ = negate;

    let pathlike = pat.starts_with("./")
        || pat.starts_with("../")
        || pat.starts_with('/')
        || (pat.contains('/') && std::path::Path::new(pat).exists());

    if !pathlike {
        return Some(NormalizedExclude {
            pattern: pat.to_owned(),
            canonical_path: None,
        });
    }

    let raw_path = std::path::Path::new(pat);
    let canon = match std::fs::canonicalize(raw_path) {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!("--exclude {raw}: path does not exist; treating as glob");
            return Some(NormalizedExclude {
                pattern: pat.to_owned(),
                canonical_path: None,
            });
        }
    };
    let rel = match canon.strip_prefix(target_canon.as_std_path()) {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!("--exclude {raw}: outside target; ignored");
            return None;
        }
    };
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if rel_str.is_empty() {
        tracing::warn!("--exclude {raw}: resolves to target root; ignored");
        return None;
    }
    let is_dir = canon.is_dir();
    let mut pattern = String::with_capacity(rel_str.len() + 2);
    pattern.push('/');
    pattern.push_str(&rel_str);
    if is_dir && !pattern.ends_with('/') {
        pattern.push('/');
    }
    Some(NormalizedExclude {
        pattern,
        canonical_path: Some(canon),
    })
}
