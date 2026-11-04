use super::*;
use crate::{
    editor::analysis::preparation_tests::{observed, request},
    scan_regions::Areas,
};

mod film_edges;

struct Resources {
    sources: Arc<dyn SourceProvider>,
    sampling_inputs: SamplingInputs,
    exposures: Arc<crate::exposure_metadata::Store>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderers: Arc<crate::renderer::RendererCache>,
}

impl Resources {
    fn fit(&self, request: Request, input: source::Context) -> Result<Fit, Error> {
        Fit::new(
            self.device.clone(),
            self.queue.clone(),
            self.renderers.get(&self.device, &self.queue)?,
            request,
            input,
        )
    }

    async fn acquire(
        &self,
        request: Arc<Request>,
        index: usize,
        cancelled: Cancellation,
    ) -> Result<Option<Scan>, Error> {
        request
            .document
            .acquire_source(
                request.document.project.order[index],
                self.sources.as_ref(),
                &self.exposures,
                &cancelled,
            )
            .await
    }
}

async fn run(
    request: Request,
    input: source::Context,
    resources: Resources,
    cancelled: Cancellation,
    progress: impl FnMut(usize, usize),
) -> Result<Option<Outcome>, Error> {
    let state = resources.fit(request, input)?;
    state
        .run(
            resources.sources,
            resources.sampling_inputs,
            resources.exposures,
            cancelled,
            progress,
        )
        .await
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
            crate::project::SourceAnalysis { source: id, areas }
        })
        .collect();
    let files = request
        .document
        .project
        .order
        .iter()
        .map(|&id| thumbnails::FileKey::new(&request.document.source_path(id).unwrap()))
        .collect();
    request.areas = Some(analysis::Approved { entries, files });
}

fn input(services: &Resources, request: &Request) -> source::Context {
    let scan = futures_lite::future::block_on(services.acquire(
        Arc::new(request.clone()),
        0,
        Cancellation::default(),
    ))
    .unwrap()
    .unwrap();
    source::Context {
        reference: None,
        source: scan.source().clone(),
        correction: None,
    }
}

#[test]
fn captured_inputs_fit_reviewed_support_and_keep_only_bounded_statistics_per_frame() {
    let (mut request, _directory, device, queue) = request();
    approved(&mut request);
    request.document.project.set_gamma(0.7);
    let before = request.document.project.clone();
    let (scheduler, _) = observed();
    let services = Resources {
        exposures: Arc::default(),
        sampling_inputs: Default::default(),
        sources: scheduler.provider(None),
        device,
        queue,
        renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
    };
    let captured = input(&services, &request);
    // Inspect the sampling boundary directly: every ordered frame contributes both
    // populations, with the same per-frame bound used by the numerical estimator.
    let mut state = services.fit(request.clone(), captured.clone()).unwrap();
    state.geometry().unwrap();
    for index in 0..before.order.len() {
        let scan = futures_lite::future::block_on(services.acquire(
            Arc::new(request.clone()),
            index,
            Cancellation::default(),
        ))
        .unwrap()
        .unwrap();
        assert!(state.sample(index, scan, || false).unwrap());
        let frame = &state.owned[index];
        assert!(frame.film.pixels.len() <= MAX_SAMPLES_PER_FRAME);
        assert!(
            (MIN_PICTURE_SAMPLES..=MAX_SAMPLES_PER_FRAME).contains(&frame.picture.pixels.len())
        );
    }
    assert_eq!(state.owned.len(), before.order.len());
    drop(state);
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
        request.areas.as_ref().unwrap().entries
    );
    assert!(
        after
            .source(before.order[0])
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .photo
            .exposure
            > after
                .source(before.order[1])
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure
    );
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
        exposures: Arc::default(),
        sampling_inputs: Default::default(),
        sources: scheduler.provider(None),
        device,
        queue,
        renderers: Arc::new(crate::renderer::RendererCache::new(Default::default())),
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
