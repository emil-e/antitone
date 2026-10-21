use super::*;
use crate::raw::sony_fixture as sony;

#[test]
fn sony_source_inspection_and_positive_share_automatic_development() {
    let order = sony::Order::Little;
    let mut tags = sony::profile_tags(order);
    tags.push((0x7032, 8, order.signed(&[2, 8192, 8192])));
    let file = sony::source(order, tags, false, false);
    editor_harness(|mut h, _| {
        // The authored TIFF has sensor/lens data but no maker-note WB; declare fixed roll gains.
        h.update(|editor| {
            editor.document.project.development =
                Some(Development::new(crate::raw::version(), [1.; 4]))
        });
        h.update(|editor| editor.add_raws(vec![file.path().into()]));
        wait_for(&mut h, |e| {
            !e.busy && e.document_pending.is_none() && e.working.support_ready()
        });
        h.read(|editor| assert!(editor.source));
        h.render().unwrap();
        h.step();
        let accepted = h.read(|editor| editor.document.project.clone());
        let development = h.read(|editor| {
            editor
                .document
                .visit
                .ready
                .as_ref()
                .unwrap()
                .record
                .development
                .clone()
        });
        assert!(development.lens_correction.is_some());
        assert!(development.input_floor > crate::processing::CAMERA_INPUT_FLOOR);
        let original = h.read(|editor| editor.working.regional().unwrap().clone());
        assert!(original.spec.is_inspection());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        h.step();
        enter_numeric(&mut h, "Exposure", "0.37");
        h.read(|editor| assert_eq!(editor.source_exposure, 0.37));
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
            .click();
        h.step();
        h.render().unwrap();
        h.read(|editor| assert!(editor.working.regional().unwrap().same_source_as(&original)));
        h.read(|editor| assert_eq!(editor.document.project, accepted));

        h.update(|editor| editor.dmin = Some(DVec3::splat(0.7)));
        h.update(|editor| editor.dmax = Some(DVec3::splat(0.01)));
        h.update(|editor| editor.sync_document());
        let calibrated = h.read(|editor| editor.document.project.clone());
        for source in [false, true] {
            h.get_by_label("View source").click();
            h.step();
            wait_for(&mut h, |e| {
                e.working.support_ready()
                    && e.working
                        .displayed
                        .as_ref()
                        .is_some_and(|v| v.spec.is_inspection() == source)
            });
            h.render().unwrap();
            h.read(|editor| assert_eq!(editor.source, source));
            h.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .visit
                        .ready
                        .as_ref()
                        .unwrap()
                        .record
                        .development,
                    development
                )
            });
            h.read(|editor| assert_eq!(editor.document.project, calibrated));
        }
        h.read(|editor| assert_eq!(editor.source_exposure, 0.37));
    });
}

#[test]
fn physical_view_uncalibrated_source_keeps_detail_and_underlay_through_inspection_and_navigation() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| {
            editor.thumbnails.set_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            })
        });
        harness.update(|editor| editor.add_raws(vec![fixture("patches.dng")]));
        wait_for(&mut harness, |e| {
            !e.busy && e.document_pending.is_none() && e.working.support_ready()
        });
        harness.read(|editor| assert!(editor.source));
        harness.read(|editor| assert!(editor.calibration().is_none()));
        stage(&mut harness, "Rendering");
        harness.update(|editor| editor.viewport.zoom = Some(4.));
        harness.step();
        wait_for(&mut harness, |e| e.working.support_ready());
        harness.render().unwrap();
        let retained = harness.read(|editor| editor.working.regional().unwrap().clone());
        assert!(retained.spec.is_inspection());
        let mut project = harness.read(|editor| editor.document.project.clone());
        // A remembered positive comparison must neither replace source working detail on idle nor
        // force an optional reference past the zero budget while its control is hidden.
        harness.update(|editor| editor.working.compare = true);
        let jobs = harness.read(|editor| editor.workflow_jobs.clone());
        let hold = crate::editor::tests::hold_blocking_work(&jobs);
        for (index, delta) in [egui::vec2(400., 0.), egui::vec2(-800., 100.)]
            .into_iter()
            .enumerate()
        {
            let layout = harness.read(|editor| editor.last_layout.unwrap().0);
            harness.update(|editor| editor.viewport.pan(layout, 1., delta));
            harness.step();
            harness.step();
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            enter_numeric(
                &mut harness,
                "Exposure",
                if index == 0 { "0.37" } else { "1.2" },
            );
            harness.read(|editor| {
                assert_eq!(editor.source_exposure, if index == 0 { 0.37 } else { 1.2 })
            });
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            if index == 1 {
                select_target(&mut harness, OutputTarget::Srgb);
                project.output.tiff = crate::project::TiffTarget::Srgb;
            }
            wait_for(&mut harness, |e| {
                e.working.displayed.as_ref().is_some_and(|v| {
                    v.spec.frame == e.working.current_frame().unwrap()
                        && Some(&v.processing) == e.working.processing()
                        && v.target == e.target
                })
            });
            harness.render().unwrap();
            harness.read(|editor| assert!(!editor.working.support_ready()));
            harness.read(|editor| {
                assert!(editor.working.regional().unwrap().same_source_as(&retained))
            });
            harness.read(|editor| {
                assert!(
                    editor
                        .working
                        .displayed
                        .as_ref()
                        .unwrap()
                        .spec
                        .is_inspection()
                )
            });
            harness.read(|editor| {
                assert!(
                    editor.scope_request().is_err(),
                    "source inspection still suspends photographic scopes"
                )
            });
            harness.read(|editor| assert!(!editor.working.reference_ready()));
            harness.read(|editor| assert!(editor.original.refining.is_none()));
            harness.read(|editor| {
                assert_eq!(
                    editor.document.project, project,
                    "inspection EV and navigation do not alter the document"
                )
            });
            harness.read(|editor| assert!(
            !Arc::ptr_eq(
                editor.working.displayed.as_ref().unwrap(),
                editor.working.regional().unwrap()
            ),
            "the physical-frame underlay composite, not only a transformed regional patch, is painted"
        ));
            let held = harness.read(|editor| editor.working.displayed.as_ref().unwrap().clone());
            for _ in 0..5 {
                harness.step();
            }
            harness.render().unwrap();
            harness.read(|editor| {
                assert!(
                    Arc::ptr_eq(&held, editor.working.displayed.as_ref().unwrap()),
                    "idle does not replace source detail"
                )
            });
            harness.read(|editor| assert!(editor.document.visit.working.task.is_some()));
        }
        drop(hold);
        wait_for(&mut harness, |e| e.working.support_ready());
        harness.render().unwrap();
        harness
            .read(|editor| assert!(!editor.working.regional().unwrap().same_source_as(&retained)));
        harness.read(|editor| {
            assert!(Arc::ptr_eq(
                editor.working.displayed.as_ref().unwrap(),
                editor.working.regional().unwrap()
            ))
        });
    });
}
