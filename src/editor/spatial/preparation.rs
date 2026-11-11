//! Shared acquisition and physical-region preparation, independent of consumer transactions.
use crate::editor::{Document, Error, document_scope::SamplingInputs};
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
    camera: crate::source::CameraSamples,
}

struct GeometryFrame {
    input: crate::spatial_metadata::Input,
    size: [u32; 2],
    rgb: Vec<[f32; 3]>,
}

fn geometry_frames(
    captures: Vec<CapturedSource>,
    checkpoint: &impl Fn() -> Result<(), String>,
) -> Result<Vec<GeometryFrame>, Error> {
    captures
        .into_iter()
        .map(|capture| {
            let (size, rgb) = geometry_preview(&capture.camera.pixels(), 768, checkpoint)?;
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
    maximum: u32,
    checkpoint: &impl Fn() -> Result<(), String>,
) -> Result<([u32; 2], Vec<[f32; 3]>), Error> {
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
            rgb.push([pixel[0], pixel[1], pixel[2]]);
        }
    }
    Ok((size, rgb))
}

pub(in crate::editor) async fn prepare(
    document: Arc<Document>,
    sources: Arc<dyn SourceProvider>,
    mut retained: SamplingInputs,
    initialization: Arc<crate::source::Initialization>,
    cancelled: Cancellation,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Prepared>, Error> {
    let known = document.spatial_inputs_with(&initialization);
    let cached = document
        .spatial_metadata
        .matching(&document.project, &known);
    let regions_ready = cached.groups.iter().map(|g| g.sources.len()).sum::<usize>()
        == document.project.order.len();
    let total = document.project.order.len();
    retained.retain(&document, &initialization);
    let mut frames = Vec::with_capacity(total);
    let mut work = Vec::with_capacity(total);
    for &source in &document.project.order {
        let physical = document.source_input(source, &initialization)?;
        if !regions_ready || !retained.contains(source, &physical) {
            work.push((source, physical));
        }
    }
    let mut completed = total - work.len();
    progress(completed, total);
    {
        let existing = retained.clone();
        let captures = futures_util::stream::iter(work.into_iter().map(|(source, physical)| {
            capture_source(sources.as_ref(), &existing, &cancelled, source, physical)
        }))
        .buffered(2);
        let mut captures = std::pin::pin!(captures);
        while let Some(capture) = captures.next().await {
            let Some(capture) = capture? else {
                return Ok(None);
            };
            retained.record(capture.input.source, &capture.camera);
            if !regions_ready {
                frames.push(capture);
            }
            completed += 1;
            if completed < total {
                progress(completed, total);
            }
        }
    }
    let inputs = document.spatial_inputs_with(&initialization);
    let metadata = if regions_ready {
        cached
    } else {
        let retired = cancelled.clone();
        let Some(metadata) = background::compute(&cancelled, move || {
            let development = inputs
                .first()
                .ok_or(Error::UnusableSample("no roll development"))?
                .development
                .decode
                .clone();
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
            if metadata.matching(&document.project, &inputs) != metadata {
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
    sources: &dyn SourceProvider,
    retained: &SamplingInputs,
    cancelled: &Cancellation,
    source: crate::project::SourceId,
    physical: crate::source::Input,
) -> Result<Option<CapturedSource>, Error> {
    let Some(camera) = retained
        .acquire(source, &physical, sources, cancelled)
        .await?
    else {
        return Ok(None);
    };
    let input = crate::spatial_metadata::Input {
        source,
        revision: camera.file().revision.clone().ok_or(Error::UnusableSample(
            "spatial source revision is unavailable",
        ))?,
        development: camera.development().clone(),
        correction: None,
    };
    Ok(Some(CapturedSource { input, camera }))
}

#[cfg(test)]
mod tests;
