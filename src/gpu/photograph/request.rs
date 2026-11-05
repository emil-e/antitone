//! Select a photograph or its source-view branch; prepared arguments stay in graph operations.

use crate::Error;
use crate::gpu::interpretation::InspectionView;
use crate::gpu::parameters::ProcessingParameters;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::image_recipe::ImageRecipe;
use crate::looks::LutChain;
use std::sync::Arc;

static NO_LUTS: LutChain = LutChain {
    lut: None,
    film_and_paper: None,
};

#[derive(Clone, PartialEq)]
pub(crate) enum PhotographRequest {
    Recipe(Arc<ImageRecipe>),
    BeforeLooks(Arc<ImageRecipe>),
    Inspection(InspectionView),
    #[cfg(test)]
    Fixture(AuthoredPhotograph),
}

impl PhotographRequest {
    pub fn inspection(parameters: ProcessingParameters) -> Result<Self, Error> {
        Ok(Self::Inspection(InspectionView::from_parameters(
            parameters,
        )?))
    }

    pub fn recipe(&self) -> Option<&Arc<ImageRecipe>> {
        match self {
            Self::Recipe(recipe) | Self::BeforeLooks(recipe) => Some(recipe),
            _ => None,
        }
    }

    pub fn is_inspection(&self) -> bool {
        match self {
            Self::Recipe(_) | Self::BeforeLooks(_) => false,
            Self::Inspection(_) => true,
            #[cfg(test)]
            Self::Fixture(input) => !input.is_positive(),
        }
    }

    pub fn luts(&self) -> &LutChain {
        match self {
            Self::Recipe(recipe) => &recipe.luts,
            Self::BeforeLooks(_) | Self::Inspection(_) => &NO_LUTS,
            #[cfg(test)]
            Self::Fixture(input) => input.luts(),
        }
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.luts().id()
    }
}
