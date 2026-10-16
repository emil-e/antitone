use super::*;
#[path = "editor_kittest_source_working.rs"]
mod source_working;
#[path = "editor_kittest_underlay.rs"]
mod underlay;

pub(super) fn enter_numeric(harness: &mut Harness<'_, Editor>, label: &str, text: &str) {
    harness.try_run_realtime().unwrap();
    let role = egui::accesskit::Role::SpinButton;
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_role_and_label(role, label).click();
    harness.step();
    harness.event(egui::Event::Text(text.into()));
    harness.event(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

pub(super) fn calibrated_workflow() -> (
    Harness<'static, Editor>,
    Rc<RefCell<FloatCapture>>,
    tempfile::TempDir,
) {
    let directory = tempfile::tempdir().unwrap();
    let dmax = directory.path().join("dmax.dng");
    let photo = directory.path().join("working-photo.dng");
    std::fs::copy(fixture("patches.dng"), &dmax).unwrap();
    std::fs::copy(fixture("roll-0.dng"), &photo).unwrap();
    let (mut harness, capture) = editor_harness();
    harness
        .state_mut()
        .add_raws(vec![fixture("patches.dng"), dmax, photo]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    assert!(harness.state().calibration().is_none());
    for (name, action, point, expected) in [
        (
            "patches.dng",
            "Pick Dmin (film base)",
            egui::pos2(32., 32.),
            3000.,
        ),
        ("dmax.dng", "Pick Dmax", egui::pos2(96., 32.), 128.),
    ] {
        harness.get_by_label(name).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        assert_eq!(
            harness
                .state()
                .ready
                .as_ref()
                .unwrap()
                .path
                .file_name()
                .unwrap(),
            name
        );
        harness.get_by_label(action).scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label(action).click();
        harness.step();
        assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
        let point = harness.state().last_layout.unwrap().0.screen_point(point);
        harness.hover_at(point);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        assert!(harness.state().pick.is_none());
        let measured = if expected == 3000. {
            harness.state().dmin
        } else {
            harness.state().dmax
        }
        .unwrap();
        assert!(
            (measured - DVec3::splat(expected / 4031.))
                .abs()
                .max_element()
                <= 2. / 65535.
        );
    }
    assert!(harness.state().calibration().is_some());
    harness.get_by_label("working-photo.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(
        harness
            .state()
            .ready
            .as_ref()
            .unwrap()
            .path
            .file_name()
            .unwrap(),
        "working-photo.dng"
    );
    if harness.state().source {
        harness.get_by_label("View source").click();
        harness.step();
    }
    enter_numeric(&mut harness, "Gamma", "0.75");
    assert_eq!(harness.state().roll.gamma, 0.75);
    stage(&mut harness, "Photo");
    enter_numeric(&mut harness, "Exposure", "0.5");
    assert_eq!(
        harness.state().photo.exposure,
        0.5,
        "{}",
        harness.state().status
    );
    enter_numeric(&mut harness, "Warmth", "0.1");
    enter_numeric(&mut harness, "Tint", "-0.05");
    assert_eq!(harness.state().photo.exposure, 0.5);
    assert_eq!(
        harness.state().photo.wb,
        Balance {
            warmth: 0.1,
            tint: -0.05
        }
    );
    assert!(!harness.state().source);
    (harness, capture, directory)
}

pub(super) fn wait_settled(harness: &mut Harness<'_, Editor>) {
    wait_for(harness, |e| {
        assert!(e.original.refinement_error.is_none(), "{}", e.status);
        if e.working.revision().is_some() {
            let expected = if e.source {
                e.source_parameters(e.source_exposure)
                    .map(ProcessingInput::from)
            } else {
                e.positive_processing(e.calibration().unwrap(), e.roll, e.photo)
            }
            .unwrap();
            !e.working.updating
                && e.working.displayed.as_ref().is_some_and(|view| {
                    view.processing == expected.signature()
                        && view.target == e.target
                        && (!e.working.compare || e.source || e.working.reference_ready())
                })
        } else {
            e.original
                .signature
                .as_ref()
                .is_some_and(|s| s.1 == e.target)
                && e.original
                    .settled
                    .as_ref()
                    .is_some_and(|(revision, _, _)| *revision == e.original.revision())
        }
    });
}

#[test]
fn uncalibrated_source_working_view_uses_saved_roll_wb_in_both_targets_without_idle_refinement() {
    let directory = tempfile::tempdir().unwrap();
    let (mut harness, capture) = editor_harness_with_thumbnails(true);
    let wb = [2., 1., 1.5, 1.];
    harness.state_mut().document.project.development =
        Some(Development::new(crate::raw::version(), wb));
    harness.state_mut().add_raws(vec![fixture("patches.dng")]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    let path = directory.path().join("source.antitone.json");
    let saved = harness
        .state()
        .document
        .save(&path, crate::project::Save::New)
        .unwrap();
    harness.state_mut().document = saved;
    let project = harness.state().document.project.clone();
    open_project(&mut harness, path);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert_eq!(harness.state().document.project, project);
    assert!(harness.state().source);
    assert!(harness.state().calibration().is_none());
    let scan = Scan::open(&fixture("patches.dng"), Some(wb)).unwrap();
    stage(&mut harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        for ev in [0.0_f64, 2.] {
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            enter_numeric(&mut harness, "Exposure", &ev.to_string());
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
                .click();
            harness.step();
            assert_eq!(harness.state().source_exposure, ev);
            wait_settled(&mut harness);
            for phase in 0..2 {
                if phase == 1 {
                    std::thread::sleep(Duration::from_millis(300));
                    harness.step();
                }
                harness.render().unwrap();
                let geometry = harness.state().last_layout.unwrap().0;
                let output = capture.borrow();
                for [x, y] in [[96, 32], [32, 160], [224, 32]] {
                    let point = geometry.screen_point(egui::pos2(x as f32, y as f32));
                    let pixel = output.pixels[point.y.floor() as usize * output.size[0] as usize
                        + point.x.floor() as usize];
                    // max(WB)/green = 2, plus one preview stop, then relative manual EV.
                    let working = scan.camera_to_ap1 * scan.pixel(x, y).unwrap() * (4. * ev.exp2());
                    let linear = target.linear_to_srgb()
                        * crate::rendering::CpuRenderer::new(target)
                            .unwrap()
                            .render(&[working])
                            .unwrap()[0]
                            .as_dvec3();
                    for c in 0..3 {
                        let value = linear[c].abs();
                        let expected = linear[c].signum()
                            * if value <= 0.0031308 {
                                value * 12.92
                            } else {
                                1.055 * value.powf(1. / 2.4) - 0.055
                            };
                        assert!(pixel[c].is_finite());
                        assert!(
                            (f64::from(pixel[c]) - expected).abs() <= 0.001,
                            "{target:?}, EV {ev}: {pixel:?} vs {expected}"
                        );
                    }
                }
            }
            assert!(harness.state().original.signature.is_none());
            assert!(harness.state().original.refining.is_none());
            assert!(harness.state().original.settled.is_none());
            let mut expected = project.clone();
            expected.output.tiff = if target == OutputTarget::Srgb {
                crate::project::TiffTarget::Srgb
            } else {
                crate::project::TiffTarget::DisplayP3
            };
            assert_eq!(harness.state().document.project, expected);
        }
        wait_for(&mut harness, |e| {
            e.thumbnails.ready_count(&e.document, target) == 1
        });
    }
}

#[test]
fn calibration_grade_source_inspection_and_native_pixel_picker_return_use_widgets() {
    let (mut harness, capture, _directory) = calibrated_workflow();
    stage(&mut harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        wait_settled(&mut harness);
        let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
        harness.state_mut().selection_timing = Some(timing.clone());
        harness.step();
        let png = harness.render().unwrap();
        assert!(timing.ready_was_painted());
        save_capture(
            &png,
            if target == OutputTarget::Srgb {
                "workflow-positive-srgb-offscreen"
            } else {
                "workflow-positive-p3-offscreen"
            },
        );
    }
    let project = harness.state().document.project.clone();
    harness.get_by_label("View source").click();
    harness.step();
    assert!(harness.state().source);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    harness.step();
    enter_numeric(&mut harness, "Exposure", "2");
    assert_eq!(harness.state().source_exposure, 2.);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Source inspection")
        .click();
    harness.step();
    let metadata = crate::raw::RawFile::open(&fixture("roll-0.dng"))
        .unwrap()
        .metadata;
    let working = metadata.camera_to_ap1().unwrap() * DVec3::splat(8. * 3000. / 4031.);
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
        harness.state_mut().selection_timing = Some(timing.clone());
        harness.step();
        let png = harness.render().unwrap();
        assert!(timing.ready_was_painted());
        let point = harness
            .state()
            .last_layout
            .unwrap()
            .0
            .screen_point(egui::pos2(32., 32.));
        let output = capture.borrow();
        let pixel = output.pixels
            [point.y.floor() as usize * output.size[0] as usize + point.x.floor() as usize];
        let linear = target.linear_to_srgb()
            * crate::rendering::CpuRenderer::new(target)
                .unwrap()
                .render(&[working])
                .unwrap()[0]
                .as_dvec3();
        for c in 0..3 {
            let value = linear[c].abs();
            let expected = linear[c].signum()
                * if value <= 0.0031308 {
                    value * 12.92
                } else {
                    1.055 * value.powf(1. / 2.4) - 0.055
                };
            assert!(
                (f64::from(pixel[c]) - expected).abs() <= 0.001,
                "{pixel:?} vs {expected}"
            );
        }
        let mut expected = project.clone();
        expected.output.tiff = if target == OutputTarget::Srgb {
            crate::project::TiffTarget::Srgb
        } else {
            crate::project::TiffTarget::DisplayP3
        };
        assert_eq!(harness.state().document.project, expected);
        save_capture(
            &png,
            if target == OutputTarget::Srgb {
                "workflow-source-srgb-offscreen"
            } else {
                "workflow-source-p3-offscreen"
            },
        );
    }
    assert_eq!(harness.state().document.project, project);
    assert_eq!(
        harness.state().scope_input().err(),
        Some("Positive grade only")
    );
    harness.get_by_label("View source").click();
    harness.step();
    assert!(!harness.state().source);
    set_layout(&mut harness, egui::vec2(800., 600.), 315.);
    harness.get_by_label("100%").click();
    harness.step();
    assert_eq!(harness.state().viewport.zoom, Some(1.));
    wait_settled(&mut harness);
    let frame = harness
        .state()
        .working
        .displayed
        .as_ref()
        .unwrap()
        .spec
        .frame;
    for c in 0..2 {
        assert!(
            ((frame.max.0[c] - frame.min.0[c]) - f64::from(frame.output[c])).abs() < 1e-4,
            "100% retains one source-frame pixel per physical display pixel"
        );
    }
    assert!(frame.bounds().unwrap().width < harness.state().ready.as_ref().unwrap().area.width);
    let png = harness.render().unwrap();
    save_capture(&png, "workflow-native-p3-offscreen");
    let selected = harness.state().current_source;
    stage(&mut harness, "Roll");
    harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Pick Dmin (film base)").click();
    harness.step();
    assert_eq!(harness.state().pick, Some(Pick::Dmin));
    assert!(harness.state().region.is_none());
    assert_eq!(harness.state().current_source, selected);
    assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
    harness.get_by_label("patches.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(
        harness.state().ready.as_ref().unwrap().path,
        fixture("patches.dng")
    );
    assert_eq!(
        harness.state().current_source,
        harness
            .state()
            .document
            .find_source(&fixture("patches.dng"))
    );
    assert!(harness.state().region.is_none());
    harness.get_by_label("Cancel").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(harness.state().current_source, selected);
    assert_eq!(
        harness
            .state()
            .ready
            .as_ref()
            .unwrap()
            .path
            .file_name()
            .unwrap(),
        "working-photo.dng"
    );
    assert_eq!(harness.state().viewport.zoom, Some(1.));
    assert!(!harness.state().source);
    assert!(harness.state().pick.is_none());
    assert_eq!(harness.state().document.project, project);
    assert!(harness.state().preference_store.is_none());
    wait_settled(&mut harness);
}

#[test]
fn physical_view_local_edits_use_current_input_and_clear_recovered_render_errors() {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
        project::LocalAdjustments,
    };
    let (mut harness, _capture, _directory) = calibrated_workflow();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    let initial = harness.state().working.displayed.clone().unwrap();
    let source_id = harness.state().current_source.unwrap();
    let area = harness.state().ready.as_ref().unwrap().area;
    let mut locals = LocalAdjustments::default();
    for exposure in [0.5, -0.25] {
        locals
            .add(
                format!("Local {exposure}"),
                LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([f64::from(area.x), f64::from(area.y)]),
                        end: SourcePoint([
                            f64::from(area.x + area.width),
                            f64::from(area.y + area.height),
                        ]),
                    },
                    photo: PhotoSettings {
                        exposure,
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 0.5,
                },
            )
            .unwrap();
    }
    for edit in 0..4 {
        match edit {
            1 => locals.adjustments[0].adjustment.photo.exposure = 0.75,
            2 => locals.adjustments.reverse(),
            3 => {
                locals.adjustments.remove(0);
            }
            _ => (),
        }
        let editor = harness.state_mut();
        editor
            .document
            .project
            .source_mut(source_id)
            .unwrap()
            .local_adjustments = Some(locals.clone());
        editor
            .validate_document_candidate(&editor.document)
            .unwrap();
        let input = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        harness.step();
        harness.render().unwrap();
        let editor = harness.state();
        assert!(editor.working.error.is_none(), "{:?}", editor.working.error);
        let displayed = editor.working.displayed.as_ref().unwrap();
        assert!(displayed.same_source_as(&initial));
        assert_eq!(displayed.processing, input.signature());
        assert!(
            editor
                .ready
                .as_ref()
                .unwrap()
                .record
                .local_adjustments
                .is_none(),
            "Ready remains the original accepted-load record"
        );
    }
    let editor = harness.state_mut();
    let bad = editor
        .positive_processing(
            editor.calibration().unwrap(),
            editor.roll,
            PhotoSettings {
                exposure: editor.photo.exposure + 1.,
                ..editor.photo
            },
        )
        .unwrap();
    // Mismatched Global controls are rejected before publishing output.
    let bad = ProcessingInput::from(bad.parameters());
    let frame = editor.working.current_frame().unwrap();
    let error = editor
        .working_view(frame, &bad, &egui::Context::default())
        .err();
    assert!(error.is_some());
    editor.working.error = error.map(|e| e.to_string());
    harness.step();
    harness.render().unwrap();
    assert!(harness.state().working.error.is_none());
    assert!(
        harness
            .state()
            .working
            .displayed
            .as_ref()
            .unwrap()
            .same_source_as(&initial)
    );
}

#[test]
fn physical_view_global_widgets_keep_detail_through_edits_idle_targets_and_reference_switching() {
    let (mut harness, _, _directory) = calibrated_workflow();
    harness.state_mut().scopes.visible = true;
    wait_settled(&mut harness);
    harness.render().unwrap();
    let first = harness
        .state()
        .working
        .displayed
        .clone()
        .expect("working view");
    assert_eq!(first.image.processing_byte_len(), (0, 0));
    assert!(
        !harness
            .state()
            .ready
            .as_ref()
            .unwrap()
            .proxy
            .output_retained(),
        "scopes must not recreate unused provisional storage after accepted working presentation"
    );
    stage(&mut harness, "Photo");
    for (label, value) in [
        ("Exposure", "1.1"),
        ("Warmth", "-0.2"),
        ("Tint", "0.15"),
        ("Contrast", "1.3"),
        ("Saturation", "1.4"),
    ] {
        enter_numeric(&mut harness, label, value);
        harness.render().unwrap();
        let current = harness.state().working.displayed.as_ref().unwrap();
        assert!(
            current.same_source_as(&first),
            "{label} must not reacquire or demote source detail"
        );
        assert!(current.spec == first.spec);
        assert!(
            !harness
                .state()
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .output_retained()
        );
        assert_eq!(
            &current.processing,
            harness.state().working.processing().unwrap()
        );
    }
    harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    let slider = harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
        .rect();
    let start = slider.left_center() + egui::vec2(8., 0.);
    harness.hover_at(start);
    harness.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let mut exposures = Vec::new();
    for n in 0..12 {
        harness.hover_at(start + egui::vec2((slider.width() - 16.) * n as f32 / 11., 0.));
        harness.step();
        harness.render().unwrap();
        let view = harness.state().working.displayed.as_ref().unwrap();
        assert!(
            view.same_source_as(&first),
            "dragging must not switch detail levels"
        );
        assert_eq!(
            &view.processing,
            harness.state().working.processing().unwrap()
        );
        assert!(
            !harness
                .state()
                .ready
                .as_ref()
                .unwrap()
                .proxy
                .output_retained(),
            "dragging/scopes must not allocate a second presentation path"
        );
        exposures.push(harness.state().photo.exposure);
    }
    let end = slider.right_center() - egui::vec2(8., 0.);
    harness.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.render().unwrap();
    assert!(exposures.last().unwrap() > exposures.first().unwrap());
    let edited = harness.state().working.displayed.clone().unwrap();
    wait_settled(&mut harness);
    harness.render().unwrap();
    assert!(
        Arc::ptr_eq(&edited, harness.state().working.displayed.as_ref().unwrap()),
        "idle must not replace the working view"
    );
    stage(&mut harness, "Rendering");
    select_target(&mut harness, OutputTarget::Srgb);
    harness.render().unwrap();
    let srgb = harness.state().working.displayed.clone().unwrap();
    assert!(srgb.same_source_as(&edited));
    assert!(
        srgb.same_grade_as(&edited),
        "target-only changes reuse graded AP1"
    );
    wait_settled(&mut harness);
    assert!(
        harness.state().original.signature.is_none(),
        "normal grading does not prepare hidden original work"
    );
    let project = harness.state().document.project.clone();
    harness
        .get_by_label("Original-resolution reference")
        .click();
    harness.step();
    wait_settled(&mut harness);
    harness.render().unwrap();
    let reference = harness.state().working.displayed.clone().unwrap();
    assert!(Arc::ptr_eq(
        &reference.image,
        &harness.state().original.settled.as_ref().unwrap().2
    ));
    assert!(reference.spec == srgb.spec);
    harness
        .get_by_label("Original-resolution reference")
        .click();
    harness.step();
    harness.render().unwrap();
    assert!(Arc::ptr_eq(
        &srgb,
        harness.state().working.displayed.as_ref().unwrap()
    ));
    assert_eq!(harness.state().document.project, project);
    match harness.state().scope_request().unwrap() {
        scopes::ScopeInput::Rendered(view, _) => assert!(Arc::ptr_eq(
            &view,
            harness.state().working.displayed.as_ref().unwrap()
        )),
        _ => panic!("scopes must consume the viewer's accepted target-linear result"),
    }
    harness.get_by_label("100%").click();
    harness.step();
    wait_for(&mut harness, |e| e.working.support_ready());
    harness.render().unwrap();
    let navigated = harness.state().working.displayed.clone().unwrap();
    assert!(
        !navigated.same_source_as(&srgb),
        "navigation reacquires authoritative originals"
    );
    harness
        .state_mut()
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    harness.step();
    harness.render().unwrap();
    assert!(harness.state().original.settled.is_none());
    assert!(!harness.state().working.reference_ready());
    assert!(
        harness
            .state()
            .working
            .displayed
            .as_ref()
            .unwrap()
            .same_source_as(&navigated),
        "mandatory working detail survives zero optional retention"
    );
}

fn open_scope_settings(harness: &mut Harness<'_, Editor>) {
    harness.step();
    let current = harness.state().scopes.menu_label();
    harness
        .get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().as_deref() == Some(current.as_str())
        })
        .click();
    harness.step();
    harness.step();
}

