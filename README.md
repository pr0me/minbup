# minbup

Single-shot migration backup tool. Walks a directory, produces one `tar.zst` archive, and reconstitutes git projects on the new machine via sidecar metadata. Optional passphrase encryption. Streaming end-to-end; no temporary copies.

## Install

```sh
cargo install --path .
```

Or run from the build dir: `cargo build --release && ./target/release/minbup …`.

Requires `git` on PATH for both backup (gathering remotes / tracked files) and restore (rehydration).

## Backup

```sh
minbup backup <target> [-o <archive>] [--encrypt] [--large-files prompt|keep|skip]
                       [-e <glob>...] [--no-default-excludes]
                       [--zstd-level N] [--zstd-workers N]
                       [--large-threshold BYTES]
```

Output defaults to `<target-name>-<UTC-stamp>.tar.zst` (or `.tar.zst.age` with `--encrypt`).

What it does:

1. Walks `<target>`, skipping `.git/`, language caches (`__pycache__`, `.venv`, `.mypy_cache`, …), editor / OS junk (`.DS_Store`, `.idea/`, `.vscode/`, `*.swp`, `Thumbs.db`), and Rust `target/` directories that have a sibling `Cargo.toml` (cargo build artifacts).
2. Honors `.bupignore` files (gitignore syntax) anywhere in the tree.
3. For each git project, emits a `.gitmeta` TOML sidecar containing remotes, current branch, current commit, and the verbatim `.git/config`. The `.git/` directory itself is **not** archived.
4. Streams everything into `tar` → `zstd` (multi-threaded) → optional `age` (passphrase) → file. The output is written to `<archive>.partial` and renamed atomically on success.
5. Files larger than `--large-threshold` (default 100 MiB) are queued and reviewed interactively after the small-file pass, with metadata (size, mtime, git-tracked y/n) shown per entry. `--large-files keep|skip` bypasses the prompt.
6. A `MANIFEST.blake3` listing every entry's BLAKE3 hash is appended as the final tar entry.

Symlinks, FIFOs, sockets, device files, and non-UTF-8 paths are dropped with a warning. Permission errors don't abort the run; they're counted and surfaced in the summary.

The output archive cannot live inside `<target>` — preflight rejects it.

## Restore

```sh
minbup restore <archive> <dest> [--no-git-rehydrate] [--full-history] [--skip-verify]
```

1. Streams the archive (decrypts with `MINBUP_PASSPHRASE` or TTY prompt if `.age`), extracts into `<dest>`.
2. Verifies every extracted file's BLAKE3 hash against `MANIFEST.blake3`. Mismatches fail the run.
3. For each `.gitmeta` sidecar found in the extracted tree:
   - `git init -b <branch>`
   - re-adds remotes from the sidecar
   - `git fetch --depth=1 origin <branch>` (use `--full-history` for a full clone)
   - `git update-ref refs/heads/<branch> FETCH_HEAD`, `symbolic-ref HEAD …`, `git reset` — points HEAD at the captured commit without disturbing the just-extracted working tree, leaving `git status` clean.
   - adds `.gitmeta` to `.git/info/exclude` so the sidecar doesn't show up in `git status`.

A project whose remote is unreachable is logged and skipped; the working tree is preserved as-is and the run exits non-zero with a summary.

## Encryption

`--encrypt` wraps the compressed stream with [age](https://age-encryption.org) using a passphrase. The passphrase is read from `MINBUP_PASSPHRASE` if set, otherwise prompted on the TTY. Order is compress → encrypt (encrypted ciphertext is incompressible).

Output extension: `.tar.zst.age`. Decryption with the standard `age` CLI also works:

```sh
age -d archive.tar.zst.age | tar --zstd -xf -
```

## TUI

When stderr is a TTY, a ratatui interface shows phase, progress gauge, ETA, current path, throughput sparkline, and live counters (scanned, uncompressed, compressed-on-disk + ratio, files done / total, projects, large-files queued, errors skipped). The large-file review is an interactive modal: `↑/↓ j/k` to move, `space`/`enter` to toggle, `a` keep all, `s` skip all, `q` confirm.

`Ctrl+C`, `Esc`, or `q` aborts the run cleanly — the `.partial` archive is removed and the process exits non-zero. SIGINT is also handled in non-TTY mode.

When stderr isn't a TTY (CI, redirects), a plain `[phase] files X/Y archive Z (rate/s)` line is logged to stderr every 2s, and `--large-files prompt` defaults to `keep`.

## Manifest format

`MANIFEST.blake3` is the last entry in the archive. Plain text, tab-separated, sorted by path:

```
# minbup manifest v1
# blake3-hex	size	path
abcd…	12345	src/main.rs
…
```

## .gitmeta format

Per-project TOML sidecar at `<project>/.gitmeta` inside the archive:

```toml
schema_version = 1
captured_at = "2026-04-27T18:08:49Z"
project_path = "proj-a"

[head]
branch = "main"
detached = false
commit = "868b69fa…"

[[remotes]]
name = "origin"
fetch = "git@github.com:user/repo.git"
push  = "git@github.com:user/repo.git"

[config]
raw = "<verbatim .git/config>"
```

## Out of scope

- Resumable / incremental backups (interrupt = invalid `.partial`, removed automatically)
- Hardlink deduplication
- macOS extended attributes / resource forks
- Windows
- Shallow vs. full repo restore beyond `--full-history`

## License

MIT
