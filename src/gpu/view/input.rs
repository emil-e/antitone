use super::*;
use crate::gpu::input::ProcessingInput;

/// A view retains either photographic intent or directly supplied processing inputs.
#[derive(Clone)]
pub enum ViewInput {
    Recipe(Arc<crate::image_recipe::ImageRecipe>),
    Processing(ProcessingInput),
}

impl PartialEq for ViewInput {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Recipe(a), Self::Recipe(b)) => a.same_photographic(b) && a.luts == b.luts,
            (Self::Processing(a), Self::Processing(b)) => a == b,
            _ => false,
        }
    }
}

impl ViewInput {
    pub fn luts(&self) -> &crate::looks::LutChain {
        match self {
            Self::Recipe(recipe) => &recipe.luts,
            Self::Processing(input) => input.luts(),
        }
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts().id()
    }

    pub fn processing(
        &self,
        limits: &wgpu::Limits,
        repairs: &crate::gpu::repairs::Snapshot,
    ) -> Result<ProcessingInput, Error> {
        let mut input = match self {
            Self::Recipe(recipe) => ProcessingInput::from_recipe(limits, recipe)?,
            Self::Processing(input) => input.clone(),
        };
        input.repairs = repairs.clone();
        Ok(input)
    }
}
