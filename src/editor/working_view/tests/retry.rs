use super::*;

#[test]
fn changed_demand_retires_failure_but_repaints_do_not_restart_it() {
    let pixels = source::Context {
        reference: None,
        source: crate::editor::tests::developed_fixture(),
        correction: Default::default(),
    };
    let identity = crate::acquisition::camera::identity::Identity::camera(pixels.source.identity());
    let mut working = WorkingView::default();
    working.request(Demand::fixture(
        spec(0.).with_identity(identity.clone()),
        pixels.clone(),
    ));
    let wanted = working.wanted.clone().unwrap();
    let error = Error::GpuOutOfMemory("injected allocation failure".into());
    assert!(wanted.rendering.set(Err(error.clone())).is_ok());
    working.acquisition_failed(error.clone());
    assert!(working.acquisition().is_none());

    working.request((*wanted).clone());
    assert_eq!(working.error, Some(error.clone()));
    assert!(Arc::ptr_eq(working.wanted.as_ref().unwrap(), &wanted));
    assert!(
        working.acquisition().is_none(),
        "unchanged demand must stay failed"
    );

    working.request(Demand::fixture(spec(8.).with_identity(identity), pixels));
    assert!(working.error.is_none());
    assert!(working.acquisition().is_some());
    let changed = working.wanted.as_ref().unwrap();
    assert!(!wanted.same_calculation(changed));
    assert!(
        changed.rendering.get().is_none(),
        "the new request must not inherit a cached error"
    );

    working.acquired(Completion {
        owner: Arc::downgrade(&working.mailbox),
        demand: wanted,
        result: Err(error),
    });
    assert!(
        working.error.is_none(),
        "a superseded failure cannot fail the new request"
    );
    assert!(working.acquisition().is_some());

    working.acquisition_failed(Error::Gpu("injected calculation failure".into()));
    let mut revisit = working.for_visit();
    assert!(revisit.error.is_none());
    revisit.request((*working.wanted.as_ref().unwrap().as_ref()).clone());
    assert!(
        revisit.acquisition().is_some(),
        "a new visit must not inherit a failure"
    );
}
