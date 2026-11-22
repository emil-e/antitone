use super::*;

fn calibrated(h: &mut Harness<'_, Editor>) {
    wait_for(h, |editor| !editor.interaction_blocked());
    h.update(|editor| {
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        let current = editor.document.visit.current_source.unwrap();
        editor.document.project.source_mut(current).unwrap().crop = Some(Crop {
            center: [120., 100.],
            size: [16, 16],
            angle: 0.,
        });
    });
    h.step();
    workflow::wait_settled(h);
}

#[test]
fn lightroom_return_uses_accepted_state_and_normal_departure_with_crop_or_look_work() {
    for preparing_look in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("roll.antitone");
        let session = directory.path().join("session");
        let input = request(
            &session,
            &project,
            false,
            &originals(directory.path(), &["patches.dng"]),
        );
        let held = editor_harness(|mut h, _| {
            h.enter(|editor, cx| editor.open_external(input, cx));
            calibrated(&mut h);
            let expected = h.read(|editor| editor.document.capture().project);
            let (draft, held) = if preparing_look {
                let gate = h.update(crate::editor::look::testing::Gate::install);
                let asset = Arc::new(
                    crate::looks::LookAsset::new(
                        "Pending look".into(),
                        crate::looks::LookOrigin::Imported,
                        Arc::new(
                            crate::looks::Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.5; 3]; 8])
                                .unwrap(),
                        ),
                    )
                    .unwrap(),
                );
                h.enter(|editor, cx| editor.select_look(look::Source::Loaded(asset), cx));
                let held = gate.next(&mut h.application);
                assert!(held.valid);
                (None, Some(held))
            } else {
                stage(&mut h, "Photo");
                (Some(tools::crop_draft(&mut h)), None)
            };
            h.read(|editor| {
                assert!(
                    !editor.editing(),
                    "private work does not reserve a project edit"
                );
                assert!(editor.document_command_available());
                assert!(editor.external_return_available());
            });
            h.step();
            button(&mut h);
            wait_for(&mut h, |editor| !editor.document.external_edit.pending());
            h.read(|editor| {
                assert!(!editor.dialogs.is_open(), "{}", editor.status);
                assert!(
                    editor.allow_close,
                    "return follows ordinary clean-document departure"
                );
                assert!(!editor.document.dirty());
                assert_eq!(editor.document.project, expected);
                if let Some(draft) = draft {
                    assert_eq!(
                        editor.document.crop_edit().unwrap().draft,
                        draft,
                        "return does not apply the crop draft"
                    );
                } else {
                    assert!(
                        editor.document.lut.pending.is_some(),
                        "preparation stays independent until departure"
                    );
                }
            });
            assert!(session.join("result.json").is_file());
            assert_eq!(crate::project::Project::load(&project).unwrap(), expected);
            assert_eq!(
                tiff_dimensions(&returned_paths(&session.join("result.json"))[0]),
                (16, 16)
            );
            held
        });
        if let Some(held) = held {
            assert!(
                held.cancelled.is_cancelled(),
                "window departure retires pending acceptance"
            );
            held.finish();
        }
    }
}

