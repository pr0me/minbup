use crate::error::{Error, Result};

const MANIFEST_HEADER: &str = "# minbup manifest v1\n# blake3-hex\tsize\tpath\n";

#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub blake3_hex: String,
    pub size: u64,
    pub path: String,
}

#[derive(Default, Debug)]
pub struct ManifestBuilder {
    entries: Vec<ManifestEntry>,
}

impl ManifestBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, entry: ManifestEntry) {
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn into_bytes(mut self) -> Result<Vec<u8>> {
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = String::with_capacity(self.entries.len() * 96 + MANIFEST_HEADER.len());
        out.push_str(MANIFEST_HEADER);
        for e in &self.entries {
            if e.path.contains('\t') || e.path.contains('\n') {
                return Err(Error::Manifest(format!(
                    "path contains tab or newline: {}",
                    e.path
                )));
            }
            out.push_str(&e.blake3_hex);
            out.push('\t');
            out.push_str(&e.size.to_string());
            out.push('\t');
            out.push_str(&e.path);
            out.push('\n');
        }
        Ok(out.into_bytes())
    }
}

pub fn parse(bytes: impl AsRef<[u8]>) -> Result<Vec<ManifestEntry>> {
    let s = std::str::from_utf8(bytes.as_ref())
        .map_err(|e| Error::Manifest(format!("non-utf8 manifest: {e}")))?;
    let mut out = Vec::new();
    for (i, line) in s.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let hex = parts
            .next()
            .ok_or_else(|| Error::Manifest(format!("line {}: missing hash", i + 1)))?;
        let size_s = parts
            .next()
            .ok_or_else(|| Error::Manifest(format!("line {}: missing size", i + 1)))?;
        let path = parts
            .next()
            .ok_or_else(|| Error::Manifest(format!("line {}: missing path", i + 1)))?;
        let size = size_s
            .parse::<u64>()
            .map_err(|e| Error::Manifest(format!("line {}: bad size: {e}", i + 1)))?;
        out.push(ManifestEntry {
            blake3_hex: hex.into(),
            size,
            path: path.into(),
        });
    }
    Ok(out)
}
