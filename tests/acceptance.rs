//! Integration tests for all 10 acceptance criteria.

use darq_eval_context::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ===========================================================================
// Test Decision types
// ===========================================================================

#[derive(Debug)]
struct TestDecision;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TestInput {
    value: f64,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TestOutput {
    result: f64,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestConstraints {
    max_result: f64,
}

impl Decision for TestDecision {
    const KIND: &'static str = "test.decision";
    type Input = TestInput;
    type Output = TestOutput;
    type Constraints = TestConstraints;

    fn validate_input(input: &Self::Input) -> Result<(), ValidationError> {
        if input.value < 0.0 {
            return Err(ValidationError::InvalidInput {
                reason: "value must be non-negative".into(),
                field: Some("value".into()),
            });
        }
        Ok(())
    }

    fn validate_output(
        output: &Self::Output,
        constraints: &Self::Constraints,
    ) -> Result<(), ValidationError> {
        if output.result > constraints.max_result {
            return Err(ValidationError::ConstraintViolation {
                reason: format!(
                    "result {} exceeds max {}",
                    output.result, constraints.max_result
                ),
                constraint_id: Some("max_result".into()),
            });
        }
        Ok(())
    }
}

// ===========================================================================
// Helper: build context
// ===========================================================================

fn test_policy() -> PolicyBundle {
    PolicyBundle::new("test-policy", "1.0.0")
        .with_value("multiplier", PolicyValue::Float(2.0))
        .with_value("threshold", PolicyValue::Float(100.0))
}

fn test_meta() -> RunMeta {
    RunMeta::new("test.decision", "case-1", "test-tenant", "test", "test run")
}

fn test_ctx(value: f64) -> EvalContext<TestDecision> {
    EvalContext::<TestDecision>::new(
        TestInput {
            value,
            label: "test".into(),
        },
        TestConstraints { max_result: 1000.0 },
        test_policy(),
        test_meta(),
        EvalMode::Live,
    )
}

// ===========================================================================
// Test strategies
// ===========================================================================

/// Enrich strategy that uses a capability.
#[derive(Debug)]
struct TestEnricher {
    id: String,
}

impl EvalStrategy<TestDecision> for TestEnricher {
    fn id(&self) -> &str {
        &self.id
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Enrich
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<TestDecision>,
        capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        let result = capabilities.fetch("test_data", &ctx.input.label);
        let artifact = match result {
            Ok(cap_result) => Artifact::new_external(
                format!("test.enriched.{}", self.id),
                cap_result.value,
                &self.id,
                "1.0.0",
                "v1",
            ),
            Err(_) => Artifact::new_external(
                format!("test.enriched.{}", self.id),
                serde_json::json!({"fallback": true}),
                &self.id,
                "1.0.0",
                "v1",
            ),
        };

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: self.id.clone(),
                kind: ExplanationKind::Enrichment,
                detail: "Test enrichment".into(),
                artifact_refs: vec![format!("test.enriched.{}", self.id)],
                policy_refs: vec![],
            }],
            control: Control::Continue,
        })
    }
}

/// Evaluate strategy that emits a finding.
#[derive(Debug)]
struct TestEvaluator;

impl EvalStrategy<TestDecision> for TestEvaluator {
    fn id(&self) -> &str {
        "test_evaluator"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Evaluate
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        let multiplier = ctx.policy.get_f64("multiplier", 1.0);
        let result = ctx.input.value * multiplier;

        let artifact = Artifact::new(
            "test.computed.v1",
            serde_json::json!({"result": result}),
            "test_evaluator",
            "1.0.0",
            "v1",
        );

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Continue,
        })
    }
}

/// Decider that auto-approves.
#[derive(Debug)]
struct TestAutoDecider;

impl EvalStrategy<TestDecision> for TestAutoDecider {
    fn id(&self) -> &str {
        "test_auto_decider"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Decide
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        let computed = ctx
            .artifacts
            .get_required("test.computed.v1")
            .map_err(|e| EvalError::MissingArtifact { key: e.to_string() })?;

        let result = computed.value["result"].as_f64().unwrap_or(0.0);

        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Finalize(DecisionDisposition::Auto(TestOutput {
                result,
                label: ctx.input.label.clone(),
            })),
        })
    }
}

