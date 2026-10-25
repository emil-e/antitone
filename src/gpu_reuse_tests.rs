use super::*;
use crate::processing::Calibration;
use crate::{
    color::Balance,
    geometry::{SourcePoint, SourceRegion},
    local_adjustments::{GradientMask, LocalAdjustment, PhotoGradeSnapshot},
    processing::{DEFAULT_GAMMA, NegativeInput, Response},
};

#[cfg(feature = "raw")]
#[path = "gpu_reuse_raw_tests.rs"]
mod raw_tests;

#[path = "gpu_stage_performance_tests.rs"]
mod stage_tests;

#[path = "gpu_log_prefix_tests.rs"]
mod log_prefix_tests;

#[cfg(all(feature = "ui", feature = "render"))]
#[path = "gpu_screen_performance_tests.rs"]
mod screen_tests;

const SIZE: [u32; 2] = [1024, 640];
const RECORD_BYTES: u64 = 40; // Nine scalar f32 words and one u32 flag, no vec3 padding.
const ENTRIES: [&str; 9] = [
    "build_response_codes",
    "lookup_main",
    "build_camera_prefix",
    "camera_prefix_main",
    "build_global_prefix",
    "global_prefix_main",
    "build_local_prefix_lookup",
    "build_log_prefix",
    "log_prefix_main",
];

// Counterfactual reuse probes key native inversion separately from downstream grades.
#[derive(Debug, PartialEq, Eq)]
struct ResponseKey([u8; 176]);

