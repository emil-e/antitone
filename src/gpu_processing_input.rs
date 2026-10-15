#[cfg(all(feature = "ui", feature = "render"))]
use super::ProcessingSource;
use super::{GpuPhotoGrade, ProcessingParameters};
#[cfg(all(feature = "ui", feature = "render"))]
use crate::Error;
use std::sync::Arc;

/// Immutable processing input shared by image consumers, never their mutable image storage.
#[derive(Clone)]
pub struct ProcessingInput {
    parameters: ProcessingParameters,
    grade: Option<Arc<GpuPhotoGrade>>,
}
impl From<ProcessingParameters> for ProcessingInput {
    fn from(parameters: ProcessingParameters) -> Self {
        Self {
            parameters,
            grade: None,
        }
    }
}
impl From<Arc<GpuPhotoGrade>> for ProcessingInput {
    fn from(grade: Arc<GpuPhotoGrade>) -> Self {
        Self {
            parameters: grade.parameters,
            grade: Some(grade),
        }
    }
}
impl ProcessingInput {
    pub fn signature(&self) -> ProcessingSignature {
        ProcessingSignature {
            parameters: bytemuck::bytes_of(&self.parameters).try_into().unwrap(),
            grade: self.grade.clone(),
        }
    }
}
#[cfg(all(feature = "ui", feature = "render"))]
impl ProcessingInput {
    pub(crate) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        if let Some(grade) = &self.grade {
            grade.require_source(source)?;
        }
        Ok(())
    }
    pub(crate) fn parameters(&self) -> ProcessingParameters {
        self.parameters
    }
    pub(crate) fn grade(&self) -> Option<&GpuPhotoGrade> {
        self.grade.as_deref()
    }
}

/// Content identity excludes GPU allocation identity and document-only adjustment metadata.
#[derive(Clone)]
pub struct ProcessingSignature {
    parameters: [u8; ProcessingParameters::BYTE_LEN],
    grade: Option<Arc<GpuPhotoGrade>>,
}
impl PartialEq for ProcessingSignature {
    fn eq(&self, other: &Self) -> bool {
        self.parameters == other.parameters
            && match (&self.grade, &other.grade) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a.same_content(b),
                _ => false,
            }
    }
}
impl std::fmt::Debug for ProcessingSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessingSignature")
            .field("parameters", &self.parameters)
            .field("snapshot", &self.grade.as_ref().map(|g| g.snapshot()))
            .finish_non_exhaustive()
    }
}
