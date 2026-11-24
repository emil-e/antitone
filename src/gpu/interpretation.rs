//! Captured photographic input transforms and source-inspection interpretation.

#[cfg(feature = "editor")]
use crate::gpu::evaluation::{Function, Value, value};
use crate::gpu::{parameters::ProcessingParameters, source::ProcessingSource};
use crate::{Error, processing::InputSettings};
use glam::DMat3;

/// The source viewer's transform over ungraded working light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectionView {
    pub(super) matrix: DMat3,
}

impl InspectionView {
    pub fn new(transform: DMat3, exposure: f64) -> Result<Self, Error> {
        let matrix = transform * crate::processing::exposure_multiplier(exposure)?;
        let parameters = ProcessingParameters::source_with_matrix(matrix)?;
        let packed = DMat3::from_cols(
            parameters.post[0].truncate().as_dvec3(),
            parameters.post[1].truncate().as_dvec3(),
            parameters.post[2].truncate().as_dvec3(),
        );
        if packed.determinant() == 0. {
            return Err(Error::InvalidParameter("singular source-view transform"));
        }
        Ok(Self { matrix })
    }

    pub(crate) fn from_parameters(parameters: ProcessingParameters) -> Result<Self, Error> {
        let matrix = DMat3::from_cols(
            parameters.post[0].truncate().as_dvec3(),
            parameters.post[1].truncate().as_dvec3(),
            parameters.post[2].truncate().as_dvec3(),
        );
        let view = Self::new(matrix, 0.)?;
        if bytemuck::bytes_of(&view.parameters()?) != bytemuck::bytes_of(&parameters) {
            return Err(Error::InvalidParameter(
                "inspection requires a source-view transform",
            ));
        }
        Ok(view)
    }

    pub(crate) fn parameters(self) -> Result<ProcessingParameters, Error> {
        ProcessingParameters::source_with_matrix(self.matrix)
    }
}

#[cfg(feature = "editor")]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct FilteredInspection {
    columns: [[f32; 4]; 3],
    coverage: [u32; 4],
}

#[cfg(feature = "editor")]
impl std::hash::Hash for FilteredInspection {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for value in self.columns.into_iter().flatten() {
            crate::float_hash::f32(value, state);
        }
        self.coverage.hash(state);
    }
}

/// Apply the viewing transform to already-filtered, ungraded working light.
#[cfg(feature = "editor")]
pub(crate) fn inspect_filtered(
    input: Value,
    parameters: ProcessingParameters,
) -> Result<Value, Error> {
    let operation = FilteredInspection {
        columns: parameters.post.map(|column| column.to_array()),
        coverage: [input.domain().grid.coverage, 0, 0, 0],
    };
    value::pointwise(
        input,
        value::Operation::pointwise_function(
            "source inspection v1",
            operation,
            Function::new(
                include_str!("interpretation/inspection.wgsl"),
                "inspect_source",
            )
            .with_uniform(0, &operation),
        ),
    )
}

#[cfg(all(test, feature = "editor"))]
pub(crate) fn has_filtered_inspection(image: &Value) -> bool {
    image
        .ancestors()
        .any(|node| node.parameters::<FilteredInspection>().is_some())
}

/// Source execution captured separately from photographic controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputTransform {
    pub(super) input: InputSettings,
    pub(super) matrix: DMat3,
}

#[cfg(feature = "editor")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SourceInterpretation {
    Photograph(InputTransform),
    Inspection(DMat3),
}

#[cfg(feature = "editor")]
impl SourceInterpretation {
    pub(crate) fn matrix(self) -> DMat3 {
        match self {
            Self::Photograph(program) => program.matrix,
            Self::Inspection(matrix) => matrix,
        }
    }
}

impl InputTransform {
    #[cfg(feature = "project")]
    pub(crate) fn from_recipe(recipe: &crate::image_recipe::NeutralRecipe) -> Result<Self, Error> {
        let (input, matrix) = recipe.processing_transform()?;
        Self::new(input, matrix)
    }

    pub fn new(input: impl Into<InputSettings>, matrix: DMat3) -> Result<Self, Error> {
        let input = input.into();
        crate::processing::validate_camera_matrix(matrix)?;
        if !matches!(input, InputSettings::Digital) {
            input
                .negative_input()
                .ok_or(Error::InvalidCalibration {
                    channel: 0,
                    reason: "measure Dmin and Dmax",
                })?
                .parameters()?;
        }
        Ok(Self { input, matrix })
    }

    pub(crate) fn input(&self) -> InputSettings {
        self.input
    }

    pub(crate) fn matrix(&self) -> DMat3 {
        self.matrix
    }

    #[cfg(all(test, feature = "raw"))]
    pub(crate) fn for_scan(scan: &crate::scan::Scan, input: InputSettings) -> Result<Self, Error> {
        Self::new(input, scan.positive_matrix(&input)?)
    }

    pub fn source_continuous(
        &self,
        device: &wgpu::Device,
        pixels: &[[f32; 4]],
        geometry: Option<crate::gpu::source::SourceGeometry>,
    ) -> Result<ProcessingSource, Error> {
        ProcessingSource::new_continuous(device, pixels, self.matrix, geometry)
    }

    pub(crate) fn neutral_parameters(&self) -> Result<ProcessingParameters, Error> {
        crate::gpu::parameters::input_parameters(self.input, self.matrix)
    }

    #[cfg(feature = "raw")]
    pub(crate) fn matches_native(&self, scan: &crate::scan::Scan) -> Result<bool, Error> {
        Ok(scan.positive_matrix(&self.input)? == self.matrix)
    }

    #[cfg(feature = "raw")]
    pub(crate) fn matches_proxy(&self, proxy: &crate::scan::CameraProxy) -> Result<bool, Error> {
        Ok(proxy.positive_matrix(&self.input)? == self.matrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_content_is_not_the_packed_shader_encoding() {
        let input = crate::processing::InputSettings::Negative {
            dmin: Some(glam::DVec3::splat(0.8)),
            dmax: Some(glam::DVec3::splat(0.02)),
            response: crate::processing::Response::film(),
            gamma: 0.75,
            balance: Default::default(),
        };
        let a = InputTransform::new(input, DMat3::IDENTITY).unwrap();
        let mut changed = input;
        if let InputSettings::Negative { gamma, .. } = &mut changed {
            *gamma = gamma.next_up();
        }
        let b = InputTransform::new(changed, DMat3::IDENTITY).unwrap();
        let packed_a = a.neutral_parameters().unwrap();
        let packed_b = b.neutral_parameters().unwrap();
        assert_eq!(
            bytemuck::bytes_of(&packed_a),
            bytemuck::bytes_of(&packed_b),
            "control must collide after packing"
        );
        assert_ne!(
            a, b,
            "captured input content must distinguish the original parameters"
        );
        assert_eq!(a, InputTransform::new(input, DMat3::IDENTITY).unwrap());
    }
}