#[test]
fn compact_scope_header_fits_and_disclosure_preserves_state_at_both_densities() {
    fn key(harness: &mut Harness<'_, Editor>, key: egui::Key) {
        for pressed in [true, false] {
            harness.event(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
        }
    }
    let mut harness = tools::calibrated_harness();
    let project = harness.state().document.project.clone();
    let source = harness.state().current_source;
    for scale in [1., 2.] {
        harness
            .input_mut()
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(scale);
        harness.step();
        for (layout, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            wait_settled(&mut harness);
            for (plot, label) in [
                (crate::scopes::Plot::Vector, "Vectorscope"),
                (crate::scopes::Plot::Rgb, "RGB waveform"),
                (crate::scopes::Plot::Luma, "Luma waveform"),
            ] {
                open_scope_settings(&mut harness);
                harness.get_by_label(label).click();
                harness.step();
                harness.step();
                let current = harness.state().scopes.menu_label();
                let header = harness.get_by_label("Scopes");
                assert_eq!(header.accesskit_node().data().is_expanded(), Some(true));
                let heading = header.rect();
                let menu = harness
                    .get_by(|node| {
                        node.role() == egui::accesskit::Role::ComboBox
                            && node.value().as_deref() == Some(current.as_str())
                    })
                    .rect();
                assert!(heading.left() >= size.x - width);
                assert!(heading.right() <= menu.left());
                assert!(menu.right() <= size.x);
                assert!((heading.center().y - menu.center().y).abs() <= 1. / scale);
                harness.event(egui::Event::PointerGone);
                harness.step();
                harness.render().unwrap();
                wait_for(&mut harness, |editor| {
                    let input = editor.scope_request().unwrap();
                    let settings = crate::scopes::Settings {
                        target: editor.target,
                        plot,
                        amplification: 1,
                    };
                    matches!(input.status(editor.scopes.gpu.as_ref().unwrap(), settings), Some(Ok(counts)) if counts.invalid == 0)
                });
                let png = harness.render().unwrap();
                let trace = harness
                    .output()
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Callback(callback)
                            if callback.rect.left() >= size.x - width =>
                        {
                            Some(callback.rect)
                        }
                        _ => None,
                    })
                    .expect("scope callback in inspector");
                let plot_rect = harness
                    .output()
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.rect.left() >= size.x - width
                                && rect.fill == egui::Color32::from_gray(6)
                                && rect.rect.height() >= 96. =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!(
                    plot_rect.top() - heading.bottom() <= 8.,
                    "plot follows the header"
                );
                for shape in &harness.output().shapes {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.pos.x >= size.x - width
                        && ["0", "25", "50", "75", "100"].contains(&text.galley.job.text.as_str())
                    {
                        assert!(
                            plot_rect.contains_rect(text.visual_bounding_rect()),
                            "waveform endpoint label is clipped"
                        );
                    }
                }
                assert!(trace.top() > heading.bottom());
                assert!(trace.bottom() < size.y - 180.);
                if plot == crate::scopes::Plot::Vector {
                    assert!((trace.width() - trace.height()).abs() < 0.01);
                }
                save_capture(&png, &format!("scopes-compact-{layout}-{scale}x-{plot:?}"));
            }
            open_scope_settings(&mut harness);
            harness.get_by_label("Vectorscope").click();
            harness.step();
            open_scope_settings(&mut harness);
            for label in [
                "Vectorscope",
                "RGB waveform",
                "Luma waveform",
                "1×",
                "2×",
                "4×",
                "8×",
            ] {
                let rect = harness.get_by_label(label).rect();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
            }
            let png = harness.render().unwrap();
            save_capture(&png, &format!("scopes-menu-{layout}-{scale}x"));
            key(&mut harness, egui::Key::Escape);
            let expanded = harness.get_by_label("Scopes").rect();
            let current = harness.state().scopes.menu_label();
            let menu_right = harness
                .get_by(|node| {
                    node.role() == egui::accesskit::Role::ComboBox
                        && node.value().as_deref() == Some(current.as_str())
                })
                .rect()
                .right();
            harness.get_by_label("Scopes").focus();
            harness.step();
            key(&mut harness, egui::Key::Space);
            assert!(!harness.state().scopes.visible);
            assert!(!harness.state().preferences.scopes_visible);
            assert_eq!(
                harness
                    .get_by_label("Scopes")
                    .accesskit_node()
                    .data()
                    .is_expanded(),
                Some(false)
            );
            let collapsed = harness.get_by_label("Scopes").rect();
            assert_eq!(collapsed.left(), expanded.left());
            assert!((collapsed.right() - menu_right).abs() <= 1. / scale);
            assert!(collapsed.width() > expanded.width());
            let expand_at = collapsed.right_center() - egui::vec2(12., 0.);
            harness.hover_at(expand_at);
            harness.step();
            let png = harness.render().unwrap();
            save_capture(&png, &format!("scopes-collapsed-{layout}-{scale}x"));
            let reductions = harness
                .state()
                .scopes
                .gpu
                .as_ref()
                .unwrap()
                .reduction_count();
            for _ in 0..3 {
                harness.step();
                harness.render().unwrap();
            }
            assert_eq!(
                harness
                    .state()
                    .scopes
                    .gpu
                    .as_ref()
                    .unwrap()
                    .reduction_count(),
                reductions
            );
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton {
                    pos: expand_at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
            }
            harness.step();
            assert!(harness.state().scopes.visible);
            assert!(harness.state().preferences.scopes_visible);
            key(&mut harness, egui::Key::Tab);
            let current = harness.state().scopes.menu_label();
            assert!(
                harness
                    .get_by(|node| {
                        node.role() == egui::accesskit::Role::ComboBox
                            && node.value().as_deref() == Some(current.as_str())
                    })
                    .is_focused()
            );
            key(&mut harness, egui::Key::Space);
            assert!(harness.query_by_label("Magnification").is_some());
            key(&mut harness, egui::Key::Escape);
            assert_eq!(harness.state().document.project, project);
            assert_eq!(harness.state().current_source, source);
        }
    }
}

