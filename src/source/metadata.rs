//! Header/development compatibility shared by source loading and document registration.
use crate::{
    Error,
    project::{Source, SourceDevelopment, SourceId},
    raw::{Metadata, RawFile},
    scan::DevelopedSource,
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

pub(crate) fn open_source(path: &Path, wb: Option<[f64; 4]>) -> Result<(Source, RawFile)> {
    let path = absolute(path)?;
    let started = std::time::Instant::now();
    let (raw, development) = SourceDevelopment::read_header(&path, wb)
        .map_err(|error| failure(format!("{}: {error}", path.display())))?;
    log::debug!(header_seconds = started.elapsed().as_secs_f64(); "source_registration");
    Ok((source_record(path, &raw.metadata, development)?, raw))
}

#[cfg(test)]
pub(crate) fn developed_record(source: &DevelopedSource) -> Result<Source> {
    source_record(
        source.path().to_owned(),
        &source.metadata(),
        SourceDevelopment::from_developed(source)?,
    )
}

fn source_record(
    path: PathBuf,
    metadata: &Metadata,
    development: SourceDevelopment,
) -> Result<Source> {
    Ok(Source {
        id: SourceId(0),
        path,
        capture_time: metadata.capture_time(),
        development_mode: if development.decode.white_balance == crate::project::UNITY_WB {
            crate::project::DevelopmentMode::Unity
        } else {
            crate::project::DevelopmentMode::AsShot
        },
        development,
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

pub(crate) fn verify_development(source: &Source, developed: &DevelopedSource) -> Result<()> {
    let actual = SourceDevelopment::from_developed(developed)?;
    if !source.development.same_processing(&actual) {
        return Err(failure(
            "developed RAW differs from its recorded header identity",
        ));
    }
    Ok(())
}
