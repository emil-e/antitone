//! Captured photographic arguments and source-prefix values used by composition.

use super::*;
use crate::gpu::grading::Grading;
use crate::gpu::interpretation::InputTransform;

#[derive(Clone)]
pub(crate) struct Assembly {
    pub capture: Option<Arc<Captured>>,
    pub grading: Option<Grading>,
    pub interpretation: Interpretation,
    pub repairs: Vec<crate::dust_repair::Repair>,
    pub grain: Option<crate::grain::GrainResponse>,
    pub halation: Option<crate::halation::HalationParameters>,
    pub seed: u64,
    #[cfg(test)]
    pub(crate) author: Option<Arc<super::authoring::Author>>,
    #[cfg(test)]
    pub(crate) preferred_pitch: Option<f64>,
    #[cfg(test)]
    pub replacements: Vec<crate::gpu::repairs::fixtures::Replacement>,
}

pub(crate) struct SourceImages {
    pub codes: Value,
    pub camera: Value,
    pub camera_positive: Value,
    pub unrepaired: Value,
}

impl Default for Assembly {
    fn default() -> Self {
        Self {
            capture: None,
            grading: None,
            interpretation: Interpretation::Unavailable,
            repairs: Vec::new(),
            grain: None,
            halation: None,
            seed: 0,
            #[cfg(test)]
            replacements: Vec::new(),
            #[cfg(test)]
            author: None,
            #[cfg(test)]
            preferred_pitch: None,
        }
    }
}

impl Assembly {
    /// Resolve crop-relative lengths against the accepted photographic composition.
    #[cfg(feature = "project")]
    pub(crate) fn resolve_source_scale(
        &mut self,
        recipe: &crate::image_recipe::ImageRecipe,
    ) -> Result<(), Error> {
        let size = recipe.composition.crop().size;
        self.grain = recipe.grain.parameters(size)?;
        self.halation = recipe.halation.parameters(size)?;
        self.seed = recipe.neutral.source.0;
        Ok(())
    }

    pub fn photographic(
        limits: &wgpu::Limits,
        definition: CameraInput,
        program: InputTransform,
        grading: Grading,
        repairs: Vec<crate::dust_repair::Repair>,
    ) -> Result<Self, Error> {
        let area = grading.area;
        admission::prepare(
            limits,
            program.input,
            grading.roll,
            grading.snapshot(),
            program.matrix,
            area,
        )?;
        let mut assembly =
            Self::captured(Captured::new(definition, Interpretation::Program(program))?);
        assembly.grading = Some(grading);
        assembly.repairs = repairs;
        Ok(assembly)
    }

    pub fn captured(capture: Arc<Captured>) -> Self {
        let interpretation = capture.interpretation;

        Self {
            capture: Some(capture),
            grading: None,
            interpretation,
            repairs: Default::default(),
            ..Self::default()
        }
    }

    #[cfg(feature = "editor")]
    pub fn native(program: InputTransform) -> Self {
        Self {
            capture: None,
            grading: None,
            interpretation: Interpretation::Program(program),
            repairs: Default::default(),
            ..Self::default()
        }
    }

    #[cfg(feature = "editor")]
    pub fn filtered(grading: Grading) -> Self {
        Self {
            capture: None,
            grading: Some(grading),
            interpretation: Interpretation::Unavailable,
            repairs: Default::default(),
            ..Self::default()
        }
    }

    #[cfg(feature = "editor")]
    pub fn input_transform(
        image: &Value,
    ) -> Result<Option<crate::gpu::interpretation::InputTransform>, Error> {
        let mut captured = None;
        for node in image.ancestors() {
            if let Some(native) = node.parameters::<super::operations::NativeIdentity>()
                && let Interpretation::Program(program) = native.0
            {
                if captured.is_some_and(|prior| prior != program) {
                    return Err(Error::InvalidParameter(
                        "calculation mixes source interpretations",
                    ));
                }
                captured = Some(program);
            }
        }
        Ok(captured)
    }
}
