//! Shared acquisition and physical-region preparation, independent of consumer transactions.
use crate::editor::{Document, Error, document, document_scope::SamplingInputs};
use crate::{
    background,
    cancellation::Cancellation,
    scan::AnalysisInput,
    scan_regions::{Evidence, FrameInput, prepare_spatial},
    source::SourceProvider,
    spatial_metadata::Metadata,
};
use futures_util::StreamExt;
use std::sync::Arc;

pub(in crate::editor) struct Prepared {
    pub metadata: Metadata,
    pub sampling_inputs: SamplingInputs,
}

struct CapturedSource {
    source: crate::project::SourceId,
    frame: Option<FrameInput>,
    camera: Arc<AnalysisInput>,
}

pub(in crate::editor) async fn prepare(
    document: Arc<Document>,
    device: wgpu::Device,
    sources: Arc<dyn SourceProvider>,
    mut retained: SamplingInputs,
    exposures: Arc<crate::exposure_metadata::Store>,
    cancelled: Cancellation,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Prepared>, Error> {
    let captured = document.clone();
    let Some((inputs, cached)) = background::compute(&cancelled, move || {
        captured.validate_for_device(&device)?;
        let inputs = captured.spatial_inputs();
        if captured
            .project
            .order
            .iter()
            .any(|id| !inputs.iter().any(|input| input.source == *id))
        {
            return Err(Error::UnusableSample("spatial source file is unavailable"));
        }
        let cached = captured
            .spatial_metadata
            .matching(&captured.project, &inputs);
        Ok((inputs, cached))
    })
    .await?
    else {
        return Ok(None);
    };
    let regions_ready =
        cached.groups.iter().map(|g| g.sources.len()).sum::<usize>() == inputs.len();
    let total = inputs.len();
    retained.retain(&document);
    let mut frames = Vec::with_capacity(total);
    let mut work = Vec::with_capacity(total);
    for input in inputs {
        let existing = retained.get(input.source, &document.source_key(input.source)?);
        if !regions_ready || existing.is_none() {
            work.push((input, existing));
        }
    }
    let mut completed = total - work.len();
    progress(completed, total);
    {
        // Bound source leases and CPU captures together, not just native development.
        let captures = futures_util::stream::iter(work.into_iter().map(|(input, existing)| {
            capture_source(
                document.clone(),
                sources.as_ref(),
                &exposures,
                &cancelled,
                input,
                existing,
                regions_ready,
            )
        }))
        .buffered(2);
        let mut captures = std::pin::pin!(captures);
        while let Some(capture) = captures.next().await {
            let Some(capture) = capture? else {
                return Ok(None);
            };
            if let Some(frame) = capture.frame {
                frames.push(frame);
            }
            retained.insert(
                capture.source,
                document.source_key(capture.source)?,
                capture.camera,
            );
            completed += 1;
            if completed < total {
                progress(completed, total);
            }
        }
    }
    let metadata = if regions_ready {
        cached
    } else {
        let retired = cancelled.clone();
        let Some(metadata) = background::compute(&cancelled, move || {
            let development = document
                .project
                .scan_basis()
                .ok_or(Error::UnusableSample("no roll development"))
                .and_then(|_| {
                    document
                        .project
                        .sources
                        .first()
                        .map(|source| source.development.decode.clone())
                        .ok_or(Error::UnusableSample("no roll development"))
                })?;
            let metadata =
                prepare_spatial(frames, development, document.spatial_code_scale(), || {
                    if retired.is_cancelled() {
                        Err("spatial preparation cancelled".into())
                    } else {
                        Ok(())
                    }
                })
                .map_err(Error::Project)?;
            if metadata.matching(&document.project, &document.spatial_inputs()) != metadata {
                return Err(Error::UnusableSample(
                    "source changed during spatial preparation",
                ));
            }
            Ok(metadata)
        })
        .await?
        else {
            return Ok(None);
        };
        metadata
    };
    progress(total, total);
    Ok(Some(Prepared {
        metadata,
        sampling_inputs: retained,
    }))
}

async fn capture_source(
    document: Arc<Document>,
    sources: &dyn SourceProvider,
    exposures: &crate::exposure_metadata::Store,
    cancelled: &Cancellation,
    input: crate::spatial_metadata::Input,
    existing: Option<Arc<AnalysisInput>>,
    regions_ready: bool,
) -> Result<Option<CapturedSource>, Error> {
    let Some(scan) = document
        .acquire_source(input.source, sources, exposures, cancelled)
        .await?
    else {
        return Ok(None);
    };
    let stopped = cancelled.clone();
    let id = input.source;
    let Some(Some((frame, camera))) = background::compute(cancelled, move || {
        let path = document.source_path(input.source)?;
        let actual =
            document::source_metadata(&path, &scan.source().metadata(), scan.source().fixed_wb())?;
        document::verify_source(document.project.source(input.source)?, &actual)?;
        document::verify_scan(&actual, &scan)?;
        document.project.require_scan_source(&actual.development)?;
        let camera = match existing {
            Some(camera) => camera,
            None => {
                let Some(camera) = AnalysisInput::capture(&scan, || stopped.is_cancelled())? else {
                    return Ok(None);
                };
                Arc::new(camera)
            }
        };
        let frame = if regions_ready {
            None
        } else {
            let (w, h, pixels) = scan.measurement_preview(768, input.code_scale)?;
            let rgb: Vec<_> = pixels.into_iter().map(|p| [p[0], p[1], p[2]]).collect();
            let evidence = Evidence::observe([w, h], &rgb)?;
            Some(FrameInput::new(input, [w, h], rgb, evidence)?)
        };
        Ok(Some((frame, camera)))
    })
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(CapturedSource {
        source: id,
        frame,
        camera,
    }))
}
