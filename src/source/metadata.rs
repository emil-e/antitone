//! Header/development compatibility shared by source loading and document registration.
use crate::{
    Error,
    project::{Source, SourceDevelopment, SourceId},
    raw::{Metadata, RawFile},
    scan::Scan,
};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, Error>;
fn failure(error: impl std::fmt::Display) -> Error {
    Error::Project(error.to_string())
}
fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(failure)?.join(path)
    })
}

pub(crate) fn metadata_development(
    metadata: &Metadata,
    fixed_wb: [f64; 4],
) -> Result<SourceDevelopment> {
    SourceDevelopment::from_metadata(metadata, fixed_wb)
}

pub(crate) fn open_source(path: &Path, wb: Option<[f64; 4]>) -> Result<(Source, RawFile)> {
    let path = absolute(path)?;
    let started = std::time::Instant::now();
    let raw = RawFile::open_with_wb(&path, wb)
        .map_err(|error| failure(format!("{}: {error}", path.display())))?;
    log::debug!(header_seconds = started.elapsed().as_secs_f64(); "source_registration");
    let fixed_wb = raw.fixed_wb(wb.unwrap_or_else(|| raw.metadata.as_shot_wb()));
    Ok((source_metadata(&path, &raw.metadata, fixed_wb)?, raw))
}

pub(crate) fn source_metadata(
    path: &Path,
    metadata: &Metadata,
    fixed_wb: [f64; 4],
) -> Result<Source> {
    Ok(Source {
        id: SourceId(0),
        path: absolute(path)?,
        development: metadata_development(metadata, fixed_wb)?,
        reference_only: false,
        adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
            Default::default(),
        )),
        repairs: Default::default(),
        orientation: metadata.photo_orientation()?,
        crop: None,
    })
}

pub(crate) fn verify_source(expected: &Source, actual: &Source) -> Result<()> {
    if !expected.development.same_processing(&actual.development) {
        return Err(failure(format!(
            "{}: RAW development identity differs",
            actual.path.display()
        )));
    }
    Ok(())
}

pub(crate) fn verify_scan(source: &Source, scan: &Scan) -> Result<()> {
    let (width, height) = scan.dimensions()?;
    let area = scan.area;
    let actual = SourceDevelopment {
        decode: scan
            .source()
            .metadata()
            .development(scan.source().fixed_wb()),
        lens_correction: scan.source().metadata().lens_identity()?,
        dng_development: scan.source().metadata().dng_identity(),
        dimensions: [width, height],
        active_area: [area.x, area.y, area.width, area.height],
        camera_model: scan.camera_model.clone(),
        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
        input_floor: scan.source().input_floor(),
    };
    if !source.development.same_processing(&actual) {
        return Err(failure(
            "developed RAW differs from its recorded header identity",
        ));
    }
    Ok(())
}
