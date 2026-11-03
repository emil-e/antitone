use super::*;
use crate::{
    adjustment_layers::{
        AdjustmentLayer, AdjustmentLayerCoverage, AdjustmentLayerSettings, AdjustmentLayerSnapshot,
    },
    geometry::{SourcePoint, SourceRegion},
    gpu::Grading,
    processing::RollSettings,
};
use glam::DVec3;
use wgpu::util::DeviceExt;

fn buffer(device: &wgpu::Device, values: &[[f32; 4]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("captured neutral test samples"),
        contents: bytemuck::cast_slice(values),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    })
}

fn record(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &crate::renderer::GpuRenderer,
    root: Arc<ImageNode>,
    region: CellRegion,
    input: &wgpu::Buffer,
) -> (Vec<[f32; 4]>, usize) {
    let plan = Plan::with_policy(
        root,
        region,
        Budget {
            tile: [region.width, region.height],
            scratch_bytes: 8 << 20,
        },
        Policy::Composed,
    )
    .unwrap();
    let computes = plan.tiles[0]
        .fragment
        .work
        .iter()
        .filter(|work| matches!(work, Work::Compute { .. }))
        .count();
    let recorded = Executor::with_programs(renderer.evaluation.programs.clone())
        .prepare(device, plan)
        .unwrap();
    let destination = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("requested photographic region"),
        size: u64::from(region.width) * u64::from(region.height) * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        recorded.encode(device, &mut encoder, input).is_err(),
        "a borrowed source cannot be repurposed as writable storage"
    );
    recorded
        .encode_into(device, &mut encoder, input, &destination)
        .unwrap();
    queue.submit([encoder.finish()]);
    (execute::read(device, queue, &destination), computes)
}

#[test]
fn photographic_constants_and_layer_queries_can_be_captured_without_a_device() {
    let area = SourceRegion::from([11, 17, 8, 6]);
    let layer = AdjustmentLayer::global(AdjustmentLayerSettings {
        exposure: 0.5,
        ..Default::default()
    });
    let input = crate::gpu::ProcessingInput::new(
        &wgpu::Limits::default(),
        crate::processing::InputSettings::Digital,
        RollSettings::default(),
        AdjustmentLayerSnapshot::new(vec![layer]).unwrap(),
        glam::DMat3::IDENTITY,
        area,
    )
    .unwrap();
    let source = borrowed_input(Domain {
        bounds: Some(area.try_into().unwrap()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    });
    let grade = input.grade().unwrap().compose(source.clone()).unwrap();
    let weights = Grading::weight_image(source, layer).unwrap();
    for node in [grade, weights] {
        let function = node.operation.pointwise().unwrap();
        assert!(
            function.bindings.iter().all(|(_, binding)| matches!(
                binding,
                FunctionBinding::Uniform(_) | FunctionBinding::Storage(_)
            )),
            "logical operations capture data, not allocated GPU buffers"
        );
    }
}

#[test]
fn a_borrowed_source_can_deliver_a_whole_or_partial_region_without_processing() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: Some([11, 17, 8, 5].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Neutral,
    };
    let pixels: Vec<_> = (0..40)
        .map(|i| [i as f32 * 0.1 - 0.4, i as f32, 1.5, 0.])
        .collect();
    let input = buffer(&device, &pixels);
    for region in [domain.finite().unwrap(), CellRegion::from([13, 18, 3, 2])] {
        let (actual, _) = record(
            &device,
            &queue,
            &renderer,
            borrowed_input(domain),
            region,
            &input,
        );
        let expected: Vec<_> = (region.y..region.bottom())
            .flat_map(|y| {
                (region.x..region.right()).map(move |x| domain.finite().unwrap().index([x, y]))
            })
            .map(|i| pixels[i])
            .collect();
        assert_eq!(actual, expected);
    }
    assert_eq!(execute::read(&device, &queue, &input), pixels);
}

