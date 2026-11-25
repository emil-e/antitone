use super::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, PartialEq)]
struct Colliding(u32);

impl Hash for Colliding {
    fn hash<H: Hasher>(&self, state: &mut H) {
        0u32.hash(state);
    }
}

fn retain(scope: &Materializations, key: RetainedKey, value: f32) {
    pollster::block_on(scope.obtain(key, Arc::new(()), 16, move || async move {
        Ok(ImageValue::host(vec![[value, -0.25, 3., 9.]]))
    }))
    .unwrap();
}

#[test]
fn bounded_lookup_skips_unrelated_entries_but_checks_and_charges_collisions() {
    let (device, _, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 16 * 80,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let requested = RetainedKey::new(Colliding(1));
    let other = RetainedKey::new(Colliding(2));
    retain(&scope, requested.clone(), 1.);
    retain(&scope, other.clone(), 2.);
    for key in 0..64u32 {
        retain(&scope, RetainedKey::new(key), 100. + key as f32);
    }

    let mut remaining = 1;
    assert!(
        scope
            .ready_bounded::<()>(&requested, &mut remaining)
            .is_empty()
    );
    assert_eq!(remaining, 0, "a rejected collision still uses a probe");

    let mut remaining = 2;
    let result = scope.ready_bounded::<()>(&requested, &mut remaining);
    assert_eq!(
        result.len(),
        1,
        "unrelated newer entries must not exhaust the bound"
    );
    assert_eq!(result[0].1.as_host().unwrap(), [[1., -0.25, 3., 9.]]);
    assert_eq!(
        scope.ready::<()>(&other)[0].image.as_host().unwrap()[0][0],
        2.
    );
}

#[test]
fn promoting_a_colliding_entry_preserves_global_eviction_order() {
    let (device, _, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 32,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let older = RetainedKey::new(Colliding(1));
    let newer = RetainedKey::new(Colliding(2));
    retain(&scope, older.clone(), 1.);
    retain(&scope, newer.clone(), 2.);
    let older_image = pollster::block_on(scope.find::<()>(&older, |_| true).unwrap()).unwrap();

    let unrelated = RetainedKey::new(3u32);
    retain(&scope, unrelated.clone(), 3.);
    assert!(scope.ready::<()>(&newer).is_empty());
    assert_eq!(scope.ready::<()>(&unrelated).len(), 1);
    assert!(Arc::ptr_eq(
        &scope.ready::<()>(&older)[0].image,
        &older_image.image
    ));

    scope.evict_optional();
    assert_eq!(context.retained_bytes(), 0);
    assert!(scope.ready::<()>(&unrelated).is_empty());
    assert!(Arc::ptr_eq(
        &scope.ready::<()>(&older)[0].image,
        &older_image.image
    ));
    drop(older_image);
    assert!(scope.ready::<()>(&older).is_empty());
}

#[test]
fn separately_constructed_keys_share_the_same_exact_value() {
    fn publish_key() -> RetainedKey {
        RetainedKey::new((String::from("sample"), 3u32))
    }

    let (device, _, _) = crate::photographic::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 32,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    retain(&scope, publish_key(), 7.);

    let query = RetainedKey::new(("sample".to_owned(), 3u32));
    assert_eq!(query.get::<(String, u32)>().unwrap().1, 3);
    assert_eq!(
        scope.ready::<()>(&query)[0].image.as_host().unwrap()[0][0],
        7.
    );
    assert!(
        scope
            .ready::<()>(&RetainedKey::new(("sample".to_owned(), 4u32)))
            .is_empty()
    );
}

#[test]
fn equal_image_content_with_signed_zero_grids_uses_the_same_bucket() {
    let (_, _, renderer) = crate::photographic::test_support::render_context();
    let scope = renderer.evaluation().materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let region = CellRegion::new([0, 0], [1, 1]);
    let output_key = |origin| {
        Output::calculated(
            Domain {
                bounds: Some(region),
                grid: Grid::axis_aligned([origin, 0.], [1.; 2]),
            },
            region,
            Some(RetainedKey::new("constant plane")),
            [],
        )
        .identity()
        .unwrap()
        .clone()
    };
    retain(&scope, output_key(-0.), 1.);
    assert_eq!(scope.ready::<()>(&output_key(0.)).len(), 1);
    assert!(scope.ready::<()>(&output_key(0.5)).is_empty());
}
