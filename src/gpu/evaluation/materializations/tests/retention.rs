use super::*;
mod execution;
use super::super::RequestPolicy;
use std::future::Future;

#[test]
fn live_requests_can_gain_optional_ownership_from_an_independent_consumer() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    for pending in [false, true] {
        let context = Context::new(
            &device,
            CachePolicy {
                generated_bytes: 128,
            },
        );
        let scope = context.materializations();
        scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
        let value = ImageValue::new(buffer(&device, 7.));
        let (send, receive) = futures_channel::oneshot::channel();
        let mut send = Some(send);
        let key = RetainedKey::new(("live then optional", pending));
        let mut live = Box::pin(scope.obtain_for(
            key.clone(),
            Arc::new(64u64),
            64,
            RequestPolicy {
                budget: None,
                retain: false,
            },
            move || async move { receive.await.map_err(|_| Error::Cancelled) },
        ));
        let mut task = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        assert!(live.as_mut().poll(&mut task).is_pending());
        let held = if pending {
            None
        } else {
            send.take().unwrap().send(value.clone()).unwrap();
            Some(pollster::block_on(live.as_mut()).unwrap())
        };
        assert_eq!(
            context.retained_bytes(),
            0,
            "the first consumer did not select optional ownership"
        );
        let optional = scope.obtain(key.clone(), Arc::new(64u64), 64, || async {
            panic!("a joined request must not recompute")
        });
        if pending {
            send.take().unwrap().send(value.clone()).unwrap();
        }
        let joined = pollster::block_on(optional).unwrap();
        let first = held.unwrap_or_else(|| pollster::block_on(live.as_mut()).unwrap());
        assert!(Arc::ptr_eq(&first.image, &joined.image));
        assert_eq!(context.retained_bytes(), 64);
        drop((first, joined, live, value));
        assert!(
            scope.find_ready::<u64>(&key, |_| true).is_some(),
            "the independent optional consumer keeps its ownership after live consumers retire"
        );
    }
}

#[test]
fn live_consumers_do_not_remove_an_independently_retained_value() {
    let (device, _, _) = crate::gpu::test_support::render_context();
    let context = Context::new(
        &device,
        CachePolicy {
            generated_bytes: 128,
        },
    );
    let scope = context.materializations();
    scope.set_test_pressure(crate::sys::MemoryPressure::Normal);
    let value = ImageValue::new(buffer(&device, 4.));
    let key = RetainedKey::new("optional then live");
    let first = pollster::block_on(scope.obtain(
        key.clone(),
        Arc::new(64u64),
        64,
        move || async move { Ok(value) },
    ))
    .unwrap();
    let joined = pollster::block_on(scope.obtain_for(
        key.clone(),
        Arc::new(64u64),
        64,
        RequestPolicy {
            budget: None,
            retain: false,
        },
        || async { panic!("live join must consume the existing value") },
    ))
    .unwrap();
    assert!(Arc::ptr_eq(&first.image, &joined.image));
    drop((first, joined));
    assert_eq!(context.retained_bytes(), 64);
    assert!(scope.find_ready::<u64>(&key, |_| true).is_some());
}
