//! Hashing for numeric float equality, which treats the two signed zeros as equal.

use std::hash::{Hash, Hasher};

pub(crate) fn f64(value: f64, state: &mut impl Hasher) {
    (if value == 0. { 0 } else { value.to_bits() }).hash(state);
}

#[cfg(feature = "gpu")]
pub(crate) fn f32(value: f32, state: &mut impl Hasher) {
    (if value == 0. { 0 } else { value.to_bits() }).hash(state);
}
