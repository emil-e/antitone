use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        AdjustmentLayerSettings, OutputTarget, ProcessingParameters, RollSettings, tests::fixture,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[test]
fn grade_owner_survives_output_and_ui_changes_but_retires_for_effective_preview_content() {
    let mut e = fixture();
    crate::editor::tests::add_ready_fixture(&mut e);
    let device = e.document.visit.working_view.device().unwrap().clone();
    let parameters = |ev| {
        ProcessingInput::from(
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
                crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                    exposure: ev,
                    ..Default::default()
                }),
            ])
            .and_then(|snapshot| {
                crate::gpu::GpuPipeline::new(
                    &device,
                    e.calibration().unwrap(),
                    RollSettings::default(),
                    snapshot,
                    glam::DMat3::IDENTITY,
                    e.document.visit.ready.as_ref().unwrap().area,
                )
            })
            .map(Arc::new)
            .unwrap(),
        )
    };
    let accepted = parameters(0.);
    let preview = parameters(0.2);
    let dropped = Arc::new(AtomicBool::new(false));
    with_application(e, |app| {
        let before = app.read(|e| e.document.state.project.clone());
        app.enter(|e, access| {
            e.observe_grade(&accepted);
            let guard = Dropped(dropped.clone());
            e.document.visit.grade.task = access.spawn(move |_| async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
        });
        app.pump();
        app.update(|e| {
            e.document.target = OutputTarget::Srgb;
            e.tab = 2;
            e.source_exposure = 1.;
            e.observe_grade(&accepted);
        });
        app.pump();
        assert!(!dropped.load(Ordering::Relaxed));
        app.update(|e| e.observe_grade(&preview));
        app.pump();
        assert!(dropped.load(Ordering::Relaxed));
        app.read(|e| {
            assert!(e.document.visit.grade.matches(&preview));
            assert_eq!(
                e.document.state.project, before,
                "an admitted visual snapshot is not a persisted edit"
            );
        });
        app.update(|e| e.observe_grade(&accepted));
        app.read(|e| assert!(e.document.visit.grade.matches(&accepted)));
    });
}

#[test]
fn inspection_ev_is_output_demand_not_a_photographic_grade() {
    let scope = GradeScope::new(&ProcessingParameters::source_inspection(0.).unwrap().into());
    for ev in [-2., 0., 1.2, 3.] {
        assert!(scope.matches(&ProcessingParameters::source_inspection(ev).unwrap().into()));
    }
}
