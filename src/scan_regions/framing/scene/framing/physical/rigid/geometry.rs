use super::*;

fn source_scales(capture: &Capture) -> [f64; 2] {
    [
        f64::from(capture.area.width) / f64::from(capture.size[0]),
        f64::from(capture.area.height) / f64::from(capture.size[1]),
    ]
}

pub(super) fn frame_from_pair(
    capture: &Capture,
    source_size: [f64; 2],
    vertical: usize,
    horizontal: usize,
    bounds: [f64; 2],
    pose: f64,
) -> Option<PictureGeometry> {
    if vertical >= 2
        || !(2..4).contains(&horizontal)
        || !pose.is_finite()
        || bounds.iter().any(|value| !value.is_finite())
        || source_size
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.)
    {
        return None;
    }
    let scales = model::axis_scales(pose, source_scales(capture));
    let [width, height] = std::array::from_fn(|axis| source_size[axis] / scales[axis]);
    let left = bounds[0] - if vertical == 1 { width } else { 0. };
    let top = bounds[1] - if horizontal == 3 { height } else { 0. };
    Some(PictureGeometry {
        size: capture.size,
        angle: pose,
        sides: [left, left + width, top, top + height].map(Some),
    })
}

pub(in super::super::super) fn native_bound(
    line: &Line,
    side: usize,
    size: [u32; 2],
    pose: f64,
) -> Option<f64> {
    let along_axis = usize::from(side < 2);
    let values = line
        .spans()
        .into_iter()
        .flat_map(|[lo, hi]| {
            [lo, (lo + hi) * 0.5, hi].map(|fraction| {
                let along = f64::from(size[along_axis] - 1) * fraction + 0.5;
                let cross = line.cross(side, size, along);
                project(
                    if side < 2 {
                        [cross, along]
                    } else {
                        [along, cross]
                    },
                    pose,
                )[1 - along_axis]
            })
        })
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| median(values.into_iter()))
}

pub(super) fn validation_template(
    capture: &Capture,
    calibration: &Calibration,
    frame: &PictureGeometry,
) -> Option<Template> {
    // Size is fixed at construction. Old peer placement/pose boxes are discovery
    // priors, not reasons to reject independently placed adjacent anchors.
    let mut template = calibration.template.clone()?;
    template.size = capture.size;
    template.scale = source_scales(capture);
    template.pose = Band {
        center: frame.angle,
        radius: 1e-7,
    };
    template.centers = std::array::from_fn(|axis| Band {
        center: (frame.sides[axis * 2].unwrap() + frame.sides[axis * 2 + 1].unwrap()) * 0.5,
        radius: 1e-6,
    });
    template.width = Band {
        center: calibration.source_size[0],
        radius: 1e-6,
    };
    template.height = Band {
        center: calibration.source_size[1],
        radius: 1e-6,
    };
    Some(template)
}

/// Compare corresponding boundary lines, not combined diagonal motion of their corners.
/// Both directions and endpoints matter; never chain a sequence of nearby rectangles.
pub(super) fn boundary_distance(a: &PictureGeometry, b: &PictureGeometry) -> f64 {
    [(a, b), (b, a)]
        .into_iter()
        .flat_map(|(from, to)| {
            (0..4).flat_map(move |side| {
                [0., 1.].map(move |fraction| {
                    let point = project(perimeter::point(from, side, fraction), to.angle);
                    (point[usize::from(side >= 2)] - to.sides[side].unwrap()).abs()
                })
            })
        })
        .fold(0., f64::max)
}
