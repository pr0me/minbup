use std::io;
use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("walk: {0}")]
    Walk(String),

    #[error("archive: {0}")]
    Archive(String),

    #[error("encryption: {0}")]
    Encrypt(String),

    #[error("git metadata for {path}: {message}")]
    GitMeta { path: PathBuf, message: String },

    #[error("manifest: {0}")]
    Manifest(String),

    #[error("restore: {0}")]
    Restore(String),

    #[error("output {output} is inside target {target}; choose a path outside the target")]
    OutputInsideTarget { output: PathBuf, target: PathBuf },

    #[error("user aborted")]
    UserAbort,

    #[error("config: {0}")]
    Config(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
