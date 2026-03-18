//! Strategy trait and supporting types — the pluggable evaluation logic.
//!
//! Strategies are the building blocks of evaluation pipelines. Each strategy
//! runs in a specific phase (Enrich, Evaluate, or Decide) and produces a
//! `StrategyPatch` containing artifacts, findings, proposals, and control flow.

use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use crate::artifact::Artifact;
use crate::capability::CapabilitySet;
use crate::context::EvalContext;
use crate::decision::Decision;
use crate::trace::ExplanationNode;

/// The three phases of pipeline evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Phase {
    /// Gather external data and enrich the context.
    Enrich = 0,
    /// Score, rank, and assess the enriched data.
    Evaluate = 1,
    /// Make a final disposition decision.
    Decide = 2,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Phase::Enrich => write!(f, "Enrich"),
            Phase::Evaluate => write!(f, "Evaluate"),
            Phase::Decide => write!(f, "Decide"),
        }
    }
}

/// Semantic version of a strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for StrategyVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A pluggable evaluation strategy that runs in a specific pipeline phase.
///
/// Strategies receive an immutable reference to the evaluation context
/// (`&EvalContext<D>`) and produce a `StrategyPatch` with their results.
/// They cannot mutate the context directly — only the pipeline merges patches.
pub trait EvalStrategy<D: Decision>: Debug + Send + Sync {
    /// Unique identifier for this strategy.
    fn id(&self) -> &str;

    /// The version of this strategy implementation.
    fn version(&self) -> StrategyVersion;

    /// Which pipeline phase this strategy runs in.
    fn phase(&self) -> Phase;

    /// Execution priority within a phase (lower runs first).
    fn priority(&self) -> i32 {
        0
    }

    /// Execute the strategy against the evaluation context.
    ///
    /// Returns a `StrategyPatch` containing any artifacts, findings, proposals,
    /// explanations, and control flow produced by this strategy.
    fn evaluate(
        &self,
        ctx: &EvalContext<D>,
        capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<D>, EvalError>;
}

/// The output of a strategy execution — merged into the pipeline context.
#[derive(Debug)]
pub struct StrategyPatch<D: Decision> {
    /// Artifacts produced by this strategy.
    pub artifacts: Vec<Artifact>,
    /// Findings (observations, warnings, alerts) produced by this strategy.
    pub findings: Vec<Finding>,
    /// Proposed outputs with supporting basis.
    pub proposals: Vec<Proposal<D>>,
    /// Explanation nodes for audit.
    pub explanations: Vec<ExplanationNode>,
    /// Control flow directive.
    pub control: Control<D>,
}

impl<D: Decision> StrategyPatch<D> {
    /// Create a new patch that continues pipeline execution with no outputs.
    pub fn empty() -> Self {
        Self {
            artifacts: Vec::new(),
            findings: Vec::new(),
            proposals: Vec::new(),
            explanations: Vec::new(),
            control: Control::Continue,
        }
    }
}

/// A proposed decision output with its supporting evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal<D: Decision> {
    /// The proposed output value.
    pub output: D::Output,
    /// Which strategy proposed this output.
    pub proposed_by: String,
    /// The evidence and rationale supporting this proposal.
    pub basis: DecisionBasis,
}

/// The evidence and rationale supporting a proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionBasis {
    /// Named scores and metrics supporting the decision.
    pub metrics: Vec<BasisMetric>,
    /// Keys of artifacts referenced as evidence.
    pub artifact_refs: Vec<String>,
    /// Keys of policy values referenced.
    pub policy_refs: Vec<String>,
    /// Human-readable rationale.
    pub rationale: String,
}

/// A named metric in a decision basis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasisMetric {
    /// Name of the metric (e.g., `"margin_pct"`).
    pub name: String,
    /// The metric value.
    pub value: f64,
    /// Optional unit (e.g., `"%"`, `"USD"`).
    pub unit: Option<String>,
}

/// Control flow directive from a strategy.
#[derive(Debug)]
pub enum Control<D: Decision> {
    /// Continue to the next strategy.
    Continue,
    /// Finalize the pipeline with a disposition. **Only valid in Phase::Decide.**
    Finalize(DecisionDisposition<D>),
    /// Escalate for human review.
    Escalate(ReviewReason),
    /// Halt the pipeline immediately.
    Halt(HaltReason),
}

/// The final disposition of a decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecisionDisposition<D: Decision> {
    /// Automatically approved output.
    Auto(D::Output),
    /// Requires human review before proceeding.
    Review {
        provisional: Option<D::Output>,
        reasons: Vec<ReviewReason>,
    },
    /// Denied — the decision cannot proceed.
    Deny { reasons: Vec<DenyReason> },
}

/// A finding produced by a strategy — an observation, warning, or alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Which strategy produced this finding.
    pub source: String,
    /// The severity of the finding.
    pub severity: Severity,
    /// Short identifier for the finding type.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Keys of related artifacts.
    pub artifact_refs: Vec<String>,
}

/// Severity levels for findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Informational observation.
    Info,
    /// Something noteworthy that may need attention.
    Warning,
    /// A critical issue that should block auto-approval.
    Critical,
}

/// Reason for escalating a decision to human review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReason {
    /// Short code identifying the review reason.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Reason for denying a decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenyReason {
    /// Short code identifying the denial reason.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Reason for halting the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaltReason {
    /// Short code identifying the halt reason.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
}

/// Errors from strategy evaluation (engine failures, not business outcomes).
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum EvalError {
    /// A required artifact was not found.
    #[error("missing artifact: {key}")]
    MissingArtifact { key: String },

    /// A capability fetch failed.
    #[error("capability error: {reason}")]
    CapabilityError { reason: String },

    /// An internal error occurred in the strategy.
    #[error("internal error: {reason}")]
    Internal { reason: String },

    /// Input validation failed.
    #[error("validation error: {reason}")]
    ValidationError { reason: String },
}
