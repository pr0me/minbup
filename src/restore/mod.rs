pub mod extract;
pub mod rehydrate;

#[derive(Debug, Default)]
pub struct RestoreSummary {
    pub files_extracted: u64,
    pub bytes_extracted: u64,
    pub manifest_entries: u64,
    pub verification_failures: u64,
    pub projects_rehydrated: u64,
    pub projects_failed: u64,
}
