use super::{FileKey, Input};
use crate::{
    Error, development_metadata, exposure_metadata,
    project::{Project, SourceId},
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

/// Recorded source initialization outlives pixel retention and is shared by captured consumers.
#[derive(Default)]
pub(crate) struct Initialization {
    pub exposures: exposure_metadata::Store,
    pub developments: development_metadata::Store,
}

impl Initialization {
    pub fn new(
        exposures: exposure_metadata::Store,
        developments: development_metadata::Store,
    ) -> Self {
        Self {
            exposures,
            developments,
        }
    }

    pub fn input(
        &self,
        project: &Project,
        id: SourceId,
        path: PathBuf,
        file: FileKey,
        metadata: &development_metadata::Metadata,
        basis: Arc<OnceLock<crate::project::ScanBasis>>,
    ) -> Result<Input, Error> {
        let record = project.source(id)?.clone();
        let development = self
            .developments
            .request(&record, &file.path, project.input, metadata);
        let reference = self.exposures.memo(id, &file.path);
        if let Some(scan) = &project.scan {
            let _ = basis.set(scan.basis.clone());
        }
        let expected_camera = project
            .scan
            .as_ref()
            .map(|scan| scan.basis.camera.camera_model.clone())
            .or_else(|| {
                project
                    .order
                    .first()
                    .and_then(|id| project.source(*id).ok())
                    .map(|source| source.development.camera_model.clone())
            });
        Ok(Input::photo(
            path,
            file,
            project.input,
            reference,
            super::input::Photograph {
                record,
                development,
                basis,
                expected_camera,
            },
        ))
    }
}