/// Strategy that tries to finalize in Evaluate phase (for AC-7 test).
#[derive(Debug)]
struct EarlyFinalizer;

impl EvalStrategy<TestDecision> for EarlyFinalizer {
    fn id(&self) -> &str {
        "early_finalizer"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Evaluate
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Finalize(DecisionDisposition::Auto(TestOutput {
                result: 999.0,
                label: ctx.input.label.clone(),
            })),
        })
    }
}

/// Strategy that always fails.
#[derive(Debug)]
struct FailingStrategy;

impl EvalStrategy<TestDecision> for FailingStrategy {
    fn id(&self) -> &str {
        "failing_strategy"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Evaluate
    }
    fn evaluate(
        &self,
        _ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        Err(EvalError::Internal {
            reason: "intentional test failure".into(),
        })
    }
}

/// Decider that denies.
#[derive(Debug)]
struct TestDenyDecider;

impl EvalStrategy<TestDecision> for TestDenyDecider {
    fn id(&self) -> &str {
        "test_deny_decider"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Decide
    }
    fn evaluate(
        &self,
        _ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Finalize(DecisionDisposition::Deny {
                reasons: vec![DenyReason {
                    code: "DENIED".into(),
                    message: "Test denial".into(),
                }],
            }),
        })
    }
}

/// Decider that escalates to review.
#[derive(Debug)]
struct TestReviewDecider;

impl EvalStrategy<TestDecision> for TestReviewDecider {
    fn id(&self) -> &str {
        "test_review_decider"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Decide
    }
    fn evaluate(
        &self,
        _ctx: &EvalContext<TestDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<TestDecision>, EvalError> {
        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Finalize(DecisionDisposition::Review {
                provisional: None,
                reasons: vec![ReviewReason {
                    code: "REVIEW".into(),
                    message: "Test review".into(),
                }],
            }),
        })
    }
}

/// Test capability.
#[derive(Debug)]
struct TestDataCapability {
    data: serde_json::Value,
}

impl Capability for TestDataCapability {
    fn name(&self) -> &str {
        "test_data"
    }
    fn fetch(&self, _key: &str) -> Result<CapabilityResult, CapabilityError> {
        Ok(CapabilityResult {
            value: self.data.clone(),
        })
    }
}

// ===========================================================================
// AC-1: Canonical artifact digests
// ===========================================================================

#[test]
fn ac1_canonical_digest_is_insertion_order_independent() {
    let v1 = serde_json::json!({"b": 2, "a": 1, "c": {"z": 26, "y": 25}});
    let v2 = serde_json::json!({"a": 1, "c": {"y": 25, "z": 26}, "b": 2});
    let a1 = Artifact::new("test.key", v1, "test", "1.0", "v1");
    let a2 = Artifact::new("test.key", v2, "test", "1.0", "v1");
    assert_eq!(a1.digest, a2.digest);
}

#[test]
fn ac1_different_values_produce_different_digests() {
    let v1 = serde_json::json!({"a": 1});
    let v2 = serde_json::json!({"a": 2});
    let a1 = Artifact::new("test.key", v1, "test", "1.0", "v1");
    let a2 = Artifact::new("test.key", v2, "test", "1.0", "v1");
    assert_ne!(a1.digest, a2.digest);
}

#[test]
fn ac1_nested_objects_are_canonicalized() {
    let v1 = serde_json::json!({"outer": {"b": 2, "a": 1}, "items": [{"z": 1, "a": 2}]});
    let v2 = serde_json::json!({"outer": {"a": 1, "b": 2}, "items": [{"a": 2, "z": 1}]});
    let a1 = Artifact::new("test.key", v1, "test", "1.0", "v1");
    let a2 = Artifact::new("test.key", v2, "test", "1.0", "v1");
    assert_eq!(a1.digest, a2.digest);
}

// ===========================================================================
// AC-2: Logical sequence numbers
// ===========================================================================

