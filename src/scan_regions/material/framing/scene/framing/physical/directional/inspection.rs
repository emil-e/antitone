//! Passive profile export using the detector's cell sampler, never its decision input.
use super::*;
use serde_json::{Value, json};

pub(in super::super) fn profiles(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
) -> Value {
    let mut rows = vec![];
    for index in 0..64 {
        let p = perimeter::point(frame, side, (index as f64 + 0.5) / 64.);
        let normal = perimeter::normal(frame.angle, side);
        let tangent = [-normal[1], normal[0]];
        let mut profile = [None; 12];
        let available = role_at(capture, roles, p).is_some();
        if available {
            for sign in [-1., 1.] {
                for step in 0..6 {
                    let distance = sign * (step as f64 + 0.5);
                    let q = std::array::from_fn(|axis| p[axis] - distance * normal[axis]);
                    let Some(sample) = cell(capture, roles, frame, side, q, tangent, distance < 0.)
                    else {
                        break;
                    };
                    profile[(distance + 5.5).round() as usize] = Some(sample);
                }
            }
        }
        let gradients: Vec<_> = (-3..=3)
            .map(|offset| json!({"offset":offset,"contrast":gradient(&profile,f64::from(offset))}))
            .collect();
        rows.push(json!({"index":index,"point":p,"point_available":available,"cells":profile,"gradients":gradients}));
    }
    json!({"normal_distances":(0..12).map(|i|i as f64-5.5).collect::<Vec<_>>(),"stations":rows,"limits":"Read-only replay of existing contiguous cell sampler; no detection decisions, target fitting or inference sampling changes."})
}
