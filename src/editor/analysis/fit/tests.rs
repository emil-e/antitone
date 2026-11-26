use super::*;
use crate::{
    editor::analysis::preparation_tests::{observed, request},
    scan_regions::Areas,
};

mod boundaries;
mod correction;
mod film_edges;
mod interpretation;

struct Resources {
    sources: Arc<dyn SourceProvider>,
    sampling_inputs: SamplingInputs,
    initialization: Arc<crate::source::Initialization>,
    device: wgpu::Device,
    queue: crate::gpu::submission::Queue,
    renderers: Arc<crate::editor::display::DisplayFixture>,
}

impl Resources {
    fn classifier_drt(&self) -> drt::Target {
        self.renderers
            .get(&self.device, &self.queue)
            .unwrap()
            .drt(OutputTarget::Srgb)
            .clone()
    }

    fn fit(&self, request: Request, input: Option<Arc<Correction>>) -> Result<Fit, Error> {
        let renderer = self.renderers.get(&self.device, &self.queue)?;
        pollster::block_on(Fit::acquire(
            renderer.evaluation().clone(),
            self.queue.clone(),
            request,
            super::Resources {
                correction: input.into(),
                sources: self.sources.clone(),
                sampling_inputs: self.sampling_inputs.clone(),
                initialization: self.initialization.clone(),
            },
            &Cancellation::default(),
        ))
        .map(|fit| fit.expect("uncancelled fitting acquisition"))
    }

    async fn acquire(
        &self,
        request: Arc<Request>,
        index: usize,
        cancelled: Cancellation,
    ) -> Result<Option<Arc<DevelopedSource>>, Error> {
        let physical = request
            .document
            .source_input(request.document.project.order[index], &self.initialization)?;
        futures_lite::future::or(
            async {
                cancelled.wait().await;
                Ok(None)
            },
            async {
                physical
                    .source(self.sources.as_ref(), crate::source::Priority::Foreground)
                    .await
                    .map(|acquired| Some(acquired.prepared.source))
            },
        )
        .await
    }
}

async fn run(
    request: Request,
    input: Option<Arc<Correction>>,
    resources: Resources,
    cancelled: Cancellation,
    progress: impl FnMut(usize, usize),
) -> Result<Option<Outcome>, Error> {
    let renderer = resources
        .renderers
        .get(&resources.device, &resources.queue)?;
    let state = Fit::acquire(
        renderer.evaluation().clone(),
        resources.queue,
        request,
        super::Resources {
            correction: input.into(),
            sources: resources.sources,
            sampling_inputs: resources.sampling_inputs,
            initialization: resources.initialization,
        },
        &cancelled,
    )
    .await?;
    match state {
        Some(state) => {
            state
                .run(
                    renderer.drt(OutputTarget::Srgb).clone(),
                    cancelled,
                    progress,
                )
                .await
        }
        None => Ok(None),
    }
}

fn approved(request: &mut Request) {
    let entries = request
        .document
        .project
        .order
        .iter()
        .map(|&id| {
            let active = request
                .document
                .project
                .source(id)
                .unwrap()
                .development
                .active_area
                .into();
            let mut areas = Areas::default();
            areas.edit_opening(Coverage::rectangle(active));
            areas.edit_picture(Coverage::rectangle(rect([96, 96, 320, 320])));
            analysis::ReviewedPhoto {
                source: id,
                areas,
                file: request
                    .document
                    .file_key(&request.document.source_path(id).unwrap()),
            }
        })
        .collect();
    request.areas = Some(analysis::Approved { entries });
}

fn input(services: &Resources, request: &Request) -> Option<Arc<Correction>> {
    Correction::prepare(
        &services.device,
        request.document.project.active_flat_field(),
        None,
    )
    .unwrap()
}