#[test]
fn ac2_trace_events_have_monotonic_sequence_numbers() {
    let mut trace = TraceHandle::new(Uuid::new_v4());
    trace.record(TraceEvent::InputValidated);
    trace.record(TraceEvent::InputValidated);
    trace.record(TraceEvent::InputValidated);
    assert_eq!(trace.events[0].seq, 0);
    assert_eq!(trace.events[1].seq, 1);
    assert_eq!(trace.events[2].seq, 2);
}

#[test]
fn ac2_sequence_numbers_never_repeat() {
    let mut trace = TraceHandle::new(Uuid::new_v4());
    for _ in 0..100 {
        trace.record(TraceEvent::InputValidated);
    }
    let seqs: Vec<u64> = trace.events.iter().map(|e| e.seq).collect();
    for i in 1..seqs.len() {
        assert!(
            seqs[i] > seqs[i - 1],
            "seq {} should be > seq {}",
            seqs[i],
            seqs[i - 1]
        );
    }
}

// ===========================================================================
// AC-3: Deterministic artifact read helpers
// ===========================================================================

#[test]
fn ac3_artifact_store_deterministic_reads() {
    let mut store = ArtifactStore::new();
    store.append(Artifact::new(
        "pricing.basis.v1",
        serde_json::json!({"price": 5.0}),
        "strategy_a",
        "1.0",
        "v1",
    ));
    store.append(Artifact::new(
        "pricing.basis.v1",
        serde_json::json!({"price": 6.0}),
        "strategy_b",
        "1.0",
        "v1",
    ));

    // get_all returns both in emission order
    let all = store.get_all("pricing.basis.v1");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].source, "strategy_a");
    assert_eq!(all[1].source, "strategy_b");

    // get_latest returns second
    let latest = store.get_latest("pricing.basis.v1").unwrap();
    assert_eq!(latest.source, "strategy_b");

    // get_required returns latest
    let required = store.get_required("pricing.basis.v1").unwrap();
    assert_eq!(required.source, "strategy_b");

    // get_required on missing key returns error
    assert!(store.get_required("nonexistent").is_err());

    // get_latest_by_source distinguishes
    let from_a = store
        .get_latest_by_source("pricing.basis.v1", "strategy_a")
        .unwrap();
    assert_eq!(from_a.value["price"], 5.0);
}

#[test]
fn ac3_from_source_and_with_prefix() {
    let mut store = ArtifactStore::new();
    store.append(Artifact::new(
        "pricing.a",
        serde_json::json!(1),
        "src1",
        "1.0",
        "v1",
    ));
    store.append(Artifact::new(
        "pricing.b",
        serde_json::json!(2),
        "src1",
        "1.0",
        "v1",
    ));
    store.append(Artifact::new(
        "other.c",
        serde_json::json!(3),
        "src2",
        "1.0",
        "v1",
    ));

    assert_eq!(store.from_source("src1").len(), 2);
    assert_eq!(store.from_source("src2").len(), 1);
    assert_eq!(store.with_prefix("pricing.").len(), 2);
    assert_eq!(store.with_prefix("other.").len(), 1);
}

#[test]
fn ac3_external_artifacts_filter() {
    let mut store = ArtifactStore::new();
    store.append(Artifact::new("a", serde_json::json!(1), "s", "1.0", "v1")); // deterministic
    store.append(Artifact::new_external(
        "b",
        serde_json::json!(2),
        "s",
        "1.0",
        "v1",
    ));
    store.append(Artifact::new_derived(
        "c",
        serde_json::json!(3),
        "s",
        "1.0",
        "v1",
    ));

    let external = store.external_artifacts();
    assert_eq!(external.len(), 1);
    assert_eq!(external[0].key, "b");
}

// ===========================================================================
// AC-4: Pipeline nesting
// ===========================================================================

