# darq-eval-context

**Reusable evaluation context library for DARQ Labs business decisions.**

Versioned evidence in, append-only patches through, policy-driven disposition out.

[![CI](https://github.com/Quigles1337/DARQ-Eval/actions/workflows/ci.yml/badge.svg)](https://github.com/Quigles1337/DARQ-Eval/actions/workflows/ci.yml)

---

## What is this?

`darq-eval-context` is a Rust library that provides a structured framework for making auditable, replayable business decisions. Instead of scattering decision logic across ad-hoc code, you define typed decisions, compose evaluation strategies into pipelines, and get back a full audit trail with every run.

The library was designed for DARQ Labs use cases — material pricing, artist payouts, risk assessment, resource allocation — but the architecture is domain-agnostic. If you have a business decision that needs to be:

- **Typed** — validated inputs and constrained outputs
- **Auditable** — every step traced with explanations
- **Replayable** — deterministic reproduction from snapshots
- **Policy-driven** — business rules separated from evaluation logic

...this crate gives you the scaffolding.

## Architecture

```
                    ┌─────────────────────────────────────────────┐
                    │              EvalPipeline<D>                │
                    │                                             │
  Input ──────────►│  Enrich ──► Evaluate ──► Decide             │
  Constraints       │    │           │            │               │
  PolicyBundle      │    ▼           ▼            ▼               │
  Capabilities      │  patches     patches     disposition       │
                    │    │           │            │               │
                    │    └─────┬─────┘            │               │
                    │          ▼                   ▼               │
                    │    ArtifactStore      DecisionDisposition   │
                    │    (append-only)      Auto / Review / Deny  │
                    │                                             │
                    └──────────────────────────────┬──────────────┘
                                                   │
                                                   ▼
                                             EvalResult<D>
                                          ├── disposition
                                          ├── artifacts
                                          ├── findings
                                          ├── trace (seq-numbered)
                                          └── snapshot (for replay)
```

### Core flow

1. You define a **Decision** — typed `Input`, `Output`, and `Constraints` with validation
2. You write **Strategies** that run in one of three phases:
   - **Enrich** — fetch external data, populate the artifact store
   - **Evaluate** — score, rank, detect anomalies, emit findings
   - **Decide** — read everything and finalize as `Auto`, `Review`, or `Deny`
3. You assemble strategies into an **EvalPipeline** and execute it
4. The pipeline returns an **EvalResult** with the disposition, all artifacts, findings, trace events, and a replayable snapshot

### Key invariants

- **Strategies are pure readers.** They receive `&EvalContext<D>` (immutable). Only the pipeline merges patches between strategy executions.
- **Artifacts are append-only.** The `ArtifactStore` never overwrites — multiple strategies can write the same key, and consumers read them in emission order.
- **Content-addressed digests.** Every artifact is hashed via blake3 over canonicalized JSON (sorted keys at every level), so two semantically identical values always produce the same digest regardless of insertion order.
- **Finalize only in Decide.** If a strategy emits `Control::Finalize` outside the Decide phase, the pipeline downgrades it to `Continue`, records `StepOutcome::FinalizeIgnored`, and logs an `InvariantViolation` explanation.
- **Business outcomes are not errors.** `Deny` and `Review` are valid dispositions returned in `Ok(EvalResult)`. Only engine failures (strategy crashes, validation errors) produce `Err(PipelineError)`.

## Quick start

Add to your `Cargo.toml`:

```toml
[dependencies]
darq-eval-context = { git = "https://github.com/Quigles1337/DARQ-Eval.git" }
```

### Define a decision

```rust
use darq_eval_context::prelude::*;
use serde::{Deserialize, Serialize};

struct MyPricing;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricingInput {
    base_cost: f64,
    customer_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricingOutput {
    unit_price: f64,
    margin_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PricingConstraints {
    min_margin: f64,
    max_price: f64,
}

impl Decision for MyPricing {
    const KIND: &'static str = "my.pricing";
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
        Ok(())
    }

    fn validate_output(output: &Self::Output, constraints: &Self::Constraints) -> Result<(), ValidationError> {
        if output.margin_pct < constraints.min_margin {
            return Err(ValidationError::ConstraintViolation {
                reason: "margin below minimum".into(),
                constraint_id: Some("min_margin".into()),
            });
        }
        Ok(())
    }
}
```

### Write a strategy

```rust
#[derive(Debug)]
struct CostPlusScorer;

impl EvalStrategy<MyPricing> for CostPlusScorer {
    fn id(&self) -> &str { "cost_plus_scorer" }
    fn version(&self) -> StrategyVersion { StrategyVersion { major: 1, minor: 0, patch: 0 } }
    fn phase(&self) -> Phase { Phase::Evaluate }

    fn evaluate(
        &self,
        ctx: &EvalContext<MyPricing>,
        _capabilities: &CapabilitySet,
    ) -> Result<StrategyPatch<MyPricing>, EvalError> {
        let margin = ctx.policy.get_f64("pricing.base_margin", 0.25);
        let unit_price = ctx.input.base_cost * (1.0 + margin);

        let artifact = Artifact::new(
            "pricing.cost_plus.v1",
            serde_json::json!({ "unit_price": unit_price, "margin": margin }),
            "cost_plus_scorer", "1.0.0", "v1",
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
```

### Assemble and run the pipeline

```rust
let policy = PolicyBundle::new("pricing-v1", "1.0.0")
    .with_value("pricing.base_margin", PolicyValue::Float(0.25));

let pipeline = EvalPipeline::<MyPricing>::new("pricing_pipeline")
    .add_strategy(Box::new(CostPlusScorer))
    .add_strategy(Box::new(MyDecider));  // your Decide-phase strategy

let mut ctx = EvalContext::<MyPricing>::new(
    PricingInput { base_cost: 100.0, customer_tier: "preferred".into() },
    PricingConstraints { min_margin: 0.05, max_price: 200.0 },
    policy,
    RunMeta::new("my.pricing", "case-001", "my-tenant", "api", "quote request"),
    EvalMode::Live,
);

let result = pipeline.execute(&mut ctx)?;

match &result.disposition {
    Some(DecisionDisposition::Auto(output)) => println!("Approved: ${:.2}", output.unit_price),
    Some(DecisionDisposition::Review { reasons, .. }) => println!("Needs review: {:?}", reasons),
    Some(DecisionDisposition::Deny { reasons }) => println!("Denied: {:?}", reasons),
    None => println!("No disposition reached"),
}
```

## Modules

| Module | Purpose |
|--------|---------|
| `decision` | `Decision` trait — typed `Input`/`Output`/`Constraints` with validation |
| `artifact` | `Artifact` and `ArtifactStore` — append-only, content-addressed evidence |
| `policy` | `PolicyBundle` and `PolicyValue` — versioned business rule parameters |
| `context` | `EvalContext<D>` — immutable evaluation environment, `MemoStore`, `RunMeta`, `EvalMode` |
| `trace` | `TraceHandle` — monotonic seq-numbered events, `ExplanationNode` tree |
| `capability` | `Capability` trait, `CapabilitySet`, `RecordedCapability` for replay |
| `strategy` | `EvalStrategy<D>` trait, `StrategyPatch`, `Phase`, `Control`, `DecisionDisposition` |
| `pipeline` | `EvalPipeline<D>` — orchestrates strategy execution across 3 phases |
| `result` | `EvalResult<D>` — complete output with `Snapshot` for replay |

Everything is re-exported from `darq_eval_context::prelude`.

## Examples

### BlockHB Material Pricing

Full three-phase pipeline: competitor enrichment via capability, cost-plus scoring with tier discounts, demand-aware scarcity detection, policy-driven auto-approval or review escalation.

```bash
cargo run --example blockhb_pricing
```

```
=== BlockHB Material Pricing Decision ===

Disposition:
  AUTO-APPROVED
  Unit price: $120.00
  Margin: 20.0%
  Discount: 5.0%
  Tier: preferred

Artifacts (3):
  pricing.competitor.v1 (from competitor_enrichment, external)
  pricing.cost_plus.v1 (from cost_plus_scorer, deterministic)
  pricing.demand.v1 (from demand_aware_scorer, deterministic)

Pipeline Steps:
  competitor_enrichment (Enrich) → Continued
  cost_plus_scorer (Evaluate) → Continued
  demand_aware_scorer (Evaluate) → Continued
  pricing_decider (Decide) → Finalized

Explanation Tree:
  [ENRICHMENT] competitor_enrichment — Fetched competitor pricing from market data service
  [RULE_MATCHED] cost_plus_scorer — Applied 25.0% margin with 5.0% tier discount
  [ENRICHMENT] demand_aware_scorer — Computed 25 days of supply
  [RULE_MATCHED] pricing_decider — Pricing decision: margin=20.0%, scarcity=false
```

### DARQ Records Artist Payout

Demonstrates all three disposition paths — auto-approve, review, and deny — for the DARQ Records streaming payout flow.

```bash
cargo run --example darq_records_payout
```

```
=== DARQ Records Payout Decision ===

--- Scenario A: Small Auto-Approved Payout ---
  AUTO-APPROVED: $25.00 to rDARQ1337xrpl...
  Earned: $40.00

--- Scenario B: Large Payout — Review Required ---
  REVIEW REQUIRED
  Provisional: $100.00
  Reason: [EXCEEDS_AUTO_APPROVE] Payout $100.00 exceeds auto-approve threshold $50.00

--- Scenario C: Exceeds Earned — Denied ---
  DENIED
  Reason: [EXCEEDS_EARNED] Requested $500.00 exceeds earned $4.00
```

## Replay

Every pipeline run captures a `Snapshot` that can reproduce the evaluation deterministically:

1. **Run live** — the pipeline executes with real capabilities and captures external artifacts
2. **Build `RecordedCapability`** — `RecordedCapability::from_artifacts(&snapshot.artifacts)` reconstructs capabilities from the snapshot
3. **Replay** — rebuild the pipeline with recorded capabilities, deserialize input/constraints/policy from the snapshot, execute in `EvalMode::Replay`

Two fidelity levels:

- **`ReplayFidelity::Semantic`** — only external artifacts are stored; deterministic artifacts recompute from code
- **`ReplayFidelity::Exact`** — full artifact store captured; exact reproduction regardless of code changes

See test `ac9_pricing_replay_produces_identical_result` for a working example.

## Pipeline nesting

`EvalPipeline<D>` implements `EvalStrategy<D>`, so you can compose pipelines hierarchically. A sub-pipeline:

- Creates a child context with cloned input/constraints/policy
- Copies the parent's current artifact store
- Executes its own strategy chain
- Returns child artifacts, findings, proposals, and explanations as a `StrategyPatch`

See test `ac4_pipeline_nesting` for a working example.

## Test suite

21 tests covering 10 acceptance criteria:

| AC | What it verifies | Tests |
|----|-----------------|-------|
| AC-1 | Canonical artifact digests (insertion-order independent) | 3 |
| AC-2 | Monotonic trace sequence numbers | 2 |
| AC-3 | Deterministic artifact read helpers | 3 |
| AC-4 | Pipeline nesting (sub-pipeline as strategy) | 1 |
| AC-5 | Snapshot replay fidelity (Exact vs Semantic) | 2 |
| AC-6 | Memo non-semantic enforcement (strategies can't mutate context) | 1 |
| AC-7 | Non-Decide finalization is downgraded to Continue | 1 |
| AC-8 | Business outcomes (Deny/Review) vs engine errors | 3 |
| AC-9 | Deterministic replay produces identical results | 1 |
| AC-10 | All three payout dispositions (Auto/Review/Deny) | 3 |

```bash
cargo test --all-targets
```

## Design constraints

- `#![deny(unsafe_code)]` — no unsafe anywhere
- `thiserror` for all error types
- `serde` on everything that enters patches, results, or snapshots
- `blake3` for all digests, always over canonical JSON
- `tracing` for operational logging (no `println!` in library code)
- No async in v1
- No proc macros
- No runtime plugin registry — compile-time strategy registration only

## Dependencies

| Crate | Purpose |
|-------|---------|
| `serde` + `serde_json` | Serialization for all data types |
| `thiserror` | Ergonomic error types |
| `chrono` | Timestamps with serde support |
| `uuid` | Evaluation IDs (v4) |
| `tracing` | Structured logging |
| `blake3` | Content-addressed artifact digests |

## Future work (not in v1)

- Cross-type sub-decisions (e.g., pricing invoking a risk decision)
- Async strategy trait
- Runtime plugin registry / config-driven strategy instantiation
- WASM plugin loading
- Proc macro derive for `Decision`
- Full simulation/shadow mode infrastructure
- Outcome attachment / feedback loop
- 6-phase refinement (v1 uses 3 phases: Enrich, Evaluate, Decide)

## License

MIT

## Authors

Built by [Quigles1337](https://github.com/Quigles1337) for DARQ Labs.
Co-designed with Claude (Anthropic) and GPT (OpenAI).
