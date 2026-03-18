//! Decision trait — the core abstraction for typed business decisions.
//!
//! Every business decision (pricing, payouts, risk assessment, etc.) implements
//! this trait, which provides typed input/output/constraints and validation.

use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;

/// A typed business decision with validated inputs and constrained outputs.
///
/// Implementors define the shape of a specific decision domain (e.g., pricing,
/// payouts) by specifying associated types for input, output, and constraints.
pub trait Decision: Send + Sync + 'static {
    /// Stable kind identifier, e.g. `"blockhb.pricing"`.
    const KIND: &'static str;

    /// The input data for this decision.
    type Input: Debug + Clone + Send + Sync + Serialize + DeserializeOwned;
    /// The output produced by this decision.
    type Output: Debug + Clone + Send + Sync + Serialize + DeserializeOwned;
    /// Constraints that bound acceptable outputs.
    type Constraints: Debug + Clone + Send + Sync + Serialize + DeserializeOwned;

    /// Validate that the input is well-formed before evaluation begins.
    fn validate_input(input: &Self::Input) -> Result<(), ValidationError>;

    /// Validate that the output satisfies the given constraints.
    fn validate_output(
        output: &Self::Output,
        constraints: &Self::Constraints,
    ) -> Result<(), ValidationError>;
}

/// Errors arising from input or output validation.
#[derive(Debug, Clone, thiserror::Error, Serialize, serde::Deserialize)]
pub enum ValidationError {
    /// The input data is invalid.
    #[error("invalid input: {reason}")]
    InvalidInput {
        reason: String,
        field: Option<String>,
    },

    /// The output violates a constraint.
    #[error("constraint violation: {reason}")]
    ConstraintViolation {
        reason: String,
        constraint_id: Option<String>,
    },

    /// A required field is missing.
    #[error("missing field: {field}")]
    MissingField { field: String },
}
