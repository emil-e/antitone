use super::{FileKey, Input};
use crate::{
    Error, development_metadata, exposure_metadata,
    project::{Project, SourceId},
};
use std::path::PathBuf;

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
    ) -> Result<Input, Error> {
        let record = project.source(id)?.clone();
        let development = self
            .developments
            .request(&record, &file.path, project.input, metadata);
        let reference = self.exposures.memo(id, &file.path);
        Ok(Input::photo(
            path,
            file,
            project.input,
            reference,
            super::input::Photograph {
                record,
                development,
            },
        ))
    }
}
