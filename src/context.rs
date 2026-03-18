//! Evaluation context — the immutable environment passed to strategies.
//!
//! `EvalContext<D>` bundles all inputs, policy, artifacts, and metadata that a
//! strategy needs to evaluate a decision. Strategies receive `&EvalContext<D>`
//! (shared reference), enforcing that they cannot mutate the context directly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use uuid::Uuid;

use crate::artifact::ArtifactStore;
use crate::decision::Decision;
use crate::policy::PolicyBundle;

/// The evaluation context for a decision, passed to strategies as `&EvalContext<D>`.
///
/// The pipeline owns the mutable context and mutates it between strategy
/// executions (merging patches). Strategies only see a shared reference.
#[derive(Debug)]
pub struct EvalContext<D: Decision> {
    /// Unique identifier for this evaluation run.
    pub eval_id: Uuid,
    /// The input to the decision.
    pub input: D::Input,
    /// The constraints bounding acceptable outputs.
    pub constraints: D::Constraints,
    /// The policy bundle parameterizing this evaluation.
    pub policy: PolicyBundle,
    /// The artifact store — written between phases by the pipeline, read by strategies.
    pub artifacts: ArtifactStore,
    /// Ephemeral scratch storage. **Not replayed.**
    pub memo: MemoStore,
    /// Metadata about this evaluation run.
    pub meta: RunMeta,
    /// The evaluation mode (live, replay, shadow, simulation).
    pub mode: EvalMode,
}

impl<D: Decision> EvalContext<D> {
    /// Create a new evaluation context.
    pub fn new(
        input: D::Input,
        constraints: D::Constraints,
        policy: PolicyBundle,
        meta: RunMeta,
        mode: EvalMode,
    ) -> Self {
        Self {
            eval_id: Uuid::new_v4(),
            input,
            constraints,
            policy,
            artifacts: ArtifactStore::new(),
            memo: MemoStore::new(),
            meta,
            mode,
        }
    }
}

/// Ephemeral scratch storage for performance caches.
///
/// # Invariant
///
/// Memo contents MUST NOT affect decision semantics. Any value that
/// influences the final disposition, proposals, findings, or artifacts
/// MUST be stored as an artifact with full provenance. Memo is for
/// performance caches only (e.g., deserialized objects, pre-computed
/// intermediate values that are ALSO stored as artifacts).
///
/// Memo is excluded from replay snapshots by design.
#[derive(Debug, Default)]
pub struct MemoStore {
    entries: HashMap<String, Box<dyn Any + Send + Sync>>,
}

impl MemoStore {
    /// Create an empty memo store.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Store a value in the memo. Overwrites any existing value for the key.
    pub fn set<T: Any + Send + Sync>(&mut self, key: impl Into<String>, value: T) {
        self.entries.insert(key.into(), Box::new(value));
    }

    /// Retrieve a value from the memo by key and type.
    pub fn get<T: Any + Send + Sync>(&self, key: &str) -> Option<&T> {
        self.entries.get(key)?.downcast_ref::<T>()
    }
}

/// Metadata about an evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    /// Identifier for the decision type.
    pub decision_id: String,
    /// Identifier for the specific case being evaluated.
    pub case_id: String,
    /// Tenant or organization identifier.
    pub tenant: String,
    /// What triggered this evaluation (e.g., `"api"`, `"scheduler"`).
    pub triggered_by: String,
    /// Human-readable reason for this evaluation.
    pub reason: String,
    /// Version of the evaluation engine.
    pub engine_version: String,
    /// When this evaluation was initiated.
    pub timestamp: DateTime<Utc>,
    /// Arbitrary tags for filtering and grouping.
    pub tags: HashMap<String, String>,
}

impl RunMeta {
    /// Create a new `RunMeta` with the engine version set from the crate version.
    pub fn new(
        decision_id: impl Into<String>,
        case_id: impl Into<String>,
        tenant: impl Into<String>,
        triggered_by: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            case_id: case_id.into(),
            tenant: tenant.into(),
            triggered_by: triggered_by.into(),
            reason: reason.into(),
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now(),
            tags: HashMap::new(),
        }
    }

    /// Add a tag, returning self for builder chaining.
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }
}

/// The mode of evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvalMode {
    /// Live production evaluation.
    Live,
    /// Replaying a previous evaluation from a snapshot.
    Replay,
    /// Shadow mode — evaluate but do not act on the result.
    Shadow,
    /// Simulation with synthetic data.
    Simulation,
}
