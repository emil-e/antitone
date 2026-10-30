use super::*;

const FEATURE_EMPHASIS: f64 = 8.;
const MAX_RELATIVE_ENERGY: f64 = 16.;
const MIN_TENSOR_COHERENCE: f64 = 0.15;
const MIN_SPATIAL_COHERENCE: f64 = 0.65;
const MIN_DIRECTION_AGREEMENT: f64 = 0.99;

/// Search-only descriptors: reconstruction still consumes the stored translation
/// and the original AP1 samples. Grain should not outweigh a thin boundary feature.
pub(super) struct Matcher<'a> {
    targets: &'a [SearchTarget],
    descriptors: Vec<Descriptor>,
}

struct Descriptor {
    weights: Vec<f64>,
    direction: Option<(f64, [f64; 2])>,
}

impl<'a> Matcher<'a> {
    pub(super) fn new(targets: &'a [SearchTarget], stopped: &dyn Fn() -> bool) -> Option<Self> {
        let mut descriptors = Vec::new();
        for target in targets {
            let [width, height] = target.size;
            let distances = distance_to_unknown(&target.known, width, height, stopped)?;
            let mut weights = Vec::with_capacity(distances.len());
            let mut boundary_weights = Vec::with_capacity(distances.len());
            for (index, distance) in distances.into_iter().enumerate() {
                if index.is_multiple_of(1024) && stopped() {
                    return None;
                }
                let energy = gradient_energy(target.gradients[index]);
                let relative_energy = energy / (6. * target.gradient_scale.powi(2));
                // A bounded feature emphasis keeps thin edges competitive with
                // broad background agreement. Decay has no floor: the corners of
                // a large circular mask's bounding box are not boundary evidence.
                let boundary = (-(distance - 1.).max(0.) / 3.).exp();
                boundary_weights.push(boundary);
                weights.push(
                    boundary * (1. + FEATURE_EMPHASIS * relative_energy.min(MAX_RELATIVE_ENERGY)),
                );
            }
            let direction = structure_direction(target, &boundary_weights, stopped);
            if stopped() {
                return None;
            }
            descriptors.push(Descriptor { weights, direction });
        }
        Some(Self {
            targets,
            descriptors,
        })
    }

    pub(super) fn score(
        &self,
        support: &SupportMask,
        near: &NearSupport,
        offset: [i32; 2],
        base: &mut impl BaseImage,
        execution: &mut Execution<'_>,
    ) -> Result<Option<f64>, Error> {
        let mut color_error = 0.;
        let mut color_weight = 0.;
        let mut gradient_error = 0.;
        let mut gradient_weight = 0.;
        for (target, descriptor) in self.targets.iter().zip(&self.descriptors) {
            let [width, height] = target.size;
            let Some(donor) = fetch_crop(base, target.origin, target.size, offset, execution)?
            else {
                return Ok(None);
            };
            let Some(colors) = gaussian_blur(&donor, width, height, execution.cancelled) else {
                return Ok(None);
            };
            let Some(gradients) = sobel(&colors, width, height, execution.cancelled) else {
                return Ok(None);
            };
            for index in 0..descriptor.weights.len() {
                if index.is_multiple_of(1024) && (execution.cancelled)() {
                    return Ok(None);
                }
                let donor_point = [
                    target.origin[0] as i32 + (index % width) as i32 + offset[0],
                    target.origin[1] as i32 + (index / width) as i32 + offset[1],
                ];
                if target.known[index]
                    && !support.contains([donor_point[0] as u32, donor_point[1] as u32])
                {
                    let weight = target.weights[index];
                    color_weight += weight;
                    for ((a, b), center) in target.image[index]
                        .into_iter()
                        .zip(donor[index])
                        .zip(target.center)
                    {
                        let a = (f64::from(a) - center) / target.contrast;
                        let b = (f64::from(b) - center) / target.contrast;
                        color_error += (a - b).powi(2) * weight;
                    }
                }
                // Smoothed derivatives need uncontaminated filter support in
                // both images; raw colour above can still use the near boundary.
                if !target.valid_target[index] || near.contains(donor_point) {
                    continue;
                }
                let weight = if descriptor.direction.is_some() {
                    descriptor.weights[index]
                } else {
                    target.weights[index]
                };
                gradient_weight += weight;
                for (a, b) in target.gradients[index].into_iter().zip(gradients[index]) {
                    let delta = (f64::from(a) - f64::from(b)) / target.gradient_scale;
                    gradient_error += delta.powi(2) * weight;
                }
            }
        }
        Ok(Some(if color_weight > 0. || gradient_weight > 0. {
            (if color_weight > 0. {
                0.75 * color_error / (color_weight * 3.)
            } else {
                0.
            }) + if gradient_weight > 0. {
                0.25 * gradient_error / (gradient_weight * 6.)
            } else {
                0.
            }
        } else {
            f64::INFINITY
        }))
    }