impl ResponseKey {
    fn new(p: ProcessingParameters) -> Self {
        let words = [
            p.dmin_floor,
            p.dmin_low,
            p.dmin_tail,
            p.gamma_mode.xyz().extend(0.),
            p.gamma_low.xyz().extend(0.),
            p.gamma_tail.xyz().extend(0.),
            p.shape,
            p.shape_tail,
            p.anchor,
            p.anchor_low,
            p.anchor_tail,
        ];
        Self(bytemuck::cast_slice(&words).try_into().unwrap())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PrefixKey {
    response: ResponseKey,
    // Image-owned revision must advance for source writes, correction and geometry changes.
    source_identity_revision: [u64; 2],
    matrix: [u64; 9],
    geometry: Vec<u8>,
    global: Option<Vec<u8>>,
}

impl PrefixKey {
    fn new(
        identity_revision: [u64; 2],
        matrix: DMat3,
        geometry: flat::FieldGeometry,
        parameters: ProcessingParameters,
        after_global: bool,
    ) -> Self {
        Self {
            response: ResponseKey::new(parameters),
            source_identity_revision: identity_revision,
            matrix: matrix.to_cols_array().map(f64::to_bits),
            geometry: bytemuck::bytes_of(&geometry).to_vec(),
            global: after_global.then(|| bytemuck::bytes_of(&parameters).to_vec()),
        }
    }
}

fn calibration() -> Calibration {
    Calibration {
        dmin: DVec3::new(0.19, 0.14, 0.12),
        dmax: DVec3::new(0.008, 0.004, 0.002),
        input_floor: 1. / 65535.,
    }
}

fn negative(cal: Calibration, gamma: f64, response: Response, balance: Balance) -> NegativeInput {
    NegativeInput {
        calibration: cal,
        response,
        gamma,
        balance,
    }
}

fn camera_matrix() -> DMat3 {
    DMat3::from_cols(
        DVec3::new(0.6062, 0.0161, -0.0031),
        DVec3::new(0.3802, 1.1927, -0.1490),
        DVec3::new(0.0136, -0.2088, 1.1521),
    )
}

fn zero_y_matrix() -> DMat3 {
    let rgb = Inversion::new(calibration())
        .unwrap()
        .apply(DVec3::ZERO)
        .unwrap()
        .rgb;
    let y = AP1_TO_XYZ.row(1);
    let cancel = (-y.x * rgb.x - y.y * rgb.y + 0.25 * y.z * rgb.z) / (y.y * rgb.z);
    DMat3::from_cols(DVec3::X, DVec3::Y, DVec3::new(0., cancel, -0.25))
}

fn area() -> SourceRegion {
    SourceRegion {
        x: 12,
        y: 12,
        width: 8192,
        height: 5472,
    }
}

fn geometry() -> SourceGeometry {
    let a = area();
    SourceGeometry::new([a.x, a.y, a.width, a.height], SIZE).unwrap()
}

fn photographic() -> PhotoSettings {
    PhotoSettings {
        exposure: 0.25,
        wb: Balance {
            warmth: 0.005,
            tint: -0.003,
        },
        contrast: 1.01,
        contrast_colour_mix: 0.25,
        saturation: 1.02,
        saturation_density: 0.4,
        ..Default::default()
    }
}

fn snapshot(global: PhotoSettings, count: usize, nonlinear: bool) -> PhotoGradeSnapshot {
    let locals = (0..count)
        .map(|i| LocalAdjustment {
            mask: if i % 2 == 0 {
                GradientMask::Linear {
                    start: SourcePoint([900., 800.]),
                    end: SourcePoint([7000., 4400.]),
                }
            } else {
                GradientMask::Radial {
                    center: SourcePoint([4100., 2750.]),
                    radii: [3500., 2300.],
                    angle: 0.3,
                    feather: 0.8,
                }
            },
            photo: if nonlinear {
                photographic()
            } else {
                PhotoSettings {
                    exposure: if i % 2 == 0 { 0.25 } else { -0.125 },
                    ..Default::default()
                }
            },
            enabled: true,
            invert: i % 3 == 0,
            strength: 0.6,
        })
        .collect();
    PhotoGradeSnapshot::new(global, locals).unwrap()
}

#[test]
fn experimental_reuse_keys_preserve_response_and_prefix_dependencies() {
    let c = calibration();
    let r = RollSettings::default();
    let photo = PhotoSettings::default();
    let p = ProcessingParameters::positive(c, r, photo, camera_matrix()).unwrap();
    let field = || flat::FieldGeometry::uncorrected(Some(geometry()));
    let key = |p, global| PrefixKey::new([1, 0], camera_matrix(), field(), p, global);
    for edit in [
        PhotoSettings {
            wb: photographic().wb,
            ..photo
        },
        PhotoSettings {
            contrast: 1.1,
            ..photo
        },
        PhotoSettings {
            saturation: 1.1,
            ..photo
        },
    ] {
        let changed = ProcessingParameters::positive(c, r, edit, camera_matrix()).unwrap();
        assert_eq!(ResponseKey::new(p), ResponseKey::new(changed));
        assert_eq!(key(p, false), key(changed, false));
        assert_ne!(key(p, true), key(changed, true));
    }
    let base = negative(c, DEFAULT_GAMMA, Response::film(), Balance::default());
    for (_calibration, interpretation, roll, photo) in [
        (
            c,
            base,
            r,
            PhotoSettings {
                exposure: 1.,
                ..photo
            },
        ),
        (c, base, RollSettings { exposure: 1., ..r }, photo),
        (
            c,
            negative(c, 0.7, Response::film(), Balance::default()),
            r,
            photo,
        ),
        (
            c,
            negative(c, DEFAULT_GAMMA, Response::Original, Balance::default()),
            r,
            photo,
        ),
        (
            Calibration {
                dmin: c.dmin * 1.01,
                ..c
            },
            negative(
                Calibration {
                    dmin: c.dmin * 1.01,
                    ..c
                },
                DEFAULT_GAMMA,
                Response::film(),
                Balance::default(),
            ),
            r,
            photo,
        ),
        (
            Calibration {
                input_floor: c.input_floor * 2.,
                ..c
            },
            negative(
                Calibration {
                    input_floor: c.input_floor * 2.,
                    ..c
                },
                DEFAULT_GAMMA,
                Response::film(),
                Balance::default(),
            ),
            r,
            photo,
        ),
    ] {
        let changed =
            ProcessingParameters::positive(interpretation, roll, photo, camera_matrix()).unwrap();
        assert_ne!(ResponseKey::new(p), ResponseKey::new(changed));
        assert_ne!(key(p, false), key(changed, false));
    }
    let matrix = DMat3::IDENTITY;
    let changed = ProcessingParameters::positive(c, r, photo, matrix).unwrap();
    assert_eq!(ResponseKey::new(p), ResponseKey::new(changed));
    assert_ne!(
        key(p, false),
        PrefixKey::new([1, 0], matrix, field(), changed, false)
    );
    for stamp in [[2, 0], [1, 1]] {
        assert_ne!(
            key(p, false),
            PrefixKey::new(stamp, camera_matrix(), field(), p, false)
        );
    }
    let tile = SourceGeometry::new([100, 200, 1024, 256], [1024, 256]).unwrap();
    assert_ne!(
        key(p, false),
        PrefixKey::new(
            [1, 0],
            camera_matrix(),
            flat::FieldGeometry::uncorrected(Some(tile)),
            p,
            false
        )
    );
}

pub(super) fn read_words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
) -> Vec<u32> {
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &read, 0, buffer.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    read.map_async(wgpu::MapMode::Read, .., move |r| tx.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = read.get_mapped_range(..).unwrap();
    bytemuck::cast_slice(&mapped).to_vec()
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

pub(super) fn exact(case: &str, candidate: &str, expected: &[u32], actual: &[u32]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            a,
            b,
            "{case} {candidate} pixel={} channel={} actual={} reference={}",
            i / 4,
            i % 4,
            f32::from_bits(*a),
            f32::from_bits(*b)
        );
    }
}
