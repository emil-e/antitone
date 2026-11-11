use super::*;

fn image(device: &wgpu::Device, key: u32, value: f32) -> Image<ImageLayout> {
    let region: CellRegion = [0, 0, 2, 2].into();
    Image {
        identity: RetainedKey::new(key),
        descriptor: Arc::new(ImageLayout {
            domain: Domain {
                bounds: Some(region),
                grid: Grid::axis_aligned([0.; 2], [1.; 2]),
                meaning: Meaning::Neutral,
            },
            region,
            format: ImageFormat::Rgba32Float,
            backing: ImageBacking::Buffer,
        }),
        image: ImageValue::new(buffer(device, value)),
    }
}

#[test]
fn phase_inputs_remain_discoverable_until_their_last_live_owner_retires() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let store = context.materializations();
    let input = image(&device, 1, 2.);
    store.submitted_input(&input);
    store.submitted_input(&input);
    assert_eq!(
        context.retained_bytes(),
        0,
        "publishing the same phase argument twice must not turn it into optionally retained output"
    );
    let joined = store
        .find_ready::<ImageLayout>(&input.identity, |_| true)
        .unwrap();
    assert!(Arc::ptr_eq(&joined.image, &input.image));
    let key = input.identity.clone();
    drop(input);
    assert_eq!(
        execute::read(&device, &queue, joined.as_buffer().unwrap()),
        [[2., -0.25, 3., 9.]; 4],
        "another live consumer keeps the real pixels usable"
    );
    drop(joined);
    assert!(
        store.find_ready::<ImageLayout>(&key, |_| true).is_none(),
        "an unused phase argument must release discovery rather than occupy the cache"
    );
}

#[test]
fn a_stream_of_phase_inputs_does_not_evict_a_selected_calculation_result() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let store = context.materializations();
    let selected = image(&device, 1000, 7.);
    let key = selected.identity.clone();
    store.submitted_image(&selected);
    drop(selected);
    for index in 0..64 {
        let input = image(&device, index, index as f32);
        store.submitted_input(&input);
        assert!(
            store
                .find_ready::<ImageLayout>(&input.identity, |_| true)
                .is_some()
        );
        drop(input);
    }
    assert_eq!(
        context.retained_bytes(),
        64,
        "only the selected output needs optional cache ownership"
    );
    assert!(
        store.find_ready::<ImageLayout>(&key, |_| true).is_some(),
        "transient input churn must not discard the selected reuse boundary"
    );
}

#[test]
fn phase_publication_preserves_an_independently_selected_results_retention() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let store = context.materializations();
    let selected = image(&device, 1, 4.);
    let key = selected.identity.clone();
    store.submitted_image(&selected);
    store.submitted_input(&selected);
    drop(selected);
    let retained = store.find_ready::<ImageLayout>(&key, |_| true).unwrap();
    assert_eq!(context.retained_bytes(), 64);
    assert_eq!(
        execute::read(&device, &queue, retained.as_buffer().unwrap()),
        [[4., -0.25, 3., 9.]; 4]
    );
}

#[test]
fn phase_publication_preserves_an_independent_pending_requests_optional_ownership() {
    use std::future::Future;
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let store = context.materializations();
    let input = image(&device, 1, 5.);
    let (send, receive) = futures_channel::oneshot::channel::<Arc<ImageValue>>();
    let mut pending = Box::pin(store.obtain(
        input.identity.clone(),
        input.descriptor.clone(),
        64,
        move || async move { receive.await.map_err(|_| Error::Cancelled) },
    ));
    let mut task = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(
        pending.as_mut().poll(&mut task).is_pending(),
        "the independent request must actually be pending when the phase publishes"
    );
    store.submitted_input(&input);
    let published = store
        .find_ready::<ImageLayout>(&input.identity, |_| true)
        .unwrap();
    assert_eq!(
        context.retained_bytes(),
        64,
        "an explicit pending calculation still selects this result for retention"
    );
    send.send(input.image.clone()).unwrap();
    let completed = pollster::block_on(pending).unwrap();
    assert!(Arc::ptr_eq(&completed.image, &published.image));
}