    pub(super) fn proposals(
        &self,
        support: &SupportMask,
        stopped: &dyn Fn() -> bool,
    ) -> Vec<[i32; 2]> {
        if stopped() {
            return Vec::new();
        }
        let mut directions = self
            .descriptors
            .iter()
            .filter_map(|descriptor| descriptor.direction)
            .collect::<Vec<_>>();
        directions.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut offsets = Vec::new();
        for (_, [tx, ty]) in directions.into_iter().take(3) {
            let [width, height] = support.size();
            // Start beyond a bounding-box separation, then probe nearby normal
            // offsets for integer-grid alignment. Ordinary geometry checks still
            // decide which of these proposals are valid.
            let separation = ((f64::from(width) + 2.) / tx.abs())
                .min((f64::from(height) + 2.) / ty.abs())
                .ceil();
            for scale in [1., 1.25, 1.5, 2.] {
                for normal in [0., -1., 1., -2., 2., -3., 3.] {
                    for sign in [-1., 1.] {
                        offsets.push([
                            (sign * tx * separation * scale + ty * normal).round() as i32,
                            (sign * ty * separation * scale - tx * normal).round() as i32,
                        ]);
                    }
                }
            }
        }
        offsets
    }
}

fn gradient_energy(gradient: [f32; 6]) -> f64 {
    gradient
        .into_iter()
        .map(|value| f64::from(value).powi(2))
        .sum()
}

/// A structure tensor supplies a tangent even when a thin feature occupies few
/// pixels. Fit the positions of strong responses to reduce derivative-grid bias:
/// a small angular error can miss the line by several pixels at a distant donor.
fn structure_direction(
    target: &SearchTarget,
    boundary_weights: &[f64],
    stopped: &dyn Fn() -> bool,
) -> Option<(f64, [f64; 2])> {
    let mut xx = 0.;
    let mut xy = 0.;
    let mut yy = 0.;
    let mut energies = Vec::new();
    for (index, gradient) in target.gradients.iter().enumerate() {
        if index.is_multiple_of(1024) && stopped() {
            return None;
        }
        if !target.valid_target[index] {
            continue;
        }
        // Use the same distance emphasis as matching, before feature emphasis.
        let boundary = boundary_weights[index];
        for channel in 0..3 {
            let x = f64::from(gradient[channel]);
            let y = f64::from(gradient[channel + 3]);
            xx += x * x * boundary;
            xy += x * y * boundary;
            yy += y * y * boundary;
        }
        energies.push((gradient_energy(*gradient), index));
    }
    let coherence = tensor_coherence(xx, xy, yy);
    if coherence < MIN_TENSOR_COHERENCE || !coherence.is_finite() || energies.is_empty() {
        return None;
    }
    let angle = 0.5 * (2. * xy).atan2(xx - yy);
    let tangent = [-angle.sin(), angle.cos()];
    let median = energies.len() / 2;
    let (_, middle, _) = energies.select_nth_unstable_by(median, |a, b| a.0.total_cmp(&b.0));
    let floor = middle.0 * 4.;
    let mut mass = 0.;
    let mut center = [0.; 2];
    for &(energy, index) in &energies {
        let weight = (energy - floor).max(0.);
        mass += weight;
        center[0] += weight * (index % target.size[0]) as f64;
        center[1] += weight * (index / target.size[0]) as f64;
    }
    if mass > 0. {
        center.iter_mut().for_each(|value| *value /= mass);
        let mut xx = 0.;
        let mut xy = 0.;
        let mut yy = 0.;
        for &(energy, index) in &energies {
            let weight = (energy - floor).max(0.);
            let x = (index % target.size[0]) as f64 - center[0];
            let y = (index / target.size[0]) as f64 - center[1];
            xx += weight * x * x;
            xy += weight * x * y;
            yy += weight * y * y;
        }
        let angle = 0.5 * (2. * xy).atan2(xx - yy);
        let fitted = [angle.cos(), angle.sin()];
        // Grain and ordinary texture can yield an anisotropic derivative
        // tensor by chance. Require elongated spatial evidence agreeing with
        // that direction before changing the metric or proposing line offsets.
        if tensor_coherence(xx, xy, yy) > MIN_SPATIAL_COHERENCE
            && (fitted[0] * tangent[0] + fitted[1] * tangent[1]).abs() > MIN_DIRECTION_AGREEMENT
        {
            return Some((coherence, fitted));
        }
    }
    None
}

fn tensor_coherence(xx: f64, xy: f64, yy: f64) -> f64 {
    ((xx - yy).powi(2) + 4. * xy.powi(2)).sqrt() / (xx + yy).max(1e-30)
}
