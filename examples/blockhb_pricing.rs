//! BlockHB material pricing decision example.
//!
//! Demonstrates the full three-phase pipeline:
//! - Enrich: fetch competitor pricing via capability
//! - Evaluate: cost-plus scoring + demand-aware scarcity detection
//! - Decide: policy-driven auto-approval or review escalation

use darq_eval_context::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Decision definition
// ---------------------------------------------------------------------------

/// BlockHB material pricing decision.
struct BlockHBPricing;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricingInput {
    material_id: String,
    base_cost: f64,
    customer_tier: String,
    current_inventory: u64,
    daily_demand: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PricingOutput {
    unit_price: f64,
    margin_pct: f64,
    discount_applied: f64,
    pricing_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricingConstraints {
    min_margin: f64,
    max_price: f64,
}

impl Decision for BlockHBPricing {
    const KIND: &'static str = "blockhb.pricing";
    type Input = PricingInput;
    type Output = PricingOutput;
    type Constraints = PricingConstraints;

    fn validate_input(input: &Self::Input) -> Result<(), ValidationError> {
        if input.base_cost <= 0.0 {
            return Err(ValidationError::InvalidInput {
                reason: "base_cost must be positive".into(),
                field: Some("base_cost".into()),
            });
        }
        if input.material_id.is_empty() {
            return Err(ValidationError::MissingField {
                field: "material_id".into(),
            });
        }
        Ok(())
    }

    fn validate_output(
        output: &Self::Output,
        constraints: &Self::Constraints,
    ) -> Result<(), ValidationError> {
        if output.margin_pct < constraints.min_margin {
            return Err(ValidationError::ConstraintViolation {
                reason: format!(
                    "margin {:.1}% below minimum {:.1}%",
                    output.margin_pct * 100.0,
                    constraints.min_margin * 100.0
                ),
                constraint_id: Some("min_margin".into()),
            });
        }
        if output.unit_price > constraints.max_price {
            return Err(ValidationError::ConstraintViolation {
                reason: format!(
                    "price ${:.2} exceeds maximum ${:.2}",
                    output.unit_price, constraints.max_price
                ),
                constraint_id: Some("max_price".into()),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Enrich phase: fetch competitor pricing from external source.
#[derive(Debug)]
struct CompetitorEnrichment;

impl EvalStrategy<BlockHBPricing> for CompetitorEnrichment {
    fn id(&self) -> &str {
        "competitor_enrichment"
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
        ctx: &EvalContext<BlockHBPricing>,
        capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<BlockHBPricing>, EvalError> {
        let key = format!("competitor.{}", ctx.input.material_id);
        let result =
            capabilities
                .fetch("market_data", &key)
                .map_err(|e| EvalError::CapabilityError {
                    reason: e.to_string(),
                })?;

        let artifact = Artifact::new_external(
            "pricing.competitor.v1",
            result.value,
            "competitor_enrichment",
            "1.0.0",
            "v1",
        );

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "competitor_enrichment".into(),
                kind: ExplanationKind::Enrichment,
                detail: "Fetched competitor pricing from market data service".into(),
                artifact_refs: vec!["pricing.competitor.v1".into()],
                policy_refs: vec![],
            }],
            control: Control::Continue,
        })
    }
}

/// Evaluate phase: cost-plus scoring with tier-based discounts.
#[derive(Debug)]
struct CostPlusScorer;

impl EvalStrategy<BlockHBPricing> for CostPlusScorer {
    fn id(&self) -> &str {
        "cost_plus_scorer"
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
        ctx: &EvalContext<BlockHBPricing>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<BlockHBPricing>, EvalError> {
        let base_margin = ctx.policy.get_f64("pricing.base_margin", 0.25);
        let discount = match ctx.input.customer_tier.as_str() {
            "preferred" => ctx.policy.get_f64("pricing.preferred_discount", 0.05),
            "enterprise" => ctx.policy.get_f64("pricing.enterprise_discount", 0.10),
            _ => 0.0,
        };

        let margin = base_margin - discount;
        let unit_price = ctx.input.base_cost * (1.0 + margin);

        let artifact = Artifact::new(
            "pricing.cost_plus.v1",
            serde_json::json!({
                "base_cost": ctx.input.base_cost,
                "base_margin": base_margin,
                "discount": discount,
                "effective_margin": margin,
                "unit_price": unit_price,
            }),
            "cost_plus_scorer",
            "1.0.0",
            "v1",
        );

        let proposal = Proposal {
            output: PricingOutput {
                unit_price,
                margin_pct: margin,
                discount_applied: discount,
                pricing_tier: ctx.input.customer_tier.clone(),
            },
            proposed_by: "cost_plus_scorer".into(),
            basis: DecisionBasis {
                metrics: vec![
                    BasisMetric {
                        name: "margin_pct".into(),
                        value: margin,
                        unit: Some("%".into()),
                    },
                    BasisMetric {
                        name: "unit_price".into(),
                        value: unit_price,
                        unit: Some("USD".into()),
                    },
                ],
                artifact_refs: vec!["pricing.cost_plus.v1".into()],
                policy_refs: vec![
                    "pricing.base_margin".into(),
                    "pricing.preferred_discount".into(),
                    "pricing.enterprise_discount".into(),
                ],
                rationale: format!(
                    "Cost-plus pricing: ${:.2} base * {:.1}% margin (after {:.1}% {} discount)",
                    ctx.input.base_cost,
                    margin * 100.0,
                    discount * 100.0,
                    ctx.input.customer_tier
                ),
            },
        };

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings: vec![],
            proposals: vec![proposal],
            explanations: vec![ExplanationNode {
                source: "cost_plus_scorer".into(),
                kind: ExplanationKind::RuleMatched,
                detail: format!(
                    "Applied {:.1}% margin with {:.1}% tier discount",
                    base_margin * 100.0,
                    discount * 100.0
                ),
                artifact_refs: vec!["pricing.cost_plus.v1".into()],
                policy_refs: vec!["pricing.base_margin".into()],
            }],
            control: Control::Continue,
        })
    }
}

/// Evaluate phase: demand-aware scarcity detection.
#[derive(Debug)]
struct DemandAwareScorer;

impl EvalStrategy<BlockHBPricing> for DemandAwareScorer {
    fn id(&self) -> &str {
        "demand_aware_scorer"
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
    fn priority(&self) -> i32 {
        1
    }

    fn evaluate(
        &self,
        ctx: &EvalContext<BlockHBPricing>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<BlockHBPricing>, EvalError> {
        let scarcity_days = ctx.policy.get_f64("pricing.scarcity_days", 14.0) as u64;
        let excess_days = ctx.policy.get_f64("pricing.excess_days", 90.0) as u64;

        let days_of_supply = if ctx.input.daily_demand > 0 {
            ctx.input.current_inventory / ctx.input.daily_demand
        } else {
            u64::MAX
        };

        let mut findings = Vec::new();

        if days_of_supply < scarcity_days {
            findings.push(Finding {
                source: "demand_aware_scorer".into(),
                severity: Severity::Warning,
                code: "SCARCITY".into(),
                message: format!(
                    "Only {} days of supply remaining (threshold: {})",
                    days_of_supply, scarcity_days
                ),
                artifact_refs: vec!["pricing.demand.v1".into()],
            });
        } else if days_of_supply > excess_days {
            findings.push(Finding {
                source: "demand_aware_scorer".into(),
                severity: Severity::Info,
                code: "EXCESS".into(),
                message: format!(
                    "{} days of supply — consider promotional pricing",
                    days_of_supply
                ),
                artifact_refs: vec!["pricing.demand.v1".into()],
            });
        }

        let artifact = Artifact::new(
            "pricing.demand.v1",
            serde_json::json!({
                "days_of_supply": days_of_supply,
                "scarcity_threshold": scarcity_days,
                "excess_threshold": excess_days,
                "current_inventory": ctx.input.current_inventory,
                "daily_demand": ctx.input.daily_demand,
            }),
            "demand_aware_scorer",
            "1.0.0",
            "v1",
        );

        Ok(StrategyPatch {
            artifacts: vec![artifact],
            findings,
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "demand_aware_scorer".into(),
                kind: ExplanationKind::Enrichment,
                detail: format!("Computed {} days of supply", days_of_supply),
                artifact_refs: vec!["pricing.demand.v1".into()],
                policy_refs: vec!["pricing.scarcity_days".into(), "pricing.excess_days".into()],
            }],
            control: Control::Continue,
        })
    }
}

/// Decide phase: finalize pricing based on artifacts, findings, and policy.
#[derive(Debug)]
struct PricingDecider;

impl EvalStrategy<BlockHBPricing> for PricingDecider {
    fn id(&self) -> &str {
        "pricing_decider"
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
        ctx: &EvalContext<BlockHBPricing>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<BlockHBPricing>, EvalError> {
        let review_threshold = ctx.policy.get_f64("pricing.review_threshold_margin", 0.10);

        // Read the cost-plus artifact
        let cost_plus = ctx
            .artifacts
            .get_required("pricing.cost_plus.v1")
            .map_err(|e| EvalError::MissingArtifact { key: e.to_string() })?;

        let margin = cost_plus.value["effective_margin"].as_f64().unwrap_or(0.0);
        let unit_price = cost_plus.value["unit_price"].as_f64().unwrap_or(0.0);
        let discount = cost_plus.value["discount"].as_f64().unwrap_or(0.0);

        // Check for scarcity findings
        let has_scarcity = ctx
            .artifacts
            .get_latest("pricing.demand.v1")
            .and_then(|a| a.value["days_of_supply"].as_u64())
            .map(|days| days < ctx.policy.get_f64("pricing.scarcity_days", 14.0) as u64)
            .unwrap_or(false);

        let output = PricingOutput {
            unit_price,
            margin_pct: margin,
            discount_applied: discount,
            pricing_tier: ctx.input.customer_tier.clone(),
        };

        // Decide: auto-approve or review
        let control = if margin < review_threshold || has_scarcity {
            let mut reasons = Vec::new();
            if margin < review_threshold {
                reasons.push(ReviewReason {
                    code: "LOW_MARGIN".into(),
                    message: format!(
                        "Margin {:.1}% below review threshold {:.1}%",
                        margin * 100.0,
                        review_threshold * 100.0
                    ),
                });
            }
            if has_scarcity {
                reasons.push(ReviewReason {
                    code: "SCARCITY".into(),
                    message: "Material is in scarcity — pricing review required".into(),
                });
            }
            Control::Finalize(DecisionDisposition::Review {
                provisional: Some(output),
                reasons,
            })
        } else {
            Control::Finalize(DecisionDisposition::Auto(output))
        };

        Ok(StrategyPatch {
            artifacts: vec![],
            findings: vec![],
            proposals: vec![],
            explanations: vec![ExplanationNode {
                source: "pricing_decider".into(),
                kind: ExplanationKind::RuleMatched,
                detail: format!(
                    "Pricing decision: margin={:.1}%, scarcity={}",
                    margin * 100.0,
                    has_scarcity
                ),
                artifact_refs: vec!["pricing.cost_plus.v1".into(), "pricing.demand.v1".into()],
                policy_refs: vec!["pricing.review_threshold_margin".into()],
            }],
            control,
        })
    }
}

// ---------------------------------------------------------------------------
// Simulated capability
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct MarketDataCapability;

impl Capability for MarketDataCapability {
    fn name(&self) -> &str {
        "market_data"
    }

    fn fetch(&self, key: &str) -> Result<CapabilityResult, CapabilityError> {
        // Simulated competitor pricing
        Ok(CapabilityResult {
            value: serde_json::json!({
                "key": key,
                "competitor_price": 125.50,
                "competitor": "AcmeCorp",
                "fetched_at": chrono::Utc::now().to_rfc3339(),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    println!("=== BlockHB Material Pricing Decision ===\n");

    // Build policy
    let policy = PolicyBundle::new("blockhb-pricing-v1", "1.0.0")
        .with_value("pricing.base_margin", PolicyValue::Float(0.25))
        .with_value("pricing.preferred_discount", PolicyValue::Float(0.05))
        .with_value("pricing.enterprise_discount", PolicyValue::Float(0.10))
        .with_value("pricing.scarcity_days", PolicyValue::Float(14.0))
        .with_value("pricing.excess_days", PolicyValue::Float(90.0))
        .with_value("pricing.review_threshold_margin", PolicyValue::Float(0.10))
        .with_changelog("Initial pricing policy for BlockHB materials");

    // Build capabilities
    let mut capabilities = CapabilitySet::new();
    capabilities.register(Box::new(MarketDataCapability));

    // Build input
    let input = PricingInput {
        material_id: "BHB-2024-STEEL".into(),
        base_cost: 100.0,
        customer_tier: "preferred".into(),
        current_inventory: 500,
        daily_demand: 20,
    };

    let constraints = PricingConstraints {
        min_margin: 0.05,
        max_price: 200.0,
    };

    let meta = RunMeta::new(
        "blockhb.pricing",
        "BHB-2024-STEEL-001",
        "blockhb",
        "api",
        "Customer quote request",
    );

    // Build and run pipeline
    let pipeline = EvalPipeline::<BlockHBPricing>::new("blockhb_pricing_v1")
        .add_strategy(Box::new(CompetitorEnrichment))
        .add_strategy(Box::new(CostPlusScorer))
        .add_strategy(Box::new(DemandAwareScorer))
        .add_strategy(Box::new(PricingDecider))
        .with_capabilities(capabilities);

    let mut ctx =
        EvalContext::<BlockHBPricing>::new(input, constraints, policy, meta, EvalMode::Live);

    let result = pipeline.execute(&mut ctx).expect("pipeline should succeed");

    // Print results
    println!("Disposition:");
    match &result.disposition {
        Some(DecisionDisposition::Auto(output)) => {
            println!("  AUTO-APPROVED");
            println!("  Unit price: ${:.2}", output.unit_price);
            println!("  Margin: {:.1}%", output.margin_pct * 100.0);
            println!("  Discount: {:.1}%", output.discount_applied * 100.0);
            println!("  Tier: {}", output.pricing_tier);
        }
        Some(DecisionDisposition::Review {
            provisional,
            reasons,
        }) => {
            println!("  REVIEW REQUIRED");
            if let Some(output) = provisional {
                println!("  Provisional price: ${:.2}", output.unit_price);
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
        None => println!("  No disposition reached"),
    }

    println!("\nArtifacts ({}):", result.artifacts.len());
    for artifact in result.artifacts.iter() {
        println!(
            "  {} (from {}, {})",
            artifact.key, artifact.source, artifact.replay_class
        );
    }

    println!("\nFindings ({}):", result.findings.len());
    for finding in &result.findings {
        println!(
            "  [{:?}] {} — {}",
            finding.severity, finding.code, finding.message
        );
    }

    println!("\nPipeline Steps:");
    for step in &result.steps {
        println!("  {} ({}) → {:?}", step.strategy, step.phase, step.outcome);
    }

    println!("\nExplanation Tree:");
    for node in &result.trace.explanations {
        println!("  {}", node.render());
    }

    println!("\nSnapshot:");
    println!("  Decision kind: {}", result.snapshot.decision_kind);
    println!("  Fidelity: {:?}", result.snapshot.fidelity);
    println!("  Artifacts captured: {}", result.snapshot.artifacts.len());
    println!(
        "  Strategy versions: {:?}",
        result.snapshot.strategy_versions
    );
}
