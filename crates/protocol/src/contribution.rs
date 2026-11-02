//! Portable, versioned analysis artifacts shared by the collector and server.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;
pub const ANALYSIS_VERSION: i64 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: i64,
    pub committer_date: i64,
    pub additions: i64,
    pub deletions: i64,
    /// Changed-file extension counts relative to the first parent, as upstream.
    pub language_stats: BTreeMap<String, i64>,
    pub complete: bool,
}

impl Commit {
    pub fn effectives(&self) -> i64 {
        (self.additions - self.deletions).max(0)
    }
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
    pub fn message_type(&self) -> String {
        let message = self.message.replace('：', ":");
        message
            .split_once(':')
            .map(|(prefix, _)| {
                prefix
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn valid(&self) -> bool {
        valid_sha(&self.sha)
            && self.parents.iter().all(|p| valid_sha(p))
            && self.additions >= 0
            && self.deletions >= 0
            && self.additions.checked_add(self.deletions).is_some()
            && self.language_stats.values().all(|n| *n >= 0)
    }
}

pub fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub schema_version: u32,
    pub analysis_version: i64,
    pub task_id: String,
    pub generation: i64,
    pub repository: String,
    pub coverage_start: i64,
    pub refs: BTreeMap<String, String>,
    /// Complete in-range reachable SHA set per frozen branch, including cached facts.
    pub ref_commits: BTreeMap<String, Vec<String>>,
    pub commits: Vec<Commit>,
    pub complete: bool,
}
