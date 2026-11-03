use super::*;

/// A photograph retains its original intent; standalone mathematical inputs have no recipe.
#[derive(Clone)]
pub(crate) enum Capture {
    Photograph(Arc<crate::image_recipe::ImageRecipe>),
    Primitive(ProcessingInput),
}

impl PartialEq for Capture {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Photograph(a), Self::Photograph(b)) => {
                a.same_photographic(b)
                    && a.look.as_ref().map(|look| look.lut().id())
                        == b.look.as_ref().map(|look| look.lut().id())
            }
            (Self::Primitive(a), Self::Primitive(b)) => a == b,
            _ => false,
        }
    }
}

impl Capture {
    pub(crate) fn look(&self) -> Option<&Arc<crate::looks::LookAsset>> {
        match self {
            Self::Photograph(recipe) => recipe.look.as_ref(),
            Self::Primitive(input) => input.look(),
        }
    }

    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        self.look().map(|look| look.lut().id())
    }

    pub(crate) fn processing(
        &self,
        limits: &wgpu::Limits,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<ProcessingInput, Error> {
        let mut input = match self {
            Self::Photograph(recipe) => ProcessingInput::from_recipe(limits, recipe)?,
            Self::Primitive(input) => input.clone(),
        };
        input.repairs = repairs.clone();
        Ok(input)
    }
}