#[test]
fn lightroom_save_as_keeps_return_destination_and_ordinary_save_updates_its_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("canonical.antitone");
    let copy = directory.path().join("project-copy.antitone");
    let session = directory.path().join("session");
    let paths = originals(directory.path(), &["patches.dng"]);
    let input = request(&session, &project, false, &paths);
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        calibrated(&mut h);
        // Supply chooser results; execute the ordinary save controller and worker.
        h.enter(|editor, cx| {
            editor.save_project(project.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.path.as_ref(), Some(&project)));
        h.enter(|editor, cx| {
            editor.save_project(copy.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.read(|editor| assert_eq!(editor.document.path.as_ref(), Some(&copy)));
        assert!(
            !session.join("result.json").exists(),
            "ordinary Save does not publish a return"
        );
        assert!(returned_paths_if_present(paths[0].parent().unwrap()).is_empty());
        h.update(|editor| editor.document.project.roll.exposure = 0.5);
        h.step();
        button(&mut h);
        wait_for(&mut h, |editor| {
            editor.allow_close || editor.dialogs.is_open()
        });
        h.read(|editor| assert!(editor.allow_close, "{}", editor.status));
    });
    assert_eq!(
        crate::project::Project::load(&project)
            .unwrap()
            .roll
            .exposure,
        0.5
    );
    assert_eq!(
        crate::project::Project::load(&copy).unwrap().roll.exposure,
        0.
    );
    let receipt: Value =
        serde_json::from_str(&std::fs::read_to_string(session.join("result.json")).unwrap())
            .unwrap();
    assert_eq!(receipt["project"], project.to_str().unwrap());
    assert_eq!(
        returned_paths(&session.join("result.json"))[0].parent(),
        paths[0].parent()
    );
}

#[test]
fn lightroom_project_save_failure_keeps_edits_without_rendering_or_publishing() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("project-folder");
    std::fs::create_dir(&parent).unwrap();
    let project = parent.join("roll.antitone");
    let session = directory.path().join("session");
    let paths = originals(directory.path(), &["patches.dng"]);
    let input = request(&session, &project, false, &paths);
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        calibrated(&mut h);
        let expected = h.read(|editor| editor.document.project.clone());
        std::fs::remove_dir(&parent).unwrap();
        button(&mut h);
        wait_for(&mut h, |editor| editor.dialogs.is_open());
        h.read(|editor| {
            assert!(!editor.allow_close);
            assert!(
                editor.document.dirty(),
                "failed persistence cannot accept a saved baseline"
            );
            assert_eq!(editor.document.project, expected);
            assert!(
                editor.document.external_edit.session.is_some(),
                "the caller remains available for retry"
            );
        });
        assert!(!project.exists());
        assert!(!session.join("result.json").exists());
        assert!(
            returned_paths_if_present(paths[0].parent().unwrap()).is_empty(),
            "rendering starts only after project save succeeds"
        );
    });
}

#[test]
fn lightroom_retirement_signals_a_return_waiting_on_cold_source_before_publication() {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let session = directory.path().join("session");
    let paths = originals(directory.path(), &["patches.dng", "roll-0.dng"]);
    // Make a saved roll, then request just one member in the external session.
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(paths.clone(), cx));
        calibrated(&mut h);
        h.enter(|editor, cx| {
            editor.save_project(project.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut h, |editor| !editor.interaction_blocked());
    });
    let input = request(&session, &project, true, &paths[..1]);
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.enter(|editor, cx| editor.select_roll_photo(1, egui::Modifiers::NONE, cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && !editor.document.visit.source.pending()
        });
        let entered = Arc::new(AtomicBool::new(false));
        let observed = entered.clone();
        let (release, resume) = mpsc::channel();
        struct Release(mpsc::Sender<()>);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let release = Release(release);
        let resume = Mutex::new(resume);
        let target = paths[0].clone();
        let loader = crate::source::SourceLoader::new_observed(move |path, phase, _| {
            if path == target && matches!(phase, crate::source::NativePhase::UnpackBegin) {
                observed.store(true, Ordering::Release);
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap();
            }
            Ok(())
        });
        h.update(|editor| {
            editor.document.cache.set_budget(0);
            editor.source_services.as_mut().unwrap().developments = loader.clone();
            let services = editor.source_services.as_ref().unwrap().clone();
            editor.output_services = Some(crate::editor::output::Services::new(
                services.evaluation.clone(),
                services.looks.clone(),
                services.display.clone(),
                services.queue.clone(),
                services,
            ));
            editor.developments = Some(loader);
        });
        h.step();
        button(&mut h);
        wait_for(&mut h, |_| entered.load(Ordering::Acquire));
        let cancellation = h.read(|editor| editor.document.external_edit.cancelled.clone());
        assert!(
            !cancellation.is_cancelled(),
            "return must really be awaiting source work"
        );
        h.update(|editor| editor.document.external_edit = Default::default());
        h.step(); // The executor retires the cancelled future outside the UI transaction.
        assert!(cancellation.is_cancelled());
        drop(release);
        h.run_steps(5);
        assert!(project.is_file(), "already saved state survives retirement");
        assert!(!session.join("result.json").exists());
        assert!(returned_paths_if_present(paths[0].parent().unwrap()).is_empty());
    });
    assert!(
        !paths[0]
            .parent()
            .unwrap()
            .join("patches-Antitone.tif")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn lightroom_return_uses_the_catalogue_alias_folder_not_the_projects_source_locator() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let session = directory.path().join("session");
    let paths = originals(directory.path(), &["patches.dng"]);
    let folder = directory.path().join("catalogue-originals");
    std::fs::create_dir(&folder).unwrap();
    let alias = folder.join("catalogue-frame.dng");
    std::os::unix::fs::symlink(&paths[0], &alias).unwrap();
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(paths.clone(), cx));
        calibrated(&mut h);
        h.enter(|editor, cx| {
            editor.save_project(project.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut h, |editor| !editor.interaction_blocked());
    });
    let input = request(&session, &project, true, std::slice::from_ref(&alias));
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        wait_for(&mut h, |editor| !editor.interaction_blocked());
        h.step();
        button(&mut h);
        wait_for(&mut h, |editor| {
            editor.allow_close || editor.dialogs.is_open()
        });
        h.read(|editor| assert!(editor.allow_close, "{}", editor.status));
    });
    let receipt = session.join("result.json");
    let rendered = returned_paths(&receipt);
    assert_eq!(rendered[0].parent(), Some(folder.as_path()));
    assert!(returned_paths_if_present(paths[0].parent().unwrap()).is_empty());
    assert_eq!(tiff_dimensions(&rendered[0]), (16, 16));
    let receipt: Value = serde_json::from_str(&std::fs::read_to_string(receipt).unwrap()).unwrap();
    assert_eq!(receipt["sources"][0]["path"], alias.to_str().unwrap());
}

