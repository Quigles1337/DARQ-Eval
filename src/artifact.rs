//! Artifact store — append-only, content-addressed evidence produced by strategies.
//!
//! Artifacts are the primary data-exchange mechanism between pipeline phases.
//! Each artifact is content-addressed via blake3 over canonical JSON, ensuring
//! deterministic digests regardless of key insertion order.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Classification of an artifact for replay purposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayClass {
    /// Computed deterministically from inputs — can be recomputed on replay.
    Deterministic,
    /// Fetched from an external source — must be recorded for replay.
    External,
    /// Derived from other artifacts — can be recomputed if parents are available.
    Derived,
}

impl std::fmt::Display for ReplayClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayClass::Deterministic => write!(f, "deterministic"),
            ReplayClass::External => write!(f, "external"),
            ReplayClass::Derived => write!(f, "derived"),
        }
    }
}

/// A single piece of versioned evidence produced by a strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Namespaced key, e.g. `"pricing.cost_basis.v1"`.
    pub key: String,
    /// The artifact payload.
    pub value: Value,
    /// Which strategy produced this artifact.
    pub source: String,
    /// Version of the producing strategy.
    pub source_version: String,
    /// When the artifact was observed/produced.
    pub observed_at: DateTime<Utc>,
    /// Schema version for the value format.
    pub schema_version: String,
    /// Blake3 digest of the canonical JSON serialization.
    pub digest: String,
    /// How this artifact behaves during replay.
    pub replay_class: ReplayClass,
}

impl Artifact {
    /// Create a new artifact with a canonical blake3 digest.
    pub fn new(
        key: impl Into<String>,
        value: Value,
        source: impl Into<String>,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
    ) -> Self {
        let digest = compute_digest(&value);
        Self {
            key: key.into(),
            value,
            source: source.into(),
            source_version: source_version.into(),
            observed_at: Utc::now(),
            schema_version: schema_version.into(),
            digest,
            replay_class: ReplayClass::Deterministic,
        }
    }

    /// Create a new artifact marked as external (for replay recording).
    pub fn new_external(
        key: impl Into<String>,
        value: Value,
        source: impl Into<String>,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
    ) -> Self {
        let mut artifact = Self::new(key, value, source, source_version, schema_version);
        artifact.replay_class = ReplayClass::External;
        artifact
    }

    /// Create a new artifact marked as derived.
    pub fn new_derived(
        key: impl Into<String>,
        value: Value,
        source: impl Into<String>,
        source_version: impl Into<String>,
        schema_version: impl Into<String>,
    ) -> Self {
        let mut artifact = Self::new(key, value, source, source_version, schema_version);
        artifact.replay_class = ReplayClass::Derived;
        artifact
    }

    /// Set the replay class on this artifact, returning self for chaining.
    pub fn with_replay_class(mut self, class: ReplayClass) -> Self {
        self.replay_class = class;
        self
    }
}

/// Compute a blake3 digest of a JSON value using canonical serialization.
fn compute_digest(value: &Value) -> String {
    let bytes = canonical_json_bytes(value);
    blake3::hash(&bytes).to_hex().to_string()
}

/// Recursively sort all object keys and produce canonical JSON bytes.
///
/// This ensures two semantically identical `Value`s always produce the same
/// digest, regardless of key insertion order.
fn canonical_json_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Object(map) => {
            let mut sorted: BTreeMap<&str, Value> = BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.as_str(), canonicalize(v));
            }
            serde_json::to_vec(&sorted).unwrap_or_default()
        }
        Value::Array(arr) => {
            let canonical: Vec<Value> = arr.iter().map(canonicalize).collect();
            serde_json::to_vec(&canonical).unwrap_or_default()
        }
        other => serde_json::to_vec(other).unwrap_or_default(),
    }
}

/// Recursively canonicalize a JSON value (sort object keys at every level).
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = serde_json::Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Errors from artifact operations.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum ArtifactError {
    /// Requested artifact key was not found in the store.
    #[error("artifact not found: {key}")]
    NotFound { key: String },
}

/// Append-only store of artifacts produced during pipeline evaluation.
///
/// Artifacts are stored in emission order and never overwritten. Multiple
/// artifacts may share the same key (e.g., from different strategies).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtifactStore {
    entries: Vec<Artifact>,
}

impl ArtifactStore {
    /// Create an empty artifact store.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append a single artifact. Never overwrites existing entries.
    pub fn append(&mut self, artifact: Artifact) {
        self.entries.push(artifact);
    }

    /// Append multiple artifacts.
    pub fn append_all(&mut self, artifacts: impl IntoIterator<Item = Artifact>) {
        self.entries.extend(artifacts);
    }

    /// Get all artifacts with the given key, in emission order.
    pub fn get_all(&self, key: &str) -> Vec<&Artifact> {
        self.entries.iter().filter(|a| a.key == key).collect()
    }

    /// Get the last emitted artifact with the given key.
    pub fn get_latest(&self, key: &str) -> Option<&Artifact> {
        self.entries.iter().rev().find(|a| a.key == key)
    }

    /// Get the latest artifact with the given key, or return an error.
    pub fn get_required(&self, key: &str) -> Result<&Artifact, ArtifactError> {
        self.get_latest(key).ok_or_else(|| ArtifactError::NotFound {
            key: key.to_string(),
        })
    }

    /// Get the latest artifact with the given key from a specific source.
    pub fn get_latest_by_source(&self, key: &str, source: &str) -> Option<&Artifact> {
        self.entries
            .iter()
            .rev()
            .find(|a| a.key == key && a.source == source)
    }

    /// Get all artifacts produced by a specific strategy.
    pub fn from_source(&self, source: &str) -> Vec<&Artifact> {
        self.entries.iter().filter(|a| a.source == source).collect()
    }

    /// Get all artifacts whose key starts with the given prefix.
    pub fn with_prefix(&self, prefix: &str) -> Vec<&Artifact> {
        self.entries
            .iter()
            .filter(|a| a.key.starts_with(prefix))
            .collect()
    }

    /// Get all artifacts with `ReplayClass::External`.
    pub fn external_artifacts(&self) -> Vec<&Artifact> {
        self.entries
            .iter()
            .filter(|a| a.replay_class == ReplayClass::External)
            .collect()
    }

    /// Number of artifacts in the store.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all artifacts in emission order.
    pub fn iter(&self) -> impl Iterator<Item = &Artifact> {
        self.entries.iter()
    }

    /// Consume the store and return the underlying entries.
    pub fn into_entries(self) -> Vec<Artifact> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_digest_is_insertion_order_independent() {
        let v1 = serde_json::json!({"b": 2, "a": 1, "c": {"z": 26, "y": 25}});
        let v2 = serde_json::json!({"a": 1, "c": {"y": 25, "z": 26}, "b": 2});
        let a1 = Artifact::new("test.key", v1, "test", "1.0", "v1");
        let a2 = Artifact::new("test.key", v2, "test", "1.0", "v1");
        assert_eq!(a1.digest, a2.digest);
    }
}
