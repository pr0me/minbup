use std::collections::HashSet;
use std::fs;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::SCHEMA_VERSION;
use crate::error::{Error, Result};
use crate::util::{relativize, rfc3339_now};

#[derive(Debug, Clone)]
pub struct Project {
    pub root_abs: Utf8PathBuf,
    pub root_rel: Utf8PathBuf,
    pub gitmeta: GitMeta,
    /// Tracked file paths relative to `root_abs`.
    pub tracked: HashSet<Utf8PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitMeta {
    pub schema_version: u32,
    pub captured_at: String,
    pub project_path: String,
    pub head: GitHead,
    pub remotes: Vec<GitRemote>,
    pub config: GitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHead {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub detached: bool,
    pub commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRemote {
    pub name: String,
    pub fetch: String,
    pub push: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub raw: String,
}

/// Read .git metadata for a project root (parent of the .git dir).
pub fn gather(project_root_abs: &Utf8Path, archive_root: &Utf8Path) -> Result<Project> {
    let git_dir = project_root_abs.join(".git");
    if !git_dir.is_dir() {
        return Err(Error::GitMeta {
            path: git_dir.as_std_path().to_path_buf(),
            message: "not a directory".into(),
        });
    }

    let head = read_head(project_root_abs)?;
    let remotes = read_remotes(project_root_abs).unwrap_or_else(|e| {
        warn!("could not read remotes for {project_root_abs}: {e}");
        Vec::new()
    });
    let config_raw = fs::read_to_string(git_dir.join("config")).unwrap_or_default();
    let tracked = list_tracked(project_root_abs).unwrap_or_else(|e| {
        warn!("could not list tracked files for {project_root_abs}: {e}");
        HashSet::new()
    });

    let root_rel = relativize(project_root_abs, archive_root);
    let project_path = if root_rel.as_str().is_empty() { ".".into() } else { root_rel.to_string() };

    let gitmeta = GitMeta {
        schema_version: SCHEMA_VERSION,
        captured_at: rfc3339_now(),
        project_path,
        head,
        remotes,
        config: GitConfig { raw: config_raw },
    };

    Ok(Project { root_abs: project_root_abs.to_path_buf(), root_rel, gitmeta, tracked })
}

fn read_head(project_root: &Utf8Path) -> Result<GitHead> {
    let head_path = project_root.join(".git").join("HEAD");
    let raw = fs::read_to_string(&head_path).map_err(|e| Error::GitMeta {
        path: head_path.as_std_path().to_path_buf(),
        message: format!("read head: {e}"),
    })?;
    let trimmed = raw.trim();

    if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
        let branch = rest.to_owned();
        let commit = resolve_commit(project_root, &branch).unwrap_or_default();
        Ok(GitHead { branch: Some(branch), detached: false, commit })
    } else {
        Ok(GitHead { branch: None, detached: true, commit: trimmed.to_owned() })
    }
}

fn resolve_commit(project_root: &Utf8Path, branch: &str) -> Option<String> {
    let direct = project_root.join(".git").join("refs").join("heads").join(branch);
    if let Ok(s) = fs::read_to_string(&direct) {
        return Some(s.trim().to_owned());
    }
    let packed = project_root.join(".git").join("packed-refs");
    let body = fs::read_to_string(&packed).ok()?;
    let target = format!("refs/heads/{branch}");
    body.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let r = parts.next()?;
            (r == target).then(|| sha.to_owned())
        })
}

fn read_remotes(project_root: &Utf8Path) -> Result<Vec<GitRemote>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(project_root.as_str())
        .args(["remote", "-v"])
        .output()
        .map_err(|e| Error::GitMeta {
            path: project_root.as_std_path().to_path_buf(),
            message: format!("git remote -v: {e}"),
        })?;
    if !out.status.success() {
        return Err(Error::GitMeta {
            path: project_root.as_std_path().to_path_buf(),
            message: "git remote -v failed".into(),
        });
    }
    let text = String::from_utf8_lossy(&out.stdout);

    let mut by_name = std::collections::BTreeMap::<String, (String, String)>::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let name = match it.next() {
            Some(n) => n,
            None => continue,
        };
        let url = match it.next() {
            Some(u) => u,
            None => continue,
        };
        let kind = it.next().unwrap_or("(fetch)");
        let entry = by_name.entry(name.to_owned()).or_insert_with(|| (String::new(), String::new()));
        if kind.contains("fetch") {
            entry.0 = url.to_owned();
        } else if kind.contains("push") {
            entry.1 = url.to_owned();
        }
    }
    Ok(by_name
        .into_iter()
        .map(|(name, (fetch, push))| {
            let push = if push.is_empty() { fetch.clone() } else { push };
            GitRemote { name, fetch, push }
        })
        .collect())
}

fn list_tracked(project_root: &Utf8Path) -> Result<HashSet<Utf8PathBuf>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(project_root.as_str())
        .args(["ls-files", "-z"])
        .output()
        .map_err(|e| Error::GitMeta {
            path: project_root.as_std_path().to_path_buf(),
            message: format!("git ls-files: {e}"),
        })?;
    if !out.status.success() {
        return Err(Error::GitMeta {
            path: project_root.as_std_path().to_path_buf(),
            message: "git ls-files failed".into(),
        });
    }
    Ok(out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| std::str::from_utf8(s).ok().map(|s| Utf8PathBuf::from(s)))
        .collect())
}

pub fn serialize(meta: &GitMeta) -> Result<String> {
    toml::to_string_pretty(meta).map_err(|e| Error::GitMeta {
        path: Default::default(),
        message: format!("toml serialize: {e}"),
    })
}

pub fn deserialize(raw: impl AsRef<str>) -> Result<GitMeta> {
    toml::from_str(raw.as_ref()).map_err(|e| Error::GitMeta {
        path: Default::default(),
        message: format!("toml parse: {e}"),
    })
}
