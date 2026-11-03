use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn nested_production_reports_observed_source_work_submissions_allocations_and_live_scratch() {
    let (input, reads) = fixture([512, 256], vec![[2., -3., 4., 1.]; 512 * 256]);
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[512., 0.], [0., 256.]],
    };
    let inner = reconstruct_mapped(input, output(mapping, [0, 0, 1, 1].into()), mapping).unwrap();
    let root = ImageNode::new(inner.domain, Some(inner), Square);
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let budget = Budget {
        tile: [1, 1],
        scratch_bytes: 256 << 10,
    };
    let evaluation = context
        .request(
            root.clone(),
            Region {
                bounds: root.domain.finite().unwrap(),
                grid: root.domain.grid,
            },
            budget,
        )
        .unwrap();
    let plan = evaluation.plans().next().unwrap().unwrap();
    assert!(matches!(
        plan.tiles[0].fragment.work[0],
        Work::Produce { .. }
    ));
    let parent_bytes = plan.peak_scratch_bytes;

    for delivery in ["read", "sink", "production"] {
        reads.lock().unwrap().clear();
        let gate = crate::gpu::SubmissionGate::default();
        let submissions = Arc::new(AtomicUsize::new(0));
        let observed = submissions.clone();
        gate.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        let pixels = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        crate::gpu::test_support::take_evaluation_buffer_creations();
        let mut actual = Vec::new();
        let stats = match delivery {
            "read" => context
                .read(
                    &queue,
                    &gate,
                    &evaluation,
                    || false,
                    |_, values| {
                        actual.extend_from_slice(values);
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap(),
            "sink" => context
                .evaluate_with(
                    &queue,
                    &gate,
                    &evaluation,
                    || false,
                    Sink {
                        encode: |encoder: &mut wgpu::CommandEncoder,
                                 _: PlannedImage,
                                 input: &wgpu::Buffer| {
                            encoder.copy_buffer_to_buffer(input, 0, &pixels, 0, 16);
                            Ok(())
                        },
                        submitted: || {},
                    },
                )
                .unwrap()
                .unwrap(),
            _ => Production {
                context: &context,
                queue: &queue,
                gate: &gate,
                scope: &context.materializations(),
                budget,
                layout: ImageLayout {
                    domain: root.domain,
                    region: root.domain.finite().unwrap(),
                    format: ImageFormat::Rgba32Float,
                    backing: ImageBacking::Buffer,
                },
                pixels: &pixels,
                cancelled: &|| false,
                progress: &|| {},
            }
            .produce(&evaluation)
            .unwrap(),
        };
        let [buffers, _] = crate::gpu::test_support::take_evaluation_buffer_creations();
        assert_eq!(
            stats.submissions,
            submissions.load(Ordering::Relaxed),
            "{delivery}"
        );
        assert!(
            stats.submissions > 3,
            "include contributions and clear/final readouts"
        );
        assert_eq!(
            stats.buffer_allocations as u64,
            buffers + u64::from(delivery == "read"),
            "{delivery}: include captures, reduction storage and the host readback"
        );
        let reads = reads.lock().unwrap();
        let cells = |region: &Rect| u64::from(region.width) * u64::from(region.height);
        assert!(reads.len() > 1);
        assert_eq!(
            stats.source_cells,
            reads.iter().map(cells).sum::<u64>(),
            "{delivery}"
        );
        assert_eq!(stats.source_cells, 512 * 256);
        let peak_cells = reads.iter().map(cells).max().unwrap();
        assert_eq!(stats.peak_source_cells, peak_cells, "{delivery}");
        let largest_source = peak_cells * 16;
        assert!(
            stats.host_upload_bytes >= largest_source,
            "{delivery}: {stats:?}"
        );
        assert!(
            stats.scratch_bytes >= parent_bytes + largest_source + 20,
            "{delivery}: {stats:?}"
        );
        assert!(
            stats.scratch_bytes <= budget.scratch_bytes,
            "{delivery}: {stats:?}"
        );
        drop(reads);
        if delivery != "read" {
            actual = super::super::super::super::execute::read(&device, &queue, &pixels);
        }
        assert_eq!(actual, vec![[4., 9., 16., 9.]]);
    }
}
