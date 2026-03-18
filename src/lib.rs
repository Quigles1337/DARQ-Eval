#![deny(unsafe_code)]
//! # darq-eval-context
//!
//! Reusable evaluation context library for DARQ Labs business decisions.
//!
//! **Architecture**: Versioned evidence in, append-only patches through,
//! policy-driven disposition out.
//!
//! ## Core abstractions
//!
//! - [`decision::Decision`] — typed business decision with validated inputs and constrained outputs
//! - [`context::EvalContext`] — immutable evaluation environment passed to strategies
//! - [`strategy::EvalStrategy`] — pluggable evaluation logic running in Enrich/Evaluate/Decide phases
//! - [`pipeline::EvalPipeline`] — orchestrates strategy execution with patch merging
//! - [`artifact::ArtifactStore`] — append-only, content-addressed evidence store
//! - [`policy::PolicyBundle`] — versioned, auditable business rule parameters
//! - [`capability::Capability`] — injectable external data sources with record/replay
//! - [`trace::TraceHandle`] — structured audit trail with monotonic sequence numbers
//! - [`result::EvalResult`] — complete pipeline output with snapshot for replay

pub mod artifact;
pub mod capability;
pub mod context;
pub mod decision;
pub mod pipeline;
pub mod policy;
pub mod result;
pub mod strategy;
pub mod trace;

/// Re-exports of all public types for convenient use.
///
/// ```rust
/// use darq_eval_context::prelude::*;
/// ```
pub mod prelude {
    // decision
    pub use crate::decision::{Decision, ValidationError};

    // artifact
    pub use crate::artifact::{Artifact, ArtifactError, ArtifactStore, ReplayClass};

    // policy
    pub use crate::policy::{PolicyBundle, PolicyValue};

    // context
    pub use crate::context::{EvalContext, EvalMode, MemoStore, RunMeta};

    // trace
    pub use crate::trace::{
        ExplanationKind, ExplanationNode, TimestampedEvent, TraceEvent, TraceHandle,
    };

    // capability
    pub use crate::capability::{
        Capability, CapabilityError, CapabilityResult, CapabilitySet, RecordedCapability,
    };

    // strategy
    pub use crate::strategy::{
        BasisMetric, Control, DecisionBasis, DecisionDisposition, DenyReason, EvalError,
        EvalStrategy, Finding, HaltReason, Phase, Proposal, ReviewReason, Severity, StrategyPatch,
        StrategyVersion,
    };

    // pipeline
    pub use crate::pipeline::{EvalPipeline, PipelineError};

    // result
    pub use crate::result::{EvalResult, PipelineStep, ReplayFidelity, Snapshot, StepOutcome};
}
