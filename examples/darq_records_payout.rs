//! DARQ Records artist payout decision example.
//!
//! Demonstrates all three disposition paths:
//! - Auto: small payout within auto-approve threshold
//! - Review: large payout exceeding auto-approve threshold
//! - Deny: requested amount exceeds earned amount

use darq_eval_context::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Decision definition
// ---------------------------------------------------------------------------

/// DARQ Records artist payout decision.
struct DarqRecordsPayout;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PayoutInput {
    artist_id: String,
    requested_amount: f64,
    total_streams: u64,
    wallet_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PayoutOutput {
    payout_amount: f64,
    artist_id: String,
    wallet_address: String,
    earned_amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PayoutConstraints {
    max_single_payout: f64,
}

impl Decision for DarqRecordsPayout {
    const KIND: &'static str = "darq_records.payout";
    type Input = PayoutInput;
    type Output = PayoutOutput;
    type Constraints = PayoutConstraints;

    fn validate_input(input: &Self::Input) -> Result<(), ValidationError> {
        if input.artist_id.is_empty() {
            return Err(ValidationError::MissingField {
                field: "artist_id".into(),
            });
        }
        if input.requested_amount <= 0.0 {
            return Err(ValidationError::InvalidInput {
                reason: "requested_amount must be positive".into(),
                field: Some("requested_amount".into()),
            });
        }
        if input.wallet_address.is_empty() {
            return Err(ValidationError::MissingField {
                field: "wallet_address".into(),
            });
        }
        Ok(())
    }

    fn validate_output(
        output: &Self::Output,
        constraints: &Self::Constraints,
    ) -> Result<(), ValidationError> {
        if output.payout_amount > constraints.max_single_payout {
            return Err(ValidationError::ConstraintViolation {
                reason: format!(
                    "payout ${:.2} exceeds max single payout ${:.2}",
                    output.payout_amount, constraints.max_single_payout
                ),
                constraint_id: Some("max_single_payout".into()),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Enrich phase: fetch XRPL balance via capability.
#[derive(Debug)]
struct LedgerEnrichment;

impl EvalStrategy<DarqRecordsPayout> for LedgerEnrichment {
    fn id(&self) -> &str {
        "ledger_enrichment"
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
        ctx: &EvalContext<DarqRecordsPayout>,
        capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<DarqRecordsPayout>, EvalError> {
        let result = capabilities
            .fetch("xrpl_balance", &ctx.input.wallet_address)
            .map_err(|e| EvalError::CapabilityError {
                reason: e.to_string(),
            })?;

        let artifact = Artifact::new_external(
            "payout.ledger_balance.v1",
            result.value,
            "ledger_enrichment",
            "1.0.0",
            "v1",
        );

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "ledger_enrichment".into(),
                kind: ExplanationKind::Enrichment,
                detail: "Fetched XRPL wallet balance".into(),
                artifact_refs: vec!["payout.ledger_balance.v1".into()],
                policy_refs: vec![],
            }],
            control: Control::Continue,
        })
    }
}

/// Evaluate phase: check eligibility and compute earned amount.
#[derive(Debug)]
struct EligibilityChecker;

impl EvalStrategy<DarqRecordsPayout> for EligibilityChecker {
    fn id(&self) -> &str {
        "eligibility_checker"
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
        ctx: &EvalContext<DarqRecordsPayout>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<DarqRecordsPayout>, EvalError> {
        let min_streams = ctx.policy.get_i64("payout.min_streams", 100) as u64;
        let min_amount = ctx.policy.get_f64("payout.min_amount", 1.0);
        let per_stream_rate = ctx.policy.get_f64("payout.per_stream_rate", 0.004);

        let earned_amount = ctx.input.total_streams as f64 * per_stream_rate;

        let mut findings = Vec::new();

        if ctx.input.total_streams < min_streams {
            findings.push(Finding {
                source: "eligibility_checker".into(),
                severity: Severity::Warning,
                code: "BELOW_MIN_STREAMS".into(),
                message: format!(
                    "Artist has {} streams, below minimum {} for payout",
                    ctx.input.total_streams, min_streams
                ),
                artifact_refs: vec![],
            });
        }

        if ctx.input.requested_amount < min_amount {
            findings.push(Finding {
                source: "eligibility_checker".into(),
                severity: Severity::Warning,
                code: "BELOW_MIN_AMOUNT".into(),
                message: format!(
                    "Requested ${:.2} below minimum payout ${:.2}",
                    ctx.input.requested_amount, min_amount
                ),
                artifact_refs: vec![],
            });
        }

        if ctx.input.requested_amount > earned_amount {
            findings.push(Finding {
                source: "eligibility_checker".into(),
                severity: Severity::Critical,
                code: "EXCEEDS_EARNED".into(),
                message: format!(
                    "Requested ${:.2} exceeds earned ${:.2}",
                    ctx.input.requested_amount, earned_amount
                ),
                artifact_refs: vec!["payout.eligibility.v1".into()],
            });
        }

        let artifact = Artifact::new(
            "payout.eligibility.v1",
            serde_json::json!({
                "earned_amount": earned_amount,
                "total_streams": ctx.input.total_streams,
                "per_stream_rate": per_stream_rate,
                "requested_amount": ctx.input.requested_amount,
                "eligible": ctx.input.total_streams >= min_streams
                    && ctx.input.requested_amount >= min_amount,
            }),
            "eligibility_checker",
            "1.0.0",
            "v1",
        );

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings,
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "eligibility_checker".into(),
                kind: ExplanationKind::RuleMatched,
                detail: format!(
                    "Earned ${:.2} from {} streams at ${}/stream",
                    earned_amount, ctx.input.total_streams, per_stream_rate
                ),
                artifact_refs: vec!["payout.eligibility.v1".into()],
                policy_refs: vec!["payout.min_streams".into(), "payout.per_stream_rate".into()],
            }],
            control: Control::Continue,
        })
    }
}

/// Decide phase: approve, review, or deny payout.
#[derive(Debug)]
struct PayoutDecider;

impl EvalStrategy<DarqRecordsPayout> for PayoutDecider {
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
        ctx: &EvalContext<DarqRecordsPayout>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<DarqRecordsPayout>, EvalError> {
        let auto_approve_max = ctx.policy.get_f64("payout.auto_approve_max", 50.0);

        let eligibility = ctx
            .artifacts
            .get_required("payout.eligibility.v1")
            .map_err(|e| EvalError::MissingArtifact { key: e.to_string() })?;

        let earned_amount = eligibility.value["earned_amount"].as_f64().unwrap_or(0.0);

        let output = PayoutOutput {
            payout_amount: ctx.input.requested_amount,
            artist_id: ctx.input.artist_id.clone(),
            wallet_address: ctx.input.wallet_address.clone(),
            earned_amount,
        };

        let control = if ctx.input.requested_amount > earned_amount {
            // Deny: exceeds earned
            Control::Finalize(DecisionDisposition::Deny {
                reasons: vec![DenyReason {
                    code: "EXCEEDS_EARNED".into(),
                    message: format!(
                        "Requested ${:.2} exceeds earned ${:.2}",
                        ctx.input.requested_amount, earned_amount
                    ),
                }],
            })
        } else if ctx.input.requested_amount > auto_approve_max {
            // Review: exceeds auto-approve threshold
            Control::Finalize(DecisionDisposition::Review {
                provisional: Some(output),
                reasons: vec![ReviewReason {
                    code: "EXCEEDS_AUTO_APPROVE".into(),
                    message: format!(
                        "Payout ${:.2} exceeds auto-approve threshold ${:.2}",
                        ctx.input.requested_amount, auto_approve_max
                    ),
                }],
            })
        } else {
            // Auto-approve
            Control::Finalize(DecisionDisposition::Auto(output))
        };

        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "payout_decider".into(),
                kind: ExplanationKind::RuleMatched,
                detail: format!(
                    "Payout decision: requested=${:.2}, earned=${:.2}, auto_max=${:.2}",
                    ctx.input.requested_amount, earned_amount, auto_approve_max
                ),
                artifact_refs: vec!["payout.eligibility.v1".into()],
                policy_refs: vec!["payout.auto_approve_max".into()],
            }],
            control,
        })
    }
}

