use super::*;

fn exact_values(repair: &crate::dust_repair::EvaluatedRepair) -> Vec<([u32; 2], [u32; 3])> {
    repair
        .pixels()
        .map(|(point, value)| (point, value.map(f32::to_bits)))
        .collect()
}

#[test]
fn proper_thumbnail_pending_and_failed_repairs_use_local_status_and_clear_after_disable() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        wait_for(&mut h, |editor| {
            editor
                .thumbnails
                .ready_count(&editor.document, editor.target)
                == 1
        });
        let source = h.read(|editor| editor.current_source.unwrap());
        let area = h.read(|editor| editor.document.visit.ready.as_ref().unwrap().area);
        // Valid exterior support, but no source-sized donor: exercise healing failure, not mask rejection.
        let repair = h.update(|editor| {
            editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new(
                        [area.x, area.y],
                        [area.width, area.height],
                        (0..area.width * area.height)
                            .map(|index| {
                                let x = index % area.width;
                                let y = index / area.width;
                                x >= 8
                                    && y >= 8
                                    && x + 8 < area.width
                                    && y + 8 < area.height
                                    && (x == 8
                                        || y == 8
                                        || x + 9 == area.width
                                        || y + 9 == area.height)
                            })
                            .collect(),
                    )
                    .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    DonorChoice::Automatic,
                )
                .unwrap()
        });
        h.step();
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Image, "Photo updating")
                .is_some()
        );
        let started = Instant::now();
        while h
            .query_by_role_and_label(egui::accesskit::Role::Image, "Photo error")
            .is_none()
        {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "proper thumbnail did not report its failed repair"
            );
            std::thread::sleep(Duration::from_millis(5));
            h.step();
        }
        for size in [egui::vec2(1100., 800.), egui::vec2(800., 600.)] {
            h.set_size(size);
            h.step();
            let marker = h.get_by_role_and_label(egui::accesskit::Role::Image, "Photo error");
            assert!(h.ctx.content_rect().contains_rect(marker.rect()));
            let description = marker.accesskit_node().description().unwrap().to_owned();
            assert!(description.contains("repair 1") && description.contains("donor"));
            h.read(|editor| {
                assert!(
                    editor.failure.is_none(),
                    "background card failure must not seize global focus"
                )
            });
            save_capture(
                &h.render().unwrap(),
                &format!(
                    "repair-thumbnail-failed-{}x{}",
                    size.x as u32, size.y as u32
                ),
            );
        }
        h.update(|editor| {
            editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .repairs
                .set_enabled(repair, false)
                .unwrap()
        });
        h.step();
        wait_for(&mut h, |editor| {
            editor
                .thumbnails
                .ready_count(&editor.document, editor.target)
                == 1
        });
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Image, "Photo error")
                .is_none()
        );
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Image, "Photo updating")
                .is_none()
        );
    });
}

