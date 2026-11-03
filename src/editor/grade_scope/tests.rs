use super::*;
use crate::{
    application::test_support::with_application,
    editor::{AdjustmentLayerSettings, OutputTarget, tests::fixture},
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
    let recipe = |ev| {
        let mut recipe = e
            .positive_recipe(
                e.input(),
                e.document.roll,
                e.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        recipe.grade = crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: ev,
                ..Default::default()
            }),
        ])
        .unwrap();
        Arc::new(recipe)
    };
    let accepted = recipe(0.);
    let preview = recipe(0.2);
    let dropped = Arc::new(AtomicBool::new(false));
    with_application(e, |app| {
        let before = app.read(|e| e.document.state.project.clone());
        app.enter(|e, access| {
            e.observe_grade(Some(accepted.clone()));
            let guard = Dropped(dropped.clone());
            e.document.visit.canvas.grade.original.task = access.spawn(move |_| async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
        });
        app.pump();
        app.update(|e| {
            e.document.target = OutputTarget::Srgb;
            e.tab = 2;
            e.source_exposure = 1.;
            e.observe_grade(Some(accepted.clone()));
        });
        app.pump();
        assert!(!dropped.load(Ordering::Relaxed));
        let old_look = app.update(|e| {
            let look = Arc::new(
                crate::looks::LookAsset::new(
                    "Scope lifetime control".into(),
                    crate::looks::LookOrigin::Imported,
                    Arc::new(
                        crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.; 3]; 8]).unwrap(),
                    ),
                )
                .unwrap(),
            );
            let weak = Arc::downgrade(&look);
            let mut looked = (*accepted).clone();
            looked.luts = Some(look).into();
            e.observe_grade(Some(Arc::new(looked)));
            weak
        });
        app.pump();
        assert!(
            !dropped.load(Ordering::Relaxed),
            "look changes retain the photographic continuation"
        );
        assert!(old_look.upgrade().is_some());
        app.update(|e| e.observe_grade(Some(accepted.clone())));
        assert!(
            old_look.upgrade().is_none(),
            "the owner must release a superseded look payload"
        );
        app.update(|e| e.observe_grade(Some(preview.clone())));
        app.pump();
        assert!(dropped.load(Ordering::Relaxed));
        app.read(|e| {
            assert!(e.document.visit.canvas.grade.matches(Some(&preview)));
            assert_eq!(
                e.document.state.project, before,
                "an admitted visual snapshot is not a persisted edit"
            );
        });
        app.update(|e| e.observe_grade(Some(accepted.clone())));
        app.read(|e| assert!(e.document.visit.canvas.grade.matches(Some(&accepted))));
    });
}

#[test]
fn disabled_effect_controls_do_not_retire_the_recipe_owner_but_an_active_operation_does() {
    let mut e = fixture();
    crate::editor::tests::add_ready_fixture(&mut e);
    let mut recipe = e
        .positive_recipe(
            e.input(),
            e.document.roll,
            e.document.visit.adjustment_layer_settings,
        )
        .unwrap();
    recipe.grain.enabled = false;
    recipe.halation.enabled = false;
    let scope = GradeScope::new(Some(Arc::new(recipe.clone())));
    recipe.grain.image_sigma = 0.05;
    recipe.grain.amount = 0.;
    assert!(recipe.pixel_pipeline().unwrap().stages.is_empty());
    assert!(scope.matches(Some(&recipe)));
    recipe.grain.enabled = true;
    assert!(!recipe.pixel_pipeline().unwrap().stages.is_empty());
    assert!(!scope.matches(Some(&recipe)));
    assert!(
        !scope.matches(None),
        "inspection leaves photographic ownership"
    );
}