// ---------------------------------------------------------------------------
// Simulated capability
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct XrplBalanceCapability {
    balance: f64,
}

impl Capability for XrplBalanceCapability {
    fn name(&self) -> &str {
        "xrpl_balance"
    }

    fn fetch(&self, key: &str) -> Result<CapabilityResult, CapabilityError> {
        Ok(CapabilityResult {
            value: serde_json::json!({
                "address": key,
                "balance_xrp": self.balance,
                "fetched_at": chrono::Utc::now().to_rfc3339(),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_policy() -> PolicyBundle {
    PolicyBundle::new("darq-records-payout-v1", "1.0.0")
        .with_value("payout.min_streams", PolicyValue::Int(100))
        .with_value("payout.min_amount", PolicyValue::Float(1.0))
        .with_value("payout.per_stream_rate", PolicyValue::Float(0.004))
        .with_value("payout.auto_approve_max", PolicyValue::Float(50.0))
}

fn build_pipeline(balance: f64) -> EvalPipeline<DarqRecordsPayout> {
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(XrplBalanceCapability { balance }));

    EvalPipeline::<DarqRecordsPayout>::new("darq_records_payout_v1")
        .add_strategy(Box::new(LedgerEnrichment))
        .add_strategy(Box::new(EligibilityChecker))
        .add_strategy(Box::new(PayoutDecider))
        .with_capabilities(capabilities)
}

fn run_scenario(name: &str, input: PayoutInput, balance: f64) {
    println!("--- {} ---\n", name);

    let policy = build_policy();
    let constraints = PayoutConstraints {
        max_single_payout: 1000.0,
    };

    let meta = RunMeta::new(
        "darq_records.payout",
        format!("payout-{}", input.artist_id),
        "darq_records",
        "api",
        "Artist payout request",
    );

    let pipeline = build_pipeline(balance);
    let mut ctx =
        EvalContext::<DarqRecordsPayout>::new(input, constraints, policy, meta, EvalMode::Live);

    let result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    match &result.disposition {
        Some(DecisionDisposition::Auto(output)) => {
            println!(
                "  AUTO-APPROVED: ${:.2} to {}",
                output.payout_amount, output.wallet_address
            );
            println!("  Earned: ${:.2}", output.earned_amount);
        }
        Some(DecisionDisposition::Review {
            provisional,
            reasons,
        }) => {
            println!("  REVIEW REQUIRED");
            if let Some(output) = provisional {
                println!("  Provisional: ${:.2}", output.payout_amount);
            }
            for reason in reasons {
                println!("  Reason: [{}] {}", reason.code, reason.message);
            }
        }
        Some(DecisionDisposition::Deny { reasons }) => {
            println!("  DENIED");
            for reason in reasons {
                println!("  Reason: [{}] {}", reason.code, reason.message);
            }
        }
        None => println!("  No disposition"),
    }

    println!("  Findings: {}", result.findings.len());
    for f in &result.findings {
        println!("    [{:?}] {} — {}", f.severity, f.code, f.message);
    }
    println!("  Artifacts: {}", result.artifacts.len());
    println!("  Steps: {}", result.steps.len());
    println!();
}

fn main() {
    println!("=== DARQ Records Payout Decision ===\n");

    // Scenario A: Small auto-approved payout
    run_scenario(
        "Scenario A: Small Auto-Approved Payout",
        PayoutInput {
            artist_id: "artist_001".into(),
            requested_amount: 25.0,
            total_streams: 10000,
            wallet_address: "rDARQ1337xrpl...".into(),
        },
        500.0,
    );

    // Scenario B: Large payout requiring review
    run_scenario(
        "Scenario B: Large Payout — Review Required",
        PayoutInput {
            artist_id: "artist_002".into(),
            requested_amount: 100.0,
            total_streams: 50000,
            wallet_address: "rDARQ4242xrpl...".into(),
        },
        500.0,
    );

    // Scenario C: Exceeds earned — denied
    run_scenario(
        "Scenario C: Exceeds Earned — Denied",
        PayoutInput {
            artist_id: "artist_003".into(),
            requested_amount: 500.0,
            total_streams: 1000,
            wallet_address: "rDARQ9999xrpl...".into(),
        },
        500.0,
    );
}
