use super::*;
use crate::geometry::SourceRegion as Rect;
#[test]
fn resident_linear_work_reaches_pixels_beyond_the_first_dispatch_row() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (width, height) = (2560, 1800);
    let domain = Domain {
        bounds: [0, 0, width, height].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let root = pointwise(
        resident::raster_input(domain),
        Meaning::Graded,
        Function {
            code: "fn mark(p:vec4f,position:vec2f)->vec4f{return vec4f(position,0.25,p.w);}".into(),
            symbol: "mark",
            bindings: vec![],
        },
    );
    let plan = Plan::new(
        root,
        domain.bounds,
        Budget {
            tile: [width, height],
            scratch_bytes: 128 << 20,
        },
    )
    .unwrap();
    let prepared = Executor::with_programs(renderer.pull_programs.clone())
        .prepare(&device, plan, vec![])
        .unwrap();
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("large resident image"),
        size: u64::from(width) * u64::from(height) * 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let first_row_cells = device.limits().max_compute_workgroups_per_dimension * 64;
    assert!(
        first_row_cells < width * height,
        "fixture must cross the device's one-dimensional dispatch limit"
    );
    let indices = [0, first_row_cells - 1, first_row_cells, width * height - 1];
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dispatch boundary probes"),
        size: 64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    prepared.encode(&device, &mut encoder, &input).unwrap();
    for (probe, index) in indices.into_iter().enumerate() {
        encoder.copy_buffer_to_buffer(
            &input,
            u64::from(index) * 16,
            &output,
            probe as u64 * 16,
            16,
        );
    }
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    output
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    let mapped = output.slice(..).get_mapped_range().unwrap();
    let pixels: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    for (pixel, index) in pixels.iter().zip(indices) {
        assert_eq!(
            *pixel,
            [
                (index % width) as f32 + 0.5,
                (index / width) as f32 + 0.5,
                0.25,
                0.
            ],
            "dispatch probe at cell {index}"
        );
    }
}

struct Resident {
    writable: bool,
}
impl Operation for Resident {
    fn input(&self, _: Rect) -> Result<Option<Rect>, Error> {
        Ok(None)
    }
    fn lower(&self, f: &mut Fragment, _: Option<Image>, output: Image) -> Result<(), Error> {
        f.resources[output.resource.0] = Storage::Input {
            slot: 0,
            bytes: u64::from(output.cells()) * 16,
            writable: self.writable,
        };
        Ok(())
    }
}
struct InPlace;
impl Operation for InPlace {
    fn input(&self, r: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(r))
    }
    fn in_place(&self) -> bool {
        true
    }
    fn lower(&self, f: &mut Fragment, input: Option<Image>, output: Image) -> Result<(), Error> {
        assert_eq!(
            input.unwrap().resource,
            output.resource,
            "an in-place implementation receives one storage identity"
        );
        f.compute("@group(0) @binding(0) var<storage,read_write> p:array<vec4f>; @compute @workgroup_size(64) fn mutate(@builtin(global_invocation_id) id:vec3u,@builtin(num_workgroups) groups:vec3u){let index=id.x+id.y*groups.x*64u;if index<arrayLength(&p){p[index]=p[index]*2.0;}}","mutate",vec![(0,Binding::Write(output.resource))],output.cells());
        Ok(())
    }
}
#[test]
fn borrowed_storage_is_reusable_only_with_explicit_write_permission() {
    let domain = Domain {
        bounds: [0, 0, 16, 16].into(),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    for writable in [false, true] {
        let source = Layer::new(domain, None, Resident { writable });
        let root = Layer::new(domain, Some(source), InPlace);
        let plan = Plan::new(
            root,
            domain.bounds,
            Budget {
                tile: [16, 16],
                scratch_bytes: 1 << 20,
            },
        )
        .unwrap();
        let tile = &plan.tiles[0];
        if writable {
            assert!(matches!(
                tile.fragment.resources[tile.result.resource.0],
                Storage::Input { writable: true, .. }
            ));
            assert_eq!(
                plan.peak_scratch_bytes, 0,
                "caller-owned mutable storage needs no duplicate image"
            );
        } else {
            assert!(matches!(
                tile.fragment.resources[tile.result.resource.0],
                Storage::Buffer(_)
            ));
            let borrowed = tile
                .fragment
                .resources
                .iter()
                .position(|r| matches!(r, Storage::Input { .. }))
                .unwrap();
            assert_ne!(
                tile.assignment[borrowed], tile.assignment[tile.result.resource.0],
                "read-only source storage cannot become writable scratch"
            );
        }
    }
}
