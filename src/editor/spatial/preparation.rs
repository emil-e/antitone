//! Shared acquisition and physical-region preparation, independent of consumer transactions.
use crate::editor::{
    Document, Error,
    analysis_services::{Cancellation, Services},
    document,
};
use crate::{
    scan_regions::{Evidence, FrameInput, prepare_spatial},
    spatial_metadata::Metadata,
};
use std::sync::Arc;

pub(super) async fn prepare(
    document: Arc<Document>,
    services: Services,
    cancelled: Cancellation,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Metadata>, Error> {
    let captured = document.clone();
    let device = services.device.clone();
    let Some((inputs, cached)) = services
        .cpu(&cancelled, move || {
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
    if cached.groups.iter().map(|g| g.sources.len()).sum::<usize>() == inputs.len() {
        return Ok(Some(cached));
    }
    let total = inputs.len();
    let mut frames = Vec::with_capacity(total);
    for (index, input) in inputs.into_iter().enumerate() {
        progress(index, total);
        let Some(scan) = services
            .acquire_source(document.clone(), input.source, cancelled.clone())
            .await?
        else {
            return Ok(None);
        };
        let captured = document.clone();
        let Some(frame) = services
            .cpu(&cancelled, move || {
                let path = captured.source_path(input.source)?;
                let actual = document::source_metadata(
                    &path,
                    &scan.source().metadata(),
                    scan.source().fixed_wb(),
                )?;
                document::verify_source(captured.project.source(input.source)?, &actual)?;
                document::verify_scan(&actual, &scan)?;
                captured.project.require_scan_source(&actual.development)?;
                if crate::source_revision::FileRevision::read(&path).ok()
                    != Some(input.revision.clone())
                {
                    return Err(Error::UnusableSample(
                        "source changed during spatial preparation",
                    ));
                }
                let (w, h, pixels) = scan.measurement_preview(768, input.code_scale)?;
                let rgb: Vec<_> = pixels.into_iter().map(|p| [p[0], p[1], p[2]]).collect();
                let evidence = Evidence::observe([w, h], &rgb)?;
                FrameInput::new(input, [w, h], rgb, evidence)
            })
            .await?
        else {
            return Ok(None);
        };
        frames.push(frame);
    }
    let retired = cancelled.clone();
    let result = services
        .cpu(&cancelled, move || {
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
        .await?;
    progress(total, total);
    Ok(result)
}
