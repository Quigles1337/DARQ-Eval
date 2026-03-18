//! Evaluation pipeline — orchestrates strategy execution across phases.
//!
//! The pipeline sorts strategies by `(phase, priority, id)`, executes them
//! sequentially, merges patches into the context between strategies, and
//! enforces invariants (e.g., `Finalize` only in `Phase::Decide`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::capability::CapabilitySet;
use crate::context::EvalContext;
use crate::decision::{Decision, ValidationError};
use crate::result::{EvalResult, PipelineStep, ReplayFidelity, Snapshot, StepOutcome};
use crate::strategy::{
    Control, DecisionDisposition, EvalError, EvalStrategy, Finding, Phase, Proposal, StrategyPatch,
};
use crate::trace::{ExplanationKind, ExplanationNode, TraceEvent, TraceHandle};

/// Errors from pipeline execution (engine failures, not business outcomes).
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum PipelineError {
    /// Input validation failed.
    #[error("input validation failed: {0}")]
    InputValidation(ValidationError),

    /// Output validation failed.
    #[error("output validation failed: {0}")]
    OutputValidation(ValidationError),

    /// A strategy failed and `fail_fast` is enabled.
    #[error("strategy '{strategy}' failed: {error}")]
    StrategyFailed { strategy: String, error: EvalError },

    /// An internal pipeline error.
    #[error("internal pipeline error: {reason}")]
    Internal { reason: String },
}

/// An evaluation pipeline that orchestrates strategy execution.
///
/// Strategies are sorted by `(phase, priority, id)` for deterministic execution
/// order, then executed sequentially with patches merged between each step.
pub struct EvalPipeline<D: Decision> {
    /// Name of this pipeline.
    pub name: String,
    strategies: Vec<Box<dyn EvalStrategy<D>>>,
    capabilities: CapabilitySet,
    /// Whether to validate the output against constraints after deciding.
    enforce_constraints: bool,
    /// Whether to stop on the first strategy failure.
    fail_fast: bool,
}

impl<D: Decision> std::fmt::Debug for EvalPipeline<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalPipeline")
            .field("name", &self.name)
            .field("strategies", &self.strategies.len())
            .field("enforce_constraints", &self.enforce_constraints)
            .field("fail_fast", &self.fail_fast)
            .finish()
    }
}

