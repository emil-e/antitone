//! D60 AP1 ↔ D65 OKLab cone responses for contrast colour preservation.
use super::{AP1_TO_XYZ, BRADFORD};
use glam::{DMat3, DVec3};
use std::sync::LazyLock;

// https://www.w3.org/TR/css-color-4/#color-conversion-code
const XYZ_D65_TO_LMS: DMat3 = DMat3::from_cols(
    DVec3::new(0.819022442647055, 0.0329836558550745, 0.04817719382823899),
    DVec3::new(0.3619062604571286, 0.9292868602921457, 0.2642395276821537),
    DVec3::new(
        -0.12887378595068213,
        0.03614466586101669,
        0.6335478283072317,
    ),
);
pub(crate) const LIGHTNESS: DVec3 = DVec3::new(
    0.21045426824930336,
    0.7936177747759865,
    -0.00407204302528986,
);

pub(crate) static AP1_TO_LMS: LazyLock<DMat3> = LazyLock::new(|| {
    let d60 = AP1_TO_XYZ * DVec3::ONE;
    let d65 = DVec3::new(0.3127 / 0.3290, 1., (1. - 0.3127 - 0.3290) / 0.3290);
    let adaptation =
        BRADFORD.inverse() * DMat3::from_diagonal((BRADFORD * d65) / (BRADFORD * d60)) * BRADFORD;
    XYZ_D65_TO_LMS * adaptation * AP1_TO_XYZ
});
pub(crate) static LMS_TO_AP1: LazyLock<DMat3> = LazyLock::new(|| AP1_TO_LMS.inverse());

pub(crate) fn preserve(original: DVec3, contrasted: DVec3, amount: f64) -> DVec3 {
    let before = (*AP1_TO_LMS * original).map(f64::cbrt);
    let after = (*AP1_TO_LMS * contrasted).map(f64::cbrt);
    let difference = before - after;
    // The inverse OKLab matrix's L column is (1,1,1). Removing the
    // lightness difference therefore restores a/b without changing contrasted L.
    let roots = after + amount * (difference - DVec3::splat(LIGHTNESS.dot(difference)));
    *LMS_TO_AP1 * (roots * roots * roots)
}
