use super::*;
use crate::{application::test_support::with_application, looks::spektrafilm};
use std::time::Duration;

fn accepted_bw(e: &mut Editor) {
    let selection = crate::looks::monochrome::Selection {
        negative: crate::looks::monochrome::Negative::from_id("kodak_tmax_100_d76_7m30s"),
        paper: Default::default(),
    };
    e.document.state.project.output.film_and_paper = Some(Arc::new(
        crate::looks::monochrome::Model::new(selection)
            .bake(3, &AtomicBool::new(false))
            .unwrap(),
    ));
    e.document.state.project.output.lut = Some(super::tests::asset(0.3));
}

#[test]
fn declining_a_switch_keeps_earlier_preparation_alive_even_if_it_completes_under_the_dialog() {
    let (e, scheduler) = super::tests::editor();
    with_application(e, |app| {
        let paper = crate::looks::monochrome::Paper::default();
        let activity = app.enter(|e, cx| {
            e.select_paper(Some(paper.into()), cx);
            let activity = e.document.look.as_ref().unwrap().activity.clone();
            e.select_negative(
                spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                cx,
            );
            assert!(e.document.material_switch.is_some());
            assert_eq!(e.document.look.as_ref().unwrap().activity.clone(), activity);
            activity
        });
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        let accepted = app.read(|e| {
            assert!(e.document.material_switch.is_some());
            assert!(e.document.confirming() && !e.interaction_blocked());
            assert_eq!(
                e.material_selection(),
                Some(MaterialSelection::Monochrome(paper.into()))
            );
            e.document.state.project.clone()
        });
        app.enter(|e, cx| e.resolve_material_switch(false, cx));
        app.read(|e| {
            assert!(!e.document.confirming());
            assert_eq!(e.document.state.project, accepted);
            assert!(e.activities.view().is_none(), "retired look {activity:?}");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn confirmed_switch_failure_or_worker_cancel_never_replaces_either_accepted_slot() {
    for cancel in [false, true] {
        let (mut e, scheduler) = if cancel {
            super::tests::editor()
        } else {
            super::tests::editor_with_limits(wgpu::Limits {
                max_storage_buffer_binding_size: 1024,
                ..Default::default()
            })
        };
        accepted_bw(&mut e);
        let before = e.document.state.project.clone();
        let hold = cancel.then(crate::background::testing::pause);
        with_application(e, |app| {
            let activity = app.enter(|e, cx| {
                e.select_negative(
                    spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                    cx,
                );
                assert!(e.document.material_switch.is_some());
                assert!(e.document.look.is_none());
                e.resolve_material_switch(true, cx);
                assert!(e.document.material_switch.is_none());
                assert_eq!(e.document.state.project, before);
                e.document.look.as_ref().unwrap().activity.clone()
            });
            if cancel {
                app.pump();
                app.update(|_| activity.request_cancel());
                app.pump();
                drop(hold);
            }
            app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert!(!e.interaction_blocked() && e.document.material_switch.is_none());
                assert_eq!(
                    e.history_label(crate::editor::history::Direction::Undo),
                    None
                );
                assert_eq!(e.failure.is_some(), !cancel);
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn document_replacement_retires_an_unconfirmed_material_switch() {
    let (mut e, scheduler) = super::tests::editor();
    accepted_bw(&mut e);
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.select_negative(
                spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                cx,
            );
            assert!(e.document.material_switch.is_some());
            e.start_document(Preparation::New, cx);
        });
        app.run_until(Duration::from_secs(30), |e| {
            e.document_pending.is_none() && !e.interaction_blocked()
        });
        app.read(|e| {
            assert!(e.document.material_switch.is_none() && e.document.look.is_none());
            assert!(e.document.project.output.film_and_paper.is_none());
            assert!(e.failure.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