impl<D: Decision> EvalPipeline<D> {
    /// Create a new pipeline with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            strategies: Vec::new(),
            capabilities: CapabilitySet::new(),
            enforce_constraints: true,
            fail_fast: true,
        }
    }

    /// Add a strategy to this pipeline.
    pub fn add_strategy(mut self, strategy: Box<dyn EvalStrategy<D>>) -> Self {
        self.strategies.push(strategy);
        self
    }

    /// Set the capability set for this pipeline.
    pub fn with_capabilities(mut self, capabilities: CapabilitySet) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Set whether to enforce output constraints.
    pub fn with_enforce_constraints(mut self, enforce: bool) -> Self {
        self.enforce_constraints = enforce;
        self
    }

    /// Set whether to fail fast on strategy errors.
    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    /// Execute the pipeline against the given context.
    ///
    /// # Execution semantics
    ///
    /// 1. Validate input via `D::validate_input`.
    /// 2. Sort strategies by `(phase, priority, id)`.
    /// 3. Execute each strategy, merging patches between steps.
    /// 4. Process control flow: Continue, Finalize (Decide only), Escalate, Halt.
    /// 5. Validate output if `enforce_constraints` and disposition is `Auto`.
    /// 6. Capture snapshot and return `EvalResult`.
    pub fn execute(&self, ctx: &mut EvalContext<D>) -> Result<EvalResult<D>, PipelineError> {
        let mut trace = TraceHandle::new(ctx.eval_id);

        // Step 1: Validate input
        D::validate_input(&ctx.input).map_err(PipelineError::InputValidation)?;
        trace.record(TraceEvent::InputValidated);

        // Step 2: Sort strategies by (phase, priority, id)
        let mut order: Vec<usize> = (0..self.strategies.len()).collect();
        order.sort_by(|&a, &b| {
            let sa = &self.strategies[a];
            let sb = &self.strategies[b];
            (sa.phase(), sa.priority(), sa.id()).cmp(&(sb.phase(), sb.priority(), sb.id()))
        });

        let mut steps: Vec<PipelineStep> = Vec::new();
        let mut all_findings: Vec<Finding> = Vec::new();
        let mut all_proposals: Vec<Proposal<D>> = Vec::new();
        let mut disposition: Option<DecisionDisposition<D>> = None;
        let mut strategy_versions: Vec<(String, String)> = Vec::new();
        let mut current_phase: Option<Phase> = None;
        let mut halted = false;

        // Step 3-5: Execute strategies in order
        for &idx in &order {
            let strategy = &self.strategies[idx];
            let phase = strategy.phase();
            let phase_str = phase.to_string();
            let strategy_id = strategy.id().to_string();

            // Record phase transitions
            if current_phase != Some(phase) {
                if let Some(prev) = current_phase {
                    trace.record(TraceEvent::PhaseCompleted {
                        phase: prev.to_string(),
                    });
                }
                current_phase = Some(phase);
                trace.record(TraceEvent::PhaseStarted {
                    phase: phase_str.clone(),
                });
            }

            strategy_versions.push((strategy_id.clone(), strategy.version().to_string()));

            trace.record(TraceEvent::StrategyStarted {
                strategy: strategy_id.clone(),
                phase: phase_str.clone(),
            });

            // Execute the strategy
            let patch_result = strategy.evaluate(ctx, &self.capabilities);

            match patch_result {
                Ok(patch) => {
                    trace.record(TraceEvent::StrategyCompleted {
                        strategy: strategy_id.clone(),
                        phase: phase_str.clone(),
                    });

                    // Merge patch into context
                    let artifact_count = patch.artifacts.len();
                    ctx.artifacts.append_all(patch.artifacts);
                    all_findings.extend(patch.findings);
                    all_proposals.extend(patch.proposals);
                    trace.explain_all(patch.explanations);

                    trace.record(TraceEvent::PatchMerged {
                        strategy: strategy_id.clone(),
                        artifacts: artifact_count,
                    });

                    // Process control flow
                    match patch.control {
                        Control::Continue => {
                            steps.push(PipelineStep {
                                strategy: strategy_id,
                                phase: phase_str,
                                outcome: StepOutcome::Continued,
                            });
                        }
                        Control::Finalize(disp) => {
                            // AC-7: Finalize only valid in Phase::Decide
                            if phase != Phase::Decide {
                                trace.record(TraceEvent::FinalizeIgnored {
                                    strategy: strategy_id.clone(),
                                    phase: phase_str.clone(),
                                });
                                trace.explain(ExplanationNode {
                                    source: strategy_id.clone(),
                                    kind: ExplanationKind::InvariantViolation,
                                    detail: format!(
                                        "Control::Finalize emitted in {} phase — downgraded to Continue",
                                        phase_str
                                    ),
                                    artifact_refs: vec![],
                                    policy_refs: vec![],
                                });
                                steps.push(PipelineStep {
                                    strategy: strategy_id,
                                    phase: phase_str,
                                    outcome: StepOutcome::FinalizeIgnored,
                                });
                            } else {
                                trace.record(TraceEvent::DecisionFinalized {
                                    strategy: strategy_id.clone(),
                                });
                                disposition = Some(disp);
                                steps.push(PipelineStep {
                                    strategy: strategy_id,
                                    phase: phase_str,
                                    outcome: StepOutcome::Finalized,
                                });
                                break;
                            }
                        }
                        Control::Escalate(reason) => {
                            trace.explain(ExplanationNode {
                                source: strategy_id.clone(),
                                kind: ExplanationKind::EscalationTriggered,
                                detail: reason.message.clone(),
                                artifact_refs: vec![],
                                policy_refs: vec![],
                            });
                            disposition = Some(DecisionDisposition::Review {
                                provisional: None,
                                reasons: vec![reason],
                            });
                            steps.push(PipelineStep {
                                strategy: strategy_id,
                                phase: phase_str,
                                outcome: StepOutcome::Escalated,
                            });
                            break;
                        }
                        Control::Halt(reason) => {
                            steps.push(PipelineStep {
                                strategy: strategy_id,
                                phase: phase_str,
                                outcome: StepOutcome::Halted {
                                    reason: reason.message.clone(),
                                },
                            });
                            halted = true;
                            break;
                        }
                    }
                }
                Err(e) => {
                    trace.record(TraceEvent::StrategyFailed {
                        strategy: strategy_id.clone(),
                        error: e.to_string(),
                    });
                    steps.push(PipelineStep {
                        strategy: strategy_id.clone(),
                        phase: phase_str,
                        outcome: StepOutcome::Failed {
                            error: e.to_string(),
                        },
                    });

                    if self.fail_fast {
                        return Err(PipelineError::StrategyFailed {
                            strategy: strategy_id,
                            error: e,
                        });
                    }
                }
            }
        }

        // Record final phase completion
        if let Some(phase) = current_phase {
            trace.record(TraceEvent::PhaseCompleted {
                phase: phase.to_string(),
            });
        }

        // Step 6: Validate output if enforcing constraints and disposition is Auto
        if self.enforce_constraints && !halted {
            if let Some(DecisionDisposition::Auto(ref output)) = disposition {
                D::validate_output(output, &ctx.constraints)
                    .map_err(PipelineError::OutputValidation)?;
                trace.record(TraceEvent::OutputValidated);
            }
        }

        trace.record(TraceEvent::PipelineCompleted);

        // Step 7: Capture snapshot
        let snapshot = Snapshot::capture::<D>(
            ctx.eval_id,
            ctx.mode,
            ReplayFidelity::Exact,
            ctx.meta.clone(),
            &ctx.input,
            &ctx.constraints,
            &ctx.policy,
            &ctx.artifacts,
            strategy_versions,
        );

        Ok(EvalResult {
            eval_id: ctx.eval_id,
            pipeline: self.name.clone(),
            disposition,
            proposals: all_proposals,
            findings: all_findings,
            artifacts: ctx.artifacts.clone(),
            steps,
            trace,
            snapshot,
        })
    }
}

