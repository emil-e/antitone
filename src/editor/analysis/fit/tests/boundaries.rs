use super::*;
use crate::application::test_support::with_application;
use std::sync::atomic::Ordering;

fn sampled() -> (Fit, tempfile::TempDir, source_loading::SourceLoader) {
    let (mut request, directory, device, queue) = request();
    approved(&mut request);
    let (loader, _) = observed();
    let resources = Resources {
        sources: loader.provider(None),
        sampling_inputs: Default::default(),
        initialization: Arc::default(),
        device,
        queue,
        renderers: Arc::default(),
    };
    let mut fit = resources.fit(request, None).unwrap();
    fit.geometry().unwrap();
    for index in 0..fit.cameras.len() {
        assert!(
            fit.sample_input(index, fit.cameras[index].clone(), || false)
                .unwrap()
        );
    }
    (fit, directory, loader)
}

// Declared estimator output isolates proposal admission from the estimator's search.
fn proposal(fit: &Fit) -> roll_analysis::Proposal {
    roll_analysis::Proposal {
        input: fit.request.document.project.input,
        roll: Default::default(),
        photos: vec![Default::default(); fit.cameras.len()],
        frames: fit
            .owned
            .iter()
            .map(|frame| roll_analysis::FrameStatistics {
                film_samples: frame.film.pixels.len(),
                picture_samples: frame.picture.pixels.len(),
                balance_eligible: frame.picture.pixels.len(),
                balance_samples: MIN_BALANCE_SAMPLES,
                exposure_samples: MIN_PICTURE_SAMPLES,
                grayness: None,
            })
            .collect(),
    }
}

#[test]
fn malformed_proposals_fail_at_the_fitting_boundary_without_modifying_captured_state() {
    let (fit, _directory, loader) = sampled();
    let before = fit.request.document.project.clone();
    assert!(fit.propose(proposal(&fit), || false).unwrap().is_some());
    for case in 0..5 {
        let mut proposal = proposal(&fit);
        match case {
            0 => {
                if let InputSettings::Negative { gamma, .. } = &mut proposal.input {
                    *gamma = f64::NAN;
                }
            }
            1 => {
                if let InputSettings::Negative { dmin, dmax, .. } = &mut proposal.input {
                    *dmax = *dmin;
                }
            }
            2 => {
                proposal.photos.pop();
            }
            3 => {
                proposal.frames.pop();
            }
            4 => {
                proposal.photos.last_mut().unwrap().contrast = f64::NAN;
            }
            _ => unreachable!(),
        }
        assert!(
            fit.propose(proposal, || false).is_err(),
            "malformed proposal {case}"
        );
        assert_eq!(fit.request.document.project, before);
    }
    assert!(fit.propose(proposal(&fit), || true).unwrap().is_none());
    loader.close_and_wait().unwrap();
}

#[test]
fn proposed_domain_checks_use_the_captured_matrix_instead_of_the_registered_conversion() {
    let (mut fit, _directory, loader) = sampled();
    let before = fit.request.document.project.clone();
    assert!(fit.propose(proposal(&fit), || false).unwrap().is_some());
    // The acquired f64 transform is independent, but cannot be represented by GPU constants.
    fit.matrix = glam::DMat3::IDENTITY * 1e39;
    assert!(fit.matrix.is_finite());
    assert!(!(1e39_f64 as f32).is_finite());
    let mut proposed = proposal(&fit);
    proposed.roll.exposure = 16.;
    proposed.photos.last_mut().unwrap().exposure = 16.;
    let candidate =
        proposed_document(&fit.request.document, proposed.clone(), fit.picture).unwrap();
    candidate.project.require_runtime().unwrap();
    let last = *candidate.project.order.last().unwrap();
    super::super::super::super::processing::validate_source_with_matrix(
        Some(&fit.device),
        candidate.project.input,
        candidate.project.roll,
        candidate.project.source(last).unwrap(),
        glam::DMat3::IDENTITY,
    )
    .expect("the unrelated registered/identity matrix would admit this proposal");
    assert!(
        fit.propose(proposed, || false).is_err(),
        "the captured calculation's domain must be protected"
    );
    assert_eq!(fit.request.document.project, before);
    loader.close_and_wait().unwrap();
}