#[test]
fn local_support_finds_the_bare_light_opening_and_reuses_acquisition_for_sampling() {
    let (mut request, _directory, device, queue) = request();
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/bare-light.dng");
    request.document = crate::editor::document::Document::default();
    request.document.import(&[file]).unwrap();
    request.selected = request.document.project.order[0];
    request.local_support = true;

    let (scheduler, developments) = observed();
    let services = Resources {
        sources: scheduler.provider(None),
        sampling_inputs: Default::default(),
        initialization: Arc::default(),
        renderers: Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
        device,
        queue,
    };
    let fit = services.fit(request, None).unwrap();
    assert_eq!(fit.support, Some([10, 10, 364, 300].into()));
    assert_eq!(fit.cameras.len(), 1);
    assert_eq!(developments.load(std::sync::atomic::Ordering::Relaxed), 1);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn captured_inputs_fit_reviewed_support_and_keep_only_bounded_statistics_per_frame() {
    let (mut request, _directory, device, queue) = request();
    approved(&mut request);
    request.areas.as_mut().unwrap().entries[0]
        .areas
        .edit_picture(Coverage::rectangle(rect([360, 96, 320, 320])));
    request.document.project.order.reverse();
    request.document.project.set_gamma(0.7);
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let mut services = Resources {
        initialization: Arc::default(),
        sampling_inputs: Default::default(),
        sources: scheduler.provider(None),
        device: device.clone(),
        queue,
        renderers: Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
    };
    let captured = input(&services, &request);
    let mut state = services.fit(request.clone(), captured.clone()).unwrap();
    state.geometry().unwrap();
    let mut retained_state = services.fit(request.clone(), captured.clone()).unwrap();
    retained_state.geometry().unwrap();
    for index in 0..before.order.len() {
        let source = futures_lite::future::block_on(services.acquire(
            Arc::new(request.clone()),
            index,
            Cancellation::default(),
        ))
        .unwrap()
        .unwrap();
        let retained = Arc::new(
            crate::scan::AnalysisInput::capture(&source, || false)
                .unwrap()
                .unwrap(),
        );
        if index == 0 {
            let mut smaller = source.area();
            smaller.width -= 8;
            let wrong_grid = Arc::new(crate::scan::AnalysisInput::authored(
                crate::geometry::SourceRaster::new(smaller, [1, 1]).unwrap(),
                vec![[1.; 3]],
            ));
            let mut rejected = services.fit(request.clone(), captured.clone()).unwrap();
            rejected.geometry().unwrap();
            assert!(matches!(
                rejected.sample(index, source.clone(), Some(wrong_grid), || false),
                Err(Error::InvalidParameter(
                    "captured camera grid differs from source"
                ))
            ));
            assert!(
                rejected.owned.is_empty(),
                "incompatible retained data must not silently fall back to native statistics"
            );
        }
        assert!(state.sample(index, source.clone(), None, || false).unwrap());
        assert!(
            retained_state
                .sample(index, source, Some(retained.clone()), || false)
                .unwrap()
        );
        let id = request.document.project.order[index];
        services
            .sampling_inputs
            .insert(id, request.document.source_key(id).unwrap(), retained);
        let frame = &state.owned[index];
        let reused = &retained_state.owned[index];
        assert_eq!(reused.film.pixels, frame.film.pixels);
        assert_eq!(reused.picture.pixels, frame.picture.pixels);
        let reviewed_x = if index == 0 { 96..416 } else { 360..680 };
        assert!(!frame.picture.positions.is_empty());
        assert!(
            frame
                .picture
                .positions
                .iter()
                .all(|[x, _]| reviewed_x.contains(x))
        );
        assert!(frame.film.pixels.len() <= MAX_SAMPLES_PER_FRAME);
        assert!(
            (MIN_PICTURE_SAMPLES..=MAX_SAMPLES_PER_FRAME).contains(&frame.picture.pixels.len())
        );
    }
    assert_eq!(state.owned.len(), before.order.len());
    drop(state);
    drop(retained_state);
    let outcome = futures_lite::future::block_on(run(
        request.clone(),
        captured,
        services,
        Cancellation::default(),
        |_, _| {},
    ))
    .unwrap()
    .unwrap();
    assert_eq!(outcome.frames, 2);
    let after = outcome.document.project;
    // Authored camera codes: clean film is 3000 at a 4031 white level.
    assert!(
        after
            .dmin()
            .unwrap()
            .iter()
            .all(|v| (v - 3000. / 4031.).abs() <= 2. / 65535.)
    );
    assert_eq!(after.input.gamma(), 0.7);
    assert_eq!(
        after.analysis_regions,
        request
            .areas
            .as_ref()
            .unwrap()
            .records()
            .collect::<Vec<_>>()
    );
    for &id in &before.order {
        assert_eq!(
            after
                .source(id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo,
            AdjustmentLayerSettings::default()
        );
    }
    assert_eq!(request.document.project, before);

    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancellation_after_sampling_cannot_return_a_partially_fitted_document() {
    let (mut request, _directory, device, queue) = request();
    approved(&mut request);
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let services = Resources {
        initialization: Arc::default(),
        sampling_inputs: Default::default(),
        sources: scheduler.provider(None),
        device: device.clone(),
        queue,
        renderers: Arc::new(crate::editor::display::DisplayFixture::for_device(&device)),
    };
    let captured = input(&services, &request);
    let cancelled = Cancellation::default();
    let control = cancelled.clone();
    let result = futures_lite::future::block_on(run(
        request.clone(),
        captured,
        services,
        cancelled,
        move |done, total| {
            if done == total {
                control.cancel();
            }
        },
    ))
    .unwrap();
    assert!(result.is_none());
    assert_eq!(request.document.project, before);
    scheduler.close_and_wait().unwrap();
}
