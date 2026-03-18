//! Evaluation result — the complete output of a pipeline run.
//!
//! `EvalResult<D>` contains the disposition, all proposals, findings, artifacts,
//! pipeline step outcomes, trace, and a snapshot for replay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::artifact::{Artifact, ArtifactStore, ReplayClass};
use crate::context::{EvalMode, RunMeta};
use crate::decision::Decision;
use crate::policy::PolicyBundle;
use crate::strategy::{DecisionDisposition, Finding, Proposal};
use crate::trace::TraceHandle;

/// The complete result of a pipeline evaluation.
#[derive(Debug)]
pub struct EvalResult<D: Decision> {
    /// Unique identifier for this evaluation.
    pub eval_id: Uuid,
    /// Name of the pipeline that produced this result.
    pub pipeline: String,
    /// The final disposition, if the pipeline reached a decision.
    pub disposition: Option<DecisionDisposition<D>>,
    /// All proposals produced by strategies.
    pub proposals: Vec<Proposal<D>>,
    /// All findings produced by strategies.
    pub findings: Vec<Finding>,
    /// The final artifact store.
    pub artifacts: ArtifactStore,
    /// Step-by-step execution record.
    pub steps: Vec<PipelineStep>,
    /// The full evaluation trace.
    pub trace: TraceHandle,
    /// A snapshot for replay.
    pub snapshot: Snapshot,
}

/// A record of a single pipeline step execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    /// The strategy that executed.
    pub strategy: String,
    /// The phase the strategy ran in.
    pub phase: String,
    /// The outcome of the step.
    pub outcome: StepOutcome,
}

/// The outcome of a single pipeline step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepOutcome {
    /// The strategy completed and pipeline continues.
    Continued,
    /// The strategy finalized the decision.
    Finalized,
    /// The strategy escalated to review.
    Escalated,
    /// The strategy halted the pipeline.
    Halted { reason: String },
    /// The strategy failed with an error.
    Failed { error: String },
    /// A `Control::Finalize` was ignored (not in Decide phase).
    FinalizeIgnored,
}

/// Replay fidelity level for snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayFidelity {
    /// Only external artifacts are captured. Deterministic artifacts recompute.
    Semantic,
    /// Full artifact store captured. Exact reproduction regardless of code changes.
    Exact,
}

/// A snapshot of a pipeline evaluation for replay or audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The evaluation ID this snapshot was captured from.
    pub eval_id: Uuid,
    /// The decision kind (e.g., `"blockhb.pricing"`).
    pub decision_kind: String,
    /// When this snapshot was captured.
    pub captured_at: DateTime<Utc>,
    /// The evaluation mode.
    pub mode: EvalMode,
    /// The replay fidelity level.
    pub fidelity: ReplayFidelity,
    /// Metadata from the evaluation run.
    pub meta: RunMeta,
    /// Serialized input.
    pub input_json: serde_json::Value,
    /// Serialized constraints.
    pub constraints_json: serde_json::Value,
    /// The policy bundle used.
    pub policy_bundle: PolicyBundle,
    /// Captured artifacts (scope depends on fidelity).
    pub artifacts: Vec<Artifact>,
    /// Versions of all strategies that executed.
    pub strategy_versions: Vec<(String, String)>,
}

impl Snapshot {
    /// Capture a snapshot from a completed evaluation.
    ///
    /// With `ReplayFidelity::Exact`, all artifacts are stored.
    /// With `ReplayFidelity::Semantic`, only external artifacts are stored
    /// (deterministic artifacts will be recomputed on replay).
    #[allow(clippy::too_many_arguments)]
    pub fn capture<D: Decision>(
        eval_id: Uuid,
        mode: EvalMode,
        fidelity: ReplayFidelity,
        meta: RunMeta,
        input: &D::Input,
        constraints: &D::Constraints,
        policy: &PolicyBundle,
        artifacts: &ArtifactStore,
        strategy_versions: Vec<(String, String)>,
    ) -> Self {
        let captured_artifacts = match fidelity {
            ReplayFidelity::Exact => artifacts.iter().cloned().collect(),
            ReplayFidelity::Semantic => artifacts
                .iter()
                .filter(|a| a.replay_class == ReplayClass::External)
                .cloned()
                .collect(),
        };

        Self {
            eval_id,
            decision_kind: D::KIND.to_string(),
            captured_at: Utc::now(),
            mode,
            fidelity,
            meta,
            input_json: serde_json::to_value(input).unwrap_or_default(),
            constraints_json: serde_json::to_value(constraints).unwrap_or_default(),
            policy_bundle: policy.clone(),
            artifacts: captured_artifacts,
            strategy_versions,
        }
    }
}
