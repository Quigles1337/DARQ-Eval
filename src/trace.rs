//! Structured tracing — monotonic sequence numbers, typed events, explanation trees.
//!
//! The trace captures a full audit trail of pipeline execution: every phase start,
//! strategy invocation, patch merge, and decision finalization. Sequence numbers
//! provide unambiguous ordering even when wall-clock timestamps collide.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A handle to the trace for a single evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceHandle {
    /// The evaluation ID this trace belongs to.
    pub eval_id: Uuid,
    /// All recorded events with timestamps and sequence numbers.
    pub events: Vec<TimestampedEvent>,
    /// Explanation nodes accumulated from strategies and the pipeline.
    pub explanations: Vec<ExplanationNode>,
    /// Monotonic counter for sequence numbers.
    seq_counter: u64,
}

/// A trace event paired with its timestamp and logical sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampedEvent {
    /// Logical sequence number — monotonically increasing, unambiguous ordering.
    pub seq: u64,
    /// Wall-clock timestamp.
    pub timestamp: DateTime<Utc>,
    /// The event that occurred.
    pub event: TraceEvent,
}

/// Events recorded during pipeline evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceEvent {
    /// Input validation passed.
    InputValidated,
    /// Output validation passed.
    OutputValidated,
    /// A pipeline phase has started.
    PhaseStarted { phase: String },
    /// A pipeline phase has completed.
    PhaseCompleted { phase: String },
    /// A strategy has started execution.
    StrategyStarted { strategy: String, phase: String },
    /// A strategy has completed successfully.
    StrategyCompleted { strategy: String, phase: String },
    /// A strategy has failed.
    StrategyFailed { strategy: String, error: String },
    /// A strategy patch has been merged into the context.
    PatchMerged { strategy: String, artifacts: usize },
    /// A final decision disposition has been set.
    DecisionFinalized { strategy: String },
    /// The pipeline has completed.
    PipelineCompleted,
    /// A capability was invoked during evaluation.
    CapabilityInvoked { capability: String, key: String },
    /// A capability result was recorded for replay.
    CapabilityRecorded { capability: String, key: String },
    /// A `Control::Finalize` was ignored because it was emitted outside Phase::Decide.
    FinalizeIgnored { strategy: String, phase: String },
    /// A custom event for extensibility.
    Custom { kind: String, detail: String },
}

/// A structured explanation node for audit and analytics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplanationNode {
    /// Which strategy or pipeline component produced this explanation.
    pub source: String,
    /// The kind of explanation.
    pub kind: ExplanationKind,
    /// Human-readable detail.
    pub detail: String,
    /// Keys of artifacts referenced by this explanation.
    pub artifact_refs: Vec<String>,
    /// Keys of policy values referenced by this explanation.
    pub policy_refs: Vec<String>,
}

/// Classification of explanation nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExplanationKind {
    /// A policy rule matched.
    RuleMatched,
    /// A threshold was exceeded.
    ThresholdExceeded,
    /// Required evidence was not found.
    EvidenceMissing,
    /// A proposal was dominated by another.
    ProposalDominated,
    /// The decision was escalated for review.
    EscalationTriggered,
    /// A pipeline invariant was violated (e.g., finalize outside Decide phase).
    InvariantViolation,
    /// An enrichment was added.
    Enrichment,
    /// A custom explanation kind.
    Custom(String),
}

impl TraceHandle {
    /// Create a new trace handle for the given evaluation ID.
    pub fn new(eval_id: Uuid) -> Self {
        Self {
            eval_id,
            events: Vec::new(),
            explanations: Vec::new(),
            seq_counter: 0,
        }
    }

    /// Record a trace event with a monotonically increasing sequence number.
    pub fn record(&mut self, event: TraceEvent) {
        let seq = self.seq_counter;
        self.seq_counter += 1;
        self.events.push(TimestampedEvent {
            seq,
            timestamp: Utc::now(),
            event,
        });
    }

    /// Add an explanation node to the trace.
    pub fn explain(&mut self, node: ExplanationNode) {
        self.explanations.push(node);
    }

    /// Add multiple explanation nodes to the trace.
    pub fn explain_all(&mut self, nodes: impl IntoIterator<Item = ExplanationNode>) {
        self.explanations.extend(nodes);
    }

    /// Merge events and explanations from a child trace into this trace.
    pub fn merge(&mut self, child: TraceHandle) {
        for event in child.events {
            self.record(event.event);
        }
        self.explanations.extend(child.explanations);
    }
}

impl ExplanationNode {
    /// Render this explanation as a human-readable string.
    pub fn render(&self) -> String {
        let kind_str = match &self.kind {
            ExplanationKind::RuleMatched => "RULE_MATCHED",
            ExplanationKind::ThresholdExceeded => "THRESHOLD_EXCEEDED",
            ExplanationKind::EvidenceMissing => "EVIDENCE_MISSING",
            ExplanationKind::ProposalDominated => "PROPOSAL_DOMINATED",
            ExplanationKind::EscalationTriggered => "ESCALATION_TRIGGERED",
            ExplanationKind::InvariantViolation => "INVARIANT_VIOLATION",
            ExplanationKind::Enrichment => "ENRICHMENT",
            ExplanationKind::Custom(s) => s.as_str(),
        };
        format!("[{}] {} — {}", kind_str, self.source, self.detail)
    }
}
