use super::*;

#[test]
fn resident_linear_work_reaches_pixels_beyond_the_first_dispatch_row() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let (width, height) = (2560, 1800);
    let domain = Domain {
        bounds: Some(CellRegion::new([0, 0], [width, height])),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let root = pointwise(
        resident::raster_input(domain),
        Meaning::Graded,
        Function {
            code: r#"
fn mark(p: vec4f, position: vec2f) -> vec4f {
    return vec4f(position, 0.25, p.w);
}
"#
            .into(),
            symbol: "mark",
            bindings: vec![],
        },
    );
    let plan = Plan::new(
        root,
        domain.finite().unwrap(),
        Budget {
            tile: [width, height],
            scratch_bytes: 128 << 20,
        },
    )
    .unwrap();
    let prepared = Executor::with_programs(renderer.evaluation.programs.clone())
        .prepare(&device, plan)
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

#[test]
fn borrowed_storage_is_reusable_only_with_explicit_write_permission() {
    let domain = Domain {
        bounds: Some([0, 0, 16, 16].into()),
        grid: Grid::axis_aligned([0.; 2], [1.; 2]),
        meaning: Meaning::Graded,
    };
    let region = domain.finite().unwrap();
    let bytes = u64::from(region.width) * u64::from(region.height) * 16;
    for writable in [false, true] {
        let mut fragment = Fragment::default();
        let borrowed = fragment.resource(Storage::Input {
            slot: 0,
            bytes,
            writable,
        });
        let temporary = fragment.image(domain, region);
        let output = fragment.image(domain, region);
        for (source, target) in [
            (borrowed, temporary.resource),
            (temporary.resource, output.resource),
        ] {
            fragment.work.push(Work::Copy {
                source,
                target,
                source_offset: 0,
                target_offset: 0,
                bytes,
            });
        }
        let tile = plan::schedule(fragment, output).unwrap();
        assert_eq!(
            tile.assignment[borrowed.0] == tile.assignment[tile.result.resource.0],
            writable,
            "a retired borrowed allocation is writable scratch only with explicit permission"
        );
        assert_ne!(
            tile.assignment[temporary.resource.0], tile.assignment[tile.result.resource.0],
            "the input of the last copy is still live during that copy"
        );
    }
}
