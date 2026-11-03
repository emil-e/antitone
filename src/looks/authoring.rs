//! Fixed colour-treatment authoring domain and viewing reference.
pub const DEFAULT_GRID: u32 = 129;
pub const REFERENCE: [f64; 9] = [
    100., 0.694, 0.306, 0.2175, 0.7435, 0.1405, 0.053, 0.3127, 0.3290,
];
pub const DOMAIN_MIN: f64 = -0.5;
pub fn domain_max() -> f64 {
    DOMAIN_MIN + (crate::photographic::encode(0.18) - DOMAIN_MIN) * (64. / 38.)
}