#[test]
fn ac4_pipeline_nesting() {
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(TestDataCapability {
        data: serde_json::json!({"external": true}),
    }));

    // Sub-pipeline with two enrichers
    let sub_pipeline = EvalPipeline::<TestDecision>::new("sub_enrichment")
        .add_strategy(Box::new(TestEnricher {
            id: "enricher_a".into(),
        }))
        .add_strategy(Box::new(TestEnricher {
            id: "enricher_b".into(),
        }));

    // Parent pipeline: sub-pipeline (enrichers) + evaluator + decider
    let parent = EvalPipeline::<TestDecision>::new("parent_pipeline")
        .add_strategy(Box::new(sub_pipeline))
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider))
        .with_capabilities(capabilities);

    let mut ctx = test_ctx(10.0);
    let result = parent.execute(&mut ctx).expect("pipeline should succeed");

    // Verify enrichment artifacts from sub-pipeline are present
    assert!(result
        .artifacts
        .get_latest("test.enriched.enricher_a")
        .is_some());
    assert!(result
        .artifacts
        .get_latest("test.enriched.enricher_b")
        .is_some());

    // Verify the evaluator and decider ran
    assert!(result.artifacts.get_latest("test.computed.v1").is_some());
    assert!(result.disposition.is_some());

    // Verify trace shows multiple strategies executed
    let strategy_started_count = result
        .trace
        .events
        .iter()
        .filter(|e| matches!(e.event, TraceEvent::StrategyStarted { .. }))
        .count();
    assert!(strategy_started_count >= 3); // sub-pipeline + evaluator + decider
}

// ===========================================================================
// AC-5: Snapshot replay fidelity
// ===========================================================================

#[test]
fn ac5_exact_snapshot_captures_all_artifacts() {
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(TestDataCapability {
        data: serde_json::json!({"external": true}),
    }));

    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(TestEnricher {
            id: "enricher".into(),
        }))
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider))
        .with_capabilities(capabilities);

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    // Exact snapshot should have all artifacts
    let exact_snapshot = Snapshot::capture::<TestDecision>(
        result.eval_id,
        EvalMode::Live,
        ReplayFidelity::Exact,
        result.snapshot.meta.clone(),
        &ctx.input,
        &ctx.constraints,
        &ctx.policy,
        &result.artifacts,
        vec![],
    );

    // Should include both external and deterministic artifacts
    assert!(exact_snapshot.artifacts.len() >= 2);
    let has_deterministic = exact_snapshot
        .artifacts
        .iter()
        .any(|a| a.replay_class == ReplayClass::Deterministic);
    assert!(
        has_deterministic,
        "Exact snapshot should include deterministic artifacts"
    );
}

#[test]
fn ac5_semantic_snapshot_captures_only_external() {
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(TestDataCapability {
        data: serde_json::json!({"external": true}),
    }));

    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(TestEnricher {
            id: "enricher".into(),
        }))
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider))
        .with_capabilities(capabilities);

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    // Semantic snapshot should only have external artifacts
    let semantic_snapshot = Snapshot::capture::<TestDecision>(
        result.eval_id,
        EvalMode::Live,
        ReplayFidelity::Semantic,
        result.snapshot.meta.clone(),
        &ctx.input,
        &ctx.constraints,
        &ctx.policy,
        &result.artifacts,
        vec![],
    );

    // All captured artifacts should be External
    for artifact in &semantic_snapshot.artifacts {
        assert_eq!(
            artifact.replay_class,
            ReplayClass::External,
            "Semantic snapshot should only contain external artifacts, found {:?} for key {}",
            artifact.replay_class,
            artifact.key
        );
    }
    // There should be at least one external artifact
    assert!(
        !semantic_snapshot.artifacts.is_empty(),
        "Semantic snapshot should have at least one external artifact"
    );
}

// ===========================================================================
// AC-6: Memo non-semantic enforcement
// ===========================================================================

#[test]
fn ac6_strategies_cannot_mutate_context() {
    // This test verifies at the type level that EvalStrategy::evaluate
    // receives &EvalContext<D> (not &mut). This is enforced by the trait
    // definition itself — if it compiled, the invariant holds.
    //
    // We verify by running a pipeline and confirming that the context's
    // memo and artifacts are only modified by the pipeline, not strategies.

    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider));

    let mut ctx = test_ctx(10.0);
    // Set a memo value that strategies shouldn't be able to overwrite
    ctx.memo.set("test_key", "original_value".to_string());

    let _result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    // The memo value should still be "original_value" since strategies
    // can't mutate the context (they receive &EvalContext, not &mut)
    assert_eq!(
        ctx.memo.get::<String>("test_key"),
        Some(&"original_value".to_string()),
    );
}