#[test]
fn a_region_can_stop_after_roll_or_include_layers_without_mutating_its_neutral_source() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: Some([0, 0, 9, 7].into()),
        grid: Grid {
            origin: [51., 73.],
            axes: [[1.2, 0.3], [-0.2, 1.1]],
            pitch: [1.3, 1.2],
            coverage: 8,
        },
        meaning: Meaning::Neutral,
    };
    let area = [45, 70, 48, 48].into();
    let roll = RollSettings {
        exposure: 1.,
        ..Default::default()
    };
    let global = AdjustmentLayer::global(AdjustmentLayerSettings {
        exposure: -2.,
        ..Default::default()
    });
    let local = AdjustmentLayer {
        mask: AdjustmentLayerCoverage::Linear {
            start: SourcePoint([45., 70.]),
            end: SourcePoint([93., 118.]),
        },
        strength: 0.5,
        ..AdjustmentLayer::global(AdjustmentLayerSettings {
            exposure: 2.,
            ..Default::default()
        })
    };
    let snapshot = AdjustmentLayerSnapshot::new(vec![global, local]).unwrap();
    let gpu_grade = Grading::new(&device.limits(), roll, snapshot, area).unwrap();
    let grading = gpu_grade.for_grid(domain.grid.coverage);
    let pixels: Vec<_> = (0..63)
        .map(|i| {
            [
                (i % 9) as f32 * 0.13 - 0.5,
                0.3 + i as f32 * 0.025,
                0.7,
                if i % 4 == 0 { 0. } else { (8 | (i % 4)) as f32 },
            ]
        })
        .collect();
    assert!(pixels.iter().any(|p| p[0] < 0.) && pixels.iter().any(|p| p[1] > 1.));
    let source = buffer(&device, &pixels);
    let root = borrowed_input(domain);
    let roll_image = grading.roll(root.clone()).unwrap();
    let full_image = grading.compose(root).unwrap();
    let roll_function = roll_image.operation.pointwise().unwrap();
    assert!(
        roll_function
            .bindings
            .iter()
            .all(|(_, binding)| matches!(binding, FunctionBinding::Uniform(_))),
        "Roll constants stay captured data until GPU planning"
    );
    let region: CellRegion = [2, 1, 5, 4].into();
    let (roll_pixels, roll_work) = record(&device, &queue, &renderer, roll_image, region, &source);
    let (full_pixels, full_work) = record(&device, &queue, &renderer, full_image, region, &source);
    assert_eq!(
        full_work, roll_work,
        "compatible Roll and layer functions do not require another traversal"
    );
    assert_eq!(
        execute::read(&device, &queue, &source),
        pixels,
        "prefix and full requests preserve the captured neutral samples"
    );
    let mut effect = 0_f64;
    let mut signed_and_hdr = [false; 2];
    for (i, (prefix, complete)) in roll_pixels.iter().zip(&full_pixels).enumerate() {
        let x = region.x + (i as u32 % region.width) as i32;
        let y = region.y + (i as u32 / region.width) as i32;
        let input = pixels[domain.finite().unwrap().index([x, y])];
        if input[3] as u32 & 8 == 0 {
            assert_eq!(*prefix, [0.; 4]);
            assert_eq!(*complete, [0.; 4]);
            continue;
        }
        signed_and_hdr[0] |= input[0] < 0.;
        signed_and_hdr[1] |= input[1] > 1.;
        let expected_roll = DVec3::new(input[0] as f64, input[1] as f64, input[2] as f64) * 2.;
        let [sx, sy] = domain.point([x as f64 + 0.5, y as f64 + 0.5]);
        // A smoothstep on a 48×48 diagonal, half strength; weights scale stops, not pixels.
        let t = ((sx - 45. + sy - 70.) / 96.).clamp(0., 1.);
        let weight = t * t * (3. - 2. * t) * 0.5;
        let expected_full = expected_roll * 0.25 * (2. * weight).exp2();
        effect = effect.max((expected_full - expected_roll).abs().max_element());
        for (actual, expected) in [(prefix, expected_roll), (complete, expected_full)] {
            assert!(actual.iter().all(|v| v.is_finite()));
            assert_eq!(actual[3], input[3]);
            let actual = DVec3::new(actual[0] as f64, actual[1] as f64, actual[2] as f64);
            assert!(
                (actual - expected).abs().max_element() < 2e-5,
                "{actual:?} vs {expected:?}"
            );
        }
    }
    assert_eq!(
        signed_and_hdr,
        [true, true],
        "the requested covered region must exercise signed and HDR input"
    );
    assert!(
        effect > 0.01,
        "the fixture must distinguish a Roll prefix from the complete layer stack"
    );
}

#[test]
fn private_module_slots_preserve_each_captured_resource_when_fused() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let domain = Domain {
        bounds: Some([0, 0, 3, 2].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let pixels = [[-0.2, 0.4, 1.3, 0.]; 6];
    let input = buffer(&device, &pixels);
    let uniform = |value: [f32; 4]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&value),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let first = uniform([2., 0.25, 0., 0.]);
    let second = uniform([3., 0.5, 0., 0.]);
    let code: Arc<str> = r#"
@group(0) @binding(0)
var<uniform> c: vec4f;

fn times(p: vec4f, pos: vec2f) -> vec4f {
    return vec4f(p.rgb * c.x, p.w);
}

fn add(p: vec4f, pos: vec2f) -> vec4f {
    return vec4f(p.rgb + vec3f(c.y), p.w);
}
"#
    .into();
    let independent: Arc<str> = r#"
@group(0) @binding(0)
var<uniform> d: vec4f;

fn add_other(p: vec4f, pos: vec2f) -> vec4f {
    return vec4f(p.rgb + vec3f(d.y), p.w);
}
"#
    .into();
    let mut work = Vec::new();
    for (resource, bias, module, symbol) in [
        (&first, 0.25, code.clone(), "add"),
        (&second, 0.5, code.clone(), "add"),
        (&first, 0.25, independent, "add_other"),
    ] {
        let root = pointwise(
            borrowed_input(domain),
            Meaning::Graded,
            Function {
                code: code.clone(),
                symbol: "times",
                bindings: vec![(0, Shared::Buffer(first.clone()).into())],
            },
        );
        let root = pointwise(
            root,
            Meaning::Graded,
            Function {
                code: module,
                symbol,
                bindings: vec![(0, Shared::Buffer(resource.clone()).into())],
            },
        );
        let (actual, count) = record(
            &device,
            &queue,
            &renderer,
            root,
            domain.finite().unwrap(),
            &input,
        );
        for (actual, source) in actual.iter().zip(pixels) {
            for c in 0..3 {
                assert!((actual[c] - (source[c] * 2. + bias)).abs() < 1e-6);
            }
        }
        work.push(count);
    }
    assert_eq!(
        work,
        [1, 1, 1],
        "linking private module slots does not require an intermediate image"
    );
}
