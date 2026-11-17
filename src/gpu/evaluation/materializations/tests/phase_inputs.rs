use super::*;

fn image(device: &wgpu::Device, key: u32, value: f32) -> Image<ImageLayout> {
    let region: CellRegion = [0, 0, 2, 2].into();
    Image {
        inputs: Default::default(),
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
fn zero_retention_preserves_published_inputs_until_their_last_live_owner_retires() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(&device, CachePolicy { generated_bytes: 0 });
    let store = context.materializations();
    let input = image(&device, 1, 2.);
    store.submitted_image(&input);
    store.submitted_image(&input);
    assert_eq!(context.retained_bytes(), 0);
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
    assert!(store.find_ready::<ImageLayout>(&key, |_| true).is_none());
}

#[test]
fn published_inputs_compete_with_other_results_under_the_same_retention_budget() {
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
    let mut last = None;
    for index in 0..64 {
        let input = image(&device, index, index as f32);
        store.submitted_image(&input);
        last = Some(input.identity.clone());
        drop(input);
        assert!(context.retained_bytes() <= 128);
    }
    assert_eq!(context.retained_bytes(), 128);
    assert!(store.find_ready::<ImageLayout>(&key, |_| true).is_none());
    assert!(
        store
            .find_ready::<ImageLayout>(&last.unwrap(), |_| true)
            .is_some(),
        "a submitted input remains reusable after its producer retires"
    );
}

#[test]
fn repeated_publication_retains_one_physical_result_once() {
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
    store.submitted_image(&selected);
    drop(selected);
    let retained = store.find_ready::<ImageLayout>(&key, |_| true).unwrap();
    assert_eq!(context.retained_bytes(), 64);
    assert_eq!(
        execute::read(&device, &queue, retained.as_buffer().unwrap()),
        [[4., -0.25, 3., 9.]; 4]
    );
}

#[test]
fn submitted_use_can_restore_optional_ownership_after_pressure_but_discovery_cannot() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let store = context.materializations();
    let input = image(&device, 1, 3.);
    store.submitted_image(&input);
    assert_eq!(context.retained_bytes(), 64);
    store.set_retention_enabled(false);
    assert_eq!(context.retained_bytes(), 0);
    store.set_retention_enabled(true);
    drop(
        store
            .find_ready::<ImageLayout>(&input.identity, |_| true)
            .unwrap(),
    );
    assert_eq!(context.retained_bytes(), 0);
    let target = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(input.as_buffer().unwrap(), 0, &target, 0, 64);
    queue.submit([encoder.finish()]);
    store.submitted_values([&input.image]);
    assert_eq!(context.retained_bytes(), 64);
    assert_eq!(
        execute::read(&device, &queue, &target),
        [[3., -0.25, 3., 9.]; 4]
    );
    let key = input.identity.clone();
    drop(input);
    assert!(store.find_ready::<ImageLayout>(&key, |_| true).is_some());
    store.set_retention_enabled(false);
    assert!(store.find_ready::<ImageLayout>(&key, |_| true).is_none());
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
    store.submitted_image(&input);
    let published = store
        .find_ready::<ImageLayout>(&input.identity, |_| true)
        .unwrap();
    assert_eq!(context.retained_bytes(), 64);
    send.send(input.image.clone()).unwrap();
    let completed = pollster::block_on(pending).unwrap();
    assert!(Arc::ptr_eq(&completed.image, &published.image));
}