// ===========================================================================
// AC-7: Non-Decide finalization enforcement
// ===========================================================================

#[test]
fn ac7_finalize_in_evaluate_phase_is_ignored() {
    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(EarlyFinalizer)) // Evaluate phase, tries to finalize
        .add_strategy(Box::new(TestAutoDecider)); // Decide phase, actual finalize

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    // EarlyFinalizer's finalization should be ignored
    let early_step = result
        .steps
        .iter()
        .find(|s| s.strategy == "early_finalizer")
        .expect("early_finalizer step should exist");
    assert!(
        matches!(early_step.outcome, StepOutcome::FinalizeIgnored),
        "EarlyFinalizer step should be FinalizeIgnored, got {:?}",
        early_step.outcome
    );

    // Trace should have InvariantViolation explanation
    let has_invariant_violation = result
        .trace
        .explanations
        .iter()
        .any(|e| matches!(e.kind, ExplanationKind::InvariantViolation));
    assert!(
        has_invariant_violation,
        "Trace should have InvariantViolation explanation"
    );

    // Trace should have FinalizeIgnored event
    let has_finalize_ignored = result
        .trace
        .events
        .iter()
        .any(|e| matches!(e.event, TraceEvent::FinalizeIgnored { .. }));
    assert!(
        has_finalize_ignored,
        "Trace should have FinalizeIgnored event"
    );

    // The actual disposition should come from TestAutoDecider, not EarlyFinalizer
    match &result.disposition {
        Some(DecisionDisposition::Auto(output)) => {
            // TestAutoDecider uses computed result (10.0 * 2.0 = 20.0), not 999.0
            assert!(
                (output.result - 20.0).abs() < f64::EPSILON,
                "Disposition should come from TestAutoDecider (result=20.0), got {}",
                output.result
            );
        }
        other => panic!("Expected Auto disposition, got {:?}", other),
    }
}

// ===========================================================================
// AC-8: Business outcomes vs engine failures
// ===========================================================================

#[test]
fn ac8_deny_is_business_outcome_not_error() {
    let pipeline =
        EvalPipeline::<TestDecision>::new("test_pipeline").add_strategy(Box::new(TestDenyDecider));

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx);

    // Deny is a business outcome → Ok(result) with Deny disposition
    assert!(result.is_ok(), "Deny should be Ok, not Err");
    let result = result.unwrap();
    assert!(
        matches!(result.disposition, Some(DecisionDisposition::Deny { .. })),
        "Disposition should be Deny"
    );
}

#[test]
fn ac8_review_is_business_outcome_not_error() {
    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(TestReviewDecider));

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx);

    // Review is a business outcome → Ok(result)
    assert!(result.is_ok(), "Review should be Ok, not Err");
    let result = result.unwrap();
    assert!(
        matches!(result.disposition, Some(DecisionDisposition::Review { .. })),
        "Disposition should be Review"
    );
}

#[test]
fn ac8_strategy_crash_is_engine_error() {
    let pipeline = EvalPipeline::<TestDecision>::new("test_pipeline")
        .add_strategy(Box::new(FailingStrategy))
        .with_fail_fast(true);

    let mut ctx = test_ctx(10.0);
    let result = pipeline.execute(&mut ctx);

    // Strategy failure with fail_fast → Err(PipelineError)
    assert!(result.is_err(), "Strategy failure should be Err");
    let err = result.unwrap_err();
    assert!(
        matches!(err, PipelineError::StrategyFailed { .. }),
        "Error should be StrategyFailed, got {:?}",
        err
    );
}

// ===========================================================================
// AC-9: Deterministic replay test
// ===========================================================================