#[test]
fn scope_plot_scale_and_target_widgets_produce_valid_completed_reductions() {
    let (mut harness, _capture, _directory) = calibrated_workflow();
    stage(&mut harness, "Rendering");
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        select_target(&mut harness, target);
        wait_settled(&mut harness);
        let project = harness.state().document.project.clone();
        for plot in [
            crate::scopes::Plot::Rgb,
            crate::scopes::Plot::Luma,
            crate::scopes::Plot::Vector,
        ] {
            open_scope_settings(&mut harness);
            harness
                .get_by_label(match plot {
                    crate::scopes::Plot::Vector => "Vectorscope",
                    crate::scopes::Plot::Rgb => "RGB waveform",
                    crate::scopes::Plot::Luma => "Luma waveform",
                })
                .click();
            harness.step();
            if plot != crate::scopes::Plot::Vector {
                assert!(!harness.state().scopes.vector);
                assert_eq!(harness.state().scopes.waveform, plot);
            } else {
                assert!(harness.state().scopes.vector);
            }
            for amplification in if plot == crate::scopes::Plot::Vector {
                &[1, 2, 4, 8][..]
            } else {
                &[1][..]
            } {
                if plot == crate::scopes::Plot::Vector {
                    open_scope_settings(&mut harness);
                    harness.get_by_label(&format!("{amplification}×")).click();
                    harness.step();
                    assert_eq!(harness.state().scopes.amplification, *amplification);
                }
                harness.render().unwrap();
                wait_for(&mut harness, |e| {
                    let input = e.scope_request().unwrap();
                    let frame = input.frame();
                    let settings = crate::scopes::Settings {
                        target,
                        plot,
                        amplification: *amplification,
                    };
                    let Some(counts) = input.status(e.scopes.gpu.as_ref().unwrap(), settings)
                    else {
                        return false;
                    };
                    let counts = counts.expect("scope diagnostics");
                    assert_eq!(counts.invalid, 0);
                    assert_eq!(counts.valid, frame.output[0] * frame.output[1]);
                    true
                });
                assert_eq!(harness.state().document.project, project);
            }
            let png = harness.render().unwrap();
            save_capture(
                &png,
                &format!(
                    "scope-{}-{}-offscreen",
                    if target == OutputTarget::Srgb {
                        "srgb"
                    } else {
                        "p3"
                    },
                    match plot {
                        crate::scopes::Plot::Rgb => "rgb",
                        crate::scopes::Plot::Luma => "luma",
                        crate::scopes::Plot::Vector => "vector",
                    }
                ),
            );
        }
    }
    assert!(
        harness
            .state()
            .scopes
            .gpu
            .as_ref()
            .unwrap()
            .reduction_count()
            >= 3
    );
}
