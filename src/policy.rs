//! Policy bundles — versioned, auditable configuration that drives decisions.
//!
//! Policies are injected into the evaluation context and referenced by strategies
//! to parameterize thresholds, margins, rates, and other business rules.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A versioned bundle of policy values with audit metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundle {
    /// Unique identifier for this policy bundle.
    pub id: String,
    /// Semantic version of the policy bundle.
    pub version: String,
    /// When this policy became effective.
    pub effective_at: DateTime<Utc>,
    /// Git commit hash that produced this policy, if tracked.
    pub commit_hash: Option<String>,
    /// Key-value policy parameters.
    pub values: HashMap<String, PolicyValue>,
    /// Human-readable changelog entry.
    pub changelog: Option<String>,
}

/// A dynamically-typed policy value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicyValue {
    /// Floating-point parameter (e.g., margin percentages).
    Float(f64),
    /// Integer parameter (e.g., thresholds, counts).
    Int(i64),
    /// Boolean flag.
    Bool(bool),
    /// Text parameter (e.g., tier names, identifiers).
    Text(String),
    /// List of policy values.
    List(Vec<PolicyValue>),
    /// Nested map of policy values.
    Map(HashMap<String, PolicyValue>),
}

impl PolicyBundle {
    /// Create a new policy bundle with the given id and version.
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            effective_at: Utc::now(),
            commit_hash: None,
            values: HashMap::new(),
            changelog: None,
        }
    }

    /// Add a policy value, returning self for builder chaining.
    pub fn with_value(mut self, key: impl Into<String>, value: PolicyValue) -> Self {
        self.values.insert(key.into(), value);
        self
    }

    /// Set the git commit hash, returning self for builder chaining.
    pub fn with_commit(mut self, hash: impl Into<String>) -> Self {
        self.commit_hash = Some(hash.into());
        self
    }

    /// Set the changelog entry, returning self for builder chaining.
    pub fn with_changelog(mut self, changelog: impl Into<String>) -> Self {
        self.changelog = Some(changelog.into());
        self
    }

    /// Get a policy value by key.
    pub fn get(&self, key: &str) -> Option<&PolicyValue> {
        self.values.get(key)
    }

    /// Get a float policy value, falling back to a default.
    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        match self.values.get(key) {
            Some(PolicyValue::Float(v)) => *v,
            _ => default,
        }
    }

    /// Get a boolean policy value, falling back to a default.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        match self.values.get(key) {
            Some(PolicyValue::Bool(v)) => *v,
            _ => default,
        }
    }

    /// Get an integer policy value, falling back to a default.
    pub fn get_i64(&self, key: &str, default: i64) -> i64 {
        match self.values.get(key) {
            Some(PolicyValue::Int(v)) => *v,
            _ => default,
        }
    }

    /// Get a string policy value, falling back to a default.
    pub fn get_text(&self, key: &str, default: &str) -> String {
        match self.values.get(key) {
            Some(PolicyValue::Text(v)) => v.clone(),
            _ => default.to_string(),
        }
    }

    /// Compute a blake3 fingerprint of this policy bundle using canonical serialization.
    pub fn fingerprint(&self) -> String {
        let canonical = self.canonical_bytes();
        blake3::hash(&canonical).to_hex().to_string()
    }

    /// Produce canonical bytes for fingerprinting.
    fn canonical_bytes(&self) -> Vec<u8> {
        // Sort keys for deterministic serialization
        let sorted: BTreeMap<&str, &PolicyValue> =
            self.values.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let repr = serde_json::json!({
            "id": self.id,
            "version": self.version,
            "values": sorted,
        });
        serde_json::to_vec(&repr).unwrap_or_default()
    }
}