#[test]
fn ac9_pricing_replay_produces_identical_result() {
    // --- Run 1: Live execution ---
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(TestDataCapability {
        data: serde_json::json!({"external_value": 42}),
    }));

    let pipeline = EvalPipeline::<TestDecision>::new("replay_test")
        .add_strategy(Box::new(TestEnricher {
            id: "enricher".into(),
        }))
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider))
        .with_capabilities(capabilities);

    let input = TestInput {
        value: 10.0,
        label: "replay_test".into(),
    };
    let constraints = TestConstraints { max_result: 1000.0 };
    let mut ctx = EvalContext::<TestDecision>::new(
        input.clone(),
        constraints.clone(),
        test_policy(),
        test_meta(),
        EvalMode::Live,
    );

    let result1 = pipeline.execute(&mut ctx).expect("run 1 should succeed");
    let snapshot = &result1.snapshot;

    // Extract the live result disposition
    let live_output = match &result1.disposition {
        Some(DecisionDisposition::Auto(output)) => output.clone(),
        other => panic!("Expected Auto disposition in run 1, got {:?}", other),
    };

    // --- Run 2: Replay from snapshot ---
    // Build RecordedCapability from snapshot's external artifacts
    let recorded_caps = RecordedCapability::from_artifacts(&snapshot.artifacts);
    let mut replay_capabilities = CapabilitySet::new();
    for cap in recorded_caps {
        replay_capabilities.register(Box::new(cap));
    }

    let replay_pipeline = EvalPipeline::<TestDecision>::new("replay_test")
        .add_strategy(Box::new(TestEnricher {
            id: "enricher".into(),
        }))
        .add_strategy(Box::new(TestEvaluator))
        .add_strategy(Box::new(TestAutoDecider))
        .with_capabilities(replay_capabilities);

    // Rebuild context from snapshot
    let replay_input: TestInput =
        serde_json::from_value(snapshot.input_json.clone()).expect("deserialize input");
    let replay_constraints: TestConstraints =
        serde_json::from_value(snapshot.constraints_json.clone()).expect("deserialize constraints");

    let mut replay_ctx = EvalContext::<TestDecision>::new(
        replay_input,
        replay_constraints,
        snapshot.policy_bundle.clone(),
        test_meta(),
        EvalMode::Replay,
    );

    let result2 = replay_pipeline
        .execute(&mut replay_ctx)
        .expect("replay should succeed");

    // Verify disposition matches
    let replay_output = match &result2.disposition {
        Some(DecisionDisposition::Auto(output)) => output.clone(),
        other => panic!("Expected Auto disposition in replay, got {:?}", other),
    };

    assert_eq!(
        live_output.result, replay_output.result,
        "Result should match"
    );
    assert_eq!(live_output.label, replay_output.label, "Label should match");
}

// ===========================================================================
// AC-10: All three payout dispositions (using test decision types)
// ===========================================================================

// These use the DARQ Records payout types from the example, replicated
// here as a minimal test harness.