#[test]
fn actual_fit_and_acceptance_ignore_unrelated_look_capacity_and_replace_old_grades_cold_or_warm() {
    let (request, _directory, _, _) = request();
    let paths: Vec<_> = request
        .document
        .project
        .order
        .iter()
        .map(|id| request.document.source_path(*id).unwrap())
        .collect();
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&paths).unwrap();
    document.project.input = request.document.project.input;
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_storage_buffer_binding_size: 8 * 1024 * 1024,
            ..Default::default()
        },
        ..Default::default()
    }))
    .unwrap();
    let look = Arc::new(
        crate::looks::LookAsset::new(
            "Valid look too large for the analysis device".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(81, [0.; 3], [1.; 3], vec![[0.5; 3]; 81usize.pow(3)])
                    .unwrap(),
            ),
        )
        .unwrap(),
    );
    assert!(crate::gpu::lut3d::Lut::require_limits(&device.limits(), look.lut()).is_err());
    document.project.output.lut = Some(look.clone());
    for source in document.project.sources.iter_mut() {
        let mut layers = crate::project::AdjustmentLayers::default();
        for _ in 0..10 {
            layers
                .add(
                    "Old exposure".into(),
                    crate::adjustment_layers::AdjustmentLayer::global(AdjustmentLayerSettings {
                        exposure: 16.,
                        ..Default::default()
                    }),
                )
                .unwrap();
        }
        source.adjustment_layers = Some(layers);
    }
    document.project.require_runtime().unwrap();
    let mut previous = None;
    for warm in [false, true] {
        let (loader, developments) = observed();
        let mut editor = Editor::new(&device, &queue);
        editor.document = document.clone().into();
        editor.document.visit.current_source = Some(editor.document.project.order[0]);
        editor.document.visit.working_view =
            crate::editor::working_view::WorkingView::new(device.clone(), queue.clone());
        editor.source_services = Some(source::Services {
            device: device.clone(),
            queue: queue.clone(),
            developments: loader.clone(),
            renderers: Arc::default(),
        });
        if warm {
            for id in editor.document.project.order.clone() {
                let physical = editor.document.source_input(id).unwrap();
                let camera = pollster::block_on(editor.document.sampling_inputs.acquire(
                    id,
                    &physical,
                    loader.provider(None).as_ref(),
                    &Cancellation::default(),
                ))
                .unwrap()
                .unwrap();
                editor.document.sampling_inputs.record(id, &camera);
            }
            loader.close_and_wait().unwrap();
        }
        let before = editor.document.project.clone();
        let mut captured = request.clone();
        captured.document = editor.document.capture();
        approved(&mut captured);
        with_application(editor, |app| {
            app.enter(|editor, cx| editor.start_analysis(captured, cx));
            app.run_until(std::time::Duration::from_secs(45), |editor| {
                !editor.analysis_fitting()
            });
            let accepted = app.read(|editor| {
                assert!(
                    editor.status.starts_with("Analyzed 2 frames"),
                    "{}",
                    editor.status
                );
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    Some("Analyze roll")
                );
                assert_eq!(editor.document.project.output.lut, Some(look.clone()));
                assert!(
                    editor
                        .document
                        .project
                        .dmin()
                        .unwrap()
                        .iter()
                        .all(|v| (v - 3000. / 4031.).abs() <= 2. / 65535.)
                );
                assert!(
                    editor.document.project.sources.iter().all(|s| s
                        .adjustment_layers
                        .as_ref()
                        .unwrap()
                        .layers
                        .len()
                        == 1)
                );
                editor.document.project.clone()
            });
            if let Some(previous) = &previous {
                assert_eq!(&accepted, previous);
            }
            previous = Some(accepted.clone());
            app.enter(|editor, cx| {
                editor.restore_history(crate::editor::history::Direction::Undo, cx)
            });
            app.read(|editor| assert_eq!(editor.document.project, before));
            app.enter(|editor, cx| {
                editor.restore_history(crate::editor::history::Direction::Redo, cx)
            });
            app.read(|editor| assert_eq!(editor.document.project, accepted));
        });
        assert_eq!(developments.load(Ordering::Relaxed), 2);
        loader.close_and_wait().unwrap();
    }
}
