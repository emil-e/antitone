use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn related_small_reconstructions_keep_resident_callable_execution() {
    let (input, reads) = fixture(
        [128, 64],
        (0..128 * 64)
            .map(|i| [(i % 128) as f32, (i / 128) as f32 - 10., -4., 1.])
            .collect(),
    );
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.test_materializations(false);
    let budget = Budget {
        tile: [128, 64],
        scratch_bytes: 1 << 20,
    };
    let resident = pollster::block_on(
        context
            .materialize(
                &queue,
                &Default::default(),
                &scope,
                context
                    .request(
                        input.clone(),
                        Region {
                            bounds: input.domain.finite().unwrap(),
                            grid: input.domain.grid,
                        },
                        budget,
                    )
                    .unwrap(),
                ImageFormat::Rgba32Float,
                ImageBacking::Buffer,
            )
            .unwrap(),
    )
    .unwrap();
    reads.lock().unwrap().clear();
    let mapping = Mapping {
        start: [0.; 2],
        axes: [[2., 0.], [0., 2.]],
    };
    let evaluations = [[2, 2], [3, 1]]
        .map(|size| {
            let region = Rect::new([0, 0], size);
            let root = reconstruct_mapped(input.clone(), output(mapping, region), mapping).unwrap();
            context
                .request(
                    root.clone(),
                    Region {
                        bounds: region,
                        grid: root.domain.grid,
                    },
                    budget,
                )
                .unwrap()
        })
        .to_vec();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let gate = crate::gpu::submission::SubmissionGate::default();
    gate.observe_submissions(Arc::new(move || {
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    let images = pollster::block_on(futures_util::future::try_join_all(
        context
            .materialize_many(&queue, &gate, &scope, evaluations)
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        count.load(Ordering::Relaxed),
        2,
        "one fused output submission per root; resident inputs need no separate transfer or accumulation submissions"
    );
    assert!(reads.lock().unwrap().is_empty());
    for (image, width) in images.iter().zip([2, 3]) {
        for (i, pixel) in
            super::super::super::super::execute::read(&device, &queue, image.as_buffer().unwrap())
                .iter()
                .enumerate()
        {
            assert_eq!(
                *pixel,
                [
                    2. * (i % width) as f32 + 0.5,
                    2. * (i / width) as f32 - 9.5,
                    -4.,
                    9.
                ]
            );
        }
    }
    drop(resident);
}
