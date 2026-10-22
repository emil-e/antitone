use super::*;

#[derive(Clone, Serialize)]
pub(super) struct Neighbor {
    pub(super) side: usize,
    pub(super) axis: usize,
    pub(super) gap: f64,
    pub(super) stations: usize,
    pub(super) geometry: PictureGeometry,
}

/// A neighbor needs a base prefix and a coherent second base->image transition. It is not
/// a free image label that can explain content immediately outside an internal scene line.
pub(super) fn gap_at(
    capture: &Capture,
    roles: &[Role],
    exterior: Exterior,
    point: [f64; 2],
    inward: [f64; 2],
) -> Option<f64> {
    let mut base = 0;
    let mut last_base = 0.;
    for index in 0..64 {
        let distance = index as f64 + 0.5;
        let p = std::array::from_fn(|axis| point[axis] - distance * inward[axis]);
        let (role, rgb) = role_at(capture, roles, p)?;
        if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible) {
            return None;
        }
        match exterior.classify(rgb) {
            Material::Base => {
                base += 1;
                last_base = distance;
            }
            Material::Photo if base > 0 => {
                let q = std::array::from_fn(|axis| p[axis] - inward[axis]);
                let (role, rgb) = role_at(capture, roles, q)?;
                return (matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible)
                    && exterior.classify(rgb) == Material::Photo)
                    .then_some(last_base + 0.5);
            }
            _ => return None,
        }
    }
    None
}

/// A base prefix followed by monotone attenuation into known holder has a separate
/// limit explanation. This does not stamp uncertain pixels as holder or excuse image
/// directly outside the candidate; a photographic fragment can remain an alternative.
pub(super) fn holder_after_base(
    capture: &Capture,
    roles: &[Role],
    exterior: Exterior,
    point: [f64; 2],
    inward: [f64; 2],
) -> bool {
    let mut base = false;
    let mut attenuating = false;
    let mut previous: Option<[u16; 3]> = None;
    for index in 0..64 {
        let distance = index as f64 + 0.5;
        let p = std::array::from_fn(|axis| point[axis] - distance * inward[axis]);
        let Some((role, rgb)) = role_at(capture, roles, p) else {
            return false;
        };
        if role == Role::Holder {
            return base
                && previous
                    .is_some_and(|last| (0..3).all(|channel| rgb[channel] <= last[channel]));
        }
        if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible) {
            return false;
        }
        match exterior.classify(rgb) {
            Material::Base if !attenuating => base = true,
            Material::Photo if base => {
                if previous.is_some_and(|last| (0..3).any(|channel| rgb[channel] > last[channel])) {
                    return false;
                }
                attenuating = true;
            }
            _ => return false,
        }
        previous = Some(rgb);
    }
    false
}

pub(super) fn fit(frame: &PictureGeometry, side: usize, gaps: &[(usize, f64)]) -> Option<Neighbor> {
    if gaps.len() < 12 {
        return None;
    }
    let mut bins = [0; 8];
    for &(station, _) in gaps {
        bins[station / 8] += 1;
    }
    if bins.iter().filter(|&&count| count >= 2).count() < 2 {
        return None;
    }
    let gap = median(gaps.iter().map(|(_, gap)| *gap));
    let mad = 1.4826 * median(gaps.iter().map(|(_, value)| (value - gap).abs()));
    if gap < 1. || mad > 2. {
        return None;
    }
    let axis = usize::from(side >= 2);
    let low = frame.sides[2 * axis]?;
    let high = frame.sides[2 * axis + 1]?;
    let sign = if side == 0 || side == 2 { -1. } else { 1. };
    let mut geometry = frame.clone();
    for bound in &mut geometry.sides[2 * axis..2 * axis + 2] {
        *bound = bound.map(|value| value + sign * (high - low + gap));
    }
    Some(Neighbor {
        side,
        axis,
        gap,
        stations: gaps.len(),
        geometry,
    })
}
