//! Capability injection — external data sources with record/replay support.
//!
//! Capabilities abstract over external data fetches (XRPL balance lookups,
//! competitor pricing APIs, etc.). During live evaluation, real capabilities
//! fetch from external sources. During replay, `RecordedCapability` returns
//! previously recorded values for deterministic reproduction.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;

use crate::artifact::{Artifact, ReplayClass};

/// Result of a capability fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    /// The fetched value.
    pub value: serde_json::Value,
}

/// Errors from capability operations.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum CapabilityError {
    /// The requested key was not found.
    #[error("capability key not found: {key}")]
    NotFound { key: String },

    /// The capability fetch failed.
    #[error("capability fetch failed: {reason}")]
    FetchFailed { reason: String },
}

/// A source of external data that can be injected into the evaluation pipeline.
pub trait Capability: Send + Sync + Debug {
    /// The name of this capability (e.g., `"xrpl_balance"`).
    fn name(&self) -> &str;

    /// Fetch a value by key from this capability.
    fn fetch(&self, key: &str) -> Result<CapabilityResult, CapabilityError>;
}

/// A set of named capabilities available during evaluation.
#[derive(Debug, Default)]
pub struct CapabilitySet {
    capabilities: HashMap<String, Box<dyn Capability>>,
}

impl CapabilitySet {
    /// Create an empty capability set.
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
        }
    }

    /// Register a capability by name.
    pub fn register(&mut self, capability: Box<dyn Capability>) {
        let name = capability.name().to_string();
        self.capabilities.insert(name, capability);
    }

    /// Fetch from a named capability.
    pub fn fetch(&self, name: &str, key: &str) -> Result<CapabilityResult, CapabilityError> {
        self.capabilities
            .get(name)
            .ok_or_else(|| CapabilityError::NotFound {
                key: format!("capability '{}' not registered", name),
            })?
            .fetch(key)
    }

    /// Check if a capability is registered.
    pub fn has(&self, name: &str) -> bool {
        self.capabilities.contains_key(name)
    }
}

/// A recorded capability that replays previously captured values.
///
/// Used for deterministic replay: external artifact values are stored during
/// the initial run, then replayed via this capability on subsequent runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedCapability {
    name: String,
    recordings: HashMap<String, serde_json::Value>,
}

impl RecordedCapability {
    /// Create a new recorded capability with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            recordings: HashMap::new(),
        }
    }

    /// Record a value for a key.
    pub fn record(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.recordings.insert(key.into(), value);
    }

    /// Build a `RecordedCapability` from a snapshot's external artifacts.
    ///
    /// Groups external artifacts by source (capability name), using the artifact
    /// key as the recording key and the artifact value as the recorded value.
    pub fn from_artifacts(artifacts: &[Artifact]) -> Vec<RecordedCapability> {
        let mut by_source: HashMap<String, RecordedCapability> = HashMap::new();
        for artifact in artifacts {
            if artifact.replay_class == ReplayClass::External {
                let cap = by_source
                    .entry(artifact.source.clone())
                    .or_insert_with(|| RecordedCapability::new(artifact.source.clone()));
                cap.record(artifact.key.clone(), artifact.value.clone());
            }
        }
        by_source.into_values().collect()
    }
}

impl Capability for RecordedCapability {
    fn name(&self) -> &str {
        &self.name
    }

    fn fetch(&self, key: &str) -> Result<CapabilityResult, CapabilityError> {
        self.recordings
            .get(key)
            .cloned()
            .map(|value| CapabilityResult { value })
            .ok_or_else(|| CapabilityError::NotFound {
                key: key.to_string(),
            })
    }
}
