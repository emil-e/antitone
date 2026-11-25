//! Fixed-donor healing of original, ungraded AP1 and masked replacement.

#[cfg(all(test, feature = "gpu"))]
pub(crate) mod fixtures;
#[cfg(feature = "gpu")]
pub(crate) mod operation;
mod solver;

#[cfg(feature = "raw")]
pub(crate) use solver::evaluate_with;
pub use solver::{EvaluatedRepair, evaluate, heal_ranked};
