//! Exact numeric equality for test operations and synthetic pixel sources.

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Debug, PartialEq)]
pub(crate) struct NumericKey<T>(pub T);

impl<T: NumericHash> Hash for NumericKey<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.numeric_hash(state);
    }
}

pub(crate) trait NumericHash {
    fn numeric_hash(&self, state: &mut impl Hasher);
}

impl NumericHash for f32 {
    fn numeric_hash(&self, state: &mut impl Hasher) {
        crate::float_hash::f32(*self, state);
    }
}

impl NumericHash for f64 {
    fn numeric_hash(&self, state: &mut impl Hasher) {
        crate::float_hash::f64(*self, state);
    }
}

impl<T: NumericHash, const N: usize> NumericHash for [T; N] {
    fn numeric_hash(&self, state: &mut impl Hasher) {
        for value in self {
            value.numeric_hash(state);
        }
    }
}

impl<T: NumericHash> NumericHash for Vec<T> {
    fn numeric_hash(&self, state: &mut impl Hasher) {
        self.len().hash(state);
        for value in self {
            value.numeric_hash(state);
        }
    }
}

impl<T: NumericHash> NumericHash for Arc<T> {
    fn numeric_hash(&self, state: &mut impl Hasher) {
        (**self).numeric_hash(state);
    }
}