/// Pipeline nesting: a sub-pipeline can be used as a strategy (AC-4).
///
/// The sub-pipeline creates a child context with cloned input/constraints/policy,
/// copies the parent's current artifact store, executes its own strategies, and
/// returns a `StrategyPatch` with the child's results merged back to the parent.
impl<D> EvalStrategy<D> for EvalPipeline<D>
where
    D: Decision,
    D::Input: Clone,
    D::Constraints: Clone,
{
    fn id(&self) -> &str {
        &self.name
    }

    fn version(&self) -> crate::strategy::StrategyVersion {
        crate::strategy::StrategyVersion {
            major: 0,
            minor: 1,
            patch: 0,
        }
    }

    fn phase(&self) -> Phase {
        // A sub-pipeline runs in the earliest phase of its strategies
        self.strategies
            .iter()
            .map(|s| s.phase())
            .min()
            .unwrap_or(Phase::Enrich)
    }

    fn evaluate(
        &self,
        ctx: &EvalContext<D>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<D>, EvalError> {
        // Create child context with cloned data and parent's artifact store
        let mut child_ctx = EvalContext::<D> {
            eval_id: Uuid::new_v4(),
            input: ctx.input.clone(),
            constraints: ctx.constraints.clone(),
            policy: ctx.policy.clone(),
            artifacts: ctx.artifacts.clone(),
            memo: crate::context::MemoStore::new(),
            meta: ctx.meta.clone(),
            mode: ctx.mode,
        };

        // Execute the sub-pipeline
        let result = self
            .execute(&mut child_ctx)
            .map_err(|e| EvalError::Internal {
                reason: format!("sub-pipeline '{}' failed: {}", self.name, e),
            })?;

        // Collect new artifacts (those not in the parent store)
        let parent_len = ctx.artifacts.len();
        let child_artifacts: Vec<_> = child_ctx
            .artifacts
            .into_entries()
            .into_iter()
            .skip(parent_len)
            .collect();

        // Build strategy patch from child results
        let control = match result.disposition {
            Some(disp) => Control::Finalize(disp),
            None => Control::Continue,
        };

        Ok(StrategyPatch {
            artifacts: child_artifacts,
            findings: result.findings,
            proposals: result.proposals,
            explanations: result.trace.explanations.into_iter().collect(),
            control,
        })
    }
}
