use super::*;

#[test]
fn unsuccessful_allocation_cannot_publish_or_leave_an_initializer() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let scope = context.materializations();
    let region = CellRegion::new(
        [0, 0],
        [(device.limits().max_buffer_size / 16 + 1) as u32, 1],
    );
    let layout = ImageLayout {
        region,
        domain: Domain {
            bounds: Some(region),
            ..generated_layout().domain
        },
        ..generated_layout()
    };
    let key = RetainedKey::new("invalid allocation before publication");
    assert!(matches!(
        scope.generated_image(&device, &key, layout, false, true),
        Err(Error::Gpu(_))
    ));
    assert!(scope.find::<ImageLayout>(&key, |_| true).is_none());
    assert_eq!(context.retained_bytes(), 0);
}

#[test]
fn evicting_optional_images_keeps_chosen_limit_and_live_consumers() {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 64,
        },
    );
    let scope = context.test_materializations(true);
    let image = produce(&scope, 91, buffer(&device, 2.));
    assert_eq!(context.retained_bytes(), 64);
    scope.evict_optional();
    assert_eq!(context.retained_bytes(), 0);
    assert_eq!(context.cache_policy().generated_bytes, 64);
    assert_eq!(
        execute::read(&device, &queue, image.as_buffer().unwrap()),
        [[2., -0.25, 3., 9.]; 4]
    );
    assert!(
        !scope.ready::<u32>(&image.identity).is_empty(),
        "eviction does not revoke a live consumer"
    );
}
