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
    look: Option<Arc<crate::looks::LookAsset>>,
    #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
    pub(crate) repairs: super::repairs::Snapshot,
}

impl From<ProcessingParameters> for ProcessingInput {
    fn from(parameters: ProcessingParameters) -> Self {
        Self {
            parameters,
            grade: None,
            look: None,
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: super::repairs::Snapshot::default(),
        }
    }
}

impl From<Arc<GpuPhotoGrade>> for ProcessingInput {
    fn from(grade: Arc<GpuPhotoGrade>) -> Self {
        Self {
            parameters: grade.parameters,
            grade: Some(grade),
            look: None,
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: super::repairs::Snapshot::default(),
        }
    }
}

impl ProcessingInput {
    /// Rendering is downstream of the photographic constants; inspection never adopts a look.
    pub fn with_look(mut self, look: Option<Arc<crate::looks::LookAsset>>) -> Self {
        self.look = if self.parameters.gamma_mode.w > 0.5 {
            look
        } else {
            None
        };
        self
    }

    pub fn look(&self) -> Option<&Arc<crate::looks::LookAsset>> {
        self.look.as_ref()
    }

    pub fn signature(&self) -> ProcessingSignature {
        ProcessingSignature {
            parameters: bytemuck::bytes_of(&self.parameters).try_into().unwrap(),
            grade: self.grade.clone(),
            look: self.look.as_ref().map(|look| look.lut().id()),
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: self.repairs.clone(),
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
}

#[cfg(any(
    all(feature = "ui", feature = "render"),
    all(feature = "raw", feature = "export")
))]
impl ProcessingInput {
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
    look: Option<crate::looks::LookId>,
    #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
    repairs: super::repairs::Snapshot,
}

impl ProcessingSignature {
    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        self.look
    }

    /// Look changes rerender, but may reuse an immutable graded AP1 prefix.
    pub(crate) fn same_photographic(&self, other: &Self) -> bool {
        #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
        if self.repairs != other.repairs {
            return false;
        }
        self.parameters == other.parameters
            && match (&self.grade, &other.grade) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a.same_content(b),
                _ => false,
            }
    }
}

impl PartialEq for ProcessingSignature {
    fn eq(&self, other: &Self) -> bool {
        self.look == other.look && self.same_photographic(other)
    }
}

impl std::fmt::Debug for ProcessingSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessingSignature")
            .field("parameters", &self.parameters)
            .field("look", &self.look)
            .field("snapshot", &self.grade.as_ref().map(|g| g.snapshot()))
            .finish_non_exhaustive()
    }
}
