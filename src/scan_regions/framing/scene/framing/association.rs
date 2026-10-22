use super::*;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Separator {
    pub(super) axis: usize,
    pub(super) low: f64,
    pub(super) high: f64,
}

/// A provisional repeated-base separator with target image evidence on both sides.
pub(super) fn separators(
    capture: &Capture,
    roles: &[Role],
    persistent: &[bool],
    angle: f64,
) -> Vec<Separator> {
    let origin = crate::scan_regions::rectangles::bounds(capture.size, angle, 0.);
    let mut result = vec![];
    for axis in 0..2 {
        let other = 1 - axis;
        let mut run = None;
        for across in 0..=capture.size[axis] {
            let cross = origin[2 * axis] + f64::from(across) + 0.5;
            let mut available = 0;
            let mut base = 0;
            let mut flanked = 0;
            if across < capture.size[axis] {
                for along in super::super::super::super::indices(capture.size[other], 32) {
                    let uv = if axis == 0 {
                        [cross, origin[2] + f64::from(along) + 0.5]
                    } else {
                        [origin[0] + f64::from(along) + 0.5, cross]
                    };
                    let point = project(uv, -angle);
                    let Some((role, _)) = role_at(capture, roles, point) else {
                        continue;
                    };
                    if matches!(role, Role::Holder | Role::BrighterThanBase | Role::Unknown) {
                        continue;
                    }
                    available += 1;
                    let pixel = point.map(|v| v.floor() as usize);
                    base += usize::from(
                        role == Role::BaseCompatible
                            && persistent[pixel[1] * capture.size[0] as usize + pixel[0]],
                    );
                    let sides = [-3., 3.].map(|distance| {
                        let mut shifted = uv;
                        shifted[axis] += distance;
                        role_at(capture, roles, project(shifted, -angle)).map(|(role, _)| role)
                    });
                    flanked += usize::from(sides == [Some(Role::AttenuatedCompatible); 2]);
                }
            }
            let supported = available >= 12
                && base as f64 >= 0.9 * available as f64
                && flanked as f64 >= 0.5 * available as f64;
            if supported {
                if run.is_none() {
                    run = Some(cross - 0.5);
                }
            } else if let Some(low) = run.take() {
                let high = cross - 0.5;
                if high - low >= 2. {
                    result.push(Separator { axis, low, high });
                }
            }
        }
    }
    result
}

pub(super) fn crosses_separator(
    frame: &PictureGeometry,
    limits: [Option<f64>; 4],
    separators: &[Separator],
) -> bool {
    let frame = restricted(frame, limits);
    separators.iter().any(|separator| {
        let low = frame.sides[2 * separator.axis].unwrap_or(f64::NEG_INFINITY);
        let high = frame.sides[2 * separator.axis + 1].unwrap_or(f64::INFINITY);
        low < separator.low - 2. && high > separator.high + 2.
    })
}