#[derive(Debug)]
struct PayoutDecision;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PayInput {
    artist_id: String,
    requested: f64,
    streams: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PayOutput {
    amount: f64,
    earned: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PayConstraints {
    max_payout: f64,
}

impl Decision for PayoutDecision {
    const KIND: &'static str = "test.payout";
    type Input = PayInput;
    type Output = PayOutput;
    type Constraints = PayConstraints;

    fn validate_input(input: &Self::Input) -> Result<(), ValidationError> {
        if input.requested <= 0.0 {
            return Err(ValidationError::InvalidInput {
                reason: "requested must be positive".into(),
                field: Some("requested".into()),
            });
        }
        Ok(())
    }

    fn validate_output(
        output: &Self::Output,
        constraints: &Self::Constraints,
    ) -> Result<(), ValidationError> {
        if output.amount > constraints.max_payout {
            return Err(ValidationError::ConstraintViolation {
                reason: "exceeds max".into(),
                constraint_id: Some("max_payout".into()),
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct PayoutEvaluator;

impl EvalStrategy<PayoutDecision> for PayoutEvaluator {
    fn id(&self) -> &str {
        "payout_evaluator"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Evaluate
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<PayoutDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<PayoutDecision>, EvalError> {
        let rate = ctx.policy.get_f64("per_stream_rate", 0.004);
        let earned = ctx.input.streams as f64 * rate;
        let artifact = Artifact::new(
            "payout.earned",
            serde_json::json!({"earned": earned}),
            "payout_evaluator",
            "1.0.0",
            "v1",
        );
        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control: Control::Continue,
        })
    }
}

#[derive(Debug)]
struct PayoutDeciderStrategy;

impl EvalStrategy<PayoutDecision> for PayoutDeciderStrategy {
    fn id(&self) -> &str {
        "payout_decider"
    }
    fn version(&self) -> StrategyVersion {
        StrategyVersion {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }
    fn phase(&self) -> Phase {
        Phase::Decide
    }
    fn evaluate(
        &self,
        ctx: &EvalContext<PayoutDecision>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<PayoutDecision>, EvalError> {
        let auto_max = ctx.policy.get_f64("auto_approve_max", 50.0);
        let earned_art = ctx
            .artifacts
            .get_required("payout.earned")
            .map_err(|e| EvalError::MissingArtifact { key: e.to_string() })?;
        let earned = earned_art.value["earned"].as_f64().unwrap_or(0.0);

        let output = PayOutput {
            amount: ctx.input.requested,
            earned,
        };

        let control = if ctx.input.requested > earned {
            Control::Finalize(DecisionDisposition::Deny {
                reasons: vec![DenyReason {
                    code: "EXCEEDS_EARNED".into(),
                    message: format!("${:.2} > ${:.2}", ctx.input.requested, earned),
                }],
            })
        } else if ctx.input.requested > auto_max {
            Control::Finalize(DecisionDisposition::Review {
                provisional: Some(output),
                reasons: vec![ReviewReason {
                    code: "LARGE_PAYOUT".into(),
                    message: format!(
                        "${:.2} > auto threshold ${:.2}",
                        ctx.input.requested, auto_max
                    ),
                }],
            })
        } else {
            Control::Finalize(DecisionDisposition::Auto(output))
        };

        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![],
            control,
        })
    }
}

fn payout_policy() -> PolicyBundle {
    PolicyBundle::new("payout-test", "1.0.0")
        .with_value("per_stream_rate", PolicyValue::Float(0.004))
        .with_value("auto_approve_max", PolicyValue::Float(50.0))
}

fn run_payout(requested: f64, streams: u64) -> EvalResult<PayoutDecision> {
    let pipeline = EvalPipeline::<PayoutDecision>::new("payout_test")
        .add_strategy(Box::new(PayoutEvaluator))
        .add_strategy(Box::new(PayoutDeciderStrategy));

    let mut ctx = EvalContext::<PayoutDecision>::new(
        PayInput {
            artist_id: "test_artist".into(),
            requested,
            streams,
        },
        PayConstraints { max_payout: 1000.0 },
        payout_policy(),
        RunMeta::new("test.payout", "case-1", "test", "test", "test"),
        EvalMode::Live,
    );

    pipeline.execute(&mut ctx).expect("pipeline should succeed")
}

#[test]
fn ac10_payout_auto_approve() {
    // 10000 streams * 0.004 = $40 earned, requesting $25 < $50 threshold
    let result = run_payout(25.0, 10000);
    assert!(
        matches!(result.disposition, Some(DecisionDisposition::Auto(_))),
        "Small payout should be auto-approved"
    );
}

#[test]
fn ac10_payout_review() {
    // 50000 streams * 0.004 = $200 earned, requesting $100 > $50 threshold
    let result = run_payout(100.0, 50000);
    assert!(
        matches!(result.disposition, Some(DecisionDisposition::Review { .. })),
        "Large payout should require review"
    );
}

#[test]
fn ac10_payout_deny() {
    // 1000 streams * 0.004 = $4 earned, requesting $500 > earned
    let result = run_payout(500.0, 1000);
    assert!(
        matches!(result.disposition, Some(DecisionDisposition::Deny { .. })),
        "Payout exceeding earned should be denied"
    );
}