fn returned_paths_if_present(directory: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().is_some_and(|ext| ext == "tif")).then_some(path)
        })
        .collect()
}

#[test]
fn lightroom_return_captures_accepted_gamma_pair_without_waiting_for_its_proposal() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("roll.antitone");
    let session = directory.path().join("session");
    let input = request(
        &session,
        &project,
        false,
        &originals(directory.path(), &["patches.dng"]),
    );
    let held = editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.open_external(input, cx));
        calibrated(&mut h);
        h.update(|editor| {
            let current = editor.document.visit.current_source.unwrap();
            editor.document.project.source_mut(current).unwrap().crop = None;
        });
        workflow::wait_settled(&mut h);
        stage(&mut h, "Roll");
        let gate = crate::editor::measurement::testing::Gate::default();
        h.update(|editor| editor.document.gamma_pivot.edit_gate = Some(gate.clone()));
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Gamma")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Gamma")
            .click();
        h.step();
        h.event(egui::Event::Text("0.9".into()));
        h.step();
        assert!(
            !h.get_by_label("Save Back to Lightroom")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(egui::Key::Enter);
        h.step();
        let solve = gate.next(&mut h.application);
        let accepted = h.read(|editor| {
            assert!(editor.document.gamma_pivot.calculating());
            assert!(!editor.editing());
            assert_ne!(editor.document.project.input.gamma(), 0.9);
            editor.document.project.clone()
        });
        button(&mut h);
        wait_for(&mut h, |editor| {
            editor.allow_close || editor.dialogs.is_open()
        });
        h.read(|editor| {
            assert!(editor.allow_close, "{}", editor.status);
            assert_eq!(editor.document.project.input, accepted.input);
            assert_eq!(editor.document.project.roll.wb, accepted.roll.wb);
        });
        let saved = crate::project::Project::load(&project).unwrap();
        assert_eq!(saved.input, accepted.input);
        assert_eq!(saved.roll.wb, accepted.roll.wb);
        solve
    });
    assert!(
        held.cancelled.is_cancelled(),
        "departure retires the unaccepted gamma proposal"
    );
    held.run();
    assert!(session.join("result.json").is_file());
}
