use super::*;
use wgpu::util::DeviceExt;
fn buffer(device: &wgpu::Device, value: f32) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[value, -0.25, 3., 9.]; 4]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    })
}
#[test]
fn image_retention_is_scoped_budgeted_and_lru_without_owning_active_consumers() {
    let (device, queue, renderer) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let cache = context.image_cache(2);
    let a = buffer(&device, 1.);
    let b = buffer(&device, 2.);
    let c = buffer(&device, 3.);
    let key = |n| RetainedKey::new(n);
    assert!(cache.retain(key(1u32), Arc::new(4u32), &a));
    assert!(cache.retain(key(2u32), Arc::new(4u32), &b));
    assert_eq!(context.retained_bytes(), 128);
    assert!(
        cache
            .find::<u32>(&key(1u32), |support| *support >= 3)
            .is_some()
    );
    assert!(
        cache
            .find::<u32>(&key(1u32), |support| *support >= 5)
            .is_none()
    );
    assert!(cache.find::<u64>(&key(1u32), |_| true).is_none());
    assert!(cache.retain(key(3u32), Arc::new(4u32), &c));
    assert!(cache.find::<u32>(&key(2u32), |_| true).is_none());
    assert_eq!(context.retained_bytes(), 128);
    let active = cache.find::<u32>(&key(1u32), |_| true).unwrap();
    let scope = cache.clone();
    drop(cache);
    assert_eq!(context.retained_bytes(), 128);
    scope.clear();
    assert_eq!(context.retained_bytes(), 0);
    assert_eq!(
        execute::read(&device, &queue, &active.pixels),
        [[1., -0.25, 3., 9.]; 4]
    );
    drop(renderer);
    assert!(scope.is_empty());
}
#[test]
fn disabled_or_busy_retention_cannot_hide_required_images_and_shared_payloads_charge_once() {
    let (device, queue, _renderer) = crate::gpu::test_support::render_context();
    let pixels = buffer(&device, 2.);
    for budget in [0, 63] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: budget,
            },
        );
        let cache = context.image_cache(2);
        assert!(!cache.retain(RetainedKey::new(1u32), Arc::new(4u32), &pixels));
        assert!(cache.is_empty());
        assert_eq!(context.retained_bytes(), 0);
        assert_eq!(
            execute::read(&device, &queue, &pixels),
            [[2., -0.25, 3., 9.]; 4]
        );
    }
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let first = context.image_cache(2);
    let second = context.image_cache(1);
    assert!(first.retain(RetainedKey::new(1u32), Arc::new(4u32), &pixels));
    assert!(first.retain(RetainedKey::new(1u32), Arc::new(3u32), &pixels));
    assert_eq!(first.len(), 2);
    assert_eq!(context.retained_bytes(), 64);
    assert!(!second.retain(RetainedKey::new(2u32), Arc::new(4u32), &buffer(&device, 3.)));
    assert!(!first.belongs_to(&Context::new(&device, CachePolicy::default())));
    drop(first);
    assert_eq!(context.retained_bytes(), 0);
    assert!(second.retain(RetainedKey::new(2u32), Arc::new(4u32), &pixels));
    drop(second);
    assert_eq!(context.retained_bytes(), 0);
}
