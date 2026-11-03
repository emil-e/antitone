//! Assemble review windows from completed shared detection, without acquiring image pixels.
use super::*;
use crate::editor::analysis_services::Cancellation;

#[cfg(all(test, target_os = "macos"))]
mod tests;

fn areas(
    request: &analysis::Request,
    regions: &crate::spatial_metadata::SourceRegions,
) -> Result<Areas, Error> {
    let project = &request.document.project;
    let source = regions.input.source;
    let active = regions.input.development.active_area.into();
    let previous = project
        .analysis_regions
        .iter()
        .find(|row| row.source == source);
    let mut areas = match previous {
        Some(row)
            if row.areas.ready() && (row.areas.opening_edited || row.areas.opening_shared) =>
        {
            row.areas.clone()
        }
        _ => {
            let mut suggested = regions.sampling_areas();
            if let Some(previous) = previous {
                if previous.areas.opening_edited
                    && let Some(opening) = previous.areas.opening
                {
                    suggested.edit_opening(Areas::coverage(opening)?);
                    suggested.refit_picture(active);
                }
                if previous.areas.picture_edited
                    && let Some(picture) = previous.areas.picture
                {
                    suggested.edit_picture(Areas::coverage(picture)?);
                }
            }
            suggested
        }
    };
    if areas.opening.is_none()
        && let Some(field) = project.active_flat_field()
    {
        areas.opening = Some(
            Coverage::rectangle(analysis::rect(field.map.area()))
                .corners()
                .map(|p| p.0),
        );
    }
    if let Some(explicit) = project.analysis_area {
        areas.edit_picture(Coverage::rectangle(analysis::rect(explicit)));
        if areas.opening.is_none() {
            areas.opening = Some(Coverage::rectangle(active).corners().map(|p| p.0));
        }
    } else if !areas.picture_edited
        && let Ok(picture) = regions.picture_sampling(analysis::sampling_inset(active))
    {
        areas.picture = Some(picture);
        areas.picture_geometry = None;
        areas.picture_constraints.clear();
        areas.picture_observed = [false; 4];
        areas.picture_weak = [false; 4];
    }
    areas.validate(active)?;
    Ok(areas)
}

pub(super) fn assemble(
    request: &analysis::Request,
    device: &wgpu::Device,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Approved>, Error> {
    if cancelled() {
        return Ok(None);
    }
    request.document.validate_for_device(device)?;
    request.document.project.require_scan_sources()?;
    let order = &request.document.project.order;
    let metadata = request.document.ready_spatial_metadata();
    let mut entries = Vec::with_capacity(order.len());
    let mut files = Vec::with_capacity(order.len());
    for (index, &source) in order.iter().enumerate() {
        progress(index, order.len());
        if cancelled() {
            return Ok(None);
        }
        let regions = metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == source)
            .ok_or(Error::UnusableSample("shared area detection is not ready"))?;
        entries.push(SourceAnalysis {
            source,
            areas: areas(request, regions)?,
        });
        files.push(
            request
                .document
                .file_key(&request.document.source_path(source)?),
        );
    }
    progress(order.len(), order.len());
    Ok((!cancelled()).then_some(Approved { entries, files }))
}

pub(in crate::editor) async fn prepare(
    request: analysis::Request,
    services: super::super::analysis_services::Services,
    cancelled: Cancellation,
    progress: impl FnMut(usize, usize) + Send + 'static,
) -> Result<Option<Approved>, Error> {
    let device = services.device.clone();
    let stopped = cancelled.clone();
    services
        .cpu(&cancelled, move || {
            assemble(&request, &device, || stopped.is_cancelled(), progress)
        })
        .await
        .map(Option::flatten)
}
