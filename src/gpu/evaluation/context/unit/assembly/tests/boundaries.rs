use super::*;

#[test]
fn a_single_oversize_host_or_preexisting_gpu_value_keeps_one_unit_transfer_behavior() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let working = HostWorkingSet::default();
    let region = [0, 0, 4, 2].into();
    let destination = gpu(&device, &[[0.; 4]; 8]);
    let production = Production {
        worker: Worker {
            context: &context,
            queue: &queue,
            gate: &gate,
            scope: &scope,
            budget: Budget {
                tile: [4, 2],
                scratch_bytes: 32,
            },
            working: &working,
            cancelled: &|| false,
            progress: &|| {},
        },
        layout: layout(region, ImageBacking::Buffer),
        pixels: destination.as_buffer().unwrap(),
    };
    let mut copies = Copies::default();
    for (index, output) in [[0, 0, 2, 2].into(), [2, 0, 2, 2].into()]
        .into_iter()
        .enumerate()
    {
        let values = field(output);
        let image = if index == 0 {
            ImageValue::host(values)
        } else {
            gpu(&device, &values)
        };
        let weak = Arc::downgrade(&image);
        copies.copy(&production, image, output, output).unwrap();
        assert!(
            weak.upgrade().is_none(),
            "one value exceeding the grouping allowance transfers immediately rather than accumulating"
        );
        assert_eq!(copies.bytes, 0);
    }
    assert_eq!(copies.counts.submissions, 2);
    let actual = super::super::super::super::super::execute::read(
        &device,
        &queue,
        destination.as_buffer().unwrap(),
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&actual),
        bytemuck::cast_slice::<_, u8>(&field(region))
    );
}

#[test]
fn pending_unit_wait_drains_prior_copies_before_the_other_producer_continues() {
    use std::future::Future;
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let scope = context.materializations();
    let gate = crate::gpu::submission::SubmissionGate::default();
    let working = HostWorkingSet::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let cancelled = || std::time::Instant::now() >= deadline;
    let budget = Budget {
        tile: [4, 2],
        scratch_bytes: 128,
    };
    let worker = Worker {
        context: &context,
        queue: &queue,
        gate: &gate,
        scope: &scope,
        budget,
        working: &working,
        cancelled: &cancelled,
        progress: &|| {},
    };
    let region = [0, 0, 4, 2].into();
    let destination = gpu(&device, &[[0.; 4]; 8]);
    let production = Production {
        worker,
        layout: layout(region, ImageBacking::Buffer),
        pixels: destination.as_buffer().unwrap(),
    };
    let left = [0, 0, 2, 2].into();
    let right = [2, 0, 2, 2].into();
    let input = gpu(&device, &field(left));
    let weak = Arc::downgrade(&input);
    let mut copies = Copies::default();
    copies.copy(&production, input, left, left).unwrap();
    assert!(weak.upgrade().is_some());
    let calls = Arc::new(AtomicUsize::new(0));
    let root = Value::from_operation(domain(), [], Tiles(calls.clone()));
    let (mut waiting, producer) = scope.claim_unit(
        root.identity().unwrap().clone(),
        Arc::new(layout(right, ImageBacking::Host)),
        budget,
    );
    let mut task = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(
        std::pin::Pin::new(&mut waiting)
            .poll(&mut task)
            .is_pending()
    );
    let mut producer = producer;
    let mut drained = false;
    let unit = root.plan_unit(right, budget).unwrap();
    let image = worker
        .unit_image_before(&root, unit, &mut WorkCounts::default(), || {
            copies.flush(&production)?;
            assert!(
                weak.upgrade().is_none(),
                "release previous copy inputs before waiting for more work"
            );
            producer
                .take()
                .unwrap()
                .complete(Ok(ImageValue::host(field(right))));
            drained = true;
            Ok(())
        })
        .unwrap();
    assert!(drained);
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "the waiting consumer must not steal an independently running inline producer"
    );
    assert_eq!(image.as_host().unwrap(), field(right));
    copies.copy(&production, image, right, right).unwrap();
    copies.flush(&production).unwrap();
    assert_eq!(copies.counts.submissions, 2);
    let actual = super::super::super::super::super::execute::read(
        &device,
        &queue,
        destination.as_buffer().unwrap(),
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&actual),
        bytemuck::cast_slice::<_, u8>(&field(region))
    );
}
