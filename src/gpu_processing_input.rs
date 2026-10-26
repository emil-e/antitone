#[cfg(all(feature = "ui", feature = "render"))]
use super::ProcessingSource;
use super::{GpuAdjustmentLayers, ProcessingParameters};
#[cfg(all(feature = "ui", feature = "render"))]
use crate::Error;
use std::sync::Arc;

/// Immutable processing input shared by image consumers, never their mutable image storage.
///
/// A complete grade is always a [`GpuAdjustmentLayers`], which owns the semantic
/// [`crate::adjustment_layers::AdjustmentLayerSnapshot`] it was compiled from. Bare
/// [`ProcessingParameters`] are reserved for source inspection and low-level GPU fixtures;
/// they are not an alternative grade representation.
#[derive(Clone)]
pub struct ProcessingInput {
    processing: Processing,
    look: Option<Arc<crate::looks::LookAsset>>,
    #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
    pub(crate) repairs: super::repairs::Snapshot,
}

#[derive(Clone)]
enum Processing {
    Parameters(Arc<ProcessingParameters>),
    Grade(Arc<GpuAdjustmentLayers>),
}

impl From<ProcessingParameters> for ProcessingInput {
    fn from(parameters: ProcessingParameters) -> Self {
        Self {
            processing: Processing::Parameters(Arc::new(parameters)),
            look: None,
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: super::repairs::Snapshot::default(),
        }
    }
}

impl From<Arc<GpuAdjustmentLayers>> for ProcessingInput {
    fn from(grade: Arc<GpuAdjustmentLayers>) -> Self {
        Self {
            processing: Processing::Grade(grade),
            look: None,
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: super::repairs::Snapshot::default(),
        }
    }
}

impl ProcessingInput {
    /// Rendering is downstream of the photographic constants; source inspection never adopts a look.
    pub fn with_look(mut self, look: Option<Arc<crate::looks::LookAsset>>) -> Self {
        self.look = if self.parameters().graded() {
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
            parameters: bytemuck::bytes_of(&self.parameters()).try_into().unwrap(),
            grade: match &self.processing {
                Processing::Parameters(_) => None,
                Processing::Grade(grade) => Some(grade.clone()),
            },
            look: self.look.as_ref().map(|look| look.lut().id()),
            #[cfg(any(feature = "editor", all(feature = "raw", feature = "export")))]
            repairs: self.repairs.clone(),
        }
    }
}

#[cfg(all(feature = "ui", feature = "render"))]
impl ProcessingInput {
    pub(crate) fn require_source(&self, source: &ProcessingSource) -> Result<(), Error> {
        if let Processing::Grade(grade) = &self.processing {
            grade.require_source(source)?;
        }
        Ok(())
    }
}

impl ProcessingInput {
    #[cfg(feature = "editor")]
    pub(crate) fn is_positive(&self) -> bool {
        self.parameters().graded()
    }

    pub(crate) fn parameters(&self) -> ProcessingParameters {
        match &self.processing {
            Processing::Parameters(parameters) => **parameters,
            Processing::Grade(grade) => grade.parameters,
        }
    }

    pub(crate) fn grade(&self) -> Option<&GpuAdjustmentLayers> {
        match &self.processing {
            Processing::Parameters(_) => None,
            Processing::Grade(grade) => Some(grade),
        }
    }
}

/// Content identity excludes GPU allocation identity and document-only adjustment metadata.
#[derive(Clone)]
pub struct ProcessingSignature {
    parameters: [u8; ProcessingParameters::BYTE_LEN],
    grade: Option<Arc<GpuAdjustmentLayers>>,
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
