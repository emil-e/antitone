use super::*;

pub(super) fn enter_numeric(harness: &mut Harness<'_, Editor>, label: &str, text: &str) {
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

fn calibrated_workflow() -> (
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
        assert!(e.refinement_error.is_none(), "{}", e.status);
        e.signature
            .as_ref()
            .is_some_and(|signature| signature.1 == e.target)
            && e.settled.as_ref().is_some_and(|(revision, _, _)| {
                *revision == e.generation.load(std::sync::atomic::Ordering::Relaxed)
            })
    });
    let (_, view, image) = harness.state().settled.as_ref().unwrap();
    assert_eq!([image.linear.width(), image.linear.height()], view.output);
    assert!(view.output.into_iter().all(|size| size > 0));
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
    let working = metadata.camera_to_ap1().unwrap() * DVec3::splat(4. * 3000. / 4031.);
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
    let (_, view, _) = harness.state().settled.as_ref().unwrap();
    assert_eq!(view.output, [view.area.width, view.area.height]);
    assert!(view.area.width < harness.state().ready.as_ref().unwrap().area.width);
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
            let current = if harness.state().scopes.vector {
                "Vectorscope"
            } else {
                "Waveform"
            };
            harness
                .get_by(|node| {
                    node.role() == egui::accesskit::Role::ComboBox
                        && node.value().as_deref() == Some(current)
                })
                .click();
            harness.step();
            harness.step();
            harness
                .get_by_label(if plot == crate::scopes::Plot::Vector {
                    "Vectorscope"
                } else {
                    "Waveform"
                })
                .click();
            harness.step();
            if plot != crate::scopes::Plot::Vector {
                harness
                    .get_by_label(if plot == crate::scopes::Plot::Rgb {
                        "RGB"
                    } else {
                        "Luma"
                    })
                    .click();
                harness.step();
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
                    harness.get_by_label(&format!("{amplification}×")).click();
                    harness.step();
                    assert_eq!(harness.state().scopes.amplification, *amplification);
                }
                harness.render().unwrap();
                wait_for(&mut harness, |e| {
                    let (image, parameters, frame) = e.scope_input().unwrap();
                    let settings = crate::scopes::Settings {
                        target,
                        plot,
                        amplification: *amplification,
                    };
                    let Some(counts) = e
                        .scopes
                        .gpu
                        .as_ref()
                        .unwrap()
                        .status(&image, parameters, frame, settings)
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
