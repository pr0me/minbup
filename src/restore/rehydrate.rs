use std::fs;
use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::WalkBuilder;
use tracing::{info, warn};

use crate::config::GITMETA_NAME;
use crate::error::{Error, Result};
use crate::walk::gitmeta::{self, GitMeta};

use super::RestoreSummary;

pub fn rehydrate_all(
    dest: &Utf8Path,
    full_history: bool,
    summary: &mut RestoreSummary,
) -> Result<()> {
    let metas = find_gitmeta_files(dest);
    info!("found {} .gitmeta sidecar(s)", metas.len());
    for meta_path in metas {
        match rehydrate_one(dest, &meta_path, full_history) {
            Ok(_) => summary.projects_rehydrated += 1,
            Err(e) => {
                warn!("rehydrate {}: {e}", meta_path);
                summary.projects_failed += 1;
            }
        }
    }
    Ok(())
}

fn find_gitmeta_files(dest: &Utf8Path) -> Vec<Utf8PathBuf> {
    let mut out = Vec::new();
    let walker = WalkBuilder::new(dest.as_std_path())
        .standard_filters(false)
        .hidden(false)
        .follow_links(false)
        .build();
    for r in walker {
        let entry = match r {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.file_name() == GITMETA_NAME && entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if let Some(p) = camino::Utf8Path::from_path(entry.path()) {
                out.push(p.to_path_buf());
            }
        }
    }
    out
}

fn rehydrate_one(dest: &Utf8Path, meta_path: &Utf8Path, full_history: bool) -> Result<()> {
    let raw = std::fs::read_to_string(meta_path.as_std_path())?;
    let meta: GitMeta = gitmeta::deserialize(raw)?;

    let project = if meta.project_path == "." || meta.project_path.is_empty() {
        dest.to_path_buf()
    } else {
        dest.join(&meta.project_path)
    };

    if !project.is_dir() {
        return Err(Error::Restore(format!("project dir missing: {project}")));
    }

    let g = |args: &[&str]| -> Result<()> {
        let out = Command::new("git")
            .arg("-C")
            .arg(project.as_str())
            .args(args)
            .output()
            .map_err(|e| Error::Restore(format!("git {args:?}: {e}")))?;
        if !out.status.success() {
            return Err(Error::Restore(format!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    };

    if !project.join(".git").is_dir() {
        match meta.head.branch.as_deref() {
            Some(b) => g(&["init", "-q", "-b", b])?,
            None => g(&["init", "-q"])?,
        }
    }
    for r in &meta.remotes {
        let _ = Command::new("git")
            .arg("-C")
            .arg(project.as_str())
            .args(["remote", "remove", &r.name])
            .output();
        g(&["remote", "add", &r.name, &r.fetch])?;
        if r.push != r.fetch {
            g(&["remote", "set-url", "--push", &r.name, &r.push])?;
        }
    }

    // working tree already matches the captured commit (we just extracted it).
    // fetch the ref, point HEAD at it, and `reset` to align the index without touching files.
    if let Some(branch) = meta.head.branch.as_deref() {
        if full_history {
            g(&["fetch", "origin", branch])?;
        } else {
            g(&["fetch", "--depth=1", "origin", branch])?;
        }
        g(&["update-ref", &format!("refs/heads/{branch}"), "FETCH_HEAD"])?;
        g(&["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])?;
        g(&["reset", "--quiet"])?;
    } else if !meta.head.commit.is_empty() {
        g(&["fetch", "origin", &meta.head.commit])?;
        g(&["update-ref", "HEAD", "FETCH_HEAD"])?;
        g(&["reset", "--quiet"])?;
    }

    add_to_git_exclude(&project, ".gitmeta");

    info!(
        "rehydrated {} ({})",
        project,
        meta.head
            .branch
            .as_deref()
            .unwrap_or(&meta.head.commit[..meta.head.commit.len().min(8)])
    );
    Ok(())
}

fn add_to_git_exclude(project: &Utf8Path, pattern: &str) {
    let exclude = project.join(".git").join("info").join("exclude");
    let existing = fs::read_to_string(exclude.as_std_path()).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == pattern) {
        return;
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(pattern);
    next.push('\n');
    if let Some(parent) = exclude.parent() {
        let _ = fs::create_dir_all(parent.as_std_path());
    }
    if let Err(e) = fs::write(exclude.as_std_path(), next) {
        warn!("write {exclude}: {e}");
    }
}
