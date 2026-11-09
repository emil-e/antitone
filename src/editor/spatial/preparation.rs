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
    input: crate::spatial_metadata::Input,
    camera: Arc<AnalysisInput>,
}

struct GeometryFrame {
    input: crate::spatial_metadata::Input,
    size: [u32; 2],
    rgb: Vec<[u16; 3]>,
}

fn geometry_frames(
    captures: Vec<CapturedSource>,
    checkpoint: &impl Fn() -> Result<(), String>,
) -> Result<Vec<GeometryFrame>, Error> {
    let code_scale = captures.iter().try_fold(1_f64, |scale, capture| {
        capture
            .camera
            .effective_floor(None)
            .map(|floor| scale.max(floor / crate::processing::CAMERA_INPUT_FLOOR))
    })?;
    captures
        .into_iter()
        .map(|capture| {
            let (size, rgb) = geometry_preview(&capture.camera, code_scale, 768, checkpoint)?;
            Ok(GeometryFrame {
                input: capture.input,
                size,
                rgb,
            })
        })
        .collect()
}

/// Geometry uses the whole uncorrected raster, including dark holder/film sites.
/// Resample cell centres over the same active extent, rather than integer-striding
/// a 1024-wide grid and silently changing the detector's resolution or coordinate phase.
fn geometry_preview(
    camera: &AnalysisInput,
    code_scale: f64,
    maximum: u32,
    checkpoint: &impl Fn() -> Result<(), String>,
) -> Result<([u32; 2], Vec<[u16; 3]>), Error> {
    if maximum == 0 {
        return Err(Error::InvalidParameter("preview size must be positive"));
    }
    let area = camera.area();
    let scale = (f64::from(maximum) / f64::from(area.width.max(area.height))).min(1.);
    let size = [area.width, area.height].map(|v| (f64::from(v) * scale).round().max(1.) as u32);
    let source = camera.size();
    let mut rgb = Vec::with_capacity((size[0] * size[1]) as usize);
    for y in 0..size[1] {
        checkpoint().map_err(Error::Project)?;
        for x in 0..size[0] {
            let pixel = if size == source {
                camera.pixel((y * source[0] + x) as usize)?
            } else {
                let position: [f64; 2] = std::array::from_fn(|axis| {
                    ((f64::from([x, y][axis]) + 0.5) * f64::from(source[axis])
                        / f64::from(size[axis])
                        - 0.5)
                        .clamp(0., f64::from(source[axis] - 1))
                });
                let low = position.map(|v| v.floor() as u32);
                let high: [u32; 2] =
                    std::array::from_fn(|axis| (low[axis] + 1).min(source[axis] - 1));
                let [dx, dy] = std::array::from_fn(|axis| position[axis] - f64::from(low[axis]));
                let at = |x, y| camera.pixel((y * source[0] + x) as usize);
                let [a, b, c, d] = [
                    at(low[0], low[1])?,
                    at(high[0], low[1])?,
                    at(low[0], high[1])?,
                    at(high[0], high[1])?,
                ];
                std::array::from_fn(|channel| {
                    let upper = f64::from(a[channel]) * (1. - dx) + f64::from(b[channel]) * dx;
                    let lower = f64::from(c[channel]) * (1. - dx) + f64::from(d[channel]) * dx;
                    (upper * (1. - dy) + lower * dy) as f32
                })
            };
            let codes = crate::scan::measurement_codes(pixel, code_scale)?;
            rgb.push([codes[0], codes[1], codes[2]]);
        }
    }
    Ok((size, rgb))
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
            )
        }))
        .buffered(2);
        let mut captures = std::pin::pin!(captures);
        while let Some(capture) = captures.next().await {
            let Some(capture) = capture? else {
                return Ok(None);
            };
            retained.insert(
                capture.input.source,
                document.source_key(capture.input.source)?,
                capture.camera.clone(),
            );
            if !regions_ready {
                frames.push(capture);
            }
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
            let checkpoint = || {
                if retired.is_cancelled() {
                    Err("spatial preparation cancelled".into())
                } else {
                    Ok(())
                }
            };
            let frames = geometry_frames(frames, &checkpoint)?
                .into_iter()
                .map(|frame| {
                    checkpoint().map_err(Error::Project)?;
                    let evidence = Evidence::observe(frame.size, &frame.rgb)?;
                    FrameInput::new(frame.input, frame.size, frame.rgb, evidence)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let metadata =
                prepare_spatial(frames, development, checkpoint).map_err(Error::Project)?;
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
) -> Result<Option<CapturedSource>, Error> {
    if let Some(camera) = existing {
        return Ok(Some(CapturedSource { input, camera }));
    }
    let Some(scan) = document
        .acquire_source(input.source, sources, exposures, cancelled)
        .await?
    else {
        return Ok(None);
    };
    let stopped = cancelled.clone();
    let captured = input.clone();
    let Some(Some(camera)) = background::compute(cancelled, move || {
        let path = document.source_path(captured.source)?;
        let actual = document::source_metadata(
            &path,
            &scan.source().metadata(),
            scan.source().white_balance(),
        )?;
        document::verify_source(document.project.source(captured.source)?, &actual)?;
        document::verify_scan(&actual, &scan)?;
        document.project.require_scan_source(&actual.development)?;
        Ok(AnalysisInput::capture(&scan, || stopped.is_cancelled())?.map(Arc::new))
    })
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(CapturedSource { input, camera }))
}

#[cfg(test)]
mod tests;