#[test]
fn authored_dust_save_new_open_restores_intent_exact_results_and_looked_view_reference_scopes() {
    tools::calibrated_harness_with_thumbnails(true, |mut h| {
        let nodes = (0..8)
            .map(|i| {
                [
                    0.02 + 0.9 * (i % 2) as f32,
                    0.03 + 0.8 * ((i / 2) % 2) as f32,
                    0.01 + 0.95 * (i / 4) as f32,
                ]
            })
            .collect();
        let look = Arc::new(
            crate::looks::LookAsset::new(
                "Persisted Dust look".into(),
                crate::looks::LookOrigin::Imported,
                Arc::new(crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
            )
            .unwrap(),
        );
        h.update(|editor| editor.document.project.output.look = Some(look.clone()));
        h.step();
        stage(&mut h, "Photo");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dust.antitone");
        h.update(|editor| editor.sync_document());
        let saved = h.read(|editor| {
            editor
                .document
                .save(&path, crate::project::Save::New)
                .unwrap()
        });
        h.update(|editor| *editor.document = saved);

        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();
        h.update(|editor| editor.document.dust.brush_size = 32);
        click(&mut h, egui::pos2(96., 32.), egui::Modifiers::NONE);
        ready(&mut h, 1);
        click(&mut h, egui::pos2(32., 96.), egui::Modifiers::NONE);
        ready(&mut h, 2);

        let region = h.read(|editor| editor.dust_donor_regions().remove(0));
        let point = region.support.selected_points().next().unwrap();
        let offset = [
            region.donor.x as i32 - region.target.x as i32,
            region.donor.y as i32 - region.target.y as i32,
        ];
        let start = egui::pos2(
            point[0] as f32 + offset[0] as f32 + 0.5,
            point[1] as f32 + offset[1] as f32 + 0.5,
        );
        let layout = h.read(|editor| editor.last_layout.unwrap().0);
        drag(
            &mut h,
            layout.screen_point(start),
            layout
                .screen_point(start + egui::vec2((50 - offset[0]) as f32, (50 - offset[1]) as f32)),
        );
        ready(&mut h, 2);
        let source = h.read(|editor| editor.current_source.unwrap());
        h.read(|editor| assert!(editor.document.dirty()));
        h.read(|editor| {
            assert_eq!(
                editor.dust_repairs(source).unwrap().repairs()[0].donor,
                DonorChoice::Automatic
            )
        });
        h.read(|editor| {
            assert_eq!(
                editor.dust_repairs(source).unwrap().repairs()[1].donor,
                DonorChoice::Offset([50, 50])
            )
        });

        let signature = h.read(|editor| editor.dust_signature().unwrap());
        let evaluated = h.read(|editor| editor.dust_evaluations(signature));
        let expected = h.read(|editor| editor.document.project.clone());
        file_menu(&mut h, "New");
        h.run();
        h.read(|editor| {
            assert!(
                editor.transition.is_some(),
                "repair-only edits must guard New"
            )
        });
        h.get_by_label("Cancel").click();
        h.run();
        h.read(|editor| assert!(editor.transition.is_none()));
        h.read(|editor| assert_eq!(editor.document.project, expected));
        save_existing_project(&mut h);
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &evaluated[0],
                &editor.dust_evaluations(signature)[0]
            ))
        });

        file_menu(&mut h, "New");
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.read(|editor| assert!(editor.document.project.sources.is_empty()));
        open_project(&mut h, path.clone());
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.read(|editor| assert_eq!(editor.document.project, expected));
        h.read(|editor| assert!(!editor.document.dirty()));
        h.read(|editor| assert_eq!(editor.selected_dust_repair(), None));
        ready(&mut h, 2);
        wait_for(&mut h, |e| {
            e.thumbnails.ready_count(&e.document, e.target) == 1
        });
        let restored = h.read(|editor| editor.dust_evaluations(signature));
        for (before, after) in evaluated.iter().zip(&restored) {
            assert!(!Arc::ptr_eq(before, after));
            assert_eq!(before.donors, after.donors);
            assert_eq!(exact_values(before), exact_values(after));
        }

        let view = h.read(|editor| editor.working.displayed.as_ref().unwrap().clone());
        assert_eq!(view.processing.look_id(), Some(look.lut().id()));
        let scopes::ScopeInput::Rendered(scope, _) =
            h.read(|editor| editor.scope_request().unwrap())
        else {
            panic!("scopes did not follow restored repairs")
        };
        assert!(Arc::ptr_eq(&scope, &view));
        h.get_by_label("Original-resolution reference").click();
        h.step();
        // The reference must match the complete processing intent, including admitted repairs.
        wait_for(&mut h, |e| {
            assert!(e.original.refinement_error.is_none(), "{}", e.status);
            !e.working.updating
                && e.working.reference_ready()
                && e.working.displayed.as_ref().is_some_and(|view| {
                    Some(&view.processing) == e.working.processing() && view.target == e.target
                })
        });
        h.render().unwrap();
        h.read(|editor| {
            let reference = editor.working.displayed.as_ref().unwrap();
            assert_eq!(reference.processing.look_id(), Some(look.lut().id()));
            assert!(Arc::ptr_eq(
                &reference.image,
                &editor.original.settled.as_ref().unwrap().2
            ));
            let scopes::ScopeInput::Rendered(scope, _) = editor.scope_request().unwrap() else {
                panic!("scopes did not follow the restored original reference")
            };
            assert!(Arc::ptr_eq(&scope, reference));
            assert_eq!(editor.document.project, expected);
        });
        h.get_by_label("Original-resolution reference").click();
        h.step();
        ready(&mut h, 2);

        let path = h.read(|editor| editor.document.source_path(source).unwrap());
        let scan =
            Scan::open(&path, Some(expected.development.as_ref().unwrap().fixed_wb)).unwrap();
        for repair in expected.source(source).unwrap().repairs.repairs() {
            let request = h.read(|editor| crate::editor::dust::Request {
                request: 1,
                loaded: editor.id,
                source,
                signature,
                repair: repair.clone(),
                calibration: editor.calibration().unwrap(),
                roll: editor.roll.inversion_only(),
                cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            let cpu = crate::editor::dust::evaluate(&scan, &request).unwrap();
            let gpu = restored
                .iter()
                .find(|result| result.id == repair.id)
                .unwrap();
            assert_eq!(cpu.evaluated.donors, gpu.donors);
            assert_eq!(exact_values(&cpu.evaluated), exact_values(gpu));
        }

        let jobs = h.read(|editor| editor.workflow_jobs.clone());
        let hold = crate::editor::tests::hold_blocking_work(&jobs);
        open_project(&mut h, directory.path().join("missing.antitone"));
        for _ in 0..5 {
            h.step();
            h.read(|editor| assert!(editor.document_pending.is_some()));
            h.read(|editor| {
                assert!(Arc::ptr_eq(
                    &view,
                    editor.working.displayed.as_ref().unwrap()
                ))
            });
        }
        drop(hold);
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        h.read(|editor| assert_eq!(editor.document.project, expected));
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &restored[0],
                &editor.dust_evaluations(signature)[0]
            ))
        });
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                &view,
                editor.working.displayed.as_ref().unwrap()
            ))
        });
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Dust repair")
            .click();
        h.step();

        for (name, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            h.set_size(size);
            h.get_by_label("Brush size").scroll_to_me();
            h.try_run_realtime().unwrap();
            let brush = h.get_by_label("Brush size").rect();
            assert!(h.ctx.content_rect().contains_rect(brush));
            save_capture(
                &h.render().unwrap(),
                &format!("dust-persistence-{name}-offscreen"),
            );
        }

        h.get_by_label("Done").click();
        h.step();
        file_menu(&mut h, "New");
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        open_project(&mut h, path);
        wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
        ready(&mut h, 2);
        for (before, after) in
            h.read(|editor| evaluated.iter().zip(editor.dust_evaluations(signature)))
        {
            assert_eq!(before.donors, after.donors);
            assert_eq!(exact_values(before), exact_values(&after));
        }
    });
}
