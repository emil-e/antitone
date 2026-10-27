use super::*;

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct Band {
    pub(super) center: f64,
    pub(super) radius: f64,
}

pub(super) fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    (values[(values.len() - 1) / 2] + values[values.len() / 2]) * 0.5
}

impl Band {
    fn fit(values: &[f64], floor: f64) -> Self {
        let center = median(values.iter().copied());
        let mad = 1.4826 * median(values.iter().map(|value| (value - center).abs()));
        Self {
            center,
            radius: (6. * mad).max(floor),
        }
    }
    pub(super) fn contains(self, value: f64) -> bool {
        (value - self.center).abs() <= self.radius
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct Exterior {
    pub(super) mean: [f64; 3],
    pub(super) spread: [f64; 3],
    pub(super) samples: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Material {
    Base,
    Photo,
    Other,
}

impl Exterior {
    fn fit(values: &[[f64; 3]]) -> Option<Self> {
        if values.len() < 12 {
            return None;
        }
        let mean = std::array::from_fn(|ch| median(values.iter().map(|value| value[ch])));
        let spread = std::array::from_fn(|ch| {
            (1.4826 * median(values.iter().map(|value| (value[ch] - mean[ch]).abs()))).max(0.03)
        });
        Some(Self {
            mean,
            spread,
            samples: values.len(),
        })
    }
    pub(super) fn classify(self, rgb: [u16; 3]) -> Material {
        let Some(value) = logs(rgb) else {
            return Material::Other;
        };
        let residual: [f64; 3] =
            std::array::from_fn(|ch| (value[ch] - self.mean[ch]) / self.spread[ch]);
        if residual.iter().all(|v| v.abs() <= 3.) {
            Material::Base
        } else if residual.iter().all(|v| *v <= 3.) && residual.iter().any(|v| *v < -3.) {
            Material::Photo
        } else {
            Material::Other
        }
    }
}

#[derive(Clone, Serialize)]
pub(super) struct Template {
    pub(super) pose: Band,
    pub(super) sides: [Band; 4],
    pub(super) centers: [Band; 2],
    pub(super) scale: [f64; 2],
    pub(super) width: Band,
    pub(super) height: Band,
    pub(super) exterior: [Option<Exterior>; 4],
    pub(super) donors: Vec<usize>,
    pub(super) rejected: Vec<usize>,
    pub(super) size: [u32; 2],
}

pub(super) fn axis_scales(angle: f64, scale: [f64; 2]) -> [f64; 2] {
    let (s, c) = angle.to_radians().sin_cos();
    [
        (scale[0] * c).hypot(scale[1] * s),
        (scale[0] * s).hypot(scale[1] * c),
    ]
}

fn parameters(
    frame: &PictureGeometry,
    common_angle: f64,
    scale: [f64; 2],
) -> Option<([f64; 2], [f64; 2])> {
    let [Some(l), Some(r), Some(t), Some(b)] = frame.sides else {
        return None;
    };
    if r <= l || b <= t {
        return None;
    }
    let center = project(
        project([(l + r) * 0.5, (t + b) * 0.5], -frame.angle),
        common_angle,
    );
    let scales = axis_scales(frame.angle, scale);
    Some((center, [(r - l) * scales[0], (b - t) * scales[1]]))
}

impl Template {
    #[cfg(test)]
    pub(super) fn fit(
        scene: &Scene,
        captures: &[Capture],
        roles: &[Vec<Role>],
        frames: &[(usize, PictureGeometry)],
    ) -> Option<Self> {
        Self::fit_refs(scene, &captures.iter().collect::<Vec<_>>(), roles, frames)
    }

    pub(super) fn fit_refs(
        scene: &Scene,
        captures: &[&Capture],
        roles: &[Vec<Role>],
        frames: &[(usize, PictureGeometry)],
    ) -> Option<Self> {
        if frames.len() < 3 {
            return None;
        }
        let size = frames[0].1.size;
        let pose = Band::fit(
            &frames
                .iter()
                .map(|(_, frame)| frame.angle)
                .collect::<Vec<_>>(),
            0.5,
        );
        let area = captures[frames[0].0].area;
        let scale = [
            f64::from(area.width) / f64::from(size[0]),
            f64::from(area.height) / f64::from(size[1]),
        ];
        let units = axis_scales(pose.center, scale);
        let rows: Vec<_> = frames
            .iter()
            .filter(|(_, frame)| pose.contains(frame.angle))
            .filter_map(|(id, frame)| {
                parameters(frame, pose.center, scale).map(|(center, extent)| (*id, center, extent))
            })
            .collect();
        if rows.len() < 3 {
            return None;
        }
        let initial_centers: [Band; 2] = std::array::from_fn(|axis| {
            Band::fit(
                &rows
                    .iter()
                    .map(|(_, center, _)| center[axis])
                    .collect::<Vec<_>>(),
                3.,
            )
        });
        let initial_extents: [Band; 2] = std::array::from_fn(|axis| {
            Band::fit(
                &rows
                    .iter()
                    .map(|(_, _, extent)| extent[axis])
                    .collect::<Vec<_>>(),
                4. * units[axis],
            )
        });
        let accepted: Vec<_> = rows
            .iter()
            .filter(|(_, center, extent)| {
                (0..2).all(|axis| {
                    initial_centers[axis].contains(center[axis])
                        && initial_extents[axis].contains(extent[axis])
                })
            })
            .collect();
        if accepted.len() < 3 || accepted.len() * 3 < frames.len() * 2 {
            return None;
        }
        let centers = std::array::from_fn(|axis| {
            Band::fit(
                &accepted
                    .iter()
                    .map(|(_, center, _)| center[axis])
                    .collect::<Vec<_>>(),
                3.,
            )
        });
        let sides = std::array::from_fn(|side| {
            let axis = usize::from(side >= 2);
            let sign = if side == 0 || side == 2 { -1. } else { 1. };
            Band::fit(
                &accepted
                    .iter()
                    .map(|(_, center, extent)| {
                        center[axis] + sign * extent[axis] / units[axis] * 0.5
                    })
                    .collect::<Vec<_>>(),
                3.,
            )
        });
        let width = Band::fit(
            &accepted
                .iter()
                .map(|(_, _, extent)| extent[0])
                .collect::<Vec<_>>(),
            4. * units[0],
        );
        let height = Band::fit(
            &accepted
                .iter()
                .map(|(_, _, extent)| extent[1])
                .collect::<Vec<_>>(),
            4. * units[1],
        );
        let donors: Vec<_> = accepted.iter().map(|(id, _, _)| *id).collect();
        let rejected = frames
            .iter()
            .filter(|(id, _)| !donors.contains(id))
            .map(|(id, _)| *id)
            .collect();
        let exterior = std::array::from_fn(|side| {
            let mut values = vec![];
            for (id, frame) in frames.iter().filter(|(id, _)| donors.contains(id)) {
                for station in 0..32 {
                    let point = super::perimeter::point(frame, side, (station as f64 + 0.5) / 32.);
                    let normal = super::perimeter::normal(frame.angle, side);
                    for distance in [0.5, 1.5, 2.5] {
                        let p = std::array::from_fn(|axis| point[axis] - distance * normal[axis]);
                        let Some((role, rgb)) = role_at(captures[*id], &roles[*id], p) else {
                            break;
                        };
                        if role != Role::BaseCompatible {
                            break;
                        }
                        if let Some(value) = logs(rgb) {
                            values.push(value);
                        }
                    }
                }
            }
            // The bootstrap is a provisional colour reference, not an invented material label.
            Exterior::fit(&values).or_else(|| {
                scene.base.as_ref().map(|base| Exterior {
                    mean: base.mean,
                    spread: base.spread,
                    samples: 0,
                })
            })
        });
        Some(Self {
            pose,
            sides,
            centers,
            scale,
            width,
            height,
            exterior,
            donors,
            rejected,
            size,
        })
    }

    pub(super) fn frame(&self, angle: f64) -> PictureGeometry {
        // Position and physical extents have different jobs: changing pose rotates the
        // predicted rectangle about its centre; it must not enlarge a bounding envelope.
        let center = project(
            project(self.centers.map(|b| b.center), -self.pose.center),
            angle,
        );
        let scales = axis_scales(angle, self.scale);
        let extent = [
            self.width.center / scales[0],
            self.height.center / scales[1],
        ];
        PictureGeometry {
            size: self.size,
            angle,
            sides: std::array::from_fn(|side| {
                let axis = usize::from(side >= 2);
                let sign = if side == 0 || side == 2 { -1. } else { 1. };
                Some(center[axis] + sign * extent[axis] * 0.5)
            }),
        }
    }

    pub(super) fn compatible(&self, frame: &PictureGeometry) -> bool {
        let Some((center, extent)) = parameters(frame, self.pose.center, self.scale) else {
            return false;
        };
        self.pose.contains(frame.angle)
            && (0..2).all(|axis| self.centers[axis].contains(center[axis]))
            && self.width.contains(extent[0])
            && self.height.contains(extent[1])
    }

    #[cfg(test)]
    pub(super) fn displacement(&self, frame: &PictureGeometry) -> f64 {
        parameters(frame, self.pose.center, self.scale).map_or(f64::INFINITY, |(center, extent)| {
            let position = (0..2)
                .map(|axis| {
                    ((center[axis] - self.centers[axis].center) / self.centers[axis].radius).powi(2)
                })
                .sum::<f64>();
            (position
                + ((extent[0] - self.width.center) / self.width.radius).powi(2)
                + ((extent[1] - self.height.center) / self.height.radius).powi(2))
                / 4.
        })
    }
}
